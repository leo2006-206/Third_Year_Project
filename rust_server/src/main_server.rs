use std::sync::Arc;

use smol::{
    // future::zip,
    io,
    net::{TcpListener, TcpStream},
    prelude::*,
};

use rust_obm::mod_feature::{Follower, Leader};
use rust_obm::mod_load_balancer as lb;
use rust_obm::mod_load_balancer::base_balancer as blb;
use rust_server::lib_http as http;
use rust_server::lib_main_state as main_state;
use rust_server::lib_main_state::ServerState;
use rust_server::lib_segment::{
    LookupMode, OffloadError, SegmentLookup, fetch_offload, offload_cache_add, prefetch_task,
    segment_lookup,
};
use rust_server::lib_server_main as sm;
use rust_server::lib_util as util;

async fn handle_client<P: blb::LoadBalancerPolicy + 'static>(
    mut client_stream: TcpStream,
    state: Arc<ServerState<P>>,
) -> io::Result<()> {
    let mut buffer = vec![0u8; 4096];

    let Ok(req_str) = util::read_as_str(&mut client_stream, &mut buffer).await else {
        eprintln!("Failed to read HTTP request with err");
        return Ok(());
    };

    let Some((_method, mut path)) = util::parse_method_path(&req_str, Some("?")) else {
        eprintln!("Failed to parse HTTP request");
        return Ok(());
    };

    if path.starts_with("/assets") || path.starts_with("/shows") {
        serve_resource(client_stream, path).await
    } else if path.starts_with("/offload") || path.starts_with("/times/offload") {
        if path.starts_with("/times") {
            path = path.strip_prefix("/times").expect("Must start with /times");
        }
        serve_offload(client_stream, &state, path).await
    } else {
        serve_web_page(client_stream, path).await
    }
}

async fn serve_resource(mut client_stream: TcpStream, resource_path: &str) -> io::Result<()> {
    use http::{response_404, send_file, send_raw};
    use util::{file_check, match_content_type};

    // docker path here, for docker only
    const BASE_PATH: &str = "/app/obm";

    let Some(file) = file_check(BASE_PATH, resource_path) else {
        let resp = response_404();
        return send_raw(&mut client_stream, &resp).await;
    };

    const RESOURCE_HEADERS: [(&str, &str); 2] = [
        ("Cross-Origin-Resource-Policy", "cross-origin"),
        ("Cache-Control", "no-cache, no-store, must-revalidate"),
    ];

    let content_type = match_content_type(resource_path);
    send_file(&mut client_stream, file, content_type, &RESOURCE_HEADERS).await
}

/// Offload request, in order:
/// 0. prefetch on: the request feeds the "2 continue" rule, which may start a prefetch of the
///    next segment (detached, see prefetch_task);
/// 1. segment cache hit: sent from the cache (the load balancer never sees it);
/// 2. the same path is in the in-flight table (a real leader with coalescing on, or a prefetch):
///    wait for that render (follower);
/// 3. else render on a site chosen by the load balancer (the leader, when coalescing is on):
///    the followers and the cache get the segment first, then this client.
///
/// A follower whose leader was a prefetch that failed goes back to 1. (the lookup again): with
/// coalescing on, the followers of a failed prefetch then share one new render (the first one
/// back leads, the others follow it) instead of rendering one each. Ends: a new failed prefetch
/// of the same path needs a new trigger + render, so this loops at most once per such prefetch.
async fn serve_offload<P: blb::LoadBalancerPolicy + 'static>(
    mut client_stream: TcpStream,
    state: &Arc<ServerState<P>>,
    path: &str,
) -> io::Result<()> {
    if let Some(prefetch) = &state.prefetch
        && let Some(target) = prefetch.observe(path, prefetch.now_s())
    {
        smol::spawn(prefetch_task(Arc::clone(state), path.to_string(), target)).detach();
    }
    loop {
        return match segment_lookup(state, path, LookupMode::Real) {
            SegmentLookup::Hit(file) => serve_offload_cache(client_stream, path, file).await,
            SegmentLookup::Follow(follower) => {
                match serve_offload_coalesce(client_stream, path, follower).await? {
                    Followed::Served => Ok(()),
                    Followed::PrefetchFailed(stream) => {
                        client_stream = stream;
                        continue;
                    }
                }
            }
            SegmentLookup::Lead(leader) => {
                render_and_send(client_stream, state, path, leader).await
            }
            // never for a real request; rendering is the safe answer
            SegmentLookup::Skip(_) => render_and_send(client_stream, state, path, None).await,
        };
    }
}

