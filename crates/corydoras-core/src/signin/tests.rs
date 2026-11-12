use super::chatgpt::{self, Account, Attempt, Endpoints, Hints, Session, Store};
use super::{openrouter, Loopback, Pkce};
use crate::activity::CancelToken;
use crate::testserver::{query_of, serve, Resp};
use crate::util::now_ts;
use base64::Engine;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const ISSUER: &str = "https://auth.test";
const ALL_SCOPES: &str = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn tmp() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cory-signin-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A throwaway RSA key that was made for these tests. It signs nothing outside them.
fn key_pair() -> ring::signature::RsaKeyPair {
    let der = base64::engine::general_purpose::STANDARD
        .decode(include_str!("testdata/id_token_test_key.der.b64").trim())
        .unwrap();
    ring::signature::RsaKeyPair::from_der(&der).unwrap()
}

/// The public half of the test key, the way a server publishes it.
fn jwks() -> Value {
    let pair = key_pair();
    let public: ring::signature::RsaPublicKeyComponents<Vec<u8>> = pair.public().into();
    json!({"keys": [{"kty": "RSA", "kid": "test-key", "alg": "RS256", "use": "sig", "n": b64(&public.n), "e": b64(&public.e)}]})
}

/// An ID token with these claims, signed with the test key.
fn id_token_with(header: Value, claims: Value) -> String {
    let pair = key_pair();
    let input = format!("{}.{}", b64(header.to_string().as_bytes()), b64(claims.to_string().as_bytes()));
    let mut sig = vec![0u8; pair.public().modulus_len()];
    pair.sign(&ring::signature::RSA_PKCS1_SHA256, &ring::rand::SystemRandom::new(), input.as_bytes(), &mut sig).unwrap();
    format!("{input}.{}", b64(&sig))
}

fn id_token(claims: Value) -> String {
    id_token_with(json!({"alg": "RS256", "kid": "test-key", "typ": "JWT"}), claims)
}

fn claims(sub: &str, email: &str, client_id: &str, nonce: &str) -> Value {
    json!({"iss": ISSUER, "aud": client_id, "sub": sub, "email": email, "exp": now_ts() + 3600, "iat": now_ts(), "nonce": nonce})
}

fn session(dir: &Path, base: &str) -> Session {
    Session {
        store: Store { dir: dir.to_path_buf() },
        name: "provider-p-test".into(),
        endpoints: Endpoints {
            authorize_url: format!("{base}/authorize"),
            token_url: format!("{base}/token"),
            revoke_url: format!("{base}/revoke"),
            jwks_url: format!("{base}/jwks"),
            issuer: ISSUER.into(),
            resource: "https://api.openai.com/v1".into(),
        },
    }
}

/// A signed-in account whose access token is `age` seconds old.
fn account(age: i64) -> Account {
    Account {
        email: "ada@example.com".into(),
        issuer: ISSUER.into(),
        subject: "user-1".into(),
        client_id: "oaiapp_1".into(),
        ext_agent_host_id: "urn:uuid:11111111-2222-4333-8444-555555555555".into(),
        id_token: "old-id-token".into(),
        access_token: "at-old".into(),
        refresh_token: "rt-1".into(),
        token_type: "Bearer".into(),
        expires_in: 3600,
        scopes: ALL_SCOPES.split(' ').map(str::to_string).collect(),
        saved_at: now_ts() - age,
        earliest_refresh_at: None,
    }
}

/// The browser arrives at a URL a moment later.
fn visit(url: String) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        let _ = reqwest::get(&url).await;
    });
}

fn token_answer(access: &str, refresh: &str, id: String, scope: &str) -> Resp {
    Resp::json(
        200,
        json!({"access_token": access, "refresh_token": refresh, "id_token": id, "token_type": "Bearer", "expires_in": 3600, "scope": scope}),
    )
}

/// A stand-in for OpenAI's sign-in servers. `subject` is the account that "signs in";
/// the test puts the nonce of the attempt in `nonce` when the browser opens.
async fn openai(subject: &'static str, scope: &'static str, nonce: Arc<Mutex<String>>) -> crate::testserver::Server {
    serve(move |req| match req.path.as_str() {
        "/token" => {
            let form = req.form();
            if form.get("grant_type").map(String::as_str) == Some("refresh_token") {
                return token_answer("at-2", "rt-2", String::new(), scope);
            }
            let n = nonce.lock().unwrap().clone();
            let email = format!("{subject}@example.com");
            token_answer("at-1", "rt-1", id_token(claims(subject, &email, "oaiapp_1", &n)), scope)
        }
        "/jwks" => Resp::json(200, jwks()),
        "/revoke" => Resp::empty(200),
        _ => Resp::empty(404),
    })
    .await
}

