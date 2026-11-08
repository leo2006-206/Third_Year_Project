use std::env;
use std::sync::Arc;
use std::time::Duration;

use smol::{
    channel::{Receiver, Sender},
    // future::zip,
    io,
    net::{TcpListener, TcpStream},
    prelude::*,
};

use rust_obm::mod_feature::AssetCache;
use rust_server::lib_http as http;
use rust_server::lib_server_offload as offl;
use rust_server::lib_util as util;

async fn handle_client(
    mut client_stream: TcpStream,
    jobs: Sender<offl::DanaJob>,
    main_addr: &str,
    asset_cache: Option<&'static AssetCache>,
    current: &'static offl::CurrentJob,
) -> io::Result<()> {
    use http::{response_404, send_raw};

    let mut buffer = vec![0u8; 4096];

    // one read, like util::read_as_str, but the byte count is kept: a PSC rebuild's body (its
    // sub segment) starts in this buffer
    let n = match client_stream.read(&mut buffer).await {
        Ok(0) | Err(_) => {
            eprintln!("Failed to read HTTP request with err");
            return Ok(());
        }
        Ok(n) => n,
    };
    let req_str = String::from_utf8_lossy(&buffer[..n]);

    let Some((method, path)) = util::parse_method_path(&req_str, Some("?")) else {
        eprintln!("Failed to parse HTTP request");
        return Ok(());
    };

    if path.starts_with("/psc/") {
        // Dana's own sub fetch during a rebuild: never queued (Dana is busy with that job)
        serve_psc_sub(client_stream, path, current).await
    } else if path.starts_with("/offload") || path.starts_with("/times/offload") {
        // a PSC rebuild ('<k>+<n>') comes as POST with its sub; anything else must not have a body
        let sub = match (offl::rebuild_target(path).is_some(), method) {
            (true, "POST") => match read_body(&mut client_stream, &buffer[..n]).await {
                Ok(sub) => Some(Arc::new(sub)),
                Err(e) => {
                    eprintln!("[PSC] rebuild body: {e} | {path}");
                    return send_raw(&mut client_stream, &offl::bad_request(&e.to_string())).await;
                }
            },
            (true, _) => {
                let reply = offl::bad_request("PSC rebuild needs POST with its sub segment");
                return send_raw(&mut client_stream, &reply).await;
            }
            (false, "POST") => {
                let reply = offl::bad_request("POST is only for PSC rebuilds ('<k>+<n>')");
                return send_raw(&mut client_stream, &reply).await;
            }
            (false, _) => None,
        };
        // queued for the Dana worker, which answers the client itself
        let job = offl::DanaJob {
            client: client_stream,
            path: path.to_string(),
            enqueued: std::time::Instant::now(),
            sub,
        };
        jobs.send(job)
            .await
            .map_err(|_| io::Error::other("Dana worker has stopped"))
    } else if path.starts_with("/shows") || path.starts_with("/assets") {
        match asset_cache {
            Some(cache) => serve_main_proxy_cached(client_stream, path, main_addr, cache).await,
            None => serve_main_proxy(client_stream, path, main_addr).await,
        }
    } else {
        send_raw(&mut client_stream, &response_404()).await
    }
}

/// [Dishonest I/O] The rest of a request body after the first read (`first` = the bytes read so
/// far, head included): Content-Length required, at most offl::MAX_SUB_BYTES.
async fn read_body(stream: &mut TcpStream, first: &[u8]) -> io::Result<Vec<u8>> {
    let invalid = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
    let (start, len) =
        offl::request_body_start(first).ok_or_else(|| invalid("request head too long"))?;
    let len = len.ok_or_else(|| invalid("no Content-Length"))? as usize;
    if len > offl::MAX_SUB_BYTES {
        return Err(invalid("sub segment too big"));
    }
    let mut body = first[start..].to_vec();
    if body.len() > len {
        return Err(invalid("body longer than Content-Length"));
    }
    let have = body.len();
    body.resize(len, 0);
    stream.read_exact(&mut body[have..]).await?;
    Ok(body)
}

