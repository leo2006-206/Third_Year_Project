// src/lib/lib_http.rs

use std::io;

use smol::io::AsyncWriteExt;

use crate::lib_core_type as ct;

/// [Honest / Pure Domain Logic]
/// Deterministically formats an HTTP/1.1 response buffer with status line,
/// headers, and payload body.
pub fn format_response(
    status_code: &str,
    content_type: &str,
    extra_headers: &[(&str, &str)],
    body: &[u8],
) -> Vec<u8> {
    use std::io::Write;

    let headers_len: usize = extra_headers
        .iter()
        .map(|(k, v)| k.len() + v.len() + 4) // ": " + "\r\n"
        .sum();

    let mut response = Vec::with_capacity(128 + headers_len + body.len());

    let _ = write!(
        &mut response,
        "HTTP/1.1 {status_code}\r\n\
        Content-Type: {content_type}\r\n\
        Content-Length: {}\r\n\
        Connection: close\r\n",
        body.len(),
    );

    for (key, value) in extra_headers {
        let _ = write!(&mut response, "{key}: {value}\r\n");
    }
    response.extend_from_slice(b"\r\n");
    response.extend_from_slice(body);

    response
}

/// [Dishonest / I/O Boundary Driver]
/// Writes and flushes raw bytes to the output stream.
pub async fn send_raw_response(
    data_stream: &mut impl ct::Writer,
    raw_response: &[u8],
) -> io::Result<()> {
    data_stream.write_all(raw_response).await?;
    data_stream.flush().await?;
    Ok(())
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

/// [Honest / Pure Domain Logic]
/// Deterministically formats an HTTP/1.1 bodyless request buffer (e.g. GET/HEAD).
pub fn format_request_bodyless(
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

/// [Dishonest / I/O Boundary Driver]
/// Formats and sends a bodyless HTTP request over the stream.
pub async fn request_bodyless(
    dest_stream: &mut impl ct::Writer,
    method: &str,
    method_path: &str,
    host: &str,
    user_agent: &str,
) -> io::Result<()> {
    let raw = format_request_bodyless(method, method_path, host, user_agent, &[]);
    send_raw_response(dest_stream, &raw).await
}

/// [Dishonest / I/O Boundary Driver]
/// Formats and sends an HTTP GET request over the stream.
pub async fn request_get(
    dest_stream: &mut impl ct::Writer,
    get_path: &str,
    host: &str,
    user_agent: &str,
) -> io::Result<()> {
    request_bodyless(dest_stream, "GET", get_path, host, user_agent).await
}

/// [Honest / Pure Domain Logic]
/// Extracts HTTP method and request path from the initial request line.
pub fn parse_method_path(request_str: &str) -> Option<(&str, &str)> {
    let mut words = request_str.lines().next()?.split_whitespace();

    let method = words.next()?;
    let path = words.next()?;

    Some((method, path))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_format_request_bodyless() {
        let req = format_request_bodyless("GET", "/index.html", "localhost", "TestAgent", &[]);
        let req_str = String::from_utf8(req).unwrap();
        assert!(req_str.starts_with("GET /index.html HTTP/1.1\r\n"));
        assert!(req_str.contains("Host: localhost\r\n"));
        assert!(req_str.contains("User-Agent: TestAgent\r\n"));
        assert!(req_str.ends_with("\r\n\r\n"));
    }

    #[test]
    fn test_parse_method_path() {
        assert_eq!(
            parse_method_path("GET /index.html HTTP/1.1\r\nHost: localhost"),
            Some(("GET", "/index.html"))
        );
        assert_eq!(parse_method_path(""), None);
    }
}