// ---------------------------------------------------------------------- shared parts

#[test]
fn pkce_challenge_is_the_sha256_of_the_verifier() {
    // The example of RFC 7636, appendix B.
    assert_eq!(Pkce::challenge_of("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    let (a, b) = (Pkce::new(), Pkce::new());
    assert_ne!(a.verifier, b.verifier);
    assert!((43..=128).contains(&a.verifier.len()), "{}", a.verifier.len());
    assert!(a.verifier.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    assert_eq!(a.challenge, Pkce::challenge_of(&a.verifier));
}

#[tokio::test]
async fn the_listener_waits_for_its_own_path_and_reads_the_answer() {
    let lb = Loopback::bind("/callback", None, false).await.unwrap();
    assert_eq!(lb.url("127.0.0.1"), format!("http://127.0.0.1:{}/callback", lb.port()));
    let base = format!("http://127.0.0.1:{}", lb.port());
    tokio::spawn(async move {
        // A browser asks for other things too; they are no answer.
        let other = reqwest::get(format!("{base}/favicon.ico")).await.unwrap();
        assert_eq!(other.status().as_u16(), 404);
        let page = reqwest::get(format!("{base}/callback?code=a%20b&state=s1")).await.unwrap();
        assert_eq!(page.status().as_u16(), 200);
        assert!(page.text().await.unwrap().contains("Corydoras"));
    });
    let got = lb.wait(Duration::from_secs(5), &CancelToken::default()).await.unwrap();
    assert_eq!(got.get("code").map(String::as_str), Some("a b"));
    assert_eq!(got.get("state").map(String::as_str), Some("s1"));
}

#[tokio::test]
async fn the_listener_stops_on_cancel_and_after_the_time_limit() {
    let lb = Loopback::bind("/callback", None, false).await.unwrap();
    let late = lb.wait(Duration::from_millis(60), &CancelToken::default()).await.unwrap_err();
    assert!(late.to_string().contains("No answer"), "{late}");

    let cancel = Arc::new(CancelToken::default());
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        c2.cancel();
    });
    let stopped = lb.wait(Duration::from_secs(5), &cancel).await.unwrap_err();
    assert!(stopped.to_string().contains("cancelled"), "{stopped}");
}

#[tokio::test]
async fn the_listener_takes_the_preferred_port_or_another_when_it_is_busy() {
    let first = Loopback::bind("/auth/callback", None, false).await.unwrap();
    // The port of `first` is busy now: the next listener must move to another one.
    let second = Loopback::bind("/auth/callback", Some(first.port()), false).await.unwrap();
    assert_ne!(second.port(), first.port());
    drop(first);
}

// ---------------------------------------------------------------------- OpenRouter

#[test]
fn the_openrouter_address_carries_the_callback_and_the_challenge() {
    let url = openrouter::auth_url("https://openrouter.ai", "http://localhost:5123/callback", "CHALLENGE", "STATE", "Corydoras");
    assert!(url.starts_with("https://openrouter.ai/auth?"), "{url}");
    let q = query_of(&url);
    assert_eq!(q["callback_url"], "http://localhost:5123/callback");
    assert_eq!(q["code_challenge"], "CHALLENGE");
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(q["state"], "STATE");
    assert_eq!(q["key_label"], "Corydoras");
}

#[tokio::test]
async fn openrouter_sign_in_returns_the_key_of_the_exchange() {
    let server = serve(|req| match req.path.as_str() {
        "/api/v1/auth/keys" => Resp::json(200, json!({"key": "sk-or-v1-test"})),
        _ => Resp::empty(404),
    })
    .await;
    let challenge = Arc::new(Mutex::new(String::new()));
    let c2 = challenge.clone();
    let key = openrouter::sign_in(
        &server.base,
        move |url| {
            let q = query_of(url);
            assert!(q["callback_url"].starts_with("http://localhost:"), "{url}");
            *c2.lock().unwrap() = q["code_challenge"].clone();
            visit(format!("{}?code=code-1&state={}", q["callback_url"], q["state"]));
            Ok(())
        },
        &CancelToken::default(),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_eq!(key, "sk-or-v1-test");
    let sent = server.seen_at("/api/v1/auth/keys");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, "POST");
    let body = sent[0].json();
    assert_eq!(body["code"], "code-1");
    assert_eq!(body["code_challenge_method"], "S256");
    // The verifier that goes to the exchange belongs to the challenge that the browser saw.
    assert_eq!(Pkce::challenge_of(body["code_verifier"].as_str().unwrap()), *challenge.lock().unwrap());
}

#[tokio::test]
async fn openrouter_sign_in_refuses_an_answer_of_another_attempt() {
    let server = serve(|_| Resp::json(200, json!({"key": "sk-or-v1-test"}))).await;
    let e = openrouter::sign_in(
        &server.base,
        |url| {
            visit(format!("{}?code=code-1&state=not-mine", query_of(url)["callback_url"]));
            Ok(())
        },
        &CancelToken::default(),
        Duration::from_secs(5),
    )
    .await
    .unwrap_err();
    assert!(e.to_string().contains("did not belong"), "{e}");
    assert!(server.seen().is_empty(), "no code may go to the exchange");
}

#[tokio::test]
async fn openrouter_says_why_the_exchange_failed() {
    let server = serve(|_| Resp::json(403, json!({"error": {"message": "Invalid code or code_verifier", "code": 403}}))).await;
    let e = openrouter::exchange(&server.base, "code-1", "verifier").await.unwrap_err();
    assert!(e.to_string().contains("Invalid code or code_verifier"), "{e}");
}

// ---------------------------------------------------------------------- ChatGPT: the parts

#[test]
fn the_host_id_is_made_once_and_kept() {
    let store = Store { dir: tmp() };
    let id = store.host_id().unwrap();
    assert!(id.starts_with("urn:uuid:") && id.len() == "urn:uuid:".len() + 36, "{id}");
    assert_eq!(store.host_id().unwrap(), id);
    assert_eq!(Store { dir: store.dir.clone() }.registry().host_id, id);
}

#[test]
fn an_account_record_is_a_private_file() {
    let store = Store { dir: tmp().join("keys") };
    assert!(store.account("provider-p-test").is_none());
    let a = account(0);
    store.save_account("provider-p-test", &a).unwrap();
    assert_eq!(store.account("provider-p-test"), Some(a));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: PathBuf| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(store.dir.clone()), 0o700);
        assert_eq!(mode(store.account_path("provider-p-test")), 0o600);
    }
    store.remove_account("provider-p-test").unwrap();
    assert!(store.account("provider-p-test").is_none());
    store.remove_account("provider-p-test").unwrap();
}

