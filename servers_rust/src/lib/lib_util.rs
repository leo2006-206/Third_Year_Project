use std::borrow::Cow;
use std::io;

use smol::io::AsyncReadExt;

use crate::lib_core_type as ct;

/// [Honest / Pure Domain Logic]
/// Extracts HTTP method and request path from the initial request line.
pub fn parse_method_path(request_str: &str) -> Option<(&str, &str)> {
    let mut words = request_str.lines().next()?.split_whitespace();

    let method = words.next()?;
    let path = words.next()?;

    Some((method, path))
}

/// [Dishonest / I/O Boundary Driver]
/// Asynchronously reads incoming bytes from an external stream into a buffer
/// and decodes the received slice into a UTF-8 string slice (lossy).
///
/// Dishonest because output depends on non-deterministic external network I/O
/// and remote peer timing, and mutates the reader stream state.
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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_parse_method_path() {
        assert_eq!(
            parse_method_path("GET /index.html HTTP/1.1\r\nHost: localhost"),
            Some(("GET", "/index.html"))
        );
        assert_eq!(parse_method_path(""), None);
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
}