/// [Dishonest I/O] GET /psc/show/<show>/<t0>/...: the current rebuild job's sub if show and t0 match
/// (marked served after the write), else 404 (the offload track's read-ahead of t0+10, or no
/// rebuild running). Logs '[PSC] sub served N B | path' / '[PSC] sub 404 | path'.
async fn serve_psc_sub(
    mut stream: TcpStream,
    path: &str,
    current: &'static offl::CurrentJob,
) -> io::Result<()> {
    let target = offl::sub_target(path);
    let sub = match (&*offl::lock(current), &target) {
        (Some(cur), Some((show, t0))) if cur.show == *show && cur.t0 == *t0 => {
            Some(Arc::clone(&cur.sub))
        }
        _ => None,
    };
    let Some(sub) = sub else {
        util::log_message(&format!("[PSC] sub 404 | {path}"));
        return http::send_raw(&mut stream, &http::response_404()).await;
    };
    let head =
        http::format_response_header("200 OK", "application/octet-stream", sub.len() as u64, &[]);
    stream.write_all(&head).await?;
    stream.write_all(&sub).await?;
    stream.flush().await?;
    // still the same job (Dana cannot start the next one while it waits for this sub)
    if let Some(cur) = offl::lock(current).as_mut()
        && Arc::ptr_eq(&cur.sub, &sub)
    {
        cur.served = true;
    }
    util::log_message(&format!("[PSC] sub served {} B | {path}", sub.len()));
    Ok(())
}

// The only task that talks to Dana: one request at a time, in arrival order (the reply goes back
// to the main server in its own task, see serve_dana). A failed job is answered with an error and
// skipped; the loop must not end (or panic), or every later request waits forever.
// A PSC rebuild job is published in `current` while Dana renders it (its sub fetch reads it).
async fn dana_worker(
    jobs: Receiver<offl::DanaJob>,
    dana_addr: &str,
    timeout: Duration,
    current: &'static offl::CurrentJob,
) {
    while let Ok(job) = jobs.recv().await {
        let t_queue = job.enqueued.elapsed().as_millis();
        let rebuild = job.sub.and_then(|sub| {
            offl::rebuild_target(&job.path).map(|(show, t0)| offl::CurrentSub {
                show,
                t0,
                sub,
                served: false,
            })
        });
        let psc = rebuild.is_some().then_some(current);
        *offl::lock(current) = rebuild;
        serve_dana(job.client, &job.path, t_queue, dana_addr, timeout, psc).await;
    }
}

/// [Dishonest I/O] one job: Dana's reply, or an error reply, goes back to the main server.
/// A Dana that does not answer within `timeout` is stuck (it used to hang the whole site: every later
/// job waited behind it): it is killed (the container's entrypoint restarts it) and the job gets 504.
async fn serve_dana(
    main_server: TcpStream,
    offload_path: &str,
    t_queue: u128,
    dana_addr: &str,
    timeout: Duration,
    psc: Option<&offl::CurrentJob>,
) {
    let response = match fetch_from_dana(offload_path, &t_queue.to_string(), dana_addr, timeout)
        .await
    {
        Ok(response) => response,
        Err(e) if e.kind() == io::ErrorKind::TimedOut => {
            eprintln!(
                "[offload_server] Dana gave no reply in {} s, killing it (entrypoint restarts it): {offload_path}",
                timeout.as_secs()
            );
            kill_dana();
            http::response_gateway_error(true)
        }
        Err(e) => {
            eprintln!("[offload_server] {offload_path} failed: {e}");
            http::response_gateway_error(false)
        }
    };
    // PSC rebuild: Dana must have fetched its sub, else its picture has no bottom layers
    let response = match psc.and_then(|current| offl::lock(current).take()) {
        Some(job) => {
            if !job.served {
                eprintln!(
                    "[PSC] rebuild without its sub (Dana never fetched it) -> 502: {offload_path}"
                );
            }
            offl::rebuild_reply(response, job.served)
        }
        None => response,
    };
    smol::spawn(async move {
        if let Err(e) = dana_sendback(main_server, response).await {
            eprintln!("[offload_server] send back to main server failed: {e}");
        }
    })
    .detach();
}