#[test]
fn account_states() {
    let a = account(0);
    assert!(a.signed_in() && a.plan_enabled());
    // Fresh: no refresh. In the last five minutes, or expired: refresh.
    assert!(!a.needs_refresh(now_ts()));
    assert!(account(3400).needs_refresh(now_ts()));
    assert!(account(4000).needs_refresh(now_ts()));
    // The server can name a first moment for the refresh; a token that still works waits for it.
    let mut waits = account(3400);
    waits.earliest_refresh_at = Some(now_ts() + 60);
    assert!(!waits.needs_refresh(now_ts()));
    waits.saved_at = now_ts() - 4000;
    assert!(waits.needs_refresh(now_ts()), "an expired token does not wait");
    // Without the plan permission the account cannot pay for calls.
    let mut no_plan = account(0);
    no_plan.scopes.retain(|s| s != chatgpt::PLAN_SCOPE);
    assert!(no_plan.signed_in() && !no_plan.plan_enabled());
    let mut out = account(0);
    out.access_token.clear();
    out.refresh_token.clear();
    assert!(!out.signed_in());
}

fn attempt(client_id: &str) -> Attempt {
    Attempt {
        client_id: client_id.into(),
        state: "STATE".into(),
        nonce: "NONCE".into(),
        verifier: "VERIFIER".into(),
        challenge: "CHALLENGE".into(),
        redirect_uri: "http://127.0.0.1:1455/auth/callback".into(),
    }
}