/// Renders `path` on a site and sends it to the client; with a leader, the followers and the
/// cache get the segment before this client's transfer.
async fn render_and_send<P: blb::LoadBalancerPolicy + 'static>(
    mut client_stream: TcpStream,
    state: &Arc<ServerState<P>>,
    path: &str,
    leader: Option<Leader<'_, Arc<Vec<u8>>>>,
) -> io::Result<()> {
    let segment = match fetch_offload(state, path).await? {
        Ok(segment) => segment,
        Err(OffloadError { site, err }) => {
            // the leader is dropped when this returns: its followers answer 502
            return sm::reply_gateway_error(&mut client_stream, site, path, err).await;
        }
    };

    // Followers + cache first, so they don't wait for this client's transfer
    offload_cache_add(state, path, leader, &segment).await;

    client_stream.write_all(&segment.head).await?;
    client_stream.write_all(&segment.data).await?;
    client_stream.flush().await?;

    let message = format!(
        "[OFFLOAD] -> {} | {}ms | {}",
        segment.site, segment.total_ms, path
    );
    util::log_message(&message);
    Ok(())
}

/// Sends a cached segment. The timing headers are present with zero values (so the benchmark
/// parsers keep working) and the worker is "cache".
async fn serve_offload_cache(
    mut client_stream: TcpStream,
    path: &str,
    file: std::fs::File,
) -> io::Result<()> {
    // Dana's Content-type for segments (OffloadSite_new.dn sendDataChunks)
    const SEGMENT_CONTENT_TYPE: &str = "h264";
    const HIT_HEADERS: [(&str, &str); 8] = [
        ("X-OBM-Cache", "hit"),
        ("X-OBM-Worker", "cache"),
        (
            "X-Dana-Timings",
            "queue=0;asset=0;work=0;decode=0;render=0;encode=0",
        ),
        ("X-OBM-Net", "0"),
        ("X-OBM-Wait", "0"),
        ("Server-Timing", "net;dur=0, wait;dur=0"),
        ("Access-Control-Allow-Origin", "*"),
        (
            "Access-Control-Expose-Headers",
            "X-Dana-Timings, Server-Timing, X-OBM-Net, X-OBM-Wait, X-OBM-Worker, X-OBM-Cache",
        ),
    ];
    let t_start = std::time::Instant::now();
    http::send_file(&mut client_stream, file, SEGMENT_CONTENT_TYPE, &HIT_HEADERS).await?;

    let ms = t_start.elapsed().as_millis();
    util::log_message(&format!("[OFFLOAD] -> cache | {ms}ms | {path}"));
    Ok(())
}

/// What became of a follower
enum Followed {
    /// answered: the leader's segment, or a 502 after a failed real leader
    Served,
    /// the leader was a prefetch and failed: nothing sent yet, the client goes back to the lookup
    PrefetchFailed(TcpStream),
}

/// Follower: waits for the leader rendering the same path, then sends its segment (zero timing
/// headers like a cache hit, X-OBM-Coalesce = ms waited; worker "coalesce" for a real leader,
/// "prefetch" for a prefetch). A real leader that ended without a 200 segment (gateway error,
/// other status, panic) makes this a 502: no fallback render. A failed prefetch is nobody's
/// request: the client is handed back (PrefetchFailed) for a fresh lookup in serve_offload.
/// Logs '[COALESCE] hit' / '[COALESCE] leader failed' (real leader) or '[PREFETCH] hit |
/// in flight / held' / '[PREFETCH] leader failed, lookup again' (prefetch).
async fn serve_offload_coalesce(
    mut client_stream: TcpStream,
    path: &str,
    follower: Follower<Arc<Vec<u8>>>,
) -> io::Result<Followed> {
    let t_start = std::time::Instant::now();
    let speculative = follower.speculative();
    let stage = if follower.ready() {
        "held"
    } else {
        "in flight"
    };
    let Some(data) = follower.wait().await else {
        let ms = t_start.elapsed().as_millis();
        if speculative {
            util::log_message(&format!(
                "[PREFETCH] leader failed, lookup again | {ms}ms | {path}"
            ));
            return Ok(Followed::PrefetchFailed(client_stream));
        }
        util::log_message(&format!("[COALESCE] leader failed | {ms}ms | {path}"));
        let resp = http::response_gateway_error(false);
        http::send_raw(&mut client_stream, &resp).await?;
        return Ok(Followed::Served);
    };
    let wait_ms = t_start.elapsed().as_millis();

    let head = sm::coalesce_response_head(data.len() as u64, wait_ms, speculative);
    client_stream.write_all(&head).await?;
    client_stream.write_all(&data).await?;
    client_stream.flush().await?;

    let ms = t_start.elapsed().as_millis();
    let message = if speculative {
        format!("[PREFETCH] hit | {wait_ms}ms wait | {stage} | {ms}ms | {path}")
    } else {
        format!("[COALESCE] hit | {wait_ms}ms wait | {ms}ms | {path}")
    };
    util::log_message(&message);
    Ok(Followed::Served)
}

