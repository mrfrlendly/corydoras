//! The phone link's HTTP answers. Only the page shell and its scripts are public; every route that
//! reads, spends provider tokens or saves needs the token, checked before the body is read.

use crate::http;
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

pub(crate) type Reply = Result<Value, (u16, String)>;
pub(crate) type BoxFut<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// The largest photo the phone may send to be read.
pub(crate) const MAX_READ_BODY: usize = 12 * 1024 * 1024;
/// A save carries the photo and the reviewed fields.
pub(crate) const MAX_SAVE_BODY: usize = MAX_READ_BODY + 512 * 1024;

const DISCONNECTED: &str = "This phone is no longer connected. Open Corydoras on your Mac, go to Settings › Phone, and scan the code.";

/// What the routes do. The app implements it on the open library; the tests use a stand-in.
pub(crate) trait Backend: Send + Sync + 'static {
    fn status(&self) -> Reply;
    fn read(&self, photo: Vec<u8>) -> BoxFut<Reply>;
    /// `batch` names one save on the phone. A save with a batch that was saved before must not save again.
    fn save(&self, frame: Vec<u8>, batch: Option<String>) -> BoxFut<Reply>;
    /// Bytes and MIME type of a built file, by its path in `dist/` ("phone.html", "assets/x.js").
    fn asset(&self, path: &str) -> Option<(Vec<u8>, String)>;
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        411 => "Length Required",
        413 => "Payload Too Large",
        _ => "Internal Server Error",
    }
}

fn answer(status: u16, content_type: &str, body: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        reason(status),
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

fn json_answer(status: u16, value: &Value) -> Vec<u8> {
    answer(status, "application/json", value.to_string().as_bytes())
}

fn error(status: u16, text: &str) -> Vec<u8> {
    json_answer(status, &json!({ "error": text }))
}

fn not_found() -> Vec<u8> {
    error(404, "Not found.")
}

/// A batch id from the page: 16 to 64 letters, digits or hyphens. Anything else is ignored.
fn valid_batch(id: &str) -> Option<String> {
    ((16..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')).then(|| id.to_string())
}

/// A built script or stylesheet of the page: one plain file name, no folders.
fn asset_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains("..")
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && (name.ends_with(".js") || name.ends_with(".css"))
}

fn asset<B: Backend>(backend: &B, name: &str) -> Vec<u8> {
    if !asset_name(name) {
        return not_found();
    }
    match backend.asset(&format!("assets/{name}")) {
        // Tauri answers an unknown path with the desktop page; that is not one of ours.
        Some((bytes, mime)) if !mime.starts_with("text/html") => answer(200, &mime, &bytes),
        _ => not_found(),
    }
}

fn shell<B: Backend>(backend: &B) -> Vec<u8> {
    match backend.asset("phone.html") {
        Some((bytes, _)) => answer(200, "text/html; charset=utf-8", &bytes),
        None => error(404, "The phone page is not built into this copy of Corydoras."),
    }
}

fn replied(reply: Reply) -> Vec<u8> {
    match reply {
        Ok(value) => json_answer(200, &value),
        Err((status, text)) => error(status, &text),
    }
}

/// Most connections answered at once. A device that opens more gets them closed, so it cannot use up
/// the app's open files.
pub(crate) const MAX_CONNECTIONS: usize = 16;

/// Accepts connections until `stop` turns true. Each one gets `limit` in all; the head must arrive
/// within 10 seconds, so only a token holder's label read can use the rest.
pub(crate) async fn run<B: Backend>(
    listener: tokio::net::TcpListener,
    backend: Arc<B>,
    token: Arc<String>,
    mut stop: tokio::sync::watch::Receiver<bool>,
    max_connections: usize,
    limit: std::time::Duration,
) {
    let places = Arc::new(tokio::sync::Semaphore::new(max_connections));
    loop {
        tokio::select! {
            _ = stop.changed() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let Ok(place) = places.clone().try_acquire_owned() else {
                        drop(stream);
                        continue;
                    };
                    let (backend, token) = (backend.clone(), token.clone());
                    tauri::async_runtime::spawn(async move {
                        let _place = place;
                        let _ = tokio::time::timeout(limit, serve(stream, backend, token)).await;
                    });
                }
                // Out of open files, for example: wait instead of spinning.
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
            },
        }
    }
}