#[test]
fn the_first_sign_in_asks_for_a_new_client() {
    let ep = session(Path::new("/nowhere"), "https://auth.test").endpoints;
    let url = chatgpt::authorize_url(&ep, &attempt(chatgpt::NEW_CLIENT), "urn:uuid:host", &Hints { login: None, id_token: None, consent: false });
    assert!(url.starts_with("https://auth.test/authorize?"), "{url}");
    let q = query_of(&url);
    assert_eq!(q["client_id"], "dynamic_agent_client");
    assert_eq!(q["agent_name_hint"], "Corydoras");
    assert_eq!(q["ext_agent_host_id"], "urn:uuid:host");
    assert_eq!(q["response_type"], "code");
    assert_eq!(q["redirect_uri"], "http://127.0.0.1:1455/auth/callback");
    assert_eq!(q["scope"], ALL_SCOPES);
    assert_eq!(q["resource"], "https://api.openai.com/v1");
    assert_eq!((q["state"].as_str(), q["nonce"].as_str()), ("STATE", "NONCE"));
    assert_eq!((q["code_challenge"].as_str(), q["code_challenge_method"].as_str()), ("CHALLENGE", "S256"));
    for absent in ["login_hint", "id_token_hint", "prompt"] {
        assert!(!q.contains_key(absent), "{absent} in {url}");
    }
}

#[test]
fn a_returning_sign_in_uses_the_issued_client_and_the_hints() {
    let ep = session(Path::new("/nowhere"), "https://auth.test").endpoints;
    let hints = Hints { login: Some("ada@example.com"), id_token: Some("ID.TOKEN.X"), consent: true };
    let q = query_of(&chatgpt::authorize_url(&ep, &attempt("oaiapp_1"), "urn:uuid:host", &hints));
    assert_eq!(q["client_id"], "oaiapp_1");
    assert!(!q.contains_key("agent_name_hint"), "the name goes only with a new registration");
    assert_eq!(q["login_hint"], "ada@example.com");
    assert_eq!(q["id_token_hint"], "ID.TOKEN.X");
    assert_eq!(q["prompt"], "consent");
    assert_eq!(q["ext_agent_host_id"], "urn:uuid:host");
}

#[test]
fn an_id_token_must_be_signed_and_meant_for_this_sign_in() {
    let keys = jwks();
    let check = |token: &str| chatgpt::verify_id_token(token, &keys, ISSUER, "oaiapp_1", "n-1", now_ts());
    let good = claims("user-1", "ada@example.com", "oaiapp_1", "n-1");
    let who = check(&id_token(good.clone())).unwrap();
    assert_eq!((who.subject.as_str(), who.email.as_str()), ("user-1", "ada@example.com"));
    // An audience list that holds the client is fine too.
    let mut list = good.clone();
    list["aud"] = json!(["other", "oaiapp_1"]);
    assert!(check(&id_token(list)).is_ok());

    let broken = |change: &dyn Fn(&mut Value)| {
        let mut c = good.clone();
        change(&mut c);
        check(&id_token(c)).unwrap_err().to_string()
    };
    assert!(broken(&|c| c["nonce"] = json!("other")).contains("nonce"));
    assert!(broken(&|c| c["aud"] = json!("oaiapp_2")).contains("audience"));
    assert!(broken(&|c| c["iss"] = json!("https://evil.test")).contains("issuer"));
    assert!(broken(&|c| c["exp"] = json!(now_ts() - 600)).contains("expired"));
    assert!(broken(&|c| c["sub"] = json!("")).contains("subject"));

    // Changed claims under an old signature.
    let token = id_token(good.clone());
    let mut parts: Vec<&str> = token.split('.').collect();
    let forged = b64(claims("user-2", "eve@example.com", "oaiapp_1", "n-1").to_string().as_bytes());
    parts[1] = &forged;
    assert!(check(&parts.join(".")).unwrap_err().to_string().contains("signature"));
    // No signature at all.
    let unsigned = id_token_with(json!({"alg": "none"}), good.clone());
    assert!(check(&unsigned).unwrap_err().to_string().contains("RS256"));
    // A key that the server does not publish.
    let unknown = id_token_with(json!({"alg": "RS256", "kid": "another-key"}), good);
    assert!(check(&unknown).unwrap_err().to_string().contains("signature"));
    assert!(check("not-a-token").is_err());
}

// ---------------------------------------------------------------------- ChatGPT: sign-in

