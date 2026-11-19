//! Reads one HTTP/1.1 request from a socket for the local listeners (the extension bridge and the
//! phone link). The head comes first, so a listener can refuse a request before it reads the body.

use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

const MAX_HEADERS: usize = 32 * 1024;
const HEAD_WAIT: Duration = Duration::from_secs(10);
const BODY_WAIT: Duration = Duration::from_secs(30);

/// The request line and the headers that the listeners use.
pub(crate) struct Head {
    pub method: String,
    pub path: String,
    /// The value after `Authorization: Bearer`, or empty.
    pub bearer: String,
    pub origin: String,
    pub content_length: Option<usize>,
    pub chunked: bool,
    /// `X-Batch-Id`: the phone's name for one save, so a repeated save is recognised.
    pub batch: String,
}

/// Reads up to the blank line after the headers. Returns the head and the bytes already read past it.
/// The whole head must arrive within 10 seconds, so a slow sender cannot hold a connection.
pub(crate) async fn read_head(stream: &mut TcpStream) -> Option<(Head, Vec<u8>)> {
    read_head_within(stream, HEAD_WAIT).await
}

pub(crate) async fn read_head_within(stream: &mut TcpStream, deadline: Duration) -> Option<(Head, Vec<u8>)> {
    tokio::time::timeout(deadline, head_bytes(stream)).await.ok()?.and_then(parse_head)
}

async fn head_bytes(stream: &mut TcpStream) -> Option<(Vec<u8>, usize)> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        // Look only at the new bytes and the three before them.
        let from = buf.len().saturating_sub(3);
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf[from..].windows(4).position(|w| w == b"\r\n\r\n") {
            return Some((buf, from + i));
        }
        if buf.len() > MAX_HEADERS {
            return None;
        }
    }
}

fn parse_head((buf, end): (Vec<u8>, usize)) -> Option<(Head, Vec<u8>)> {
    if end > MAX_HEADERS {
        return None;
    }
    let text = String::from_utf8_lossy(&buf[..end]).into_owned();
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let (method, path) = (first.next()?.to_string(), first.next()?.to_string());
    let mut head = Head { method, path, bearer: String::new(), origin: String::new(), content_length: None, chunked: false, batch: String::new() };
    for line in lines {
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "authorization" => head.bearer = value.strip_prefix("Bearer ").unwrap_or("").trim().to_string(),
            "origin" => head.origin = value.to_string(),
            "content-length" => head.content_length = value.parse().ok(),
            "transfer-encoding" => head.chunked = value.to_ascii_lowercase().contains("chunked"),
            "x-batch-id" => head.batch = value.to_string(),
            _ => {}
        }
    }
    if head.chunked {
        head.content_length = None;
    }
    Some((head, buf[end + 4..].to_vec()))
}

/// Reads the rest of a body of `length` bytes. None when the peer closes or waits too long.
pub(crate) async fn read_body(stream: &mut TcpStream, already: Vec<u8>, length: usize) -> Option<Vec<u8>> {
    let mut body = already;
    let mut chunk = [0u8; 64 * 1024];
    while body.len() < length {
        let n = tokio::time::timeout(BODY_WAIT, stream.read(&mut chunk)).await.ok()?.ok()?;
        if n == 0 {
            return None;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(length);
    Some(body)
}

/// Compares every byte, so the time does not tell where two tokens differ.
pub(crate) fn same_token(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use tokio::io::AsyncWriteExt;
    use tokio::net::{TcpListener, TcpStream};

    /// A connected pair: what the test client writes arrives at `server`.
    async fn pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();
        (client, server)
    }

    #[test]
    fn head_comes_back_without_the_body() {
        tauri::async_runtime::block_on(async {
            let (mut client, mut server) = pair().await;
            client
                .write_all(b"POST /api/read HTTP/1.1\r\nAuthorization: Bearer tok\r\nContent-Length: 10485760\r\n\r\n")
                .await
                .unwrap();
            let started = Instant::now();
            let (head, already) = read_head(&mut server).await.expect("a head");
            assert!(started.elapsed() < Duration::from_secs(2));
            assert_eq!(head.method, "POST");
            assert_eq!(head.path, "/api/read");
            assert_eq!(head.bearer, "tok");
            assert_eq!(head.content_length, Some(10_485_760));
            assert!(!head.chunked);
            assert!(already.is_empty());
            drop(client);
        });
    }

    #[test]
    fn bytes_after_the_head_are_kept() {
        tauri::async_runtime::block_on(async {
            let (mut client, mut server) = pair().await;
            client
                .write_all(b"POST /v1/items HTTP/1.1\r\nauthorization: Bearer tok\r\nOrigin: chrome-extension://x\r\ncontent-length: 5\r\n\r\nabc")
                .await
                .unwrap();
            let (head, already) = read_head(&mut server).await.expect("a head");
            assert_eq!(head.origin, "chrome-extension://x");
            assert_eq!(head.bearer, "tok");
            assert_eq!(already, b"abc");
            client.write_all(b"de").await.unwrap();
            let body = read_body(&mut server, already, 5).await.expect("the body");
            assert_eq!(body, b"abcde");
        });
    }

    #[test]
    fn a_body_cut_short_is_none() {
        tauri::async_runtime::block_on(async {
            let (mut client, mut server) = pair().await;
            client.write_all(b"ab").await.unwrap();
            drop(client);
            assert!(read_body(&mut server, Vec::new(), 5).await.is_none());
        });
    }

    #[test]
    fn the_batch_id_is_read() {
        tauri::async_runtime::block_on(async {
            let (mut client, mut server) = pair().await;
            client.write_all(b"POST /api/save HTTP/1.1\r\nX-Batch-Id: 0123abcd\r\nContent-Length: 0\r\n\r\n").await.unwrap();
            let (head, _) = read_head(&mut server).await.expect("a head");
            assert_eq!(head.batch, "0123abcd");
        });
    }

    #[test]
    fn chunked_is_flagged() {
        tauri::async_runtime::block_on(async {
            let (mut client, mut server) = pair().await;
            client
                .write_all(b"POST /api/save HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n")
                .await
                .unwrap();
            let (head, _) = read_head(&mut server).await.expect("a head");
            assert!(head.chunked);
            assert_eq!(head.content_length, None);
            assert_eq!(head.bearer, "");
        });
    }

    #[test]
    fn oversized_headers_are_refused() {
        tauri::async_runtime::block_on(async {
            let (mut client, mut server) = pair().await;
            let mut head = b"GET / HTTP/1.1\r\n".to_vec();
            while head.len() < 40 * 1024 {
                head.extend_from_slice(b"X-Filler: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n");
            }
            let writer = tokio::spawn(async move {
                let _ = client.write_all(&head).await;
                client
            });
            assert!(read_head(&mut server).await.is_none());
            drop(writer);
        });
    }

    #[test]
    fn a_slow_head_is_dropped_at_the_total_deadline() {
        tauri::async_runtime::block_on(async {
            let (mut client, mut server) = pair().await;
            let writer = tokio::spawn(async move {
                for byte in b"GET / HTTP/1.1\r\nX: ".iter().chain(std::iter::repeat(&b'a').take(40)) {
                    if client.write_all(&[*byte]).await.is_err() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            });
            let started = Instant::now();
            assert!(read_head_within(&mut server, Duration::from_millis(300)).await.is_none());
            assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
            writer.abort();
        });
    }

    #[test]
    fn same_token_compares_whole_strings() {
        assert!(same_token("abc", "abc"));
        assert!(!same_token("abc", "abd"));
        assert!(!same_token("abc", "ab"));
        assert!(!same_token("", "x"));
    }
}
