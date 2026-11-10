//! The local bridge: a small HTTP listener that the browser extension talks to.
//!
//! - It listens on 127.0.0.1 only, so no other computer can reach it.
//! - Every request must carry the token (`Authorization: Bearer …`). A web page
//!   does not know the token, and it gets no CORS permission, so it cannot use
//!   the bridge.
//! - It is off until you turn it on in Settings.
//!
//! Requests (JSON):
//!   GET  /v1/languages                                  -> the languages
//!   POST /v1/items   {language?, image_url, page_url?, title?}  -> adds an image
//!   POST /v1/sources {language?, url}                           -> adds a page as a source

use crate::{capture, AppState, CmdResult};
use corydoras_core::{analysis, config::AppConfig, provider, sources, Library};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Largest request body: an image sent as a data URL.
const MAX_BODY: usize = 30 * 1024 * 1024;
const MAX_HEADERS: usize = 32 * 1024;

#[derive(Default)]
pub struct BridgeState {
    /// The port that the listener runs on, and the way to stop it.
    running: Mutex<Option<(u16, tokio::sync::watch::Sender<bool>)>>,
    /// Why the listener could not start.
    error: Mutex<Option<String>>,
}

#[derive(Serialize, Clone)]
pub struct BridgeView {
    pub enabled: bool,
    pub port: u16,
    pub token: String,
    pub running: bool,
    pub error: Option<String>,
}

fn new_token() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

fn view(app: &AppHandle) -> BridgeView {
    let cfg = AppConfig::load().bridge;
    let st = app.state::<AppState>();
    let running = st.bridge.running.lock().unwrap_or_else(|e| e.into_inner()).is_some();
    let error = st.bridge.error.lock().unwrap_or_else(|e| e.into_inner()).clone();
    BridgeView { enabled: cfg.enabled, port: cfg.port, token: cfg.token, running, error }
}

fn stop(app: &AppHandle) {
    let st = app.state::<AppState>();
    let running = st.bridge.running.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some((_, tx)) = running {
        let _ = tx.send(true);
    }
}

/// Starts the listener when the settings turn it on. Call it at launch and
/// after each change of the settings.
pub fn start(app: &AppHandle) {
    stop(app);
    let st = app.state::<AppState>();
    *st.bridge.error.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let cfg = AppConfig::load().bridge;
    if !cfg.enabled || cfg.token.is_empty() {
        return;
    }
    let listener = std::net::TcpListener::bind(("127.0.0.1", cfg.port)).and_then(|l| l.set_nonblocking(true).map(|_| l));
    let listener = match listener {
        Ok(l) => l,
        Err(e) => {
            *st.bridge.error.lock().unwrap_or_else(|x| x.into_inner()) =
                Some(format!("Port {} is not free ({e}). Choose another port, and set the same port in the extension.", cfg.port));
            return;
        }
    };
    let (tx, mut rx) = tokio::sync::watch::channel(false);
    *st.bridge.running.lock().unwrap_or_else(|e| e.into_inner()) = Some((cfg.port, tx));
    let (app2, token) = (app.clone(), Arc::new(cfg.token));
    tauri::async_runtime::spawn(async move {
        let Ok(listener) = tokio::net::TcpListener::from_std(listener) else { return };
        loop {
            tokio::select! {
                _ = rx.changed() => break,
                accepted = listener.accept() => {
                    if let Ok((stream, _)) = accepted {
                        let (app3, token) = (app2.clone(), token.clone());
                        tauri::async_runtime::spawn(async move {
                            let _ = tokio::time::timeout(Duration::from_secs(120), serve(stream, app3, token)).await;
                        });
                    }
                }
            }
        }
    });
}

