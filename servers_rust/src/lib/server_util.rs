// src/lib/server_util.rs

use std::io;

use smol::{
    io::{
        // AsyncReadExt,
        AsyncWriteExt,
    },
    net::TcpStream,
};

pub async fn tcp_send_message(
    client: &mut TcpStream,
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

    client.write_all(header.as_bytes()).await?;
    client.write_all(body.as_bytes()).await?;

    client.flush().await
}

pub async fn tcp_send_ok_utf8(
    client: &mut TcpStream,
    content_type: &str,
    body: &str,
) -> io::Result<()> {
    tcp_send_message(client, "200 OK", content_type, "charset=utf-8", body).await
}

pub async fn tcp_send_404(client: &mut TcpStream) -> io::Result<()> {
    tcp_send_message(
        client,
        "404 Not Found",
        "text/plain",
        "charset=utf-8",
        "404 Not Found",
    )
    .await
}

pub async fn tcp_send_bytes(
    client: &mut TcpStream,
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
    client.write_all(header.as_bytes()).await?;
    client.write_all(body).await?;
    client.flush().await
}

pub fn parse_method_path(request_str: &str) -> Option<(&str, &str)> {
    let mut words = request_str.lines().next()?.split_whitespace();

    let method = words.next()?;
    let path = words.next()?;

    Some((method, path))
}
