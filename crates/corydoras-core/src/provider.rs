//! OpenAI-compatible chat provider. The default is DeepSeek (deepseek-flash).
//!
//! Calls stream (server-sent events), so the app can show live progress, stop
//! a call at once when you cancel it, and detect a dead connection after
//! IDLE_SECS without data. DeepSeek sends ": keep-alive" comments while a
//! request waits in its queue; they count as data.
//!
//! DeepSeek turns thinking mode on by default, and reasoning tokens count
//! against max_tokens. Every call sets the thinking mode explicitly and adds a
//! token allowance for the reasoning.

use crate::activity::{Activity, CancelToken, StreamEvent};
use crate::ai::{Api, ProviderProfile};
use crate::util::truncate;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::fmt;
use std::future::Future;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A stream with no bytes for this long is treated as dead.
pub const IDLE_SECS: u64 = 120;
/// Hard limit for one call.
pub const CALL_CAP_SECS: u64 = 25 * 60;

pub enum Part {
    Text(String),
    /// A data URL or an http(s) URL of an image.
    Image(String),
}

/// Thinking mode for one call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Thinking {
    Off,
    Low,
    High,
}

impl Thinking {
    /// "off", "low" or "high" (from the settings). Unknown values mean Off.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "low" => Thinking::Low,
            "high" => Thinking::High,
            _ => Thinking::Off,
        }
    }

    /// Extra output tokens for the reasoning, on top of the answer budget.
    pub fn token_allowance(self) -> u32 {
        match self {
            Thinking::Off => 0,
            Thinking::Low => 16_000,
            Thinking::High => 48_000,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Thinking::Off => "off",
            Thinking::Low => "low",
            Thinking::High => "high",
        }
    }
}

/// Errors of a model call that the worker treats differently.
#[derive(Debug)]
pub enum CallError {
    /// You cancelled the job.
    Cancelled,
    /// The server answered with an HTTP error.
    Http { status: u16, message: String },
    /// The connection failed or broke.
    Network(String),
    /// No data for IDLE_SECS, or the call ran past CALL_CAP_SECS.
    Stalled(String),
    /// The server sent an error inside the stream.
    Server(String),
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallError::Cancelled => write!(f, "Cancelled by you."),
            CallError::Http { status, message } => {
                let hint = match status {
                    401 => " (the API key is not valid)",
                    402 => " (the account balance is too low)",
                    429 => " (rate limit or concurrency limit)",
                    500..=599 => " (the provider has a problem)",
                    _ => "",
                };
                write!(f, "HTTP {status}{hint}: {message}")
            }
            CallError::Network(m) => write!(f, "Connection problem: {m}"),
            CallError::Stalled(m) => write!(f, "{m}"),
            CallError::Server(m) => write!(f, "The provider reported an error: {m}"),
        }
    }
}

impl std::error::Error for CallError {}

impl CallError {
    /// True for problems that can go away by themselves (network, busy server).
    pub fn is_transient(&self) -> bool {
        match self {
            CallError::Network(_) | CallError::Stalled(_) | CallError::Server(_) => true,
            CallError::Http { status, .. } => *status == 408 || *status == 429 || *status >= 500,
            CallError::Cancelled => false,
        }
    }
}

pub fn is_cancelled(e: &anyhow::Error) -> bool {
    matches!(e.downcast_ref::<CallError>(), Some(CallError::Cancelled))
}

pub fn is_transient(e: &anyhow::Error) -> bool {
    e.downcast_ref::<CallError>().map(|c| c.is_transient()).unwrap_or(false)
}

pub struct ChatResult {
    pub text: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// "stop", "length", or another reason from the server.
    pub finish_reason: String,
    /// True when the token counts are estimates (the server sent no usage).
    pub usage_estimated: bool,
}

impl ChatResult {
    pub fn cut_off(&self) -> bool {
        self.finish_reason == "length"
    }
}

/// The request format that a server understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    /// DeepSeek: "thinking" parameter, reasoning tokens in max_tokens.
    DeepSeek,
    /// OpenAI: max_completion_tokens, reasoning_effort, no temperature.
    OpenAi,
    /// OpenRouter: "reasoning": {"effort"}.
    OpenRouter,
    /// Gemini OpenAI-compatible endpoint: reasoning_effort.
    Gemini,
    /// Other OpenAI-compatible servers (Ollama, LM Studio, Mistral, xAI).
    Generic,
    /// Anthropic Messages API.
    Anthropic,
}

impl Dialect {
    /// The dialect from the address alone.
    pub fn detect(base_url: &str) -> Self {
        let b = base_url.to_ascii_lowercase();
        if b.contains("deepseek") {
            Dialect::DeepSeek
        } else if b.contains("anthropic.com") {
            Dialect::Anthropic
        } else if b.contains("api.openai.com") {
            Dialect::OpenAi
        } else if b.contains("openrouter.ai") {
            Dialect::OpenRouter
        } else if b.contains("generativelanguage.googleapis.com") {
            Dialect::Gemini
        } else {
            Dialect::Generic
        }
    }

