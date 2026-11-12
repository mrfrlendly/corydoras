//! Account sign-in, as an alternative to a pasted API key.
//!
//! Two providers offer a sign-in that an open-source desktop app may use:
//!
//! - OpenRouter (`openrouter`): the browser sign-in returns a normal API key of the
//!   person's account. The app keeps it in the provider's key file, so everything after
//!   the sign-in works as with a pasted key.
//! - OpenAI (`chatgpt`): "Sign in with ChatGPT" lets the calls use the usage that is
//!   included in the person's ChatGPT plan. The app keeps tokens and renews them.
//!
//! Both open the system browser and wait for its answer on a listener that only this
//! computer can reach. Anthropic and Google do not allow the sign-in of their own apps
//! in other software, so those providers work with an API key only.

use crate::activity::CancelToken;
use anyhow::{anyhow, bail, Result};
use base64::Engine;
use std::collections::HashMap;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub mod chatgpt;
pub mod openrouter;
#[cfg(test)]
mod tests;

/// The name that the sign-in pages show for this app.
pub const APP_NAME: &str = "Corydoras";
/// How long the app waits for the browser.
pub const WAIT: Duration = Duration::from_secs(300);

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Random bytes from the system, as URL-safe text.
pub fn random_token(bytes: usize) -> String {
    use ring::rand::SecureRandom;
    let mut buf = vec![0u8; bytes];
    ring::rand::SystemRandom::new().fill(&mut buf).expect("the system gives random bytes");
    b64url(&buf)
}

/// The secret of one sign-in attempt (PKCE, RFC 7636). The browser carries only the
/// challenge; the verifier goes to the server with the code, so a code that another
/// program catches is of no use to it.
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn new() -> Self {
        let verifier = random_token(32);
        let challenge = Self::challenge_of(&verifier);
        Pkce { verifier, challenge }
    }

    /// The S256 challenge: the SHA-256 of the verifier, URL-safe, without padding.
    pub fn challenge_of(verifier: &str) -> String {
        b64url(ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes()).as_ref())
    }
}

impl Default for Pkce {
    fn default() -> Self {
        Self::new()
    }
}

/// `base` with the parameters as its query.
pub(crate) fn url_with(base: &str, params: &[(&str, &str)]) -> String {
    reqwest::Url::parse_with_params(base, params).map(String::from).unwrap_or_else(|_| base.to_string())
}

/// The pairs of a query string.
fn pairs(query: &str) -> HashMap<String, String> {
    reqwest::Url::parse(&format!("http://x/?{query}"))
        .map(|u| u.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect())
        .unwrap_or_default()
}

pub(crate) fn http() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("Corydoras/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

/// The text of a JSON error answer, whatever shape the server uses.
pub(crate) fn error_text(body: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(body.trim()).unwrap_or_default();
    v["error"]["message"]
        .as_str()
        .or_else(|| v["error_description"].as_str())
        .or_else(|| v["message"].as_str())
        .or_else(|| v["detail"].as_str())
        .or_else(|| v["error"].as_str())
        .map(str::to_string)
        .unwrap_or_else(|| crate::util::truncate(body.trim(), 200))
}

/// The listener that the browser comes back to. It accepts connections from this
/// computer only and answers one path.
pub struct Loopback {
    v4: TcpListener,
    /// A browser can read "localhost" as ::1.
    v6: Option<TcpListener>,
    port: u16,
    path: String,
}

impl Loopback {
    /// Listens on 127.0.0.1. `prefer_port` is tried first; when it is busy (or None),
    /// the system chooses a free port. `also_v6` listens on ::1 with the same port too.
    pub async fn bind(path: &str, prefer_port: Option<u16>, also_v6: bool) -> Result<Self> {
        let preferred = match prefer_port {
            Some(p) => TcpListener::bind(("127.0.0.1", p)).await.ok(),
            None => None,
        };
        let v4 = match preferred {
            Some(l) => l,
            None => TcpListener::bind(("127.0.0.1", 0))
                .await
                .map_err(|e| anyhow!("Cannot open a port on this computer for the sign-in: {e}"))?,
        };
        let port = v4.local_addr()?.port();
        let v6 = if also_v6 { TcpListener::bind(("::1", port)).await.ok() } else { None };
        Ok(Loopback { v4, v6, port, path: path.to_string() })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The address that the browser comes back to. `host` is "127.0.0.1" or "localhost":
    /// each provider accepts one of the two.
    pub fn url(&self, host: &str) -> String {
        format!("http://{host}:{}{}", self.port, self.path)
    }

    async fn accept(&self) -> std::io::Result<TcpStream> {
        match &self.v6 {
            Some(v6) => tokio::select! {
                r = self.v4.accept() => r.map(|(s, _)| s),
                r = v6.accept() => r.map(|(s, _)| s),
            },
            None => self.v4.accept().await.map(|(s, _)| s),
        }
    }

    /// Waits until the browser arrives at the path, and returns the parameters that it
    /// brings. Requests for other paths get "not found" and the wait goes on.
    pub async fn wait(&self, timeout: Duration, cancel: &CancelToken) -> Result<HashMap<String, String>> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let stream = tokio::select! {
                s = self.accept() => s?,
                _ = cancel.cancelled() => bail!("The sign-in was cancelled."),
                _ = tokio::time::sleep_until(deadline) => {
                    bail!("No answer from the browser in {} minutes. Start the sign-in again.", (timeout.as_secs() / 60).max(1))
                }
            };
            if let Some(params) = self.answer(stream).await {
                return Ok(params);
            }
        }
    }

    async fn answer(&self, mut stream: TcpStream) -> Option<HashMap<String, String>> {
        let head = tokio::time::timeout(Duration::from_secs(5), read_head(&mut stream)).await.ok()??;
        let mut first = head.lines().next()?.split(' ');
        let (method, target) = (first.next()?, first.next()?);
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        if method != "GET" || path != self.path {
            respond(&mut stream, 404, "Not Found", "This address is not part of the sign-in.").await;
            return None;
        }
        let params = pairs(query);
        let done = params.contains_key("code") && !params.contains_key("error");
        let text = if done {
            "The browser part is done. Go back to Corydoras; you can close this tab."
        } else {
            "The sign-in was not completed. Go back to Corydoras to try again; you can close this tab."
        };
        respond(&mut stream, 200, "OK", text).await;
        Some(params)
    }
}

/// The request line and the headers.
async fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            return Some(String::from_utf8_lossy(&buf[..end]).into_owned());
        }
        if buf.len() > 32 * 1024 {
            return None;
        }
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// A small page. It loads nothing from the network and the browser keeps no copy.
async fn respond(stream: &mut TcpStream, status: u16, reason: &str, text: &str) {
    let body = format!(
        "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>Corydoras</title>\
         <body style=\"font: 16px -apple-system, system-ui, sans-serif; margin: 18vh auto; max-width: 28em; padding: 0 1.5em; color: #222\">\
         <h1 style=\"font-size: 20px\">Corydoras</h1><p>{text}</p></body></html>"
    );
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(body.as_bytes()).await;
    let _ = stream.shutdown().await;
}
