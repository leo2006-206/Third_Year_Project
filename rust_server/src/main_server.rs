use std::sync::Arc;

use smol::{
    // future::zip,
    io,
    net::{TcpListener, TcpStream},
    prelude::*,
};

use rust_obm::mod_load_balancer as lb;
use rust_obm::mod_load_balancer::base_balancer as blb;
use rust_server::lib_http as http;
use rust_server::lib_server_main as sm;
use rust_server::lib_util as util;

async fn handle_client<P: blb::LoadBalancerPolicy>(
    mut client_stream: TcpStream,
    balancer: Arc<blb::LoadBalancer<P>>,
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
        serve_offload(client_stream, balancer, path).await
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

async fn serve_offload<P: blb::LoadBalancerPolicy>(
    mut client_stream: TcpStream,
    balancer: Arc<blb::LoadBalancer<P>>,
    path: &str,
) -> io::Result<()> {
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

    let t_start = Instant::now();

    let target_offload = balancer.select(Some(path));

    // On an early return the selection guard is dropped, which reports the site as done
    let connect = with_timeout(CONNECT_TIMEOUT, TcpStream::connect(target_offload.endpoint)).await;
    let mut offload_steam = match connect {
        Ok(stream) => stream,
        Err(e) => {
            return sm::reply_gateway_error(&mut client_stream, target_offload.name, path, e).await;
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
            return sm::reply_gateway_error(&mut client_stream, target_offload.name, path, e).await;
        }
    };
    let (new_head, times) = sm::time_append_header(&head, t_start, target_offload.endpoint);

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
        return sm::reply_gateway_error(&mut client_stream, target_offload.name, path, e).await;
    }

    // The site's part is over: report it (with timings) before talking to the client
    let total_ms = times.t_net + times.t_queue + times.t_asset + times.t_work;
    let site_name = target_offload.name;
    target_offload.finish(Some(&times));

    // 3. Forward head + body to the client
    client_stream.write_all(&new_head).await?;
    client_stream.write_all(&data).await?;
    client_stream.flush().await?;

    let message = format!("[OFFLOAD] -> {} | {}ms | {}", site_name, total_ms, path);
    util::log_message(&message);

    Ok(())
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
    // policy (default rr): rr | iwrr | esce | mct (mct reads the cost model, see MCT::from_env)
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
        "mct" => match lb::policy_mct::MCT::from_env(&offload_url) {
            Ok(mct) => serve(lb::MctLB::with_policy(offload_url, mct)),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2);
            }
        },
        other => {
            eprintln!(
                "Unknown policy '{other}' (expected {})",
                lb::POLICIES.join(" | ")
            );
            std::process::exit(2);
        }
    }
}

fn serve<P: blb::LoadBalancerPolicy + 'static>(
    load_balancer: blb::LoadBalancer<P>,
) -> io::Result<()> {
    println!(
        "Load balancer type = {}",
        std::any::type_name_of_val(&load_balancer)
    );

    let balancer = Arc::new(load_balancer);

    smol::block_on(async {
        // Bind the server to a local port
        let port_addr = "0.0.0.0:7000";
        let listener = TcpListener::bind(port_addr).await?;
        println!("TCP Server listening on {port_addr}");

        // Accept incoming connections loop
        let mut incoming = listener.incoming();
        while let Some(stream) = incoming.next().await {
            let stream = stream?;
            let lb_clone = Arc::clone(&balancer);
            // Spawn an asynchronous task for each client connection
            smol::spawn(async move {
                if let Err(e) = handle_client(stream, lb_clone).await {
                    eprintln!("Error handling client: {}", e);
                }
            })
            .detach(); // Detach allows the task to run independently in the background
        }
        Ok(())
    })
}
