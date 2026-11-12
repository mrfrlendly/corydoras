//! Sign in with OpenRouter (OAuth PKCE): the browser sign-in returns an API key of the
//! person's own account. OpenRouter asks for no app registration.
//!
//! <https://openrouter.ai/docs/guides/overview/auth/oauth>

use super::{error_text, http, url_with, Loopback, Pkce, APP_NAME};
use crate::activity::CancelToken;
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::time::Duration;

pub const SITE: &str = "https://openrouter.ai";

/// The page that the browser opens. `state` comes back with the answer, unchanged.
pub fn auth_url(site: &str, callback_url: &str, challenge: &str, state: &str, label: &str) -> String {
    url_with(
        &format!("{}/auth", site.trim_end_matches('/')),
        &[
            ("callback_url", callback_url),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256"),
            ("state", state),
            ("key_label", label),
        ],
    )
}

/// Changes the code of the browser answer into the API key.
pub async fn exchange(site: &str, code: &str, verifier: &str) -> Result<String> {
    let resp = http()?
        .post(format!("{}/api/v1/auth/keys", site.trim_end_matches('/')))
        .json(&json!({"code": code, "code_verifier": verifier, "code_challenge_method": "S256"}))
        .send()
        .await
        .map_err(|e| anyhow!("Cannot reach OpenRouter: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("OpenRouter did not give a key (HTTP {}): {}", status.as_u16(), error_text(&text));
    }
    serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v["key"].as_str().map(|k| k.trim().to_string()))
        .filter(|k| !k.is_empty())
        .ok_or_else(|| anyhow!("The answer of OpenRouter holds no key."))
}

/// The whole sign-in. `open` shows the address in the browser. Returns the API key.
pub async fn sign_in<F: FnOnce(&str) -> Result<()>>(site: &str, open: F, cancel: &CancelToken, timeout: Duration) -> Result<String> {
    // The OpenRouter documentation names "localhost" for callbacks on this computer.
    let listener = Loopback::bind("/callback", None, true).await?;
    let pkce = Pkce::new();
    let state = super::random_token(16);
    open(&auth_url(site, &listener.url("localhost"), &pkce.challenge, &state, APP_NAME))?;
    let answer = listener.wait(timeout, cancel).await?;
    let Some(code) = answer.get("code").filter(|c| !c.is_empty()) else {
        bail!("OpenRouter sent no code: the sign-in was cancelled or refused.");
    };
    if answer.get("state") != Some(&state) {
        bail!("The answer in the browser did not belong to this sign-in. Start the sign-in again.");
    }
    exchange(site, code, &pkce.verifier).await
}
