//! "Sign in with ChatGPT" for open-source apps: the calls use the usage that is included
//! in the person's ChatGPT plan, not an API key.
//!
//! <https://developers.openai.com/siwc/token-sharing-open-source>
//!
//! The first sign-in registers this app for the account and this computer: the browser
//! answer carries a client ID, which the app keeps and uses from then on. The tokens are
//! in a file that only the user can read, next to the key files. The access token lives
//! one hour; the app renews it with the refresh token, which the server replaces at
//! each renewal.
//!
//! One account serves the whole app: every ChatGPT plan provider uses it.

use super::{error_text, http, url_with, Loopback, Pkce, APP_NAME};
use crate::activity::CancelToken;
use crate::provider::CallError;
use crate::secrets;
use crate::util::now_ts;
use anyhow::{anyhow, bail, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

/// The client name of a first sign-in. It asks the server for a registration and is
/// never saved or sent to the token endpoint.
pub const NEW_CLIENT: &str = "dynamic_agent_client";
/// The permission to use the plan. Without it, the sign-in pays for no call.
pub const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
/// Identity, then plan usage.
pub const SCOPE: &str = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
/// Where the person sees and limits the usage of the plan.
pub const USAGE_URL: &str = "https://chatgpt.com/settings/usage";
/// The only address that calls with a plan token go to.
pub const API_BASE: &str = "https://api.openai.com/v1";
/// The record of the app's ChatGPT account in the keys folder.
pub const ACCOUNT_NAME: &str = "chatgpt-plan";

const DISCOVERY_PATH: &str = "/.well-known/openid-configuration";
const REVOKE_FALLBACK: &str = "https://auth.openai.com/api/accounts/oauth/revoke";
const JWKS_FALLBACK: &str = "https://auth.openai.com/.well-known/jwks.json";
/// The port in OpenAI's example of the first sign-in. Later sign-ins may use any port.
const FIRST_PORT: u16 = 1455;
/// Renew an access token that has less than this many seconds left.
const REFRESH_BEFORE: i64 = 300;
/// Answers of the token endpoint that mean: this refresh token is of no use any more.
const DEAD_REFRESH: [&str; 6] = [
    "invalid_grant",
    "invalid_refresh_token",
    "token_expired",
    "refresh_token_expired",
    "refresh_token_invalidated",
    "refresh_token_reused",
];

/// One renewal at a time: the server replaces the refresh token at each renewal, so two
/// renewals with the same token would make one of them fail.
static RENEW: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The addresses of the sign-in service.
#[derive(Clone, Debug, PartialEq)]
pub struct Endpoints {
    pub authorize_url: String,
    pub token_url: String,
    /// Empty: read it from the discovery document of the issuer.
    pub revoke_url: String,
    /// Empty: read it from the discovery document of the issuer.
    pub jwks_url: String,
    pub issuer: String,
    /// What the tokens are for.
    pub resource: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints {
            authorize_url: "https://auth.openai.com/api/accounts/authorize".into(),
            token_url: "https://auth.openai.com/api/accounts/oauth/token".into(),
            revoke_url: String::new(),
            jwks_url: String::new(),
            issuer: "https://auth.openai.com".into(),
            resource: API_BASE.into(),
        }
    }
}

/// The saved account. After a sign-out the tokens are empty; the client ID, the address
/// and the subject stay, so the next sign-in reuses the registration.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub email: String,
    pub issuer: String,
    /// The account identity from the checked ID token.
    pub subject: String,
    /// The client that the server issued for this account on this computer.
    pub client_id: String,
    pub ext_agent_host_id: String,
    pub id_token: String,
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    /// Lifetime of the access token in seconds, from `saved_at`.
    pub expires_in: i64,
    pub scopes: Vec<String>,
    /// When the token answer arrived (Unix seconds).
    pub saved_at: i64,
    /// The first moment that the server wants a renewal (Unix seconds), when it names one.
    #[serde(default)]
    pub earliest_refresh_at: Option<i64>,
}