    pub fn of_profile(p: &ProviderProfile) -> Self {
        if p.api == Api::Anthropic {
            return Dialect::Anthropic;
        }
        match p.kind.as_str() {
            "deepseek" => Dialect::DeepSeek,
            "openai" => Dialect::OpenAi,
            "openrouter" => Dialect::OpenRouter,
            "gemini" => Dialect::Gemini,
            "custom" => match Dialect::detect(&p.base_url) {
                Dialect::Anthropic => Dialect::Generic, // the profile says OpenAI format
                d => d,
            },
            _ => Dialect::Generic,
        }
    }

    /// True when the dialect has a thinking or reasoning switch.
    fn thinks(self) -> bool {
        !matches!(self, Dialect::Generic)
    }
}

/// Changes to the request after a server refused a parameter (HTTP 400).
/// They stay for the life of the provider, so later calls do not fail again.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tweaks {
    pub no_temperature: bool,
    pub no_reasoning: bool,
    pub no_json: bool,
    pub no_stream_options: bool,
    pub completion_tokens: bool,
    /// Anthropic: budget_tokens instead of adaptive thinking (models before 4.6).
    pub legacy_thinking: bool,
    pub max_cap: Option<u32>,
}

impl Tweaks {
    /// Reads a 400 message and changes the request. Returns a description of the
    /// change, or None when nothing in the message can be fixed.
    pub fn adjust(&mut self, dialect: Dialect, message: &str, sent: &Value) -> Option<String> {
        let m = message.to_ascii_lowercase();
        let has = |k: &str| sent.get(k).is_some();
        if m.contains("max_completion_tokens") && has("max_tokens") && !self.completion_tokens {
            self.completion_tokens = true;
            return Some("uses max_completion_tokens".into());
        }
        if m.contains("temperature") && has("temperature") && !self.no_temperature {
            self.no_temperature = true;
            return Some("sends no temperature".into());
        }
        if m.contains("stream_options") && has("stream_options") {
            self.no_stream_options = true;
            return Some("asks for no usage in the stream".into());
        }
        if (m.contains("response_format") || m.contains("json_object") || m.contains("json mode")) && has("response_format") {
            self.no_json = true;
            return Some("asks for no JSON mode".into());
        }
        let thinking_words = ["thinking", "reasoning", "effort", "adaptive", "output_config", "budget_tokens"];
        let sent_thinking = has("thinking") || has("reasoning") || has("reasoning_effort") || has("output_config");
        if sent_thinking && thinking_words.iter().any(|w| m.contains(w)) {
            if dialect == Dialect::Anthropic && !self.legacy_thinking {
                self.legacy_thinking = true;
                return Some("uses budget_tokens thinking".into());
            }
            if !self.no_reasoning {
                self.no_reasoning = true;
                return Some("turns thinking off".into());
            }
        }
        let asked = sent
            .get("max_tokens")
            .or_else(|| sent.get("max_completion_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        if (m.contains("max_tokens") || m.contains("max_completion_tokens") || m.contains("max_output_tokens")) && asked > 2_000 {
            let cap = asked / 2;
            self.max_cap = Some(cap);
            return Some(format!("asks for at most {cap} output tokens"));
        }
        if has("response_format") && !self.no_json {
            // The old fallback: some servers refuse JSON mode with an unclear message.
            self.no_json = true;
            return Some("asks for no JSON mode".into());
        }
        None
    }
}

/// Tweaks learned for each (address, model) during this launch, so each job does
/// not meet the same refusal again.
static LEARNED: Mutex<Option<std::collections::HashMap<String, Tweaks>>> = Mutex::new(None);

fn learned_key(base_url: &str, model: &str) -> String {
    format!("{base_url}|{model}")
}

pub struct Provider {
    client: reqwest::Client,
    base_url: String,
    pub model: String,
    api_key: String,
    idle: Duration,
    pub dialect: Dialect,
    max_output: u32,
    tweaks: Mutex<Tweaks>,
}

/// A data URL as (media type, base64 data).
fn split_data_url(u: &str) -> Option<(String, String)> {
    let rest = u.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let media = meta.strip_suffix(";base64")?;
    Some((media.to_string(), data.to_string()))
}

/// The request body for one chat call.
#[allow(clippy::too_many_arguments)]
pub fn build_body(
    dialect: Dialect,
    model: &str,
    system: &str,
    parts: &[Part],
    json_mode: bool,
    max_tokens: u32,
    thinking: Thinking,
    max_output: u32,
    tw: &Tweaks,
) -> Value {
    let think = dialect.thinks() && thinking != Thinking::Off && !tw.no_reasoning;
    let cap = tw.max_cap.unwrap_or(u32::MAX).min(max_output.max(256));
    let total = (max_tokens + if think { thinking.token_allowance() } else { 0 }).min(cap);
    let effort = if thinking == Thinking::Low { "low" } else { "high" };

    if dialect == Dialect::Anthropic {
        let content: Vec<Value> = parts
            .iter()
            .map(|p| match p {
                Part::Text(t) => json!({"type": "text", "text": t}),
                Part::Image(u) => match split_data_url(u) {
                    Some((media, data)) => json!({"type": "image", "source": {"type": "base64", "media_type": media, "data": data}}),
                    None => json!({"type": "image", "source": {"type": "url", "url": u}}),
                },
            })
            .collect();
        let mut body = json!({
            "model": model,
            "system": system,
            "messages": [{"role": "user", "content": content}],
            "max_tokens": total,
            "stream": true
        });
        let mut thought = false;
        if think {
            if !tw.legacy_thinking {
                body["thinking"] = json!({"type": "adaptive"});
                body["output_config"] = json!({"effort": effort});
                thought = true;
            } else if total > 2_048 {
                let budget = thinking.token_allowance().min(total - 1_024).max(1_024);
                body["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
                thought = true;
            }
        }
        if !thought && !tw.no_temperature {
            body["temperature"] = json!(0.3);
        }
        return body;
    }

    let content: Vec<Value> = parts
        .iter()
        .map(|p| match p {
            Part::Text(t) => json!({"type": "text", "text": t}),
            Part::Image(u) => json!({"type": "image_url", "image_url": {"url": u}}),
        })
        .collect();
    let mut body = json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": content}
        ],
        "stream": true
    });
    let tokens_key = if dialect == Dialect::OpenAi || tw.completion_tokens { "max_completion_tokens" } else { "max_tokens" };
    body[tokens_key] = json!(total);
    if !tw.no_stream_options {
        body["stream_options"] = json!({"include_usage": true});
    }
    if json_mode && !tw.no_json {
        body["response_format"] = json!({"type": "json_object"});
    }
    let mut temperature = dialect != Dialect::OpenAi;
    match dialect {
        Dialect::DeepSeek if !tw.no_reasoning => {
            if think {
                body["thinking"] = json!({"type": "enabled"});
                body["reasoning_effort"] = json!(effort);
                temperature = false;
            } else {
                body["thinking"] = json!({"type": "disabled"});
            }
        }
        Dialect::OpenAi | Dialect::Gemini if think => {
            body["reasoning_effort"] = json!(effort);
        }
        Dialect::OpenRouter if think => {
            body["reasoning"] = json!({"effort": effort});
        }
        _ => {}
    }
    if temperature && !tw.no_temperature {
        body["temperature"] = json!(0.3);
    }
    body
}

