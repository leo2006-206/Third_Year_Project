use smol::{
    // future::zip,
    io,
    net::{TcpListener, TcpStream},
    prelude::*,
};

use servers_rust::lib_http as http;
use servers_rust::lib_util as util;

async fn handle_client(mut client_stream: TcpStream, offload_url: &[&str]) -> io::Result<()> {
    let mut buffer = vec![0u8; 4096];

    let Ok(req_str) = util::read_as_str(&mut client_stream, &mut buffer).await else {
        eprintln!("Failed to read HTTP request with err");
        return Ok(());
    };

    let Some((method, path)) = util::parse_method_path(&req_str) else {
        eprintln!("Failed to parse HTTP request");
        return Ok(());
    };

    dbg!(&method, &path);

    if path.starts_with("/assets") {
        todo!()
    } else if path.starts_with("/offload") {
        serve_offload(&mut client_stream, offload_url, path).await
    } else {
        serve_web_page(&mut client_stream, path).await
    }
}

async fn serve_resource(_client_stream: &mut TcpStream, _resource_path: &str) -> io::Result<()> {
    Ok(())
}

async fn serve_offload(
    _client_stream: &mut TcpStream,
    offload_url: &[&str],
    path: &str,
) -> io::Result<()> {
    use http::{request_get, send_raw};

    let mut offload_steam = TcpStream::connect(offload_url[0]).await?;

    println!("Forwarding req = {path}");

    let req = request_get(path, offload_url[0], "Load balancer/1.0", &[]);
    send_raw(&mut offload_steam, &req).await
}

async fn serve_web_page(client_stream: &mut TcpStream, path: &str) -> io::Result<()> {
    use http::{response_404, response_bytes, response_ok_utf8, send_raw};

    const INDEX_HTML: &str = include_str!("../testing_webpage/index.html");
    const APP_JS: &str = include_str!("../testing_webpage/app.js");
    const STYLE_CSS: &str = include_str!("../testing_webpage/style.css");
    const SHOW_OPTIONS: &str = include_str!("../testing_webpage/show_options.json");

    const DANA_JS: &str = include_str!("../../../obm/dana.js");
    const FILE_SYSTEM_JS: &str = include_str!("../../../obm/file_system.js");
    const DANA_WASM: &[u8] = include_bytes!("../../../obm/dana.wasm");

    let response =
        if path == "/" || path == "/client_testing" || path == "/client_testing/index.html" {
            response_ok_utf8("text/html", &[], INDEX_HTML.as_bytes())
        } else if path == "/app.js" {
            response_ok_utf8("application/javascript", &[], APP_JS.as_bytes())
        } else if path == "/style.css" {
            response_ok_utf8("text/css", &[], STYLE_CSS.as_bytes())
        } else if path == "/show_options.json" {
            response_ok_utf8("application/json", &[], SHOW_OPTIONS.as_bytes())
        } else if path == "/dana.js" {
            response_ok_utf8("application/javascript", &[], DANA_JS.as_bytes())
        } else if path == "/file_system.js" {
            response_ok_utf8("application/javascript", &[], FILE_SYSTEM_JS.as_bytes())
        } else if path == "/dana.wasm" {
            response_bytes("application/wasm", &[], DANA_WASM)
        } else {
            response_404()
        };

    send_raw(client_stream, &response).await
}

fn main() -> io::Result<()> {
    const OFFLOAD_URL: [&str; 1] = ["obm-offload-1:7010"];

    smol::block_on(async {
        // Bind the server to a local port
        let port_addr = "0.0.0.0:7000";
        let listener = TcpListener::bind(port_addr).await?;
        println!("TCP Server listening on {port_addr}");

        // Accept incoming connections loop
        let mut incoming = listener.incoming();
        while let Some(stream) = incoming.next().await {
            let stream = stream?;
            // Spawn an asynchronous task for each client connection
            smol::spawn(async move {
                if let Err(e) = handle_client(stream, &OFFLOAD_URL).await {
                    eprintln!("Error handling client: {}", e);
                }
            })
            .detach(); // Detach allows the task to run independently in the background
        }
        Ok(())
    })
}
