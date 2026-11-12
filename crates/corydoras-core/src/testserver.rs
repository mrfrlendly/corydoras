//! A small HTTP server for tests. Each request goes to a handler that returns the
//! answer, and the server keeps what it received, so a test can check both sides.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// One request as the server received it.
#[derive(Clone, Debug, Default)]
pub struct Req {
    pub method: String,
    /// The path without the query.
    pub path: String,
    /// Header names in lower case.
    pub headers: HashMap<String, String>,
    pub body: String,
}

impl Req {
    /// The fields of a form-encoded body.
    pub fn form(&self) -> HashMap<String, String> {
        pairs(&self.body)
    }

    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }

    pub fn header(&self, name: &str) -> &str {
        self.headers.get(name).map(String::as_str).unwrap_or("")
    }
}

pub struct Resp {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
}

impl Resp {
    pub fn json(status: u16, v: Value) -> Self {
        Resp { status, content_type: "application/json", body: v.to_string() }
    }

    /// A server-sent event stream: one `data:` line for each value.
    pub fn sse(events: &[Value]) -> Self {
        let body = events.iter().map(|e| format!("data: {e}\n\n")).collect();
        Resp { status: 200, content_type: "text/event-stream", body }
    }

    pub fn empty(status: u16) -> Self {
        Resp { status, content_type: "text/plain", body: String::new() }
    }
}

pub struct Server {
    /// "http://127.0.0.1:PORT"
    pub base: String,
    seen: Arc<Mutex<Vec<Req>>>,
}

impl Server {
    /// The requests so far, oldest first.
    pub fn seen(&self) -> Vec<Req> {
        self.seen.lock().unwrap().clone()
    }

    /// The requests to one path.
    pub fn seen_at(&self, path: &str) -> Vec<Req> {
        self.seen().into_iter().filter(|r| r.path == path).collect()
    }
}

/// The pairs of a query string or a form body.
pub fn pairs(s: &str) -> HashMap<String, String> {
    reqwest::Url::parse(&format!("http://x/?{s}"))
        .map(|u| u.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect())
        .unwrap_or_default()
}

/// The query of a URL as pairs.
pub fn query_of(url: &str) -> HashMap<String, String> {
    reqwest::Url::parse(url)
        .map(|u| u.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect())
        .unwrap_or_default()
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

async fn read(stream: &mut tokio::net::TcpStream) -> Option<Req> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, target) = (first.next()?.to_string(), first.next()?.to_string());
    let headers: HashMap<String, String> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let len: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    let path = target.split('?').next().unwrap_or("");
    Some(Req {
        method,
        path: path.to_string(),
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

/// Starts the server on a free port of this computer. It runs until the test ends.
pub async fn serve(handler: impl Fn(&Req) -> Resp + Send + Sync + 'static) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen: Arc<Mutex<Vec<Req>>> = Arc::default();
    let handler = Arc::new(handler);
    let log = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let (handler, log) = (handler.clone(), log.clone());
            tokio::spawn(async move {
                let Some(req) = read(&mut stream).await else { return };
                let resp = handler(&req);
                log.lock().unwrap().push(req);
                let head = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    resp.status,
                    reason(resp.status),
                    resp.content_type,
                    resp.body.len()
                );
                let _ = stream.write_all(head.as_bytes()).await;
                let _ = stream.write_all(resp.body.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    Server { base, seen }
}