/// Runs `fut`, unless the token is cancelled first.
async fn guard<F: Future>(token: Option<&CancelToken>, fut: F) -> Result<F::Output> {
    match token {
        Some(t) => {
            if t.is_cancelled() {
                return Err(CallError::Cancelled.into());
            }
            tokio::select! {
                r = fut => Ok(r),
                _ = t.cancelled() => Err(CallError::Cancelled.into()),
            }
        }
        None => Ok(fut.await),
    }
}

/// State of one streamed answer.
#[derive(Default)]
struct StreamState {
    content: String,
    reasoning_chars: usize,
    finish: Option<String>,
    usage: Option<(i64, i64)>,
    /// Anthropic sends the input tokens in message_start.
    input: i64,
    done: bool,
}

impl StreamState {
    /// Handles one SSE line. Returns the event to report, if any.
    fn line(&mut self, line: &str) -> Result<Option<StreamEvent>> {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            return Ok(None);
        }
        if line.starts_with(':') {
            return Ok(Some(StreamEvent::KeepAlive));
        }
        let Some(payload) = line.strip_prefix("data:") else {
            return Ok(None); // event:, id:, retry: lines
        };
        let payload = payload.trim();
        if payload == "[DONE]" {
            self.done = true;
            return Ok(None);
        }
        let v: Value = match serde_json::from_str(payload) {
            Ok(v) => v,
            Err(_) => return Ok(None),
        };
        if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
            let msg = err["message"].as_str().map(|s| s.to_string()).unwrap_or_else(|| err.to_string());
            return Err(CallError::Server(truncate(&msg, 300)).into());
        }
        // Anthropic events carry a "type".
        if let Some(t) = v["type"].as_str() {
            match t {
                "message_start" => {
                    let u = &v["message"]["usage"];
                    self.input = u["input_tokens"].as_i64().unwrap_or(0)
                        + u["cache_creation_input_tokens"].as_i64().unwrap_or(0)
                        + u["cache_read_input_tokens"].as_i64().unwrap_or(0);
                    return Ok(Some(StreamEvent::Data));
                }
                "content_block_delta" => {
                    let d = &v["delta"];
                    if let Some(c) = d["text"].as_str().filter(|c| !c.is_empty()) {
                        self.content.push_str(c);
                        return Ok(Some(StreamEvent::Content(c.chars().count())));
                    }
                    if let Some(r) = d["thinking"].as_str().filter(|r| !r.is_empty()) {
                        self.reasoning_chars += r.chars().count();
                        return Ok(Some(StreamEvent::Reasoning(r.chars().count())));
                    }
                    return Ok(None);
                }
                "message_delta" => {
                    if let Some(r) = v["delta"]["stop_reason"].as_str() {
                        self.finish = Some(match r {
                            "max_tokens" => "length".to_string(),
                            "end_turn" | "stop_sequence" => "stop".to_string(),
                            other => other.to_string(),
                        });
                    }
                    if let Some(o) = v["usage"]["output_tokens"].as_i64() {
                        self.usage = Some((self.input, o));
                    }
                    return Ok(None);
                }
                "message_stop" => {
                    self.done = true;
                    return Ok(None);
                }
                "ping" => return Ok(Some(StreamEvent::KeepAlive)),
                "content_block_start" | "content_block_stop" => return Ok(None),
                _ => {}
            }
        }
        if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
            self.usage = Some((
                u["prompt_tokens"].as_i64().unwrap_or(0),
                u["completion_tokens"].as_i64().unwrap_or(0),
            ));
        }
        let choice = &v["choices"][0];
        if let Some(f) = choice["finish_reason"].as_str() {
            self.finish = Some(f.to_string());
        }
        let delta = &choice["delta"];
        let mut ev = None;
        if let Some(r) = delta["reasoning_content"].as_str() {
            if !r.is_empty() {
                self.reasoning_chars += r.chars().count();
                ev = Some(StreamEvent::Reasoning(r.chars().count()));
            }
        }
        if let Some(c) = delta["content"].as_str() {
            if !c.is_empty() {
                self.content.push_str(c);
                ev = Some(StreamEvent::Content(c.chars().count()));
            }
        }
        Ok(ev)
    }
}

