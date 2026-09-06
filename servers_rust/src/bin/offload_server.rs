use std::env;

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

    Ok(())
}

fn main() -> io::Result<()> {
    let (local_url, port) = arg_url_port();
    let _ = util::set_local_url(local_url);

    smol::block_on(async {
        // Bind the server to a local port
        let port_addr = format!("0.0.0.0:{}", port);
        let listener = TcpListener::bind(&port_addr).await?;
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

fn arg_url_port() -> (String, String) {
    let mut args = env::args().skip(1);

    let local_url = args
        .next()
        .expect("Error: Missing local URL. Usage: <program> <local_url> <port>");

    let port = args
        .next()
        .expect("Error: Missing port. Usage: <program> <local_url> <port>");

    assert!(
        local_url.starts_with("https"),
        "Error: Local URL must start with https"
    );
    assert!(
        port.parse::<u16>().is_ok(),
        "Error: Port must be a valid port number (1-65535), got: '{port}'"
    );

    (local_url, port)
}