impl Account {
    pub fn signed_in(&self) -> bool {
        !self.access_token.is_empty() || !self.refresh_token.is_empty()
    }

    /// True when the person allowed the use of the plan.
    pub fn plan_enabled(&self) -> bool {
        self.scopes.iter().any(|s| s == PLAN_SCOPE)
    }

    pub fn expires_at(&self) -> i64 {
        self.saved_at + self.expires_in
    }

    pub fn needs_refresh(&self, now: i64) -> bool {
        if now >= self.expires_at() {
            return true;
        }
        if self.earliest_refresh_at.is_some_and(|t| now < t) {
            return false;
        }
        now >= self.expires_at() - REFRESH_BEFORE
    }

    fn clear_tokens(&mut self, keep_id_token: bool) {
        self.access_token.clear();
        self.refresh_token.clear();
        if !keep_id_token {
            self.id_token.clear();
        }
        self.earliest_refresh_at = None;
    }

    /// Takes a token answer. Values that the answer leaves out stay as they are.
    fn take(&mut self, t: Tokens, now: i64) {
        self.access_token = t.access_token;
        if let Some(r) = t.refresh_token.filter(|r| !r.is_empty()) {
            self.refresh_token = r;
        }
        if let Some(i) = t.id_token.filter(|i| !i.is_empty()) {
            self.id_token = i;
        }
        self.token_type = t.token_type.filter(|x| !x.is_empty()).unwrap_or_else(|| "Bearer".into());
        self.expires_in = t.expires_in.filter(|s| *s > 0).unwrap_or(3600);
        if let Some(scope) = t.scope {
            self.scopes = scope.split_whitespace().map(str::to_string).collect();
        }
        self.saved_at = now;
        self.earliest_refresh_at = t.earliest_refresh_at.as_ref().and_then(|v| moment(v, now));
    }
}

/// `earliest_refresh_at` as Unix seconds. OpenAI's pages name the field without its
/// format, so this reads the forms that such a field has in practice: a Unix time in
/// seconds or milliseconds, a number of seconds from now, or an RFC 3339 time.
fn moment(v: &Value, now: i64) -> Option<i64> {
    if let Some(s) = v.as_str() {
        return match s.trim().parse::<f64>() {
            Ok(n) => moment(&serde_json::json!(n), now),
            Err(_) => chrono::DateTime::parse_from_rfc3339(s.trim()).ok().map(|t| t.timestamp()),
        };
    }
    let n = v.as_f64().filter(|n| n.is_finite() && *n > 0.0)? as i64;
    Some(if n > 100_000_000_000 {
        n / 1000
    } else if n > 1_000_000_000 {
        n
    } else {
        now + n
    })
}

/// A client that the server issued for an account on this computer.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KnownClient {
    pub subject: String,
    pub email: String,
    pub client_id: String,
}

/// What the app keeps about ChatGPT sign-in beside the tokens.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Registry {
    /// Names this computer to the sign-in service. It is not a secret.
    #[serde(default)]
    pub host_id: String,
    /// The notice "You're using your ChatGPT plan" was shown.
    #[serde(default)]
    pub welcomed: bool,
    #[serde(default)]
    pub clients: Vec<KnownClient>,
}

/// The folder of the account records: the keys folder.
#[derive(Clone, Debug)]
pub struct Store {
    pub dir: PathBuf,
}

impl Default for Store {
    fn default() -> Self {
        Store { dir: secrets::keys_dir() }
    }
}