impl Provider {
    /// A provider with the dialect from the address (tests and the 0.4 settings).
    pub fn new(base_url: &str, model: &str, api_key: &str) -> Result<Self> {
        Self::build(base_url, model, api_key, Dialect::detect(base_url), 64_000, Duration::from_secs(IDLE_SECS))
    }

    /// A provider for a profile. `api_key` is empty for providers without a key.
    pub fn from_profile(p: &ProviderProfile, api_key: &str) -> Result<Self> {
        if p.model.trim().is_empty() {
            anyhow::bail!("Choose a model for \"{}\" in Settings.", p.name);
        }
        // A local model can work on a long prompt for minutes before the first token.
        let idle = Duration::from_secs(if p.local() { 900 } else { IDLE_SECS });
        Self::build(&p.base_url, &p.model, api_key, Dialect::of_profile(p), p.max_output, idle)
    }

    fn build(base_url: &str, model: &str, api_key: &str, dialect: Dialect, max_output: u32, idle: Duration) -> Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .user_agent(concat!("Corydoras/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            client,
            base_url: base_url.trim().trim_end_matches('/').to_string(),
            model: model.trim().to_string(),
            api_key: api_key.trim().to_string(),
            idle,
            dialect,
            max_output,
            tweaks: Mutex::new(
                LEARNED
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    .and_then(|m| m.get(&learned_key(base_url.trim().trim_end_matches('/'), model.trim())).cloned())
                    .unwrap_or_default(),
            ),
        })
    }

    fn tweaks(&self) -> Tweaks {
        self.tweaks.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Adds the headers of the dialect: a bearer token, or the Anthropic key headers.
    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.dialect == Dialect::Anthropic {
            rb.header("x-api-key", &self.api_key).header("anthropic-version", "2023-06-01")
        } else if self.api_key.is_empty() {
            rb
        } else {
            rb.bearer_auth(&self.api_key)
        }
    }

    /// Shorter idle limit (tests).
    pub fn with_idle(mut self, idle: Duration) -> Self {
        self.idle = idle;
        self
    }

    /// One chat call. When the server refuses a parameter (HTTP 400), the provider
    /// removes or changes that parameter and sends the call again, up to 4 times.
    pub async fn chat(
        &self,
        system: &str,
        parts: &[Part],
        json_mode: bool,
        max_tokens: u32,
        thinking: Thinking,
        act: Option<&Activity>,
    ) -> Result<ChatResult> {
        let mut attempt = 0;
        loop {
            let tw = self.tweaks();
            let body = build_body(self.dialect, &self.model, system, parts, json_mode, max_tokens, thinking, self.max_output, &tw);
            match self.send(&body, act).await {
                Err(e) if attempt < 4 => {
                    let msg = match e.downcast_ref::<CallError>() {
                        Some(CallError::Http { status: 400 | 422, message }) => message.clone(),
                        _ => return Err(e),
                    };
                    let mut next = tw.clone();
                    let Some(change) = next.adjust(self.dialect, &msg, &body) else { return Err(e) };
                    *self.tweaks.lock().unwrap_or_else(|e| e.into_inner()) = next.clone();
                    LEARNED
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .get_or_insert_with(Default::default)
                        .insert(learned_key(&self.base_url, &self.model), next);
                    if let Some(a) = act {
                        a.log("warn", None, format!("{} refused a parameter ({}). The next try {change}.", self.model, truncate(&msg, 160)));
                    }
                    attempt += 1;
                }
                other => return other,
            }
        }
    }

    async fn send(&self, body: &Value, act: Option<&Activity>) -> Result<ChatResult> {
        let token_arc = act.map(|a| a.token());
        let token = token_arc.as_deref();
        let body_len = body.to_string().len();
        if let Some(a) = act {
            a.stream(StreamEvent::Connecting);
        }
        let path = if self.dialect == Dialect::Anthropic { "messages" } else { "chat/completions" };
        let request = self
            .auth(self.client.post(format!("{}/{path}", self.base_url)))
            .header("Accept", "text/event-stream")
            .json(body)
            .send();
        let mut resp = guard(token, tokio::time::timeout(self.idle, request))
            .await?
            .map_err(|_| CallError::Stalled(format!("No answer from the server for {} s. The request was stopped.", self.idle.as_secs())))?
            .map_err(|e| CallError::Network(format!("cannot reach {}: {e}", self.base_url)))?;

        let status = resp.status();
        if !status.is_success() {
            let text = guard(token, resp.text()).await?.unwrap_or_default();
            let detail = serde_json::from_str::<Value>(text.trim())
                .ok()
                .and_then(|v| {
                    v["error"]["message"]
                        .as_str()
                        .or_else(|| v["error"].as_str())
                        .or_else(|| v["message"].as_str())
                        .or_else(|| v[0]["error"]["message"].as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| truncate(text.trim(), 300));
            return Err(CallError::Http { status: status.as_u16(), message: detail }.into());
        }
        let is_sse = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.contains("event-stream"))
            .unwrap_or(false);

        let started = Instant::now();
        let mut st = StreamState::default();
        let mut buf: Vec<u8> = Vec::new();
        let mut raw: Vec<u8> = Vec::new();
        loop {
            if started.elapsed() > Duration::from_secs(CALL_CAP_SECS) {
                return Err(CallError::Stalled(format!(
                    "The call ran for more than {} minutes and was stopped.",
                    CALL_CAP_SECS / 60
                ))
                .into());
            }
            let chunk = guard(token, tokio::time::timeout(self.idle, resp.chunk()))
                .await?
                .map_err(|_| {
                    CallError::Stalled(format!(
                        "No data from the server for {} s. The request was stopped.",
                        self.idle.as_secs()
                    ))
                })?
                .map_err(|e| CallError::Network(format!("the stream broke: {e}")))?;
            let Some(bytes) = chunk else { break };
            if !is_sse {
                raw.extend_from_slice(&bytes);
                if let Some(a) = act {
                    // Non-streaming DeepSeek answers send blank lines while queued.
                    let only_ws = bytes.iter().all(|b| b.is_ascii_whitespace());
                    a.stream(if only_ws && raw.iter().all(|b| b.is_ascii_whitespace()) {
                        StreamEvent::KeepAlive
                    } else {
                        StreamEvent::Data
                    });
                }
                continue;
            }
            buf.extend_from_slice(&bytes);
            let mut events: Vec<StreamEvent> = Vec::new();
            let mut saw_keepalive = false;
            while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buf.drain(..=pos).collect();
                let line = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
                match st.line(&line)? {
                    Some(StreamEvent::KeepAlive) => saw_keepalive = true,
                    Some(ev) => events.push(ev),
                    None => {}
                }
            }
            if let Some(a) = act {
                if events.is_empty() && saw_keepalive {
                    a.stream(StreamEvent::KeepAlive);
                }
                for ev in events {
                    a.stream(ev);
                }
            }
            if st.done {
                break;
            }
        }
        // A last line without a newline.
        if is_sse && !buf.is_empty() {
            let line = String::from_utf8_lossy(&buf).into_owned();
            st.line(&line)?;
        }

        if !is_sse {
            return self.parse_plain(&raw, body_len);
        }
        let (input_tokens, output_tokens, usage_estimated) = match st.usage {
            Some((i, o)) => (i, o, false),
            None => (
                (body_len / 4) as i64,
                ((st.content.chars().count() + st.reasoning_chars) / 4) as i64,
                true,
            ),
        };
        let finish = st.finish.clone().unwrap_or_else(|| "unknown".into());
        if st.content.trim().is_empty() {
            if finish == "length" && st.reasoning_chars > 0 {
                return Err(anyhow!(
                    "The model used all {output_tokens} output tokens on its reasoning and wrote no answer. \
                     Set Thinking to Low or Off in Settings, or raise the output limit of the provider."
                ));
            }
            return Err(anyhow!("The model returned an empty answer (finish reason: {finish})."));
        }
        Ok(ChatResult {
            text: st.content,
            input_tokens,
            output_tokens,
            finish_reason: finish,
            usage_estimated,
        })
    }

    /// A server that ignored "stream": true and sent one JSON body.
    fn parse_plain(&self, raw: &[u8], body_len: usize) -> Result<ChatResult> {
        let text = String::from_utf8_lossy(raw);
        let v: Value = serde_json::from_str(text.trim())
            .map_err(|e| anyhow!("The provider sent an answer that is not JSON: {e}"))?;
        if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
            let msg = err["message"].as_str().map(|s| s.to_string()).unwrap_or_else(|| err.to_string());
            return Err(CallError::Server(truncate(&msg, 300)).into());
        }
        if v["type"] == "message" {
            // Anthropic without streaming.
            let out: String = v["content"]
                .as_array()
                .map(|a| a.iter().filter_map(|b| b["text"].as_str()).collect())
                .unwrap_or_default();
            if out.trim().is_empty() {
                return Err(anyhow!("The model returned an empty answer (stop reason: {}).", v["stop_reason"]));
            }
            return Ok(ChatResult {
                text: out,
                input_tokens: v["usage"]["input_tokens"].as_i64().unwrap_or((body_len / 4) as i64),
                output_tokens: v["usage"]["output_tokens"].as_i64().unwrap_or(0),
                finish_reason: if v["stop_reason"] == "max_tokens" { "length".into() } else { "stop".into() },
                usage_estimated: v["usage"].is_null(),
            });
        }
        let message = &v["choices"][0]["message"];
        let out = message["content"].as_str().unwrap_or("").to_string();
        let finish = v["choices"][0]["finish_reason"].as_str().unwrap_or("unknown").to_string();
        let usage = v.get("usage").filter(|u| u.is_object());
        let output_tokens = usage
            .and_then(|u| u["completion_tokens"].as_i64())
            .unwrap_or((out.len() / 4) as i64);
        if out.trim().is_empty() {
            let reasoned = message["reasoning_content"].as_str().map(|r| !r.trim().is_empty()).unwrap_or(false);
            if finish == "length" && reasoned {
                return Err(anyhow!(
                    "The model used all {output_tokens} output tokens on its reasoning and wrote no answer. \
                     Set Thinking to Low or Off in Settings."
                ));
            }
            return Err(anyhow!("The model returned an empty answer (finish reason: {finish})."));
        }
        Ok(ChatResult {
            text: out,
            input_tokens: usage.and_then(|u| u["prompt_tokens"].as_i64()).unwrap_or((body_len / 4) as i64),
            output_tokens,
            finish_reason: finish,
            usage_estimated: usage.is_none(),
        })
    }

    /// The models that the provider offers (GET {base}/models).
    pub async fn list_models(&self) -> Result<Vec<String>> {
        let resp = self
            .auth(self.client.get(format!("{}/models", self.base_url)))
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|e| anyhow!("Cannot reach {}: {e}", self.base_url))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            let hint = if status.as_u16() == 401 || status.as_u16() == 403 { " Check the API key." } else { "" };
            anyhow::bail!("The provider did not list its models (HTTP {}).{hint} {}", status.as_u16(), truncate(text.trim(), 200));
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| anyhow!("The model list is not JSON: {e}"))?;
        let list = v["data"].as_array().or_else(|| v["models"].as_array()).cloned().unwrap_or_default();
        let mut ids: Vec<String> = list
            .iter()
            .filter_map(|m| m["id"].as_str().or_else(|| m["name"].as_str()).or_else(|| m.as_str()))
            .map(|id| id.trim_start_matches("models/").to_string())
            .filter(|id| !id.is_empty())
            .collect();
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    /// Proves that the key, the address and the model work, and checks whether the
    /// model reads images (a small red square).
    pub async fn check(&self) -> Result<CheckReport> {
        let text = self.test().await?;
        let png = red_square_png();
        let vision = match self
            .chat(
                "You answer in one word.",
                &[
                    Part::Image(format!("data:image/png;base64,{png}")),
                    Part::Text("What is the colour of this image? Answer with one word.".into()),
                ],
                false,
                60,
                Thinking::Off,
                None,
            )
            .await
        {
            Ok(r) => r.text.to_ascii_lowercase().contains("red"),
            Err(e) if matches!(e.downcast_ref::<CallError>(), Some(CallError::Http { status: 400..=499, .. })) => false,
            Err(e) => return Err(e),
        };
        Ok(CheckReport {
            message: format!(
                "{}. {}",
                text.trim_end_matches('.'),
                if vision { "The model reads images." } else { "The model does not read images: do not use it for the image analysis." }
            ),
            vision,
        })
    }

    /// A short call that proves the key, the URL and the model work.
    pub async fn test(&self) -> Result<String> {
        let r = self
            .chat(
                "You answer in one word.",
                &[Part::Text("Reply with the word OK.".into())],
                false,
                50,
                Thinking::Off,
                None,
            )
            .await?;
        Ok(format!("{} answered: {}", self.model, truncate(r.text.trim(), 40)))
    }
}

