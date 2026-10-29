// src/lib/lib_http.rs

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::sync::Arc;

use smol::Async;
use smol::io::{AsyncReadExt, AsyncWriteExt, Cursor};
use smol::net::TcpStream;

use crate::lib_core_type as ct;

/// [Dishonest / I/O Boundary Driver]
/// Writes and flushes raw bytes to the output stream.
pub async fn send_raw(data_stream: &mut impl ct::Writer, raw_data: &[u8]) -> io::Result<()> {
    data_stream.write_all(raw_data).await?;
    data_stream.flush().await?;
    Ok(())
}

/// [Dishonest / I/O Boundary Driver]
/// Reuses `format_response_header` to write HTTP 200 headers, then sends the file body
/// with zero copy (`send_file_zc`): the file is never loaded into RAM or copied through user space.
pub async fn send_file(
    data_stream: &mut TcpStream,
    file: File,
    content_type: &str,
    extra_headers: &[(&str, &str)],
) -> io::Result<()> {
    let file_len = file.metadata()?.len();
    let header = format_response_header("200 OK", content_type, file_len, extra_headers);
    send_raw(data_stream, &header).await?;

    send_file_zc(data_stream, &file).await?;
    Ok(())
}

/// [Dishonest / I/O Boundary Driver]
/// Reads `stream` until the end of the HTTP head and splits it from the body.
/// Returns the head bytes WITHOUT the final "\r\n\r\n" (the form `response_append` takes) and a
/// reader for the body: the body bytes that arrived in the same read as the head first, then
/// the rest of `stream`. The head can be worked on while the body keeps arriving in the socket.
/// Errors: UnexpectedEof if the stream ends before the head does, InvalidData if the head is
/// larger than 16 KB.
pub async fn split_header_body<R: ct::Reader>(
    stream: &mut R,
) -> io::Result<(Vec<u8>, impl ct::Reader + '_)> {
    const MAX_HEADER: usize = 16 * 1024;

    let mut buf = vec![0u8; MAX_HEADER];
    let mut n = 0;
    loop {
        if n == buf.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP head larger than 16 KB",
            ));
        }
        let m = stream.read(&mut buf[n..]).await?;
        if m == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        // The delimiter may straddle two reads: also look 3 bytes back into the previous one
        let from = n.saturating_sub(3);
        n += m;
        if let Some(i) = buf[from..n].windows(4).position(|w| w == b"\r\n\r\n") {
            let pos = from + i;
            let leftover = buf[pos + 4..n].to_vec();
            buf.truncate(pos);
            return Ok((buf, Cursor::new(leftover).chain(stream)));
        }
    }
}

/// [Dishonest / I/O Boundary Driver]
/// Sends the whole of `file` to `stream` with zero copy: the kernel moves page-cache pages
/// straight to the socket (`sendfile`), with no read into user space and no blocking thread.
/// Sends only the file bytes (any HTTP headers go first, e.g. via `send_raw`). Uses its own
/// offset, so the file's read position is untouched and one file can be sent to many streams
/// at once. Returns the number of bytes sent.
pub async fn send_file_zc(stream: &TcpStream, file: &File) -> io::Result<u64> {
    // Linux sends at most this much per sendfile call
    const MAX_CHUNK: usize = 0x7fff_f000;

    let len = file.metadata()?.len();
    let socket: Arc<Async<std::net::TcpStream>> = stream.clone().into();
    let mut offset: libc::off_t = 0;

    while (offset as u64) < len {
        let remaining = ((len - offset as u64) as usize).min(MAX_CHUNK);
        // write_with retries the call whenever the socket buffer is full (EAGAIN)
        let sent = socket
            .write_with(|sock| {
                // SAFETY: both descriptors stay open for the call; `offset` is a valid off_t
                let n = unsafe {
                    libc::sendfile(sock.as_raw_fd(), file.as_raw_fd(), &mut offset, remaining)
                };
                if n < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(n as usize)
                }
            })
            .await?;
        if sent == 0 {
            // The file got shorter than its metadata said: stop instead of looping forever
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
    }
    Ok(len)
}

/// [Honest / Pure Domain Logic]
/// Body length from an HTTP head's Content-Length header (name matched case-insensitively,
/// e.g. Dana sends "Content-length"). None if absent or not a number.
pub fn parse_content_length(head: &[u8]) -> Option<u64> {
    String::from_utf8_lossy(head)
        .lines()
        .skip(1) // status line
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse().ok())
}

