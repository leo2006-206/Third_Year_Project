use std::env;

use smol::{
    // future::zip,
    io,
    net::{TcpListener, TcpStream},
    prelude::*,
};

use servers_rust::lib_util as util;

async fn handle_client(mut client_stream: TcpStream) -> io::Result<()> {
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

    Ok(())
}

fn main() -> io::Result<()> {
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
                if let Err(e) = handle_client(stream).await {
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
        .expect("Error: Missing port. Usage: <program> <ip> <port>");

    assert!(
        port.parse::<u16>().is_ok(),
        "Error: Port must be a valid port number (1-65535), got: '{port}'"
    );

    port
}