struct Request {
    method: String,
    path: String,
    token: String,
    origin: String,
    body: Vec<u8>,
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> Option<Request> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > MAX_HEADERS {
            return None;
        }
        let n = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut chunk)).await.ok()?.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let (method, path) = (first.next()?.to_string(), first.next()?.to_string());
    let (mut token, mut origin, mut length) = (String::new(), String::new(), 0usize);
    for line in lines {
        let Some((k, v)) = line.split_once(':') else { continue };
        match k.trim().to_lowercase().as_str() {
            "authorization" => token = v.trim().strip_prefix("Bearer ").unwrap_or("").trim().to_string(),
            "origin" => origin = v.trim().to_string(),
            "content-length" => length = v.trim().parse().unwrap_or(0),
            _ => {}
        }
    }
    if length > MAX_BODY {
        return None;
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < length {
        let n = tokio::time::timeout(Duration::from_secs(30), stream.read(&mut chunk)).await.ok()?.ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(length);
    Some(Request { method, path, token, origin, body })
}

/// Only an extension may read the answer in a browser. A web page gets no permission.
fn extension_origin(origin: &str) -> bool {
    origin.starts_with("chrome-extension://") || origin.starts_with("moz-extension://") || origin.starts_with("safari-web-extension://")
}

fn response(status: u16, body: &Value, origin: &str) -> Vec<u8> {
    // An answer without content (the preflight) has no body.
    let text = if status == 204 { String::new() } else { body.to_string() };
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Error",
    };
    let cors = if extension_origin(origin) {
        format!("Access-Control-Allow-Origin: {origin}\r\nAccess-Control-Allow-Headers: authorization, content-type\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nVary: Origin\r\n")
    } else {
        String::new()
    };
    format!("HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\n{cors}Connection: close\r\n\r\n{text}", text.len()).into_bytes()
}

fn same(a: &str, b: &str) -> bool {
    // Compare every byte, so the time does not tell where two tokens differ.
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn serve(mut stream: tokio::net::TcpStream, app: AppHandle, token: Arc<String>) {
    let Some(req) = read_request(&mut stream).await else {
        let _ = stream.write_all(&response(400, &json!({"error": "The request is not readable or too large."}), "")).await;
        return;
    };
    let out = if req.method == "OPTIONS" {
        response(204, &json!({}), &req.origin)
    } else if !same(&req.token, &token) {
        response(401, &json!({"error": "The token is missing or wrong. Copy it from Corydoras › Settings › Browser extension."}), &req.origin)
    } else {
        match route(&app, &req).await {
            Ok(v) => response(200, &v, &req.origin),
            Err((status, e)) => response(status, &json!({"error": e}), &req.origin),
        }
    };
    let _ = stream.write_all(&out).await;
    let _ = stream.shutdown().await;
}

type Routed = Result<Value, (u16, String)>;

fn library(app: &AppHandle) -> Result<Arc<Library>, (u16, String)> {
    crate::lib(&app.state::<AppState>()).map_err(|e| (400, e))
}

/// The language that a request names, or the one that screen captures go to.
fn language_of(lib: &Library, body: &Value) -> Result<corydoras_core::models::Language, (u16, String)> {
    match body.get("language").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()) {
        Some(key) => lib.find_language(key).map_err(|e| (404, e.to_string())),
        None => capture::target_language(lib, &AppConfig::load().capture).ok_or((400, "The library has no language yet. Make one in Corydoras first.".to_string())),
    }
}