/// The result of a connection check.
#[derive(Debug, serde::Serialize)]
pub struct CheckReport {
    pub message: String,
    pub vision: bool,
}

/// A 32 x 32 red PNG, as base64.
fn red_square_png() -> String {
    use base64::Engine;
    let img = image::RgbImage::from_pixel(32, 32, image::Rgb([220, 30, 30]));
    let mut out = std::io::Cursor::new(Vec::new());
    let _ = image::DynamicImage::ImageRgb8(img).write_to(&mut out, image::ImageFormat::Png);
    base64::engine::general_purpose::STANDARD.encode(out.into_inner())
}

/// Downloads an image from a URL (for drags from a browser).
pub async fn download(url: &str) -> Result<Vec<u8>> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        anyhow::bail!("Only http and https links can be downloaded.");
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent(concat!("Mozilla/5.0 (Macintosh) Corydoras/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("Download failed: HTTP {}", resp.status().as_u16());
    }
    if let Some(len) = resp.content_length() {
        if len > 80 * 1024 * 1024 {
            anyhow::bail!("The file is larger than 80 MB.");
        }
    }
    let bytes = resp.bytes().await?;
    Ok(bytes.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(d: Dialect, json_mode: bool, max: u32, t: Thinking) -> Value {
        build_body(d, "m", "s", &[Part::Text("t".into())], json_mode, max, t, 64_000, &Tweaks::default())
    }

    #[test]
    fn deepseek_thinking_off() {
        let b = body(Dialect::DeepSeek, true, 3000, Thinking::Off);
        assert_eq!(b["thinking"]["type"], "disabled");
        assert_eq!(b["max_tokens"], 3000);
        assert_eq!(b["stream"], true);
        assert_eq!(b["stream_options"]["include_usage"], true);
        assert!(b.get("reasoning_effort").is_none());
        assert_eq!(b["response_format"]["type"], "json_object");
    }

    #[test]
    fn deepseek_thinking_low() {
        let b = body(Dialect::DeepSeek, false, 8000, Thinking::Low);
        assert_eq!(b["thinking"]["type"], "enabled");
        assert_eq!(b["reasoning_effort"], "low");
        assert_eq!(b["max_tokens"], 24000);
        assert!(b.get("response_format").is_none());
    }

    #[test]
    fn generic_server_has_no_thinking_fields() {
        let b = body(Dialect::Generic, false, 100, Thinking::High);
        assert!(b.get("thinking").is_none());
        assert!(b.get("reasoning_effort").is_none());
        assert_eq!(b["max_tokens"], 100);
        assert_eq!(b["temperature"], 0.3);
    }

    #[test]
    fn openai_body() {
        let b = body(Dialect::OpenAi, true, 4000, Thinking::High);
        assert_eq!(b["max_completion_tokens"], 52000);
        assert!(b.get("max_tokens").is_none());
        assert!(b.get("temperature").is_none());
        assert_eq!(b["reasoning_effort"], "high");
        let b = body(Dialect::OpenRouter, false, 4000, Thinking::Low);
        assert_eq!(b["reasoning"]["effort"], "low");
        assert_eq!(b["max_tokens"], 20000);
    }

    #[test]
    fn anthropic_body() {
        let parts = [Part::Image("data:image/jpeg;base64,QUJD".into()), Part::Text("hi".into())];
        let b = build_body(Dialect::Anthropic, "claude", "sys", &parts, true, 8000, Thinking::Low, 64_000, &Tweaks::default());
        assert_eq!(b["system"], "sys");
        assert_eq!(b["messages"][0]["content"][0]["source"]["media_type"], "image/jpeg");
        assert_eq!(b["messages"][0]["content"][0]["source"]["data"], "QUJD");
        assert_eq!(b["thinking"]["type"], "adaptive");
        assert_eq!(b["output_config"]["effort"], "low");
        assert_eq!(b["max_tokens"], 24000);
        assert!(b.get("temperature").is_none() && b.get("response_format").is_none());
        // A model before 4.6 refuses adaptive thinking: the next body uses a budget.
        let mut tw = Tweaks::default();
        assert!(tw.adjust(Dialect::Anthropic, "thinking.type: adaptive is not supported for this model", &b).is_some());
        let b2 = build_body(Dialect::Anthropic, "claude", "sys", &parts, true, 8000, Thinking::Low, 64_000, &tw);
        assert_eq!(b2["thinking"]["budget_tokens"], 16000);
        // Off: no thinking, a temperature.
        let b3 = build_body(Dialect::Anthropic, "claude", "sys", &parts, true, 8000, Thinking::Off, 64_000, &Tweaks::default());
        assert!(b3.get("thinking").is_none());
        assert_eq!(b3["temperature"], 0.3);
    }

    #[test]
    fn output_cap() {
        let b = build_body(Dialect::Generic, "m", "s", &[], false, 30_000, Thinking::Off, 16_000, &Tweaks::default());
        assert_eq!(b["max_tokens"], 16000);
        let mut tw = Tweaks::default();
        let change = tw.adjust(Dialect::Generic, "max_tokens is too large: 16000 > 8192", &b).unwrap();
        assert!(change.contains("8000"));
    }

    #[test]
    fn tweaks_from_errors() {
        let b = body(Dialect::OpenAi, true, 4000, Thinking::Low);
        let mut tw = Tweaks::default();
        assert!(tw.adjust(Dialect::OpenAi, "Unsupported value: 'reasoning_effort' does not support 'low' with this model.", &b).is_some());
        assert!(tw.no_reasoning);
        let b = body(Dialect::Generic, true, 4000, Thinking::Off);
        let mut tw = Tweaks::default();
        tw.adjust(Dialect::Generic, "Unsupported parameter: 'temperature'", &b).unwrap();
        assert!(tw.no_temperature);
        tw.adjust(Dialect::Generic, "unknown field stream_options", &b).unwrap();
        assert!(tw.no_stream_options);
        // An unclear message with JSON mode on: the old JSON fallback.
        tw.adjust(Dialect::Generic, "bad request", &b).unwrap();
        assert!(tw.no_json);
        assert!(tw.adjust(Dialect::Generic, "bad request", &build_body(Dialect::Generic, "m", "s", &[], false, 100, Thinking::Off, 64_000, &tw)).is_none());
    }

    #[test]
    fn parse_thinking() {
        assert_eq!(Thinking::parse("High"), Thinking::High);
        assert_eq!(Thinking::parse("low"), Thinking::Low);
        assert_eq!(Thinking::parse("x"), Thinking::Off);
    }

    #[test]
    fn sse_lines() {
        let mut st = StreamState::default();
        assert!(matches!(st.line(": keep-alive").unwrap(), Some(StreamEvent::KeepAlive)));
        assert!(st.line("").unwrap().is_none());
        let r = st.line(r#"data: {"choices":[{"delta":{"reasoning_content":"abc"}}]}"#).unwrap();
        assert!(matches!(r, Some(StreamEvent::Reasoning(3))));
        st.line(r#"data: {"choices":[{"delta":{"content":"{\"a\":"}}]}"#).unwrap();
        st.line(r#"data: {"choices":[{"delta":{"content":"1}"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#).unwrap();
        st.line("data: [DONE]").unwrap();
        assert_eq!(st.content, "{\"a\":1}");
        assert_eq!(st.finish.as_deref(), Some("stop"));
        assert_eq!(st.usage, Some((10, 5)));
        assert!(st.done);
        let mut st = StreamState::default();
        let e = st.line(r#"data: {"error":{"message":"overloaded"}}"#).unwrap_err();
        assert!(is_transient(&e));
    }

    #[test]
    fn anthropic_sse() {
        let mut st = StreamState::default();
        st.line("event: message_start").unwrap();
        st.line(r#"data: {"type":"message_start","message":{"usage":{"input_tokens":120,"output_tokens":1}}}"#).unwrap();
        assert!(matches!(st.line(r#"data: {"type":"ping"}"#).unwrap(), Some(StreamEvent::KeepAlive)));
        st.line(r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm"}}"#).unwrap();
        st.line(r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"{}"}}"#).unwrap();
        st.line(r#"data: {"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":42}}"#).unwrap();
        st.line(r#"data: {"type":"message_stop"}"#).unwrap();
        assert_eq!(st.content, "{}");
        assert_eq!(st.reasoning_chars, 3);
        assert_eq!(st.finish.as_deref(), Some("length"));
        assert_eq!(st.usage, Some((120, 42)));
        assert!(st.done);
        let mut st = StreamState::default();
        let e = st.line(r#"data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#).unwrap_err();
        assert!(is_transient(&e));
    }

    #[test]
    fn error_classes() {
        let e: anyhow::Error = CallError::Http { status: 429, message: "x".into() }.into();
        assert!(is_transient(&e));
        let e: anyhow::Error = CallError::Http { status: 401, message: "x".into() }.into();
        assert!(!is_transient(&e));
        let e: anyhow::Error = CallError::Cancelled.into();
        assert!(is_cancelled(&e) && !is_transient(&e));
        assert!(!is_transient(&anyhow!("plain")));
        assert_eq!(split_data_url("data:image/png;base64,AAA"), Some(("image/png".into(), "AAA".into())));
    }
}