#[tokio::test]
async fn the_first_chatgpt_sign_in_registers_a_client_and_saves_the_account() {
    let dir = tmp();
    let nonce = Arc::new(Mutex::new(String::new()));
    let server = openai("user-1", ALL_SCOPES, nonce.clone()).await;
    let sess = session(&dir, &server.base);
    let opened = Arc::new(Mutex::new(String::new()));
    let o2 = opened.clone();
    let done = chatgpt::sign_in(
        &sess,
        false,
        move |url| {
            *o2.lock().unwrap() = url.to_string();
            let q = query_of(url);
            *nonce.lock().unwrap() = q["nonce"].clone();
            visit(format!("{}?code=code-1&state={}&client_id=oaiapp_1&scope=x", q["redirect_uri"], q["state"]));
            Ok(())
        },
        &CancelToken::default(),
        Duration::from_secs(5),
    )
    .await
    .unwrap();

    // What the browser was sent to.
    let q = query_of(&opened.lock().unwrap());
    assert_eq!(q["client_id"], chatgpt::NEW_CLIENT);
    assert_eq!(q["agent_name_hint"], "Corydoras");
    assert!(q["redirect_uri"].starts_with("http://127.0.0.1:") && q["redirect_uri"].ends_with("/auth/callback"), "{}", q["redirect_uri"]);
    assert!(q["ext_agent_host_id"].starts_with("urn:uuid:"));

    // What went to the token endpoint: the issued client, never the registration name.
    let sent = server.seen_at("/token");
    assert_eq!(sent.len(), 1);
    let form = sent[0].form();
    assert_eq!(form["grant_type"], "authorization_code");
    assert_eq!(form["client_id"], "oaiapp_1");
    assert_eq!(form["code"], "code-1");
    assert_eq!(form["redirect_uri"], q["redirect_uri"]);
    assert_eq!(form["resource"], "https://api.openai.com/v1");
    assert_eq!(Pkce::challenge_of(&form["code_verifier"]), q["code_challenge"]);
    assert!(sent[0].header("content-type").starts_with("application/x-www-form-urlencoded"));

    // What is saved.
    assert!(done.welcome, "the first sign-in with the plan shows the notice");
    let saved = sess.store.account(&sess.name).unwrap();
    assert_eq!(saved, done.account);
    assert_eq!((saved.subject.as_str(), saved.email.as_str(), saved.client_id.as_str()), ("user-1", "user-1@example.com", "oaiapp_1"));
    assert_eq!((saved.access_token.as_str(), saved.refresh_token.as_str()), ("at-1", "rt-1"));
    assert_eq!(saved.ext_agent_host_id, q["ext_agent_host_id"]);
    assert_eq!(saved.issuer, ISSUER);
    assert!(saved.signed_in() && saved.plan_enabled());
    assert!((now_ts() - saved.saved_at).abs() < 5);
    let reg = sess.store.registry();
    assert!(reg.welcomed);
    assert_eq!(reg.clients.len(), 1);
    assert_eq!(reg.clients[0].client_id, "oaiapp_1");
}

/// Signs in the way the test above does and returns what the browser was sent to.
async fn sign_in_as(sess: &Session, nonce: Arc<Mutex<String>>, fresh: bool, answer: &'static str) -> (anyhow::Result<chatgpt::SignedIn>, String) {
    let opened = Arc::new(Mutex::new(String::new()));
    let o2 = opened.clone();
    let r = chatgpt::sign_in(
        sess,
        fresh,
        move |url| {
            *o2.lock().unwrap() = url.to_string();
            let q = query_of(url);
            *nonce.lock().unwrap() = q["nonce"].clone();
            visit(format!("{}?state={}{answer}", q["redirect_uri"], q["state"]));
            Ok(())
        },
        &CancelToken::default(),
        Duration::from_secs(5),
    )
    .await;
    let url = opened.lock().unwrap().clone();
    (r, url)
}

#[tokio::test]
async fn a_later_chatgpt_sign_in_reuses_the_client_and_shows_no_notice() {
    let dir = tmp();
    let nonce = Arc::new(Mutex::new(String::new()));
    let server = openai("user-1", ALL_SCOPES, nonce.clone()).await;
    let sess = session(&dir, &server.base);
    let (first, url1) = sign_in_as(&sess, nonce.clone(), false, "&code=c1&client_id=oaiapp_1").await;
    assert!(first.unwrap().welcome);
    chatgpt::sign_out(&sess).await.unwrap();

    // The reauthorization can come back without a client ID.
    let (second, url2) = sign_in_as(&sess, nonce.clone(), false, "&code=c2").await;
    let second = second.unwrap();
    assert!(!second.welcome, "the notice shows one time only");
    let (q1, q2) = (query_of(&url1), query_of(&url2));
    assert_eq!(q2["client_id"], "oaiapp_1");
    assert!(!q2.contains_key("agent_name_hint"));
    assert_eq!(q2["login_hint"], "user-1@example.com");
    assert!(!q2.contains_key("id_token_hint"), "no ID token is kept after sign-out");
    assert_eq!(q2["ext_agent_host_id"], q1["ext_agent_host_id"]);
    assert!(second.account.signed_in());
    assert_eq!(sess.store.registry().clients.len(), 1);

    // "Use another account" registers again, whatever is saved.
    let (_, url3) = sign_in_as(&sess, nonce, true, "&code=c3&client_id=oaiapp_1").await;
    let q3 = query_of(&url3);
    assert_eq!(q3["client_id"], chatgpt::NEW_CLIENT);
    assert!(!q3.contains_key("login_hint"));
}