/// [Honest / Pure Domain Logic]
/// Deterministically formats HTTP/1.1 response status line and headers into a byte buffer.
pub fn format_response_header(
    status_code: &str,
    content_type: &str,
    content_length: u64,
    extra_headers: &[(&str, &str)],
) -> Vec<u8> {
    use std::io::Write;

    let headers_len: usize = extra_headers
        .iter()
        .map(|(k, v)| k.len() + v.len() + 4) // ": " + "\r\n"
        .sum();

    let mut response = Vec::with_capacity(128 + headers_len);

    let _ = write!(
        &mut response,
        "HTTP/1.1 {status_code}\r\n\
        Content-Type: {content_type}\r\n\
        Content-Length: {content_length}\r\n\
        Connection: close\r\n",
    );

    for (key, value) in extra_headers {
        let _ = write!(&mut response, "{key}: {value}\r\n");
    }
    response.extend_from_slice(b"\r\n");

    response
}

/// [Honest / Pure Domain Logic]
/// Deterministically formats an HTTP/1.1 response buffer with status line,
/// headers, and payload body.
pub fn format_response(
    status_code: &str,
    content_type: &str,
    extra_headers: &[(&str, &str)],
    body: &[u8],
) -> Vec<u8> {
    let mut response =
        format_response_header(status_code, content_type, body.len() as u64, extra_headers);
    response.extend_from_slice(body);
    response
}

/// [Honest / Pure Domain Logic]
/// Convenience helper constructing a 200 OK HTTP response.
pub fn response_ok_utf8(
    content_type: &str,
    extra_headers: &[(&str, &str)],
    body: &[u8],
) -> Vec<u8> {
    format_response("200 OK", content_type, extra_headers, body)
}

/// [Honest / Pure Domain Logic]
/// Convenience helper constructing a standard 404 Not Found HTTP response.
pub fn response_404() -> Vec<u8> {
    format_response(
        "404 Not Found",
        "text/plain; charset=utf-8",
        &[],
        b"404 Not Found",
    )
}

/// [Honest / Pure Domain Logic]
/// Response when an offload site fails: 504 if it didn't answer in time, 502 for any other error
/// (e.g. connection refused).
pub fn response_gateway_error(timed_out: bool) -> Vec<u8> {
    let status = if timed_out {
        "504 Gateway Timeout"
    } else {
        "502 Bad Gateway"
    };
    format_response(status, "text/plain; charset=utf-8", &[], status.as_bytes())
}

/// [Honest / Pure Domain Logic]
/// Constructs a 200 OK response with cross-origin headers required for binary assets (e.g. WASM).
pub fn response_bytes(content_type: &str, extra_headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    const DEFAULT_HEADERS: &[(&str, &str)] = &[
        ("Access-Control-Allow-Origin", "*"),
        ("Cross-Origin-Embedder-Policy", "require-corp"),
        ("Cross-Origin-Opener-Policy", "same-origin"),
    ];

    if extra_headers.is_empty() {
        format_response("200 OK", content_type, DEFAULT_HEADERS, body)
    } else {
        let headers: Vec<(&str, &str)> = DEFAULT_HEADERS
            .iter()
            .copied()
            .chain(extra_headers.iter().copied())
            .collect();
        format_response("200 OK", content_type, &headers, body)
    }
}

pub fn response_append(header: &str, extra_headers: &[(&str, &str)]) -> Vec<u8> {
    use std::io::Write;

    let mut response = Vec::with_capacity(header.len());

    let _ = write!(&mut response, "{header}\r\n");
    for (key, value) in extra_headers {
        let _ = write!(&mut response, "{key}: {value}\r\n");
    }
    let _ = write!(&mut response, "\r\n");

    response
}

/// [Honest / Pure Domain Logic]
/// Deterministically formats an HTTP/1.1 bodyless request buffer (e.g. GET/HEAD).
pub fn request_bodyless(
    method: &str,
    method_path: &str,
    host: &str,
    user_agent: &str,
    extra_headers: &[(&str, &str)],
) -> Vec<u8> {
    use std::io::Write;

    let headers_len: usize = extra_headers
        .iter()
        .map(|(k, v)| k.len() + v.len() + 4)
        .sum();

    let mut request = Vec::with_capacity(128 + headers_len);

    let _ = write!(
        &mut request,
        "{method} {method_path} HTTP/1.1\r\n\
        Host: {host}\r\n\
        User-Agent: {user_agent}\r\n\
        Connection: close\r\n",
    );

    for (key, value) in extra_headers {
        let _ = write!(&mut request, "{key}: {value}\r\n");
    }
    request.extend_from_slice(b"\r\n");

    request
}

