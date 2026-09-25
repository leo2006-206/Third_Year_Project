use std::env;

use smol::{
    // future::zip,
    io,
    net::{TcpListener, TcpStream},
    prelude::*,
};

use servers_rust::lib_http as http;
use servers_rust::lib_util as util;

async fn handle_client(
    mut client_stream: TcpStream,
    dana_addr: &str,
    main_addr: &str,
) -> io::Result<()> {
    use http::{response_404, send_raw};

    let mut buffer = vec![0u8; 4096];

    let Ok(req_str) = util::read_as_str(&mut client_stream, &mut buffer).await else {
        eprintln!("Failed to read HTTP request with err");
        return Ok(());
    };

    let Some((method, path)) = util::parse_method_path(&req_str, Some("?")) else {
        eprintln!("Failed to parse HTTP request");
        return Ok(());
    };

    dbg!(&method, &path);

    if path.starts_with("/offload") {
        serve_dana_proxy(client_stream, path, dana_addr).await
    } else if path.starts_with("/shows") || path.starts_with("/assets") {
        serve_main_proxy(client_stream, path, main_addr).await
    } else {
        send_raw(&mut client_stream, &response_404()).await
    }
}

async fn serve_dana_proxy(
    mut main_server: TcpStream,
    offload_path: &str,
    dana_addr: &str,
) -> io::Result<()> {
    const OFFLOAD_AGENT: &str = "OBM Offload site/1.0";

    let mut dana_stream = TcpStream::connect(dana_addr).await?;

    // Forward the GET request to Dana
    let req = http::request_get(offload_path, dana_addr, OFFLOAD_AGENT, &[]);

    dana_stream.write_all(&req).await?;
    dana_stream.flush().await?;

    // Asynchronously stream Dana's response (headers + H.264 chunks) directly to caller
    smol::io::copy(&mut dana_stream, &mut main_server).await?;
    main_server.flush().await?;
    Ok(())
}

async fn serve_main_proxy(
    mut dana_stream: TcpStream,
    resource_path: &str,
    main_addr: &str,
) -> io::Result<()> {
    const OFFLOAD_AGENT: &str = "OBM Offload site/1.0";

    let mut main_stream = TcpStream::connect(main_addr).await?;

    let req = http::request_get(resource_path, main_addr, OFFLOAD_AGENT, &[]);

    main_stream.write_all(&req).await?;
    main_stream.flush().await?;

    smol::io::copy(&mut main_stream, &mut dana_stream).await?;
    dana_stream.flush().await?;
    Ok(())
}

fn main() -> io::Result<()> {
    const DANA_ADDR: &str = "127.0.0.1:9009";
    const MAIN_SERVER_ADDR: &str = "obm-main-server:7000";

    smol::block_on(async {
        // Bind the server to a local port
        let port_addr = format!("0.0.0.0:{}", arg_port());
        let listener = TcpListener::bind(&port_addr).await?;
        println!("TCP Server listening on {port_addr}");

        // Accept incoming connections loop
        let mut incoming = listener.incoming();
        while let Some(stream) = incoming.next().await {
            let stream = stream?;
            // Spawn an asynchronous task for each client connection
            smol::spawn(async move {
                if let Err(e) = handle_client(stream, DANA_ADDR, MAIN_SERVER_ADDR).await {
                    eprintln!("Error handling client: {}", e);
                }
            })
            .detach(); // Detach allows the task to run independently in the background
        }
        Ok(())
    })
}

fn arg_port() -> String {
    let mut args = env::args().skip(1);

    let port = args
        .next()
        .expect("Error: Missing port. Usage: <program> <port>");

    assert!(
        port.parse::<u16>().is_ok(),
        "Error: Port must be a valid port number (1-65535), got: '{port}'"
    );

    port
}