/// Answers one request on `stream` and closes it.
pub(crate) async fn serve<B: Backend>(mut stream: TcpStream, backend: Arc<B>, token: Arc<String>) {
    let out = match http::read_head(&mut stream).await {
        None => error(400, "The request is not readable."),
        Some((head, already)) => route(&mut stream, backend.as_ref(), &token, head, already).await,
    };
    let _ = stream.write_all(&out).await;
    let _ = stream.shutdown().await;
}

async fn route<B: Backend>(stream: &mut TcpStream, backend: &B, token: &str, head: http::Head, already: Vec<u8>) -> Vec<u8> {
    let path = head.path.split(['?', '#']).next().unwrap_or("").to_string();
    let limit = match (head.method.as_str(), path.as_str()) {
        ("GET", "/") => return shell(backend),
        ("GET", p) if p.starts_with("/assets/") => return asset(backend, &p["/assets/".len()..]),
        ("GET", "/api/status") => None,
        ("POST", "/api/read") => Some(MAX_READ_BODY),
        ("POST", "/api/save") => Some(MAX_SAVE_BODY),
        ("GET", p) if !p.starts_with("/api/") => return not_found(),
        _ => return error(405, "Not allowed."),
    };
    if !http::same_token(&head.bearer, token) {
        return error(401, DISCONNECTED);
    }
    let Some(limit) = limit else {
        return replied(backend.status());
    };
    let Some(length) = head.content_length else {
        return error(411, "Send the photo with a Content-Length.");
    };
    if length > limit {
        return error(413, "The photo is larger than 12 MiB.");
    }
    let Some(body) = http::read_body(stream, already, length).await else {
        return error(400, "The photo did not arrive completely. Try again.");
    };
    replied(if path == "/api/read" { backend.read(body).await } else { backend.save(body, valid_batch(&head.batch)).await })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    #[derive(Default)]
    struct FakeBackend {
        got: Mutex<Vec<(&'static str, Vec<u8>)>>,
        batches: Mutex<Vec<Option<String>>>,
        status_error: Option<(u16, String)>,
    }

    impl Backend for FakeBackend {
        fn status(&self) -> Reply {
            match &self.status_error {
                Some(e) => Err(e.clone()),
                None => Ok(json!({"library": "Test"})),
            }
        }
        fn read(&self, photo: Vec<u8>) -> BoxFut<Reply> {
            self.got.lock().unwrap().push(("read", photo));
            Box::pin(async { Ok(json!({"drafts": []})) })
        }
        fn save(&self, frame: Vec<u8>, batch: Option<String>) -> BoxFut<Reply> {
            self.batches.lock().unwrap().push(batch);
            self.got.lock().unwrap().push(("save", frame));
            Box::pin(async { Ok(json!({"items": []})) })
        }
        /// Like Tauri's resolver: an unknown path comes back as the desktop page.
        fn asset(&self, path: &str) -> Option<(Vec<u8>, String)> {
            Some(match path {
                "phone.html" => (b"<html>phone</html>".to_vec(), "text/html".into()),
                "assets/app.js" => (b"x".to_vec(), "text/javascript".into()),
                _ => (b"<html>desktop</html>".to_vec(), "text/html".into()),
            })
        }
    }

    /// Sends `request` on one connection and returns the whole answer and how long it took.
    /// The client never closes its side, so a server that waits for a body would time out.
    fn exchange(backend: Arc<FakeBackend>, request: &[u8]) -> (String, Duration) {
        tauri::async_runtime::block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                serve(stream, backend, Arc::new("tok".to_string())).await;
            });
            let mut client = TcpStream::connect(addr).await.unwrap();
            let started = Instant::now();
            client.write_all(request).await.unwrap();
            let mut answer = Vec::new();
            tokio::time::timeout(Duration::from_secs(3), client.read_to_end(&mut answer))
                .await
                .expect("the server answered in time")
                .unwrap();
            let took = started.elapsed();
            server.abort();
            (String::from_utf8_lossy(&answer).into_owned(), took)
        })
    }

    fn get(path: &str, token: Option<&str>) -> Vec<u8> {
        let auth = token.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
        format!("GET {path} HTTP/1.1\r\nHost: x\r\n{auth}\r\n").into_bytes()
    }

    fn post(path: &str, token: &str, body: &[u8]) -> Vec<u8> {
        let mut out = format!("POST {path} HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
        out.extend_from_slice(body);
        out
    }

    fn status_of(answer: &str) -> u16 {
        answer.split_whitespace().nth(1).and_then(|s| s.parse().ok()).expect("a status line")
    }

    #[test]
    fn no_token_is_refused_before_the_body() {
        let backend = Arc::new(FakeBackend::default());
        let (answer, took) = exchange(backend.clone(), b"POST /api/read HTTP/1.1\r\nHost: x\r\nContent-Length: 10485760\r\n\r\n");
        assert_eq!(status_of(&answer), 401, "{answer}");
        assert!(answer.contains("This phone is no longer connected. Open Corydoras on your Mac, go to Settings › Phone, and scan the code."));
        assert!(took < Duration::from_secs(2));
        assert!(backend.got.lock().unwrap().is_empty());
    }

    #[test]
    fn wrong_token_is_refused() {
        let (answer, _) = exchange(Arc::new(FakeBackend::default()), &get("/api/status", Some("nope")));
        assert_eq!(status_of(&answer), 401);
    }

    #[test]
    fn an_oversize_read_is_413_before_the_body() {
        let backend = Arc::new(FakeBackend::default());
        let request = format!("POST /api/read HTTP/1.1\r\nAuthorization: Bearer tok\r\nContent-Length: {}\r\n\r\n", MAX_READ_BODY + 1);
        let (answer, took) = exchange(backend.clone(), request.as_bytes());
        assert_eq!(status_of(&answer), 413, "{answer}");
        assert!(answer.contains("The photo is larger than 12 MiB."));
        assert!(took < Duration::from_secs(2));
        assert!(backend.got.lock().unwrap().is_empty());
    }

    #[test]
    fn a_save_may_be_larger_than_a_read() {
        let request = format!("POST /api/save HTTP/1.1\r\nAuthorization: Bearer tok\r\nContent-Length: {}\r\n\r\n", MAX_SAVE_BODY + 1);
        let (answer, _) = exchange(Arc::new(FakeBackend::default()), request.as_bytes());
        assert_eq!(status_of(&answer), 413);
        assert_eq!(MAX_SAVE_BODY, 12 * 1024 * 1024 + 512 * 1024);
    }

    #[test]
    fn a_post_without_length_is_411() {
        for request in [
            &b"POST /api/read HTTP/1.1\r\nAuthorization: Bearer tok\r\n\r\n"[..],
            &b"POST /api/save HTTP/1.1\r\nAuthorization: Bearer tok\r\nTransfer-Encoding: chunked\r\n\r\n"[..],
        ] {
            let (answer, _) = exchange(Arc::new(FakeBackend::default()), request);
            assert_eq!(status_of(&answer), 411, "{answer}");
            assert!(answer.contains("Send the photo with a Content-Length."));
        }
    }

    #[test]
    fn read_and_save_pass_the_body() {
        let backend = Arc::new(FakeBackend::default());
        let (answer, _) = exchange(backend.clone(), &post("/api/read", "tok", b"abc"));
        assert_eq!(status_of(&answer), 200);
        assert!(answer.ends_with(r#"{"drafts":[]}"#), "{answer}");
        let (answer, _) = exchange(backend.clone(), &post("/api/save", "tok", b"frame"));
        assert_eq!(status_of(&answer), 200);
        assert_eq!(*backend.got.lock().unwrap(), vec![("read", b"abc".to_vec()), ("save", b"frame".to_vec())]);
    }

    #[test]
    fn the_shell_is_public_and_has_the_safety_headers() {
        let (answer, _) = exchange(Arc::new(FakeBackend::default()), &get("/", None));
        assert_eq!(status_of(&answer), 200);
        assert!(answer.ends_with("<html>phone</html>"));
        assert!(answer.contains("Content-Type: text/html; charset=utf-8\r\n"));
        for header in ["Cache-Control: no-store\r\n", "X-Content-Type-Options: nosniff\r\n", "Referrer-Policy: no-referrer\r\n", "Connection: close\r\n"] {
            assert!(answer.contains(header), "missing {header:?} in {answer}");
        }
        assert!(!answer.contains("Access-Control-"));
    }

    #[test]
    fn only_phone_files_are_served() {
        let fetch = |path: &str| status_of(&exchange(Arc::new(FakeBackend::default()), &get(path, None)).0);
        assert_eq!(fetch("/assets/app.js"), 200);
        for path in ["/assets/missing.js", "/assets/../index.html", "/assets/a.html", "/index.html", "/phone.html", "/assets/sub/app.js"] {
            assert_eq!(fetch(path), 404, "{path}");
        }
    }

    #[test]
    fn options_gets_no_cors() {
        let request = b"OPTIONS /api/read HTTP/1.1\r\nOrigin: http://evil.example\r\nAccess-Control-Request-Method: POST\r\n\r\n";
        let (answer, _) = exchange(Arc::new(FakeBackend::default()), request);
        assert_eq!(status_of(&answer), 405);
        assert!(!answer.contains("Access-Control-"));
    }

    #[test]
    fn connections_beyond_the_cap_are_closed() {
        tauri::async_runtime::block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let (stop, stopped) = tokio::sync::watch::channel(false);
            let backend = Arc::new(FakeBackend::default());
            let server = tokio::spawn(run(listener, backend, Arc::new("tok".to_string()), stopped, 2, Duration::from_secs(5)));
            // Two idle connections take both places.
            let idle_a = TcpStream::connect(addr).await.unwrap();
            let _idle_b = TcpStream::connect(addr).await.unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
            let mut third = TcpStream::connect(addr).await.unwrap();
            let mut buf = [0u8; 16];
            let n = tokio::time::timeout(Duration::from_secs(1), third.read(&mut buf)).await.expect("closed at once").unwrap_or(0);
            assert_eq!(n, 0, "the third connection should be closed without an answer");
            // A place comes free when an idle connection goes.
            drop(idle_a);
            tokio::time::sleep(Duration::from_millis(100)).await;
            let mut fourth = TcpStream::connect(addr).await.unwrap();
            fourth.write_all(&get("/", None)).await.unwrap();
            let mut answer = Vec::new();
            tokio::time::timeout(Duration::from_secs(2), fourth.read_to_end(&mut answer)).await.unwrap().unwrap();
            assert!(String::from_utf8_lossy(&answer).starts_with("HTTP/1.1 200"));
            let _ = stop.send(true);
            tokio::time::timeout(Duration::from_secs(1), server).await.expect("the loop stops").unwrap();
        });
    }

    #[test]
    fn a_valid_batch_id_reaches_the_save() {
        let backend = Arc::new(FakeBackend::default());
        let with = |id: &str| {
            let mut request = format!("POST /api/save HTTP/1.1\r\nAuthorization: Bearer tok\r\nX-Batch-Id: {id}\r\nContent-Length: 1\r\n\r\n").into_bytes();
            request.push(b'f');
            request
        };
        exchange(backend.clone(), &with("a1b2c3d4e5f60718293a4b5c6d7e8f90"));
        exchange(backend.clone(), &with("not valid!"));
        assert_eq!(*backend.batches.lock().unwrap(), vec![Some("a1b2c3d4e5f60718293a4b5c6d7e8f90".to_string()), None]);
    }

    #[test]
    fn backend_errors_keep_their_status() {
        let text = "Workshop inventory is off for this library. Open Settings to turn it on.";
        let backend = Arc::new(FakeBackend { status_error: Some((400, text.into())), ..Default::default() });
        let (answer, _) = exchange(backend, &get("/api/status", Some("tok")));
        assert_eq!(status_of(&answer), 400);
        assert!(answer.ends_with(&json!({ "error": text }).to_string()), "{answer}");
    }
}
