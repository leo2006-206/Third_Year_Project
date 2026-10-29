use std::borrow::Cow;
use std::fs::File;
use std::future::Future;
use std::io;
use std::path::Path;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use smol::Timer;
use smol::future::FutureExt;
use smol::io::{AsyncReadExt, AsyncWriteExt};

use crate::lib_core_type as ct;

static START_TIME: LazyLock<Instant> = LazyLock::new(Instant::now);

pub fn log_message(message: &str) {
    let elapsed = START_TIME.elapsed();
    let total_secs = elapsed.as_secs();
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    let millis = elapsed.subsec_millis();

    println!("[{hours:02}:{mins:02}:{secs:02}.{millis:03}]\t{message}");
}

/// [Dishonest / I/O Boundary Driver]
/// Asynchronously reads incoming bytes from an external stream into a buffer
/// and decodes the received slice into a UTF-8 string slice (lossy).
pub async fn read_as_str<'a>(
    source_stream: &mut impl ct::Reader,
    buff: &'a mut [u8],
) -> io::Result<Cow<'a, str>> {
    let n = source_stream.read(buff).await?;
    if n == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "connection closed before data was received",
        ));
    }

    Ok(String::from_utf8_lossy(&buff[..n]))
}

/// [Dishonest / I/O Boundary Adapter]
/// Synchronously checks path safety and opens the file.
pub fn file_check(base_dir: impl AsRef<Path>, url_path: &str) -> Option<File> {
    if url_path.contains("..") {
        return None;
    }
    let clean_path = url_path.trim_start_matches('/');
    let full_path = base_dir.as_ref().join(clean_path);
    let file = File::open(&full_path).ok()?;
    if file.metadata().ok()?.is_file() {
        Some(file)
    } else {
        None
    }
}

pub async fn buf_read(source_stream: &mut impl ct::Reader, buff: &mut [u8]) -> io::Result<usize> {
    let n = source_stream.read(buff).await?;
    if n == 0 {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    Ok(n)
}

/// [Dishonest / Async Timing Adapter]
/// Runs `fut`, or fails with ErrorKind::TimedOut if it hasn't finished within `dur`.
/// The losing future is dropped, which cancels the pending connect / read.
pub async fn with_timeout<T>(
    dur: Duration,
    fut: impl Future<Output = io::Result<T>>,
) -> io::Result<T> {
    fut.or(async {
        Timer::after(dur).await;
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("timed out after {}s", dur.as_secs()),
        ))
    })
    .await
}

/// [Dishonest / Stream Driver]
/// Copies `from` into `to` until EOF; fails with TimedOut if no data arrives for `idle`
/// (an idle limit, so slow but steady transfers are never cut off).
pub async fn copy_with_idle_timeout(
    from: &mut impl ct::Reader,
    to: &mut impl ct::Writer,
    idle: Duration,
) -> io::Result<u64> {
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = with_timeout(idle, from.read(&mut buf)).await?;
        if n == 0 {
            return Ok(total);
        }
        to.write_all(&buf[..n]).await?;
        total += n as u64;
    }
}

/// [Honest / Pure Domain Logic]
/// Extracts HTTP method and request path from the initial request line.
/// If `pat` is `Some(...)` (e.g. `Some("?")`), the path is truncated at the first occurrence of that pattern.
/// If `None`, the raw path is returned unmodified.
pub fn parse_method_path<'a>(
    request_str: &'a str,
    pat: Option<&str>,
) -> Option<(&'a str, &'a str)> {
    let mut words = request_str.lines().next()?.split_whitespace();

    let method = words.next()?;
    let raw_path = words.next()?;

    let path = match pat {
        Some(pat) => raw_path.split(pat).next().unwrap_or(raw_path),
        None => raw_path,
    };

    Some((method, path))
}