#[tokio::test]
async fn a_reauthorization_that_returns_another_account_changes_nothing() {
    let dir = tmp();
    let nonce = Arc::new(Mutex::new(String::new()));
    // The server signs every token for user-2.
    let server = openai("user-2", ALL_SCOPES, nonce.clone()).await;
    let sess = session(&dir, &server.base);
    let mut mine = account(0);
    mine.access_token.clear();
    mine.refresh_token.clear();
    mine.id_token.clear();
    sess.store.save_account(&sess.name, &mine).unwrap();

    let (r, _) = sign_in_as(&sess, nonce.clone(), false, "&code=c1").await;
    let e = r.err().unwrap().to_string();
    assert!(e.contains("another ChatGPT account"), "{e}");
    assert_eq!(sess.store.account(&sess.name), Some(mine.clone()));

    // A client ID that differs from the pending one is refused before any code is exchanged.
    let before = server.seen_at("/token").len();
    let (r, _) = sign_in_as(&sess, nonce, false, "&code=c1&client_id=oaiapp_9").await;
    assert!(r.is_err());
    assert_eq!(server.seen_at("/token").len(), before);
    assert_eq!(sess.store.account(&sess.name), Some(mine));
}

#[tokio::test]
async fn a_declined_or_foreign_answer_exchanges_no_code() {
    let dir = tmp();
    let nonce = Arc::new(Mutex::new(String::new()));
    let server = openai("user-1", ALL_SCOPES, nonce.clone()).await;
    let sess = session(&dir, &server.base);

    let (r, _) = sign_in_as(&sess, nonce.clone(), false, "&error=access_denied").await;
    let e = r.err().unwrap().to_string();
    assert!(e.contains("declined"), "{e}");

    // A new registration must come back with its client ID.
    let (r, _) = sign_in_as(&sess, nonce.clone(), false, "&code=c1").await;
    assert!(r.err().unwrap().to_string().contains("client"));

    // An answer with another state is not ours.
    let r = chatgpt::sign_in(
        &sess,
        false,
        |url| {
            visit(format!("{}?code=c1&state=not-mine&client_id=oaiapp_1", query_of(url)["redirect_uri"]));
            Ok(())
        },
        &CancelToken::default(),
        Duration::from_secs(5),
    )
    .await;
    assert!(r.err().unwrap().to_string().contains("did not belong"));

    assert!(server.seen_at("/token").is_empty());
    assert!(sess.store.account(&sess.name).is_none());
    assert!(!sess.store.registry().welcomed);
}

#[tokio::test]
async fn a_refused_sign_in_code_says_what_the_service_answered() {
    // The service names its reason in more than one shape; the person must get to read it.
    for (answer, want) in [
        (json!({"error": "invalid_grant", "error_description": "The code was issued for another redirect address."}), "answered: invalid_grant: The code was issued for another redirect address."),
        (json!({"error": "invalid_grant"}), "answered: invalid_grant."),
        (json!({"error": {"code": "invalid_grant", "message": "The code is expired"}}), "answered: invalid_grant: The code is expired."),
    ] {
        let dir = tmp();
        let server = serve(move |req| match req.path.as_str() {
            "/token" => Resp::json(400, answer.clone()),
            _ => Resp::empty(404),
        })
        .await;
        let sess = session(&dir, &server.base);
        let (r, _) = sign_in_as(&sess, Arc::default(), false, "&code=c1&client_id=oaiapp_1").await;
        let e = r.err().unwrap().to_string();
        assert!(e.starts_with("ChatGPT did not accept the sign-in code."), "{e}");
        assert!(e.contains(want), "{e}");
        assert!(e.ends_with("Start the sign-in again."), "{e}");
        assert!(sess.store.account(&sess.name).is_none(), "nothing is saved");
    }
}

