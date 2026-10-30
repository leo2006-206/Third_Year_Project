use std::env;

use smol::{
    channel::{Receiver, Sender},
    // future::zip,
    io,
    net::{TcpListener, TcpStream},
    prelude::*,
};

use rust_server::lib_http as http;
use rust_server::lib_server_offload as offl;
use rust_server::lib_util as util;

async fn handle_client(
    mut client_stream: TcpStream,
    jobs: Sender<offl::DanaJob>,
    main_addr: &str,
) -> io::Result<()> {
    use http::{response_404, send_raw};

    let mut buffer = vec![0u8; 4096];

    let Ok(req_str) = util::read_as_str(&mut client_stream, &mut buffer).await else {
        eprintln!("Failed to read HTTP request with err");
        return Ok(());
    };

    let Some((_method, path)) = util::parse_method_path(&req_str, Some("?")) else {
        eprintln!("Failed to parse HTTP request");
        return Ok(());
    };

    // dbg!(&method, &path);

    if path.starts_with("/offload") || path.starts_with("/times/offload") {
        // queued for the Dana worker, which answers the client itself
        let job = offl::DanaJob {
            client: client_stream,
            path: path.to_string(),
            enqueued: std::time::Instant::now(),
        };
        jobs.send(job)
            .await
            .map_err(|_| io::Error::other("Dana worker has stopped"))
    } else if path.starts_with("/shows") || path.starts_with("/assets") {
        serve_main_proxy(client_stream, path, main_addr).await
    } else {
        send_raw(&mut client_stream, &response_404()).await
    }
}

// The only task that talks to Dana: one request at a time, in arrival order (the reply goes back
// to the main server in its own task, see serve_dana). A failed job is logged and skipped; the
// loop must not end (or panic), or every later request waits forever
async fn dana_worker(jobs: Receiver<offl::DanaJob>, dana_addr: &str) {
    while let Ok(job) = jobs.recv().await {
        let t_queue = job.enqueued.elapsed().as_millis();
        if let Err(e) = serve_dana(job.client, &job.path, t_queue, dana_addr).await {
            eprintln!("[offload_server] {} failed: {e}", job.path);
        }
    }
}

async fn serve_dana(
    main_server: TcpStream,
    offload_path: &str,
    t_queue: u128,
    dana_addr: &str,
) -> io::Result<()> {
    const OFFLOAD_AGENT: &str = "OBM Offload site/1.0";
    const MAX_RETRIES: usize = 100;
    const RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(50);

    let t_queue = t_queue.to_string();

    let mut dana_stream = {
        let mut attempt = 0;
        loop {
            match TcpStream::connect(dana_addr).await {
                Ok(stream) => break stream,
                Err(e) if attempt < MAX_RETRIES => {
                    attempt += 1;
                    eprintln!(
                        "[offload_server] Dana not reachable (attempt {attempt}/{MAX_RETRIES}): {e}."
                    );
                    smol::Timer::after(RETRY_DELAY).await;
                }
                Err(e) => {
                    eprintln!(
                        "[offload_server] Dana connection failed after {MAX_RETRIES} attempts: {e}"
                    );
                    return Err(e);
                }
            }
        }
    };

    // Simply send X-Queue-Time to Dana in the GET request:
    let extra_headers = [("X-Queue-Time", t_queue.as_str())];
    // Forward the GET request to Dana
    let req = http::request_get(offload_path, dana_addr, OFFLOAD_AGENT, &extra_headers);

    dana_stream.write_all(&req).await?;
    dana_stream.flush().await?;

    // Whole response (headers + H.264) into memory: Dana is free as soon as it has written it,
    // and the slow send to the main server runs beside the next job
    let mut response = Vec::new();
    dana_stream.read_to_end(&mut response).await?;
    smol::spawn(async move {
        if let Err(e) = dana_sendback(main_server, response).await {
            eprintln!("[offload_server] send back to main server failed: {e}");
        }
    })
    .detach();
    Ok(())
}

async fn dana_sendback(mut main_server: TcpStream, response: Vec<u8>) -> io::Result<()> {
    main_server.write_all(&response).await?;
    main_server.flush().await
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
    // Where Dana's /shows and /assets downloads go: the main server on om over the cable link
    // (om's ufw must allow docker + cable traffic, see README), or MAIN_SERVER_ADDR=<host>:7000
    let main_addr: &'static str = env::var("MAIN_SERVER_ADDR")
        .unwrap_or_else(|_| "10.10.0.1:7000".to_string())
        .leak();
    println!("Assets from main server {main_addr}");

    smol::block_on(async {
        // Bind the server to a local port
        let port_addr = format!("0.0.0.0:{}", arg_port());
        let listener = TcpListener::bind(&port_addr).await?;
        println!("TCP Server listening on {port_addr}");

        // FIFO queue in front of Dana (Dana renders one request at a time)
        let (jobs, queue) = smol::channel::unbounded();
        smol::spawn(dana_worker(queue, DANA_ADDR)).detach();

        // Accept incoming connections loop
        let mut incoming = listener.incoming();
        while let Some(stream) = incoming.next().await {
            let stream = stream?;
            let jobs = jobs.clone();
            // Spawn an asynchronous task for each client connection
            smol::spawn(async move {
                if let Err(e) = handle_client(stream, jobs, main_addr).await {
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
