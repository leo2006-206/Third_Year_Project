use std::sync::Arc;

use smol::{
    // future::zip,
    io,
    net::{TcpListener, TcpStream},
    prelude::*,
};

use rust_obm::mod_feature::BinCache;
use rust_obm::mod_load_balancer as lb;
use rust_obm::mod_load_balancer::base_balancer as blb;
use rust_server::lib_http as http;
use rust_server::lib_server_main as sm;
use rust_server::lib_server_main::ServerState;
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

/// Offload request: answered from the segment cache on a hit (the load balancer never sees it),
/// else rendered by a site chosen by the load balancer.
async fn serve_offload<P: blb::LoadBalancerPolicy + 'static>(
    client_stream: TcpStream,
    state: &Arc<ServerState<P>>,
    path: &str,
) -> io::Result<()> {
    if let Some(cache) = &state.segment_cache
        && let Some((file, _size)) = cache.get(path)
    {
        return serve_offload_cache(client_stream, path, file).await;
    }
    if let Some(segment) = serve_offload_request(client_stream, state, path).await? {
        offload_cache_add(state, path, segment);
    }
    Ok(())
}

/// A segment a site rendered and the client received, with what the cache needs to store it.
struct RenderedSegment {
    /// the video bytes (the response body)
    data: Vec<u8>,
    /// status 200: only those are cached, an error body must never be replayed
    status_ok: bool,
    /// the site's timings and render config, for the bin cache's cost
    times: lb::TimesFeedback,
    config: lb::config::Config,
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

/// Renders the request on a site chosen by the load balancer and forwards the answer to the
/// client. Returns the segment once the client has it; None after a gateway error (the client
/// got a 502 / 504 instead).
async fn serve_offload_request<P: blb::LoadBalancerPolicy + 'static>(
    mut client_stream: TcpStream,
    state: &Arc<ServerState<P>>,
    path: &str,
) -> io::Result<Option<RenderedSegment>> {
    use http::{request_get, send_raw};
    use std::time::{Duration, Instant};
    use util::{copy_with_idle_timeout, with_timeout};
    const MAIN_SERVER_AGENT: &str = "OBM Load balancer/1.0";
    // A live site accepts within ~30-60 ms; longer means it is down or the packets are dropped
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
    // Until the response headers: includes the site's queue + asset + render time
    const HEADER_TIMEOUT: Duration = Duration::from_secs(600);
    // Body streaming: limit on a gap with no data, not on the whole transfer
    const BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

    // A policy may hold the request until a site is free (throttled): that wait is reported
    // on its own (X-OBM-Wait), and the network time starts after it
    let t_wait_start = Instant::now();
    let target_offload = state.balancer.select_wait(Some(path)).await;
    let t_wait = t_wait_start.elapsed().as_millis() as i64;
    let t_start = Instant::now();

    // On an early return the selection guard is dropped, which reports the site as done
    let connect = with_timeout(CONNECT_TIMEOUT, TcpStream::connect(target_offload.endpoint)).await;
    let mut offload_steam = match connect {
        Ok(stream) => stream,
        Err(e) => {
            sm::reply_gateway_error(&mut client_stream, target_offload.name, path, e).await?;
            return Ok(None);
        }
    };

    let message = request_get(path, target_offload.endpoint, MAIN_SERVER_AGENT, &[]);
    send_raw(&mut offload_steam, &message).await?;

    // 1. Response head (queue + asset + render happen before it); the body keeps arriving
    //    in the socket meanwhile. A missing / oversized head is a gateway error like a timeout
    let split = with_timeout(HEADER_TIMEOUT, http::split_header_body(&mut offload_steam)).await;
    let (head, mut body) = match split {
        Ok(parts) => parts,
        Err(e) => {
            sm::reply_gateway_error(&mut client_stream, target_offload.name, path, e).await?;
            return Ok(None);
        }
    };
    let (new_head, times) = sm::time_append_header(&head, t_start, t_wait, target_offload.endpoint);

    // 2. Whole body into a buffer (the cache will store it from here) and check its length
    //    against Content-Length; nothing has been sent to the client yet, so a short or
    //    stalled body is still a clean gateway error
    let expected_len = http::parse_content_length(&head);
    // capacity capped so a bogus Content-Length can't make a huge allocation up front
    let capacity = expected_len.unwrap_or(0).min(64 << 20) as usize;
    let mut data = Vec::with_capacity(capacity);
    let received = copy_with_idle_timeout(&mut body, &mut data, BODY_IDLE_TIMEOUT).await;
    let checked = received.and_then(|n| match expected_len {
        Some(len) if len != n => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("body {n} B, Content-Length {len} B"),
        )),
        _ => Ok(()),
    });
    if let Err(e) = checked {
        sm::reply_gateway_error(&mut client_stream, target_offload.name, path, e).await?;
        return Ok(None);
    }

    // The site's part is over: report it (with timings) before talking to the client
    let total_ms = t_wait + times.t_net + times.t_queue + times.t_asset + times.t_work;
    let site_name = target_offload.name;
    let config = target_offload.config;
    target_offload.finish(Some(&times));

    // 3. Forward head + body to the client
    client_stream.write_all(&new_head).await?;
    client_stream.write_all(&data).await?;
    client_stream.flush().await?;

    let message = format!("[OFFLOAD] -> {} | {}ms | {}", site_name, total_ms, path);
    util::log_message(&message);

    Ok(Some(RenderedSegment {
        data,
        status_ok: sm::is_status_ok(&head),
        times,
        config,
    }))
}

