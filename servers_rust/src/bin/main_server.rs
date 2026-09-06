use smol::{
    // future::zip,
    io,
    io::AsyncReadExt,
    net::{TcpListener, TcpStream},
    prelude::*,
};

use servers_rust::util;
use servers_rust::util_http as http;

async fn handle_client(mut client_stream: TcpStream) -> io::Result<()> {
    let mut buffer = vec![0u8; 4096];

    let n = client_stream.read(&mut buffer).await?;
    if n == 0 {
        return Ok(());
    }

    let req_str = String::from_utf8_lossy(&buffer[..n]);

    let Some((method, path)) = http::parse_method_path(&req_str) else {
        eprintln!("Failed to parse HTTP request");
        return Ok(());
    };

    dbg!(&method, &path);

    if path.starts_with("/assets") {
        todo!()
    } else if path.starts_with("/offload") {
        serve_offload(&mut client_stream, path).await
    } else {
        serve_web_page(&mut client_stream, path).await
    }
}

async fn serve_offload(_client_stream: &mut TcpStream, path: &str) -> io::Result<()> {
    use http::request_get;

    const OFFLOAD_URL: [&str; 1] = ["https://obm_offload_1.leowong.space/"];

    let mut offload_steam = TcpStream::connect(OFFLOAD_URL[0]).await?;

    let local_url = util::local_url().expect("local URL must set up");

    request_get(&mut offload_steam, path, local_url).await
}

async fn serve_web_page(client_stream: &mut TcpStream, path: &str) -> io::Result<()> {
    use http::{response_404, response_bytes, response_ok_utf8};

    const INDEX_HTML: &str = include_str!("../testing_webpage/index.html");
    const APP_JS: &str = include_str!("../testing_webpage/app.js");
    const STYLE_CSS: &str = include_str!("../testing_webpage/style.css");
    const SHOW_OPTIONS: &str = include_str!("../testing_webpage/show_options.json");

    const DANA_JS: &str = include_str!("../../../obm/dana.js");
    const FILE_SYSTEM_JS: &str = include_str!("../../../obm/file_system.js");
    const DANA_WASM: &[u8] = include_bytes!("../../../obm/dana.wasm");

    if path == "/" || path == "/client_testing" || path == "/client_testing/index.html" {
        response_ok_utf8(client_stream, "text/html", INDEX_HTML).await?;
    } else if path == "/app.js" {
        response_ok_utf8(client_stream, "application/javascript", APP_JS).await?;
    } else if path == "/style.css" {
        response_ok_utf8(client_stream, "text/css", STYLE_CSS).await?;
    } else if path == "/show_options.json" {
        response_ok_utf8(client_stream, "application/json", SHOW_OPTIONS).await?;
    } else if path == "/dana.js" {
        response_ok_utf8(client_stream, "application/javascript", DANA_JS).await?;
    } else if path == "/file_system.js" {
        response_ok_utf8(client_stream, "application/javascript", FILE_SYSTEM_JS).await?;
    } else if path == "/dana.wasm" {
        response_bytes(client_stream, "application/wasm", DANA_WASM).await?;
    } else {
        response_404(client_stream).await?;
    }

    Ok(())
}

fn main() -> io::Result<()> {
    let _ = util::set_local_url("https://obm_main.leowong.space/");

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
                if let Err(e) = handle_client(stream).await {
                    eprintln!("Error handling client: {}", e);
                }
            })
            .detach(); // Detach allows the task to run independently in the background
        }
        Ok(())
    })
}