/// [Dishonest I/O] kill the Dana worker of this container (only Dana runs as 'dana' here) = `pkill -9 -x dana`,
/// done with /proc/<pid>/comm + kill(2). NOT std::process::Command: spawning links Rust std's pidfd_spawnp /
/// pidfd_getpid (GLIBC_2.39) into the host build, and the ubuntu 22.04 image (glibc 2.35) then refuses to
/// start the binary at all (container test 11-08).
fn kill_dana() {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        eprintln!("[offload_server] kill dana failed: cannot read /proc");
        return;
    };
    let mut killed = Vec::new();
    for entry in dir.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<i32>().ok())
        else {
            continue;
        };
        let is_dana = std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .is_ok_and(|comm| comm.trim_end() == "dana");
        if is_dana {
            // SAFETY: kill(2) takes a pid and a signal number; no memory is shared with the callee.
            let rc = unsafe { libc::kill(pid, libc::SIGKILL) };
            killed.push((pid, rc));
        }
    }
    eprintln!("[offload_server] kill -9 dana (pid, rc): {killed:?}");
}

/// [Honest] Dana reply timeout: OBM_DANA_TIMEOUT_S (seconds) or 300 s. A render is at most ~2 x the
/// 60 s Dana resource timeout + ~30 s cpu render, and the front sends Dana one job at a time.
fn dana_timeout(env_value: Option<String>) -> Duration {
    const DEFAULT_S: u64 = 300;
    let s = env_value
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&s| s > 0);
    Duration::from_secs(s.unwrap_or(DEFAULT_S))
}

// One job to Dana and its whole response (headers + H.264) into memory: Dana is free as soon as it
// has written it, and the slow send to the main server runs beside the next job.
// Dana recycles itself after 10 requests when its own queue is empty; a job connected in that
// instant gets a reset / closed socket before any response byte. Dana cannot have rendered it, so
// it is resent to the restarted Dana (a reply that broke after its first byte is an error)
async fn fetch_from_dana(
    offload_path: &str,
    t_queue: &str,
    dana_addr: &str,
    timeout: Duration,
) -> io::Result<Vec<u8>> {
    smol::future::or(
        fetch_from_dana_inner(offload_path, t_queue, dana_addr),
        async {
            smol::Timer::after(timeout).await;
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Dana reply timeout",
            ))
        },
    )
    .await
}

async fn fetch_from_dana_inner(
    offload_path: &str,
    t_queue: &str,
    dana_addr: &str,
) -> io::Result<Vec<u8>> {
    const OFFLOAD_AGENT: &str = "OBM Offload site/1.0";
    const MAX_RESENDS: usize = 20;
    const RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(50);

    // Simply send X-Queue-Time to Dana in the GET request:
    let extra_headers = [("X-Queue-Time", t_queue)];
    let req = http::request_get(offload_path, dana_addr, OFFLOAD_AGENT, &extra_headers);
    let mut resend = 0;
    loop {
        let mut dana_stream = connect_dana(dana_addr).await?;
        let mut response = Vec::new();
        let sent = async {
            dana_stream.write_all(&req).await?;
            dana_stream.flush().await?;
            dana_stream.read_to_end(&mut response).await
        }
        .await;
        match sent {
            Ok(_) if !response.is_empty() => return Ok(response),
            Err(e) if !response.is_empty() => return Err(e),
            // nothing came back: Dana closed (recycling) before it read the job
            result if resend < MAX_RESENDS => {
                resend += 1;
                let why = result.err().map_or("closed".to_string(), |e| e.to_string());
                eprintln!(
                    "[offload_server] Dana gave no response ({why}), resend {resend}/{MAX_RESENDS}: {offload_path}"
                );
                smol::Timer::after(RETRY_DELAY).await;
            }
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Dana gave no response",
                ));
            }
            Err(e) => return Err(e),
        }
    }
}