/// Stores a rendered segment in the segment cache (plain LRU or bins), if one is on. The file
/// write runs on a blocking thread so it never stalls the async executor; every outcome is
/// logged ('[CACHE] added | add failed | skip').
fn offload_cache_add<P: blb::LoadBalancerPolicy + 'static>(
    state: &Arc<ServerState<P>>,
    path: &str,
    segment: RenderedSegment,
) {
    if state.segment_cache.is_none() {
        return;
    }
    if !segment.status_ok {
        util::log_message(&format!("[CACHE] skip: status not 200 | {path}"));
        return;
    }
    let key = path.to_string();
    // the blocking thread needs its own handle on the state ('static)
    let state = Arc::clone(state);
    smol::unblock(move || {
        let cache = state.segment_cache.as_ref().expect("checked is_some above");
        let RenderedSegment {
            data,
            times,
            config,
            ..
        } = segment;
        let size = data.len();
        // bins: which side of the threshold this segment fell
        let cost = match cache {
            sm::SegmentCache::Bin { .. } => {
                format!(" cost {:.2} s", BinCache::cost_s(&times, &config))
            }
            sm::SegmentCache::Lru { .. } => String::new(),
        };
        let outcome = match cache.add(&key, &data, &times, &config) {
            Ok(bin) => {
                let to_bin = bin.map(|b| format!(" to {b}")).unwrap_or_default();
                format!("added {size} B{to_bin} (used {} B){cost}", cache.used())
            }
            Err(e) => {
                let reason = match e.kind() {
                    // a concurrent miss on the same path stored it first; the bins say where
                    // ("in upper bin" / "in other bin, upper"), the plain LRU has no message
                    io::ErrorKind::AlreadyExists => match e.get_ref() {
                        Some(where_) => format!("already cached ({where_})"),
                        None => "already cached".to_string(),
                    },
                    io::ErrorKind::InvalidInput => "bigger than the cache".to_string(),
                    _ => e.to_string(),
                };
                format!("add failed ({size} B){cost}: {reason}")
            }
        };
        util::log_message(&format!("[CACHE] {outcome} | {key}"));
    })
    .detach();
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
    // (mct / aco read the cost model, see load_cost_model; the _const ones use sm::CONST_WORK_S)
    let mut args = std::env::args().skip(1);
    let Some(csv_path) = args.next() else {
        eprintln!(
            "Usage: main_server <cluster_csv> [{}]",
            lb::POLICIES.join("|")
        );
        std::process::exit(2);
    };
    let policy = args.next().unwrap_or_else(|| "rr".to_string());

    let offload_url = sm::load_cluster_csv(&csv_path)?;
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
        "mct" => serve(or_exit(sm::mct_balancer(offload_url))),
        "mct_lookup" => serve(or_exit(sm::mct_lookup_balancer(offload_url))),
        "mct_const" => serve(sm::mct_const_balancer(offload_url)),
        "mct_const_flat" => serve(sm::mct_const_flat_balancer(offload_url)),
        "aco" => serve(or_exit(sm::aco_balancer(offload_url))),
        "aco_const" => serve(sm::aco_const_balancer(offload_url)),
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