impl Store {
    pub fn account_path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{}-account", secrets::safe_name(name)))
    }

    fn registry_path(&self) -> PathBuf {
        self.dir.join("chatgpt-sign-in")
    }

    pub fn account(&self, name: &str) -> Option<Account> {
        let text = std::fs::read_to_string(self.account_path(name)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save_account(&self, name: &str, account: &Account) -> Result<()> {
        secrets::write_private_file(&self.account_path(name), &serde_json::to_string_pretty(account)?)
    }

    pub fn remove_account(&self, name: &str) -> Result<()> {
        match std::fs::remove_file(self.account_path(name)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn registry(&self) -> Registry {
        std::fs::read_to_string(self.registry_path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn save_registry(&self, registry: &Registry) -> Result<()> {
        secrets::write_private_file(&self.registry_path(), &serde_json::to_string_pretty(registry)?)
    }

    /// The name of this computer for the sign-in service. It is made one time, before
    /// the first sign-in, and then kept.
    pub fn host_id(&self) -> Result<String> {
        let mut registry = self.registry();
        if registry.host_id.is_empty() {
            registry.host_id = format!("urn:uuid:{}", uuid::Uuid::new_v4());
            self.save_registry(&registry)?;
        }
        Ok(registry.host_id)
    }
}

/// Where one account lives and which service it talks to.
#[derive(Clone, Debug)]
pub struct Session {
    pub store: Store,
    pub name: String,
    pub endpoints: Endpoints,
}

impl Session {
    /// The ChatGPT account of the app.
    pub fn app() -> Self {
        Session { store: Store::default(), name: ACCOUNT_NAME.into(), endpoints: Endpoints::default() }
    }

    pub fn account(&self) -> Option<Account> {
        self.store.account(&self.name)
    }
}

/// The values of one sign-in attempt. They are made new each time.
pub struct Attempt {
    /// `NEW_CLIENT`, or the issued client of a returning account.
    pub client_id: String,
    pub state: String,
    pub nonce: String,
    pub verifier: String,
    pub challenge: String,
    pub redirect_uri: String,
}

/// What the app knows about a returning account.
pub struct Hints<'a> {
    /// The saved address: the sign-in page starts with it.
    pub login: Option<&'a str>,
    /// The ID token of the last sign-in: the page skips the account list.
    pub id_token: Option<&'a str>,
    /// Ask for the permissions again (the last sign-in did not allow the plan).
    pub consent: bool,
}

/// The page that the browser opens.
pub fn authorize_url(ep: &Endpoints, attempt: &Attempt, host_id: &str, hints: &Hints) -> String {
    let mut q: Vec<(&str, &str)> = vec![("client_id", &attempt.client_id)];
    if attempt.client_id == NEW_CLIENT {
        q.push(("agent_name_hint", APP_NAME));
    }
    q.extend([
        ("ext_agent_host_id", host_id),
        ("response_type", "code"),
        ("redirect_uri", attempt.redirect_uri.as_str()),
        ("scope", SCOPE),
        ("resource", ep.resource.as_str()),
        ("state", attempt.state.as_str()),
        ("nonce", attempt.nonce.as_str()),
        ("code_challenge_method", "S256"),
        ("code_challenge", attempt.challenge.as_str()),
    ]);
    if let Some(email) = hints.login.filter(|e| !e.is_empty()) {
        q.push(("login_hint", email));
    }
    if let Some(token) = hints.id_token.filter(|t| !t.is_empty()) {
        q.push(("id_token_hint", token));
    }
    if hints.consent {
        q.push(("prompt", "consent"));
    }
    url_with(&ep.authorize_url, &q)
}

/// Who signed in, from a checked ID token.
#[derive(Clone, Debug, PartialEq)]
pub struct Identity {
    pub subject: String,
    pub email: String,
}

fn part(text: &str) -> Result<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(text.trim_end_matches('='))
        .map_err(|_| anyhow!("The ID token is not readable."))
}

/// Checks an ID token: the signature against the published keys of the service, then
/// that it comes from the service, is meant for this client and this attempt, and is
/// not expired.
pub fn verify_id_token(id_token: &str, jwks: &Value, issuer: &str, client_id: &str, nonce: &str, now: i64) -> Result<Identity> {
    let parts: Vec<&str> = id_token.split('.').collect();
    if parts.len() != 3 {
        bail!("The ID token is not readable.");
    }
    let header: Value = serde_json::from_slice(&part(parts[0])?).map_err(|_| anyhow!("The ID token is not readable."))?;
    let claims: Value = serde_json::from_slice(&part(parts[1])?).map_err(|_| anyhow!("The ID token is not readable."))?;
    let signature = part(parts[2])?;
    if header["alg"] != "RS256" {
        bail!("The ID token is not signed with RS256.");
    }
    let signed = format!("{}.{}", parts[0], parts[1]);
    let kid = header["kid"].as_str();
    let number = |k: &Value, name: &str| -> Option<Vec<u8>> {
        let bytes = part(k[name].as_str()?).ok()?;
        let start = bytes.iter().position(|b| *b != 0)?;
        Some(bytes[start..].to_vec())
    };
    let valid = jwks["keys"].as_array().is_some_and(|keys| {
        keys.iter()
            .filter(|k| k["kty"] == "RSA" && (kid.is_none() || k["kid"].as_str() == kid))
            .filter_map(|k| Some(ring::signature::RsaPublicKeyComponents { n: number(k, "n")?, e: number(k, "e")? }))
            .any(|key| key.verify(&ring::signature::RSA_PKCS1_2048_8192_SHA256, signed.as_bytes(), &signature).is_ok())
    });
    if !valid {
        bail!("The signature of the ID token is not valid.");
    }
    if claims["iss"].as_str() != Some(issuer) {
        bail!("The ID token has another issuer.");
    }
    let for_us = match &claims["aud"] {
        Value::String(a) => a == client_id,
        Value::Array(list) => list.iter().any(|a| a.as_str() == Some(client_id)),
        _ => false,
    };
    if !for_us {
        bail!("The ID token has another audience.");
    }
    if claims["exp"].as_i64().unwrap_or(0) <= now - 60 {
        bail!("The ID token is expired.");
    }
    if claims["nonce"].as_str() != Some(nonce) {
        bail!("The ID token has another nonce.");
    }
    let subject = claims["sub"].as_str().unwrap_or("").to_string();
    if subject.is_empty() {
        bail!("The ID token names no subject.");
    }
    Ok(Identity { subject, email: claims["email"].as_str().unwrap_or("").to_string() })
}

/// The person must sign in (again) before this account can pay for a call.
#[derive(Debug)]
pub struct SignInNeeded(pub String);

impl fmt::Display for SignInNeeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SignInNeeded {}

/// True when the error means: the person must sign in again.
pub fn needs_sign_in(e: &anyhow::Error) -> bool {
    e.downcast_ref::<SignInNeeded>().is_some()
}

#[derive(Deserialize, Default)]
struct Tokens {
    #[serde(default)]
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    token_type: Option<String>,
    expires_in: Option<i64>,
    scope: Option<String>,
    earliest_refresh_at: Option<Value>,
}

enum TokenError {
    /// The code or the refresh token is of no use any more: the server's word for it,
    /// then the explanation that the server gave (often none).
    Dead(String, String),
    /// The network, the server, or the setup of the client.
    Other(String),
}

async fn token_request(ep: &Endpoints, form: &[(&str, &str)]) -> Result<Tokens, TokenError> {
    let client = http().map_err(|e| TokenError::Other(e.to_string()))?;
    let resp = client
        .post(&ep.token_url)
        .form(form)
        .send()
        .await
        .map_err(|e| TokenError::Other(format!("cannot reach the sign-in service: {e}")))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if status.is_success() {
        return serde_json::from_str::<Tokens>(&text)
            .ok()
            .filter(|t| !t.access_token.is_empty())
            .ok_or_else(|| TokenError::Other("the answer of the sign-in service holds no access token".into()));
    }
    let v: Value = serde_json::from_str(text.trim()).unwrap_or_default();
    let code = v["error"].as_str().or_else(|| v["error"]["code"].as_str()).or_else(|| v["code"].as_str()).unwrap_or("");
    if DEAD_REFRESH.contains(&code) {
        let said = v["error_description"].as_str().or_else(|| v["error"]["message"].as_str()).or_else(|| v["message"].as_str()).unwrap_or("");
        return Err(TokenError::Dead(code.to_string(), said.trim().trim_end_matches('.').to_string()));
    }
    Err(TokenError::Other(format!("HTTP {}: {}", status.as_u16(), error_text(&text))))
}

/// An address that the service publishes in its discovery document.
async fn discovered(ep: &Endpoints, set: &str, key: &str, fallback: &str) -> String {
    if !set.is_empty() {
        return set.to_string();
    }
    let found = async {
        let text = http().ok()?.get(format!("{}{DISCOVERY_PATH}", ep.issuer)).send().await.ok()?.text().await.ok()?;
        let v: Value = serde_json::from_str(&text).ok()?;
        // Only an address of the service itself.
        v[key].as_str().filter(|u| u.starts_with(&ep.issuer)).map(str::to_string)
    };
    found.await.unwrap_or_else(|| fallback.to_string())
}

/// The result of a sign-in.
pub struct SignedIn {
    pub account: Account,
    /// The first sign-in that allows the plan: show the notice about plan usage.
    pub welcome: bool,
}

/// The whole sign-in. `open` shows the address in the browser. `fresh` registers again
/// (another account); without it, a saved account signs in with its issued client.
pub async fn sign_in<F: FnOnce(&str) -> Result<()>>(sess: &Session, fresh: bool, open: F, cancel: &CancelToken, timeout: Duration) -> Result<SignedIn> {
    let ep = &sess.endpoints;
    let host_id = sess.store.host_id()?;
    let saved = sess.account();
    let returning = saved.as_ref().filter(|a| !fresh && !a.client_id.is_empty());

    // The service accepts 127.0.0.1 only, never "localhost".
    let listener = Loopback::bind("/auth/callback", if returning.is_some() { None } else { Some(FIRST_PORT) }, false).await?;
    let pkce = Pkce::new();
    let attempt = Attempt {
        client_id: returning.map(|a| a.client_id.clone()).unwrap_or_else(|| NEW_CLIENT.into()),
        state: super::random_token(16),
        nonce: super::random_token(16),
        verifier: pkce.verifier,
        challenge: pkce.challenge,
        redirect_uri: listener.url("127.0.0.1"),
    };
    let hints = Hints {
        login: returning.map(|a| a.email.as_str()),
        id_token: returning.map(|a| a.id_token.as_str()),
        consent: returning.is_some_and(|a| !a.scopes.is_empty() && !a.plan_enabled()),
    };
    open(&authorize_url(ep, &attempt, &host_id, &hints))?;
    let answer = listener.wait(timeout, cancel).await?;
    drop(listener);

    if answer.get("state") != Some(&attempt.state) {
        bail!("The answer in the browser did not belong to this sign-in. Start the sign-in again.");
    }
    if let Some(error) = answer.get("error") {
        if error == "access_denied" {
            bail!("You declined the request in ChatGPT. Nothing changed.");
        }
        let detail = answer.get("error_description").map(|d| format!(": {d}")).unwrap_or_default();
        bail!("ChatGPT did not complete the sign-in ({error}{detail}).");
    }
    let Some(code) = answer.get("code").filter(|c| !c.is_empty()) else {
        bail!("ChatGPT sent no code. Start the sign-in again.");
    };
    let issued = answer.get("client_id").filter(|c| !c.is_empty() && c.as_str() != NEW_CLIENT);
    let client_id = match (returning, issued) {
        (None, Some(id)) => id.clone(),
        (None, None) => bail!("ChatGPT sent no client ID for this app. Start the sign-in again."),
        (Some(a), Some(id)) if *id != a.client_id => bail!("ChatGPT answered for another client than the one of this app. Nothing changed."),
        (Some(a), _) => a.client_id.clone(),
    };

    let form = [
        ("grant_type", "authorization_code"),
        ("client_id", client_id.as_str()),
        ("code", code.as_str()),
        ("code_verifier", attempt.verifier.as_str()),
        ("redirect_uri", attempt.redirect_uri.as_str()),
        ("resource", ep.resource.as_str()),
    ];
    let tokens = match token_request(ep, &form).await {
        Ok(tokens) => tokens,
        Err(TokenError::Dead(code, said)) => {
            // OpenAI: after a refused code, the next authorization keeps the issued client
            // and does not register the app again. Only where no account is saved: a
            // refused try must never take the place of a sign-in that works.
            if saved.is_none() {
                let kept = Account { issuer: ep.issuer.clone(), client_id, ext_agent_host_id: host_id, ..Account::default() };
                let _ = sess.store.save_account(&sess.name, &kept);
            }
            // The service gives no reason, and this is how it refuses an account without a
            // plan that other apps may use. Say so: the answer alone reads like a fault.
            let plan = if code == "invalid_grant" {
                " To use a ChatGPT plan in Corydoras, the account needs a Go, Plus, Pro or Business plan; OpenAI refuses a Free account at this step. \
                 With such a plan, start the sign-in again. For another account, choose Use another account."
            } else {
                " Start the sign-in again."
            };
            bail!(
                "ChatGPT did not accept the sign-in code. The sign-in service answered: {code}{}.{plan}",
                if said.is_empty() { String::new() } else { format!(": {said}") }
            );
        }
        Err(TokenError::Other(m)) => bail!("The sign-in did not finish: {m}"),
    };

    let jwks_url = discovered(ep, &ep.jwks_url, "jwks_uri", JWKS_FALLBACK).await;
    let jwks: Value = http()?
        .get(&jwks_url)
        .send()
        .await
        .map_err(|e| anyhow!("Cannot read the keys of the sign-in service: {e}"))?
        .json()
        .await
        .map_err(|e| anyhow!("The keys of the sign-in service are not readable: {e}"))?;
    let now = now_ts();
    let who = verify_id_token(tokens.id_token.as_deref().unwrap_or(""), &jwks, &ep.issuer, &client_id, &attempt.nonce, now)?;
    if let Some(a) = returning.filter(|a| !a.subject.is_empty() && a.subject != who.subject) {
        bail!(
            "You signed in to another ChatGPT account ({}); this app uses {}. Nothing changed. To change the account, choose Use another account.",
            if who.email.is_empty() { "no address" } else { &who.email },
            if a.email.is_empty() { "another one" } else { &a.email }
        );
    }

    let mut account = Account {
        email: if who.email.is_empty() { returning.map(|a| a.email.clone()).unwrap_or_default() } else { who.email },
        issuer: ep.issuer.clone(),
        subject: who.subject,
        client_id,
        ext_agent_host_id: host_id,
        ..Account::default()
    };
    account.take(tokens, now);
    sess.store.save_account(&sess.name, &account)?;

    let mut registry = sess.store.registry();
    registry.clients.retain(|c| c.subject != account.subject);
    registry.clients.push(KnownClient { subject: account.subject.clone(), email: account.email.clone(), client_id: account.client_id.clone() });
    let welcome = account.plan_enabled() && !registry.welcomed;
    registry.welcomed |= welcome;
    sess.store.save_registry(&registry)?;
    Ok(SignedIn { account, welcome })
}

/// The access token for a call. An old one is renewed first.
pub async fn access_token(sess: &Session) -> Result<String> {
    token(sess, None).await
}

/// A server refused `refused`: renew it, unless another call already did.
pub async fn force_refresh(sess: &Session, refused: &str) -> Result<String> {
    token(sess, Some(refused)).await
}

async fn token(sess: &Session, refused: Option<&str>) -> Result<String> {
    let _one = RENEW.lock().await;
    let Some(mut account) = sess.account().filter(Account::signed_in) else {
        return Err(SignInNeeded("Sign in to ChatGPT in Settings to use your ChatGPT plan.".into()).into());
    };
    if !account.plan_enabled() {
        return Err(SignInNeeded(
            "This ChatGPT sign-in does not allow the use of your plan. Sign in again in Settings and allow it, or use a provider with an API key.".into(),
        )
        .into());
    }
    let now = now_ts();
    let renew = match refused {
        Some(old) => account.access_token == old,
        None => account.needs_refresh(now),
    };
    if !renew {
        return Ok(account.access_token);
    }
    let form = [
        ("grant_type", "refresh_token"),
        ("client_id", account.client_id.as_str()),
        ("refresh_token", account.refresh_token.as_str()),
        ("resource", sess.endpoints.resource.as_str()),
    ];
    let answer = if account.refresh_token.is_empty() { Err(TokenError::Dead("no refresh token".into(), String::new())) } else { token_request(&sess.endpoints, &form).await };
    match answer {
        Ok(tokens) => {
            account.take(tokens, now);
            sess.store.save_account(&sess.name, &account)?;
            Ok(account.access_token)
        }
        Err(TokenError::Dead(code, _)) => {
            // Keep the ID token: the next sign-in page can skip the account list with it.
            account.clear_tokens(true);
            sess.store.save_account(&sess.name, &account)?;
            let who = if account.email.is_empty() { String::new() } else { format!(" of {}", account.email) };
            Err(SignInNeeded(format!("The ChatGPT sign-in{who} ended ({code}). Sign in again in Settings.")).into())
        }
        // The token still works for a moment: use it and renew at the next call.
        Err(TokenError::Other(_)) if refused.is_none() && now < account.expires_at() => Ok(account.access_token),
        Err(TokenError::Other(m)) => Err(CallError::Network(format!("the ChatGPT sign-in could not be renewed ({m})")).into()),
    }
}

/// Signs out: asks the service to end the refresh token, then clears the tokens on this
/// computer. Returns false when the service did not confirm; the tokens are gone from
/// this computer all the same, and the person can disconnect the app in ChatGPT.
pub async fn sign_out(sess: &Session) -> Result<bool> {
    let _one = RENEW.lock().await;
    let Some(mut account) = sess.account() else { return Ok(true) };
    let mut confirmed = true;
    if !account.refresh_token.is_empty() {
        let url = discovered(&sess.endpoints, &sess.endpoints.revoke_url, "revocation_endpoint", REVOKE_FALLBACK).await;
        let form = [("token", account.refresh_token.as_str()), ("token_type_hint", "refresh_token"), ("client_id", account.client_id.as_str())];
        confirmed = false;
        for attempt in 1..=3u64 {
            let status = match http()?.post(&url).form(&form).send().await {
                Ok(resp) => resp.status().as_u16(),
                Err(_) => 0,
            };
            if status == 200 {
                confirmed = true;
                break;
            }
            // Only a failed connection or a server error can pass by itself.
            if status != 0 && status < 500 {
                break;
            }
            if attempt < 3 {
                tokio::time::sleep(Duration::from_millis(400 * attempt)).await;
            }
        }
    }
    if account.signed_in() || !account.id_token.is_empty() {
        account.clear_tokens(false);
        sess.store.save_account(&sess.name, &account)?;
    }
    Ok(confirmed)
}

/// The name of the account for the Settings page.
pub fn display_name(account: &Account) -> String {
    if account.email.is_empty() {
        "your ChatGPT account".into()
    } else {
        account.email.clone()
    }
}