// Connects to Dana, retrying while it is (re)starting
async fn connect_dana(dana_addr: &str) -> io::Result<TcpStream> {
    const MAX_RETRIES: usize = 100;
    const RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(50);
    let mut attempt = 0;
    loop {
        match TcpStream::connect(dana_addr).await {
            Ok(stream) => return Ok(stream),
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

/// `serve_main_proxy` with the asset cache: a hit is sent from the site's own disk (no trip to
/// the main server); a miss is streamed to Dana exactly like the uncached proxy while a copy is
/// kept, and the body is stored after Dana has it (200 only). Every outcome is logged [ASSET].
async fn serve_main_proxy_cached(
    mut dana_stream: TcpStream,
    resource_path: &str,
    main_addr: &str,
    cache: &'static AssetCache,
) -> io::Result<()> {
    const OFFLOAD_AGENT: &str = "OBM Offload site/1.0";
    // the same headers the main server sends with a resource (serve_resource)
    const RESOURCE_HEADERS: [(&str, &str); 2] = [
        ("Cross-Origin-Resource-Policy", "cross-origin"),
        ("Cache-Control", "no-cache, no-store, must-revalidate"),
    ];

    if let Some((file, size)) = cache.get(resource_path) {
        let content_type = util::match_content_type(resource_path);
        http::send_file(&mut dana_stream, file, content_type, &RESOURCE_HEADERS).await?;
        util::log_message(&format!("[ASSET] hit {size} B | {resource_path}"));
        return Ok(());
    }

    let mut main_stream = TcpStream::connect(main_addr).await?;
    let req = http::request_get(resource_path, main_addr, OFFLOAD_AGENT, &[]);
    main_stream.write_all(&req).await?;
    main_stream.flush().await?;

    // tee: every chunk goes on to Dana at once and into `response` for the cache
    let mut chunk = vec![0u8; 64 << 10];
    let mut response = Vec::new();
    loop {
        let n = main_stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        dana_stream.write_all(&chunk[..n]).await?;
        response.extend_from_slice(&chunk[..n]);
    }
    dana_stream.flush().await?;

    let key = resource_path.to_string();
    smol::unblock(move || {
        let outcome = match http::split_response(&response) {
            Some((head, body)) if http::is_status_ok(head) => match cache.add(&key, body) {
                Ok(()) => format!("added {} B (used {} B)", body.len(), cache.used()),
                Err(e) => {
                    let reason = match e.kind() {
                        // a concurrent miss on the same file stored it first
                        io::ErrorKind::AlreadyExists => "already cached".to_string(),
                        io::ErrorKind::InvalidInput => "bigger than the cache".to_string(),
                        _ => e.to_string(),
                    };
                    format!("add failed ({} B): {reason}", body.len())
                }
            },
            _ => "skip: status not 200".to_string(),
        };
        util::log_message(&format!("[ASSET] {outcome} | {key}"));
    })
    .detach();
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
    // Lives for the whole run: leaked to a 'static reference like main_addr
    let asset_cache: Option<&'static AssetCache> = match offl::asset_cache_loader() {
        Ok(cache) => cache.map(|c| &*Box::leak(Box::new(c))),
        Err(e) => {
            eprintln!("[offload_server] {e}");
            std::process::exit(2);
        }
    };
    match asset_cache {
        Some(cache) => println!(
            "Asset cache ON: {} MB, {}",
            std::env::var("OBM_ASSET_CACHE_MB").unwrap_or_default(),
            cache.policy()
        ),
        None => println!("Asset cache OFF"),
    }

    smol::block_on(async {
        // Bind the server to a local port
        let port_addr = format!("0.0.0.0:{}", arg_port());
        let listener = TcpListener::bind(&port_addr).await?;
        println!("TCP Server listening on {port_addr}");

        // FIFO queue in front of Dana (Dana renders one request at a time)
        let (jobs, queue) = smol::channel::unbounded();
        let timeout = dana_timeout(env::var("OBM_DANA_TIMEOUT_S").ok());
        println!("Dana reply timeout {} s", timeout.as_secs());
        // the PSC rebuild Dana is rendering (its sub), shared with the /psc route
        let current: &'static offl::CurrentJob = Box::leak(Box::new(offl::CurrentJob::new(None)));
        smol::spawn(dana_worker(queue, DANA_ADDR, timeout, current)).detach();

        // Accept incoming connections loop
        let mut incoming = listener.incoming();
        while let Some(stream) = incoming.next().await {
            let stream = stream?;
            let jobs = jobs.clone();
            // Spawn an asynchronous task for each client connection
            smol::spawn(async move {
                if let Err(e) = handle_client(stream, jobs, main_addr, asset_cache, current).await {
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

#[cfg(test)]
mod tests {
    use super::*;

    const LONG: Duration = Duration::from_secs(30);

    // A fake Dana that reads the job and never answers (stuck render loop), holding the socket open
    async fn silent_dana() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        smol::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf).await;
            smol::Timer::after(Duration::from_secs(30)).await;
            drop(s);
        })
        .detach();
        addr
    }

    #[test]
    fn test_fetch_times_out_on_a_silent_dana() {
        smol::block_on(async {
            let addr = silent_dana().await;
            let t = std::time::Instant::now();
            let err = fetch_from_dana("/p", "0", &addr, Duration::from_millis(300))
                .await
                .unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::TimedOut);
            assert!(t.elapsed() < Duration::from_secs(5));
        });
    }

    #[test]
    fn test_dana_timeout_env() {
        assert_eq!(dana_timeout(None), Duration::from_secs(300));
        assert_eq!(dana_timeout(Some("120".into())), Duration::from_secs(120));
        assert_eq!(dana_timeout(Some(" 7 ".into())), Duration::from_secs(7));
        assert_eq!(dana_timeout(Some("0".into())), Duration::from_secs(300));
        assert_eq!(dana_timeout(Some("x".into())), Duration::from_secs(300));
    }

    // A fake Dana: drops the first `drops` connections without answering (the recycle race),
    // then answers every request with `reply`
    async fn fake_dana(drops: usize, reply: &'static [u8]) -> (String, smol::Task<usize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let task = smol::spawn(async move {
            let mut accepted = 0;
            loop {
                let (mut s, _) = listener.accept().await.unwrap();
                accepted += 1;
                if accepted <= drops {
                    drop(s); // closed before any response byte
                    continue;
                }
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf).await.unwrap();
                s.write_all(reply).await.unwrap();
                return accepted;
            }
        });
        (addr, task)
    }

    #[test]
    fn test_fetch_resends_after_dana_closed_without_response() {
        smol::block_on(async {
            let (addr, dana) = fake_dana(2, b"HTTP/1.1 200 OK\r\n\r\nvideo").await;
            let resp = fetch_from_dana("/offload/show/x.json/0/10", "0", &addr, LONG)
                .await
                .unwrap();
            assert_eq!(resp, b"HTTP/1.1 200 OK\r\n\r\nvideo");
            assert_eq!(dana.await, 3); // two dropped, the third answered
        });
    }

    #[test]
    fn test_fetch_first_try() {
        smol::block_on(async {
            let (addr, dana) = fake_dana(0, b"HTTP/1.1 200 OK\r\n\r\n").await;
            assert!(fetch_from_dana("/p", "0", &addr, LONG).await.is_ok());
            assert_eq!(dana.await, 1);
        });
    }

    // ---- PSC rebuild through the front (in process): main server -> front -> fake Dana -> /psc

    const REBUILD: &str =
        "/offload/show/f1.json/20/30/1280/720/race/landscape/1+2/race|race/drivers|Sam";
    const SUB_URL: &str = "/psc/show/f1.json/20/30/1280/720/race/landscape/1/race|race";
    const AHEAD_URL: &str = "/psc/show/f1.json/30/40/1280/720/race/landscape/1/race|race";

    /// A front on 127.0.0.1:0 talking to `dana_addr`; returns its address.
    async fn front(dana_addr: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let current: &'static offl::CurrentJob = Box::leak(Box::new(offl::CurrentJob::new(None)));
        let dana: &'static str = dana_addr.leak();
        let (jobs, queue) = smol::channel::unbounded();
        smol::spawn(dana_worker(queue, dana, LONG, current)).detach();
        smol::spawn(async move {
            loop {
                let (s, _) = listener.accept().await.unwrap();
                let jobs = jobs.clone();
                smol::spawn(async move {
                    let _ = handle_client(s, jobs, "127.0.0.1:1", None, current).await;
                })
                .detach();
            }
        })
        .detach();
        addr
    }

    /// GET `url` from `host`, the whole response
    async fn get(host: &str, url: &str) -> Vec<u8> {
        let mut s = TcpStream::connect(host).await.unwrap();
        s.write_all(&http::request_get(url, host, "test", &[]))
            .await
            .unwrap();
        let mut r = Vec::new();
        s.read_to_end(&mut r).await.unwrap();
        r
    }

    /// A fake Dana for one job: reads it, then (if `fetch`) GETs its sub and the read-ahead from
    /// the front like Track:offload, answers 200 "REBUILT"; the task gives (sub reply, ahead reply).
    async fn psc_dana(
        front_addr: smol::channel::Receiver<String>,
        fetch: bool,
    ) -> (String, smol::Task<(Vec<u8>, Vec<u8>)>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let task = smol::spawn(async move {
            let front = front_addr.recv().await.unwrap();
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf).await.unwrap();
            let (sub, ahead) = if fetch {
                (get(&front, SUB_URL).await, get(&front, AHEAD_URL).await)
            } else {
                (Vec::new(), Vec::new())
            };
            s.write_all(b"HTTP/1.0 200 OK\r\nContent-length: 7\r\n\r\nREBUILT")
                .await
                .unwrap();
            (sub, ahead)
        });
        (addr, task)
    }

    /// The main server's side: POST the rebuild with `sub` (or GET it without), whole reply
    async fn send_rebuild(front: &str, sub: Option<&[u8]>) -> Vec<u8> {
        let mut s = TcpStream::connect(front).await.unwrap();
        match sub {
            Some(sub) => {
                s.write_all(&http::request_with_body(
                    "POST",
                    REBUILD,
                    front,
                    "main",
                    sub.len(),
                ))
                .await
                .unwrap();
                s.write_all(sub).await.unwrap();
            }
            None => s
                .write_all(&http::request_get(REBUILD, front, "main", &[]))
                .await
                .unwrap(),
        }
        let mut r = Vec::new();
        s.read_to_end(&mut r).await.unwrap();
        r
    }

    #[test]
    fn test_psc_rebuild_dana_gets_its_pushed_sub() {
        smol::block_on(async {
            // a sub bigger than the first 4 KB read: the body spans many reads
            let sub: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
            let (tx, rx) = smol::channel::bounded(1);
            let (dana, task) = psc_dana(rx, true).await;
            let front = front(dana).await;
            tx.send(front.clone()).await.unwrap();
            let reply = send_rebuild(&front, Some(&sub)).await;
            assert!(
                reply.starts_with(b"HTTP/1.0 200 OK"),
                "{}",
                String::from_utf8_lossy(&reply[..40.min(reply.len())])
            );
            assert!(reply.ends_with(b"REBUILT"));
            let (got, ahead) = task.await;
            let (head, body) = http::split_response(&got).unwrap();
            assert!(http::is_status_ok(head));
            assert_eq!(body, &sub[..]);
            assert!(
                ahead.starts_with(b"HTTP/1.1 404") || ahead.starts_with(b"HTTP/1.0 404"),
                "{}",
                String::from_utf8_lossy(&ahead)
            );
        });
    }

    #[test]
    fn test_psc_rebuild_without_sub_fetch_is_502() {
        smol::block_on(async {
            let (tx, rx) = smol::channel::bounded(1);
            let (dana, task) = psc_dana(rx, false).await;
            let front = front(dana).await;
            tx.send(front.clone()).await.unwrap();
            let reply = send_rebuild(&front, Some(b"SUB")).await;
            assert!(reply.starts_with(b"HTTP/1.0 502"));
            assert_eq!(
                http::header_value(&reply, "X-PSC-Rebuild").as_deref(),
                Some("failed")
            );
            task.await;
            // nothing is running now: a stray sub fetch is a 404
            assert!(get(&front, SUB_URL).await.windows(3).any(|w| w == b"404"));
        });
    }

    #[test]
    fn test_psc_bad_methods_are_400() {
        smol::block_on(async {
            // no Dana needed: refused before the queue
            let front = front("127.0.0.1:1".to_string()).await;
            let reply = send_rebuild(&front, None).await; // GET with '+'
            assert!(
                reply.starts_with(b"HTTP/1.0 400"),
                "{}",
                String::from_utf8_lossy(&reply)
            );
            let mut s = TcpStream::connect(&front).await.unwrap();
            let plain = REBUILD.replace("1+2", "2");
            s.write_all(&http::request_with_body("POST", &plain, &front, "main", 1))
                .await
                .unwrap();
            s.write_all(b"x").await.unwrap();
            let mut r = Vec::new();
            s.read_to_end(&mut r).await.unwrap();
            assert!(r.starts_with(b"HTTP/1.0 400"));
        });
    }
}
