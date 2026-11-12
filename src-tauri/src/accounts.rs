//! Account sign-in from the Settings page, as an alternative to an API key.
//!
//! OpenRouter's sign-in gives an API key, which goes into the key file of the provider.
//! "Sign in with ChatGPT" gives the app one ChatGPT account, which pays for the calls of
//! every ChatGPT plan provider. The browser part and the tokens are in the core
//! (`corydoras_core::signin`).

use crate::{ai_config, ai_view, e2s, find_profile, AiView, AppState, CmdResult};
use corydoras_core::activity::CancelToken;
use corydoras_core::signin::{self, chatgpt, openrouter};
use corydoras_core::{ai, secrets};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{Manager, State};
use tauri_plugin_opener::OpenerExt;

/// The sign-in that waits for the browser now. There is one at a time.
#[derive(Default)]
pub struct Pending(Mutex<Option<Arc<CancelToken>>>);

impl Pending {
    /// Starts a wait. A sign-in that still waits ends: its browser tab is of no use any more.
    fn begin(&self) -> Arc<CancelToken> {
        let mine = Arc::new(CancelToken::default());
        let before = self.0.lock().unwrap_or_else(|e| e.into_inner()).replace(mine.clone());
        if let Some(old) = before {
            old.cancel();
        }
        mine
    }

    fn end(&self, mine: &Arc<CancelToken>) {
        let mut slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().is_some_and(|c| Arc::ptr_eq(c, mine)) {
            *slot = None;
        }
    }

    fn cancel(&self) {
        if let Some(waiting) = self.0.lock().unwrap_or_else(|e| e.into_inner()).take() {
            waiting.cancel();
        }
    }
}

/// The ChatGPT account of the app, without its tokens.
#[derive(Serialize, Default)]
pub struct AccountView {
    signed_in: bool,
    /// Signed in, and the person allowed the use of the plan.
    plan: bool,
    email: String,
    /// An account signed in before on this Mac: the next sign-in is for that account.
    known: bool,
}

pub fn account_view() -> AccountView {
    match chatgpt::Session::app().account() {
        Some(a) => AccountView { signed_in: a.signed_in(), plan: a.signed_in() && a.plan_enabled(), known: !a.client_id.is_empty(), email: a.email },
        None => AccountView::default(),
    }
}

#[derive(Serialize)]
pub struct SignInResult {
    view: AiView,
    /// The first sign-in with the ChatGPT plan: the page shows the notice about plan usage.
    welcome: bool,
}

/// Opens the sign-in page of the provider in the browser and waits for the answer (five
/// minutes at most). `fresh` asks ChatGPT for another account than the saved one.
#[tauri::command]
pub async fn provider_sign_in<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: State<'_, AppState>, profile_id: String, fresh: bool) -> CmdResult<SignInResult> {
    let profile = find_profile(&state, &profile_id)?;
    let kind = ai::preset(&profile.kind).sign_in;
    if kind.is_empty() {
        return Err(format!("{} has no account sign-in. Use an API key.", profile.name));
    }
    let cancel = state.sign_in.begin();
    let browser = app.clone();
    let open = move |url: &str| browser.opener().open_url(url, None::<&str>).map_err(|e| anyhow::anyhow!("Cannot open the browser: {e}"));
    let done = match kind {
        "openrouter" => openrouter::sign_in(openrouter::SITE, open, &cancel, signin::WAIT)
            .await
            .and_then(|key| secrets::set_key(&profile.key_account, &key))
            .map(|_| false),
        _ => chatgpt::sign_in(&chatgpt::Session::app(), fresh, open, &cancel, signin::WAIT).await.map(|done| done.welcome),
    };
    state.sign_in.end(&cancel);
    let welcome = done.map_err(e2s)?;
    // The person is in the browser now: bring the app back to the front.
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
    Ok(SignInResult { view: ai_view(ai_config(&state)), welcome })
}

/// Stops the sign-in that waits for the browser.
#[tauri::command]
pub fn cancel_sign_in(state: State<'_, AppState>) {
    state.sign_in.cancel();
}

#[derive(Serialize)]
pub struct SignOutResult {
    view: AiView,
    /// False when ChatGPT did not confirm the end of the sign-in. The tokens are gone
    /// from this Mac all the same.
    confirmed: bool,
}

/// Signs the app out of ChatGPT: every ChatGPT plan provider stops.
#[tauri::command]
pub async fn provider_sign_out(state: State<'_, AppState>) -> CmdResult<SignOutResult> {
    let confirmed = chatgpt::sign_out(&chatgpt::Session::app()).await.map_err(e2s)?;
    Ok(SignOutResult { view: ai_view(ai_config(&state)), confirmed })
}
