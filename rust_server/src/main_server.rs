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
    use std::time::Instant;
    const MAIN_SERVER_AGENT: &str = "OBM Load balancer/1.0";

    let t_start = Instant::now();

    let target_offload = balancer.select(Some(path));

    let mut offload_steam = TcpStream::connect(target_offload.endpoint).await?;

    let message = request_get(path, target_offload.endpoint, MAIN_SERVER_AGENT, &[]);
    send_raw(&mut offload_steam, &message).await?;

    // 1. Read until the boundary between HTTP headers and binary video payload
    //    (the offload site may send its headers over several TCP reads)
    let mut buf = vec![0u8; 8192];
    let mut n = util::buf_read(&mut offload_steam, &mut buf).await?;
    let elapsed = t_start.elapsed().as_millis() as i64;
    let mut header_end = buf[..n].windows(4).position(|w| w == b"\r\n\r\n");
    while header_end.is_none() && n < buf.len() {
        let Ok(m) = util::buf_read(&mut offload_steam, &mut buf[n..]).await else {
            break;
        };
        n += m;
        header_end = buf[..n].windows(4).position(|w| w == b"\r\n\r\n");
    }

    let Some(pos) = header_end else {
        // If no HTTP delimiter found, forward raw buffer directly and exit
        client_stream.write_all(&buf[..n]).await?;
        smol::io::copy(&mut offload_steam, &mut client_stream).await?;
        client_stream.flush().await?;
        return Ok(());
    };

    // 2. Everything below runs without any enclosing { }
    let header_str = String::from_utf8_lossy(&buf[..pos]);
    let t_net = sm::compute_t_net(&header_str, elapsed).unwrap_or(0);

    // 3. Inject timing headers and delimiter
    let t_net_str = t_net.to_string();
    let server_timing_str = format!("net;dur={t_net}");
    let extra_headers = [
        ("X-OBM-Net", t_net_str.as_str()),
        ("Server-Timing", server_timing_str.as_str()),
        ("X-OBM-Worker", target_offload.endpoint),
        (
            "Access-Control-Expose-Headers",
            "X-Dana-Timings, Server-Timing, X-OBM-Net, X-OBM-Worker",
        ),
    ];
    let modified_headers = http::response_append(&header_str, &extra_headers);
    client_stream.write_all(&modified_headers).await?;

    // 4. Forward trailing binary payload slice
    let body_chunk = &buf[pos + 4..n];
    if !body_chunk.is_empty() {
        client_stream.write_all(body_chunk).await?;
    }

    // 5. Stream the rest of the video chunks directly
    smol::io::copy(&mut offload_steam, &mut client_stream).await?;
    client_stream.flush().await?;

    let message = format!(
        "[OFFLOAD] -> {} | {}ms | {}",
        target_offload.name, elapsed, path
    );
    util::log_message(&message);

    target_offload.finish(None);

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
    // Usage: main_server <cluster_csv>, e.g. servers_container/cluster_endpoint_c1.csv
    let Some(csv_path) = std::env::args().nth(1) else {
        eprintln!("Usage: main_server <cluster_csv>");
        std::process::exit(2);
    };
    let offload_url = sm::load_cluster_csv(&csv_path)?;
    for site in &offload_url {
        println!("Offload site: {} ({})", site.endpoint, site.name);
    }

    let load_balancer = lb::RoundRobinLB::new(offload_url);
    // let load_balancer = lb::InterleavedWeightedRRLB::new(offload_url);

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