#[tokio::test]
async fn sign_in_without_the_plan_permission_is_kept_but_cannot_pay_for_calls() {
    let dir = tmp();
    let nonce = Arc::new(Mutex::new(String::new()));
    let server = openai("user-1", "openid profile email offline_access", nonce.clone()).await;
    let sess = session(&dir, &server.base);
    let (r, _) = sign_in_as(&sess, nonce.clone(), false, "&code=c1&client_id=oaiapp_1").await;
    let done = r.unwrap();
    assert!(done.account.signed_in() && !done.account.plan_enabled());
    assert!(!done.welcome);
    let e = chatgpt::access_token(&sess).await.unwrap_err();
    assert!(e.to_string().contains("plan"), "{e}");
    assert!(!chatgpt::needs_sign_in(&e) || e.to_string().contains("Sign in again"));
    // The next sign-in asks ChatGPT to show the permission again.
    let (_, url) = sign_in_as(&sess, nonce, false, "&code=c2").await;
    assert_eq!(query_of(&url)["prompt"], "consent");
}

// ---------------------------------------------------------------------- ChatGPT: tokens

#[tokio::test]
async fn a_fresh_access_token_is_used_as_it_is() {
    let dir = tmp();
    let server = openai("user-1", ALL_SCOPES, Arc::default()).await;
    let sess = session(&dir, &server.base);
    sess.store.save_account(&sess.name, &account(10)).unwrap();
    assert_eq!(chatgpt::access_token(&sess).await.unwrap(), "at-old");
    assert!(server.seen().is_empty());
}

#[tokio::test]
async fn an_old_access_token_is_renewed_and_the_new_refresh_token_is_kept() {
    let dir = tmp();
    let server = openai("user-1", ALL_SCOPES, Arc::default()).await;
    let sess = session(&dir, &server.base);
    sess.store.save_account(&sess.name, &account(3500)).unwrap();

    assert_eq!(chatgpt::access_token(&sess).await.unwrap(), "at-2");
    let sent = server.seen_at("/token");
    assert_eq!(sent.len(), 1);
    let form = sent[0].form();
    assert_eq!(form["grant_type"], "refresh_token");
    assert_eq!(form["client_id"], "oaiapp_1");
    assert_eq!(form["refresh_token"], "rt-1");
    assert_eq!(form["resource"], "https://api.openai.com/v1");
    assert!(!form.contains_key("scope"), "no scope: the grant stays as it is");

    let saved = sess.store.account(&sess.name).unwrap();
    assert_eq!((saved.access_token.as_str(), saved.refresh_token.as_str()), ("at-2", "rt-2"));
    assert!((now_ts() - saved.saved_at).abs() < 5);
    // The answer had no ID token: the old one stays for the next sign-in.
    assert_eq!(saved.id_token, "old-id-token");
    assert_eq!(saved.client_id, "oaiapp_1");

    // The new token is fresh: no second request.
    assert_eq!(chatgpt::access_token(&sess).await.unwrap(), "at-2");
    assert_eq!(server.seen_at("/token").len(), 1);
}

#[tokio::test]
async fn a_refresh_token_that_ended_asks_for_a_new_sign_in() {
    let dir = tmp();
    let server = serve(|_| Resp::json(400, json!({"error": "invalid_grant", "error_description": "The refresh token was revoked."}))).await;
    let sess = session(&dir, &server.base);
    sess.store.save_account(&sess.name, &account(4000)).unwrap();

    let e = chatgpt::access_token(&sess).await.unwrap_err();
    assert!(chatgpt::needs_sign_in(&e), "{e}");
    assert!(e.to_string().contains("Sign in"), "{e}");
    let saved = sess.store.account(&sess.name).unwrap();
    assert!(!saved.signed_in());
    assert!(saved.access_token.is_empty() && saved.refresh_token.is_empty());
    // The client and the address stay, so the next sign-in is a reauthorization.
    assert_eq!((saved.client_id.as_str(), saved.email.as_str(), saved.subject.as_str()), ("oaiapp_1", "ada@example.com", "user-1"));
    // The second call does not ask the server again.
    let again = chatgpt::access_token(&sess).await.unwrap_err();
    assert!(chatgpt::needs_sign_in(&again));
    assert_eq!(server.seen().len(), 1);
}