/// [Honest / Pure Domain Logic]
/// Formats an HTTP GET request buffer.
pub fn request_get(
    get_path: &str,
    host: &str,
    user_agent: &str,
    extra_headers: &[(&str, &str)],
) -> Vec<u8> {
    request_bodyless("GET", get_path, host, user_agent, extra_headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test reader that returns at most `chunk` bytes per read, like a socket delivering
    /// a response in pieces.
    struct ChunkedReader {
        data: Vec<u8>,
        pos: usize,
        chunk: usize,
    }

    impl smol::io::AsyncRead for ChunkedReader {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            out: &mut [u8],
        ) -> std::task::Poll<io::Result<usize>> {
            let n = self.chunk.min(out.len()).min(self.data.len() - self.pos);
            let start = self.pos;
            out[..n].copy_from_slice(&self.data[start..start + n]);
            self.pos += n;
            std::task::Poll::Ready(Ok(n))
        }
    }

    fn split_in_chunks(data: &[u8], chunk: usize) -> io::Result<(Vec<u8>, Vec<u8>)> {
        smol::block_on(async {
            let mut reader = ChunkedReader {
                data: data.to_vec(),
                pos: 0,
                chunk,
            };
            let (head, mut body) = split_header_body(&mut reader).await?;
            let mut rest = Vec::new();
            body.read_to_end(&mut rest).await?;
            Ok((head, rest))
        })
    }

    #[test]
    fn test_parse_content_length() {
        assert_eq!(
            parse_content_length(b"HTTP/1.0 200 OK\r\nServer: x\r\nContent-length: 2229297\r\nContent-type: h264"),
            Some(2229297)
        );
        assert_eq!(parse_content_length(b"HTTP/1.1 200 OK\r\nCONTENT-LENGTH:  42 "), Some(42));
        assert_eq!(parse_content_length(b"HTTP/1.1 200 OK\r\nServer: x"), None);
        assert_eq!(parse_content_length(b"HTTP/1.1 200 OK\r\nContent-Length: abc"), None);
    }

    #[test]
    fn test_split_header_body() {
        let response = b"HTTP/1.1 200 OK\r\nX-Dana-Timings: queue=1\r\n\r\nBODY bytes";
        // whole response in one read, one byte at a time (delimiter split over 4 reads),
        // and chunk sizes that cut the delimiter at every possible place
        for chunk in [usize::MAX, 1, 2, 3, 5, 7, 41, 43] {
            let (head, body) = split_in_chunks(response, chunk).unwrap();
            assert_eq!(head, b"HTTP/1.1 200 OK\r\nX-Dana-Timings: queue=1", "chunk {chunk}");
            assert_eq!(body, b"BODY bytes", "chunk {chunk}");
        }
    }

    #[test]
    fn test_split_header_body_large_and_empty_body() {
        let mut response = b"HTTP/1.1 200 OK\r\nContent-Length: 100000\r\n\r\n".to_vec();
        let body: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        response.extend_from_slice(&body);
        let (_, got) = split_in_chunks(&response, 7000).unwrap();
        assert_eq!(got, body);

        let (head, got) = split_in_chunks(b"HTTP/1.1 204 No Content\r\n\r\n", 4096).unwrap();
        assert_eq!(head, b"HTTP/1.1 204 No Content");
        assert!(got.is_empty());
    }

    #[test]
    fn test_split_header_body_errors() {
        let truncated = split_in_chunks(b"HTTP/1.1 200 OK\r\nX-A: b\r\n", 8);
        assert_eq!(truncated.unwrap_err().kind(), io::ErrorKind::UnexpectedEof);

        let huge = vec![b'a'; 20_000];
        let oversized = split_in_chunks(&huge, 4096);
        assert_eq!(oversized.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn test_format_response() {
        let resp = format_response("200 OK", "text/plain", &[("X-Test", "123")], b"hello");
        let resp_str = String::from_utf8(resp).unwrap();
        assert!(resp_str.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(resp_str.contains("Content-Type: text/plain\r\n"));
        assert!(resp_str.contains("Content-Length: 5\r\n"));
        assert!(resp_str.contains("X-Test: 123\r\n"));
        assert!(resp_str.ends_with("\r\n\r\nhello"));
    }

    #[test]
    fn test_response_ok_utf8() {
        let resp = response_ok_utf8("text/html", &[], b"<h1>OK</h1>");
        let resp_str = String::from_utf8(resp).unwrap();
        assert!(resp_str.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(resp_str.contains("Content-Type: text/html\r\n"));
        assert!(resp_str.ends_with("\r\n\r\n<h1>OK</h1>"));
    }

    #[test]
    fn test_response_404() {
        let resp = response_404();
        let resp_str = String::from_utf8(resp).unwrap();
        assert!(resp_str.starts_with("HTTP/1.1 404 Not Found\r\n"));
        assert!(resp_str.ends_with("\r\n\r\n404 Not Found"));
    }

    #[test]
    fn test_response_bytes() {
        let resp = response_bytes("application/wasm", &[], b"\0asm");
        let resp_str = String::from_utf8_lossy(&resp);
        assert!(resp_str.contains("Access-Control-Allow-Origin: *\r\n"));
        assert!(resp_str.contains("Cross-Origin-Embedder-Policy: require-corp\r\n"));
        assert!(resp_str.contains("Cross-Origin-Opener-Policy: same-origin\r\n"));
    }

    #[test]
    fn test_request_bodyless() {
        let req = request_bodyless(
            "HEAD",
            "/status",
            "localhost",
            "TestAgent",
            &[("X-Custom", "val")],
        );
        let req_str = String::from_utf8(req).unwrap();
        assert!(req_str.starts_with("HEAD /status HTTP/1.1\r\n"));
        assert!(req_str.contains("Host: localhost\r\n"));
        assert!(req_str.contains("User-Agent: TestAgent\r\n"));
        assert!(req_str.contains("X-Custom: val\r\n"));
        assert!(req_str.ends_with("\r\n\r\n"));
    }

    #[test]
    fn test_request_get() {
        let req = request_get("/index.html", "localhost", "TestAgent", &[]);
        let req_str = String::from_utf8(req).unwrap();
        assert!(req_str.starts_with("GET /index.html HTTP/1.1\r\n"));
        assert!(req_str.contains("Host: localhost\r\n"));
        assert!(req_str.contains("User-Agent: TestAgent\r\n"));
        assert!(req_str.ends_with("\r\n\r\n"));
    }

    #[test]
    fn test_send_raw() {
        smol::block_on(async {
            use smol::io::Cursor;
            let mut cursor = Cursor::new(Vec::new());
            let data = b"test raw bytes";
            send_raw(&mut cursor, data).await.unwrap();
            assert_eq!(cursor.into_inner(), data);
        });
    }

    #[test]
    fn test_send_file_zc() {
        use smol::io::AsyncReadExt;
        use smol::net::TcpListener;
        use std::io::Write;

        // 3 MB with a position-dependent pattern: larger than the socket buffer, so the
        // sendfile loop has to wait for the reader (EAGAIN) several times
        let data: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        let path = std::env::temp_dir().join(format!("obm_send_file_zc_{}", std::process::id()));
        std::fs::File::create(&path).unwrap().write_all(&data).unwrap();
        let empty_path = path.with_extension("empty");
        std::fs::File::create(&empty_path).unwrap();

        smol::block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();

            let reader = smol::spawn(async move {
                let mut client = TcpStream::connect(addr).await.unwrap();
                smol::Timer::after(std::time::Duration::from_millis(50)).await; // slow start
                let mut received = Vec::new();
                client.read_to_end(&mut received).await.unwrap();
                received
            });

            let (server, _) = listener.accept().await.unwrap();
            let file = std::fs::File::open(&path).unwrap();
            assert_eq!(send_file_zc(&server, &file).await.unwrap(), 3_000_000);
            // an empty file sends nothing and returns at once
            let empty = std::fs::File::open(&empty_path).unwrap();
            assert_eq!(send_file_zc(&server, &empty).await.unwrap(), 0);
            drop(server);

            assert!(reader.await == data);
        });
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_file(&empty_path).unwrap();
    }

    #[test]
    fn test_send_file() {
        use smol::io::AsyncReadExt;
        use smol::net::TcpListener;

        smol::block_on(async {
            let file = std::fs::File::open("Cargo.toml").unwrap();
            let file_len = file.metadata().unwrap().len();
            let expected_body = std::fs::read("Cargo.toml").unwrap();

            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let reader = smol::spawn(async move {
                let mut client = TcpStream::connect(addr).await.unwrap();
                let mut received = Vec::new();
                client.read_to_end(&mut received).await.unwrap();
                received
            });

            let (mut server, _) = listener.accept().await.unwrap();
            send_file(
                &mut server,
                file,
                "text/plain",
                &[("X-Custom", "file-test")],
            )
            .await
            .unwrap();
            drop(server);

            let output = reader.await;
            let output_str = String::from_utf8_lossy(&output);
            // headers first, then exactly the file's bytes
            let header_end = output.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
            assert_eq!(&output[header_end..], &expected_body[..]);

            assert!(output_str.starts_with("HTTP/1.1 200 OK\r\n"));
            assert!(output_str.contains("Content-Type: text/plain\r\n"));
            assert!(output_str.contains(&format!("Content-Length: {file_len}\r\n")));
            assert!(output_str.contains("X-Custom: file-test\r\n"));
            assert!(output_str.contains("[package]"));
        });
    }

    #[test]
    fn test_response_append() {
        let base_header = "HTTP/1.0 200 OK\r\nServer: Dana";
        let extra = [("X-OBM-Net", "42"), ("Server-Timing", "net;dur=42")];
        let res = response_append(base_header, &extra);
        let s = String::from_utf8(res).unwrap();
        assert_eq!(
            s,
            "HTTP/1.0 200 OK\r\nServer: Dana\r\nX-OBM-Net: 42\r\nServer-Timing: net;dur=42\r\n\r\n"
        );
    }
}