async fn serve_web_page(mut client_stream: TcpStream, path: &str) -> io::Result<()> {
    use http::{response_404, response_ok_utf8, send_raw};

    const INDEX_HTML: &str = include_str!("./testing_webpage/index.html");
    const APP_JS: &str = include_str!("./testing_webpage/app.js");
    const STYLE_CSS: &str = include_str!("./testing_webpage/style.css");
    const SHOW_OPTIONS: &str = include_str!("../../servers_benchmark/show_options.json");

    const HEADERS: [(&str, &str); 2] = [
        ("Cache-Control", "no-cache, no-store, must-revalidate"),
        ("Cross-Origin-Resource-Policy", "cross-origin"),
    ];

    let response =
        if path == "/" || path == "/client_testing" || path == "/client_testing/index.html" {
            response_ok_utf8("text/html", &HEADERS, INDEX_HTML.as_bytes())
        } else if path == "/app.js" {
            response_ok_utf8("application/javascript", &HEADERS, APP_JS.as_bytes())
        } else if path == "/style.css" {
            response_ok_utf8("text/css", &HEADERS, STYLE_CSS.as_bytes())
        } else if path == "/show_options.json" {
            response_ok_utf8("application/json", &HEADERS, SHOW_OPTIONS.as_bytes())
        } else {
            response_404()
        };

    send_raw(&mut client_stream, &response).await
}

fn main() -> io::Result<()> {
    // Usage: main_server <cluster_csv> [policy], e.g. servers_container/cluster_endpoint_c1.csv iwrr
    // policy (default rr): rr | iwrr | esce | mct | mct_lookup | mct_const | mct_const_flat | aco | aco_const |
    // throttled | throttled2
    // (mct / aco read the cost model, see load_cost_model; the _const ones use main_state::CONST_WORK_S)
    let mut args = std::env::args().skip(1);
    let Some(csv_path) = args.next() else {
        eprintln!(
            "Usage: main_server <cluster_csv> [{}]",
            lb::POLICIES.join("|")
        );
        std::process::exit(2);
    };
    let policy = args.next().unwrap_or_else(|| "rr".to_string());

    let offload_url = main_state::load_cluster_csv(&csv_path)?;
    for site in &offload_url {
        println!("Offload site: {} ({})", site.endpoint, site.name);
    }

    match policy.as_str() {
        "rr" => serve(lb::RoundRobinLB::new(offload_url)),
        "iwrr" => serve(lb::InterleavedWeightedRRLB::new(offload_url)),
        "esce" => serve(lb::EsceLB::new(offload_url)),
        "throttled" => serve(lb::ThrottledLB::new(offload_url)),
        // at most 2 requests per site: the next job is already at the site when one ends (no idle gap)
        "throttled2" => serve(lb::Throttled2LB::new(offload_url)),
        "mct" => serve(or_exit(main_state::mct_balancer(offload_url))),
        "mct_lookup" => serve(or_exit(main_state::mct_lookup_balancer(offload_url))),
        "mct_const" => serve(main_state::mct_const_balancer(offload_url)),
        "mct_const_flat" => serve(main_state::mct_const_flat_balancer(offload_url)),
        "aco" => serve(or_exit(main_state::aco_balancer(offload_url))),
        "aco_const" => serve(main_state::aco_const_balancer(offload_url)),
        other => {
            eprintln!(
                "Unknown policy '{other}' (expected {})",
                lb::POLICIES.join(" | ")
            );
            std::process::exit(2);
        }
    }
}

/// A policy that can't load its data (cost model, lookup table) is a usage error: say why, exit 2
fn or_exit<T>(result: io::Result<T>) -> T {
    result.unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    })
}

fn serve<P: blb::LoadBalancerPolicy + 'static>(
    load_balancer: blb::LoadBalancer<P>,
) -> io::Result<()> {
    let state = Arc::new(ServerState::new(load_balancer)?);

    smol::block_on(async {
        // Bind the server to a local port
        let port_addr = "0.0.0.0:7000";
        let listener = TcpListener::bind(port_addr).await?;
        println!("TCP Server listening on {port_addr}");

        // Accept incoming connections loop
        let mut incoming = listener.incoming();
        while let Some(stream) = incoming.next().await {
            let stream = stream?;
            let state = Arc::clone(&state);
            // Spawn an asynchronous task for each client connection
            smol::spawn(async move {
                if let Err(e) = handle_client(stream, state).await {
                    eprintln!("Error handling client: {}", e);
                }
            })
            .detach(); // Detach allows the task to run independently in the background
        }
        Ok(())
    })
}