#[tokio::test]
async fn openai_names_for_a_dead_refresh_token_are_all_understood() {
    for code in ["invalid_refresh_token", "token_expired", "refresh_token_expired", "refresh_token_invalidated", "refresh_token_reused"] {
        let dir = tmp();
        let server = serve(move |_| Resp::json(401, json!({"error": {"code": code, "message": "no"}}))).await;
        let sess = session(&dir, &server.base);
        sess.store.save_account(&sess.name, &account(4000)).unwrap();
        let e = chatgpt::access_token(&sess).await.unwrap_err();
        assert!(chatgpt::needs_sign_in(&e), "{code}: {e}");
    }
}

#[tokio::test]
async fn a_refresh_that_fails_for_another_reason_keeps_the_tokens() {
    let dir = tmp();
    let server = serve(|_| Resp::json(503, json!({"error": "temporarily_unavailable"}))).await;
    let sess = session(&dir, &server.base);

    // The access token still works for some minutes: use it.
    sess.store.save_account(&sess.name, &account(3400)).unwrap();
    assert_eq!(chatgpt::access_token(&sess).await.unwrap(), "at-old");

    // Expired: the call fails, but nothing is thrown away.
    sess.store.save_account(&sess.name, &account(4000)).unwrap();
    let e = chatgpt::access_token(&sess).await.unwrap_err();
    assert!(!chatgpt::needs_sign_in(&e), "{e}");
    assert_eq!(sess.store.account(&sess.name), Some(account_like(&sess, 4000)));

    // A wrong client is a fault of the setup, not of the person: nothing is thrown away.
    let server = serve(|_| Resp::json(401, json!({"error": "invalid_client"}))).await;
    let sess = session(&dir, &server.base);
    let e = chatgpt::access_token(&sess).await.unwrap_err();
    assert!(!chatgpt::needs_sign_in(&e), "{e}");
    assert!(sess.store.account(&sess.name).unwrap().signed_in());
}

/// The saved account, to compare with: `account(age)` with the time that was saved.
fn account_like(sess: &Session, age: i64) -> Account {
    let mut a = account(age);
    a.saved_at = sess.store.account(&sess.name).unwrap().saved_at;
    a
}

#[tokio::test]
async fn a_refused_token_is_renewed_one_time_for_all_callers() {
    let dir = tmp();
    let server = openai("user-1", ALL_SCOPES, Arc::default()).await;
    let sess = session(&dir, &server.base);
    sess.store.save_account(&sess.name, &account(10)).unwrap();
    // The server refused "at-old" although it looks fresh.
    assert_eq!(chatgpt::force_refresh(&sess, "at-old").await.unwrap(), "at-2");
    // Another call that was refused with the same old token gets the renewed one.
    assert_eq!(chatgpt::force_refresh(&sess, "at-old").await.unwrap(), "at-2");
    assert_eq!(server.seen_at("/token").len(), 1);
}

#[tokio::test]
async fn sign_out_revokes_the_refresh_token_and_keeps_the_client() {
    let dir = tmp();
    let server = openai("user-1", ALL_SCOPES, Arc::default()).await;
    let sess = session(&dir, &server.base);
    sess.store.save_account(&sess.name, &account(10)).unwrap();

    assert!(chatgpt::sign_out(&sess).await.unwrap(), "the server confirmed");
    let sent = server.seen_at("/revoke");
    assert_eq!(sent.len(), 1);
    let form = sent[0].form();
    assert_eq!(form["token"], "rt-1");
    assert_eq!(form["token_type_hint"], "refresh_token");
    assert_eq!(form["client_id"], "oaiapp_1");

    let saved = sess.store.account(&sess.name).unwrap();
    assert!(!saved.signed_in());
    assert!(saved.access_token.is_empty() && saved.refresh_token.is_empty() && saved.id_token.is_empty());
    assert_eq!((saved.client_id.as_str(), saved.email.as_str()), ("oaiapp_1", "ada@example.com"));
    // Signed out already: nothing to revoke.
    assert!(chatgpt::sign_out(&sess).await.unwrap());
    assert_eq!(server.seen_at("/revoke").len(), 1);
}

#[tokio::test]
async fn sign_out_clears_the_tokens_also_when_the_server_does_not_confirm() {
    let dir = tmp();
    let server = serve(|_| Resp::empty(500)).await;
    let sess = session(&dir, &server.base);
    sess.store.save_account(&sess.name, &account(10)).unwrap();
    assert!(!chatgpt::sign_out(&sess).await.unwrap(), "not confirmed");
    assert!(server.seen_at("/revoke").len() > 1, "a server error is tried again");
    assert!(!sess.store.account(&sess.name).unwrap().signed_in());
}
