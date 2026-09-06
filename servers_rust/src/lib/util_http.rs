// src/lib/util_http.rs

use std::io;

use smol::{
    io::{
        // AsyncReadExt,
        AsyncWriteExt,
    },
    net::TcpStream,
};

pub async fn response(
    dest_stream: &mut TcpStream,
    status_code: &str,
    content_type: &str,
    text_format: &str,
    body: &str,
) -> io::Result<()> {
    let header = format!(
        "HTTP/1.1 {}\r\n\
        Content-Type: {}; {}\r\n\
        Content-Length: {}\r\n\
        Connection: close\r\n\
        \r\n\
		",
        status_code,
        content_type,
        text_format,
        body.len(),
    );

    dest_stream.write_all(header.as_bytes()).await?;
    dest_stream.write_all(body.as_bytes()).await?;

    dest_stream.flush().await
}

pub async fn response_ok_utf8(
    dest_stream: &mut TcpStream,
    content_type: &str,
    body: &str,
) -> io::Result<()> {
    response(dest_stream, "200 OK", content_type, "charset=utf-8", body).await
}

pub async fn response_404(dest_stream: &mut TcpStream) -> io::Result<()> {
    response(
        dest_stream,
        "404 Not Found",
        "text/plain",
        "charset=utf-8",
        "404 Not Found",
    )
    .await
}

pub async fn response_bytes(
    dest_stream: &mut TcpStream,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    let header = format!(
        "HTTP/1.1 200 OK\r\n\
        Content-Type: {}\r\n\
        Content-Length: {}\r\n\
        Access-Control-Allow-Origin: *\r\n\
        Cross-Origin-Embedder-Policy: require-corp\r\n\
        Cross-Origin-Opener-Policy: same-origin\r\n\
        Connection: close\r\n\
        \r\n",
        content_type,
        body.len()
    );
    dest_stream.write_all(header.as_bytes()).await?;
    dest_stream.write_all(body).await?;
    dest_stream.flush().await
}

pub async fn request_bodyless(
    dest_stream: &mut TcpStream,
    method: &str,
    method_path: &str,
    host: &str,
) -> io::Result<()> {
    let request = format!(
        "{} {} HTTP/1.1\r\n\
        Host: {}\r\n\
        User-Agent: LoadBalancer/1.0\r\n\
        Connection: close\r\n\
        \r\n",
        method, method_path, host
    );

    dest_stream.write_all(request.as_bytes()).await?;
    dest_stream.flush().await
}

pub async fn request_get(
    dest_stream: &mut TcpStream,
    get_path: &str,
    host: &str,
) -> io::Result<()> {
    request_bodyless(dest_stream, "GET", get_path, host).await
}

pub fn parse_method_path(request_str: &str) -> Option<(&str, &str)> {
    let mut words = request_str.lines().next()?.split_whitespace();

    let method = words.next()?;
    let path = words.next()?;

    Some((method, path))
}