/// [Honest / Pure Domain Logic]
/// Deterministically maps known file extensions to HTTP Content-Type strings.
pub fn match_content_type(path: &str) -> &'static str {
    match Path::new(path).extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "application/javascript",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("h264") => "video/h264",
        Some("ivf") => "video/x-ivf",
        Some("ogg") => "audio/ogg",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_parse_method_path() {
        assert_eq!(
            parse_method_path("GET /index.html HTTP/1.1\r\nHost: localhost", None),
            Some(("GET", "/index.html"))
        );
        assert_eq!(
            parse_method_path(
                "GET /shows/f1.json?nocache=1 HTTP/1.1\r\nHost: localhost",
                None
            ),
            Some(("GET", "/shows/f1.json?nocache=1"))
        );
        assert_eq!(
            parse_method_path(
                "GET /shows/f1.json?nocache=1 HTTP/1.1\r\nHost: localhost",
                Some("?")
            ),
            Some(("GET", "/shows/f1.json"))
        );
        assert_eq!(parse_method_path("", None), None);
    }

    #[test]
    fn test_read_as_str_success_and_eof() {
        smol::block_on(async {
            let mut buffer = [0u8; 64];

            // 1. Success case
            let mut mock_stream = b"GET / HTTP/1.1".as_slice();
            let text = read_as_str(&mut mock_stream, &mut buffer).await.unwrap();
            assert_eq!(text, "GET / HTTP/1.1");

            // 2. EOF case (empty input yields UnexpectedEof)
            let mut empty_stream = b"".as_slice();
            let err = read_as_str(&mut empty_stream, &mut buffer)
                .await
                .unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        });
    }

    #[test]
    fn test_with_timeout() {
        smol::block_on(async {
            let fast = with_timeout(Duration::from_millis(200), async { Ok(7) }).await;
            assert_eq!(fast.unwrap(), 7);

            let slow = with_timeout(Duration::from_millis(50), async {
                Timer::after(Duration::from_secs(5)).await;
                Ok(7)
            });
            let t = Instant::now();
            assert_eq!(slow.await.unwrap_err().kind(), io::ErrorKind::TimedOut);
            assert!(t.elapsed() < Duration::from_secs(1));
        });
    }

    #[test]
    fn test_copy_with_idle_timeout() {
        use smol::net::{TcpListener, TcpStream};
        smol::block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();

            // Sender: 3 chunks 100 ms apart, then closes -> copied in full (gaps < idle limit)
            let sender = smol::spawn(async move {
                let (mut s, _) = listener.accept().await.unwrap();
                for _ in 0..3 {
                    s.write_all(&[1u8; 1000]).await.unwrap();
                    Timer::after(Duration::from_millis(100)).await;
                }
                // then stalls without closing -> idle timeout
                let (mut s2, _) = listener.accept().await.unwrap();
                s2.write_all(&[2u8; 10]).await.unwrap();
                Timer::after(Duration::from_secs(5)).await;
                drop((s, s2));
            });

            let mut from = TcpStream::connect(addr).await.unwrap();
            let mut out = Vec::new();
            let copy = copy_with_idle_timeout(&mut from, &mut out, Duration::from_millis(300));
            // first connection stays open after the 3 chunks too, so this ends by idle timeout
            assert_eq!(copy.await.unwrap_err().kind(), io::ErrorKind::TimedOut);
            assert_eq!(out.len(), 3000);

            let mut from2 = TcpStream::connect(addr).await.unwrap();
            let mut out2 = Vec::new();
            let t = Instant::now();
            let err = copy_with_idle_timeout(&mut from2, &mut out2, Duration::from_millis(200))
                .await
                .unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::TimedOut);
            assert_eq!(out2, vec![2u8; 10]);
            assert!(t.elapsed() < Duration::from_secs(1));
            sender.cancel().await;
        });
    }

    #[test]
    fn test_copy_with_idle_timeout_eof() {
        smol::block_on(async {
            let mut from: &[u8] = &[5u8; 200_000];
            let mut out = Vec::new();
            let n = copy_with_idle_timeout(&mut from, &mut out, Duration::from_millis(100))
                .await
                .unwrap();
            assert_eq!((n, out.len()), (200_000, 200_000));
        });
    }
}