async fn route(app: &AppHandle, req: &Request) -> Routed {
    let path = req.path.split('?').next().unwrap_or("");
    let bad = |e: String| (400u16, e);
    match (req.method.as_str(), path) {
        ("GET", "/v1/languages") => {
            let lib = library(app)?;
            let target = capture::target_language(&lib, &AppConfig::load().capture).map(|l| l.id);
            let langs = lib.list_languages().map_err(|e| bad(e.to_string()))?;
            Ok(json!({
                "app": "Corydoras",
                "version": env!("CARGO_PKG_VERSION"),
                "default": target,
                "languages": langs.iter().map(|l| json!({"id": l.id, "name": l.name, "items": l.item_count})).collect::<Vec<_>>(),
            }))
        }
        ("POST", "/v1/items") => {
            let body: Value = serde_json::from_slice(&req.body).map_err(|_| bad("The body must be JSON.".into()))?;
            let lib = library(app)?;
            let lang = language_of(&lib, &body)?;
            let image = body.get("image_url").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| bad("\"image_url\" is necessary.".into()))?;
            let page = body.get("page_url").and_then(|v| v.as_str()).map(str::trim).filter(|s| s.starts_with("http"));
            let bytes = if image.starts_with("data:") {
                analysis::base64_decode(image).map_err(|e| bad(e.to_string()))?
            } else {
                provider::download(image).await.map_err(|e| bad(e.to_string()))?
            };
            let title = body.get("title").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty());
            let name = match (title, image.starts_with("data:")) {
                (Some(t), _) => t.chars().take(100).collect::<String>(),
                (None, false) => image.split(['?', '#']).next().and_then(|u| u.rsplit('/').next()).filter(|s| !s.is_empty()).unwrap_or("web-image").to_string(),
                (None, true) => "web-image".to_string(),
            };
            let before = lib.has_item_hash(&lang.id, &corydoras_core::util::fnv1a64(&bytes)).unwrap_or(false);
            let source = page.unwrap_or(if image.starts_with("data:") { "browser" } else { image });
            let item = lib.import_bytes(&lang.id, &name, &bytes, Some(source)).map_err(|e| bad(e.to_string()))?;
            let _ = app.emit("library-changed", lang.id.clone());
            if !before {
                let _ = app.emit("bridge-added", json!({"language_id": lang.id, "language_name": lang.name, "what": "image"}));
            }
            Ok(json!({"ok": true, "language": lang.name, "item": item.id, "duplicate": before}))
        }
        ("POST", "/v1/sources") => {
            let body: Value = serde_json::from_slice(&req.body).map_err(|_| bad("The body must be JSON.".into()))?;
            let lib = library(app)?;
            let lang = language_of(&lib, &body)?;
            let url = body.get("url").and_then(|v| v.as_str()).map(str::trim).filter(|s| s.starts_with("http")).ok_or_else(|| bad("\"url\" must be an http or https link.".into()))?.to_string();
            let ex = sources::fetch_link(&url).await.map_err(|e| bad(e.to_string()))?;
            let (lib2, id, url2) = (lib.clone(), lang.id.clone(), url.clone());
            let source = tauri::async_runtime::spawn_blocking(move || lib2.add_link_source(&id, &url2, &ex))
                .await
                .map_err(|e| bad(e.to_string()))?
                .map_err(|e| bad(e.to_string()))?;
            let _ = app.emit("library-changed", lang.id.clone());
            let _ = app.emit("bridge-added", json!({"language_id": lang.id, "language_name": lang.name, "what": "source"}));
            Ok(json!({"ok": true, "language": lang.name, "source": source.id, "title": source.title}))
        }
        _ => Err((404, "No such address. The bridge has /v1/languages, /v1/items and /v1/sources.".into())),
    }
}

#[tauri::command]
pub fn get_bridge(app: AppHandle) -> BridgeView {
    view(&app)
}

/// Turns the bridge on or off, changes its port, or makes a new token.
#[tauri::command]
pub fn set_bridge(app: AppHandle, enabled: bool, port: u16, new_token_now: bool) -> CmdResult<BridgeView> {
    if port < 1024 {
        return Err("Choose a port from 1024 to 65535.".into());
    }
    let mut cfg = AppConfig::load();
    cfg.bridge.enabled = enabled;
    cfg.bridge.port = port;
    if cfg.bridge.token.is_empty() || new_token_now {
        cfg.bridge.token = new_token();
    }
    cfg.save().map_err(|e| e.to_string())?;
    start(&app);
    Ok(view(&app))
}
