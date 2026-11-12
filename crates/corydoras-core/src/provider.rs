//! OpenAI-compatible chat provider. The default is DeepSeek (deepseek-flash).
//! It also speaks the Anthropic Messages API and, for a signed-in ChatGPT account, the
//! OpenAI Responses API (see signin::chatgpt).
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
use crate::signin::chatgpt;
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
    /// ChatGPT refused to pay for the call with the plan. `code` is the word of the server.
    Plan { code: String, message: String },
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
            CallError::Plan { code, message } => {
                let said = if message.is_empty() { String::new() } else { format!(" ChatGPT said: {message}") };
                match code.as_str() {
                    "subscription_sharing_usage_limit_exceeded" => write!(
                        f,
                        "Usage limit reached. Review your plan or this app's limit in ChatGPT settings (chatgpt.com/settings/usage).{said}"
                    ),
                    "subscription_sharing_usage_unavailable" | "subscription_sharing_user_unavailable" | "unavailable" => {
                        write!(f, "ChatGPT plan usage is not available at the moment.{said}")
                    }
                    "subscription_sharing_user_not_eligible" => write!(
                        f,
                        "ChatGPT plan usage is not available for this account or workspace. Use a provider with an API key instead.{said}"
                    ),
                    "subscription_sharing_invalid_user" | "not_accepted" => {
                        write!(f, "ChatGPT did not accept this sign-in. If this stays, sign out and sign in again in Settings.{said}")
                    }
                    "blocked" => write!(f, "ChatGPT does not allow plan usage for this request.{said}"),
                    _ => write!(f, "ChatGPT refused the use of the plan ({code}).{said}"),
                }
            }
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
            // Only "cannot check now" passes by itself. A used-up limit does not.
            CallError::Plan { code, .. } => {
                matches!(code.as_str(), "subscription_sharing_usage_unavailable" | "subscription_sharing_user_unavailable" | "unavailable")
            }
        }
    }
}

pub fn is_cancelled(e: &anyhow::Error) -> bool {
    matches!(e.downcast_ref::<CallError>(), Some(CallError::Cancelled))
}

/// True when the ChatGPT plan (or the limit for this app) has no usage left.
pub fn is_plan_limit(e: &anyhow::Error) -> bool {
    matches!(e.downcast_ref::<CallError>(), Some(CallError::Plan { code, .. }) if code == "subscription_sharing_usage_limit_exceeded")
}

fn plan_code(code: &str) -> bool {
    code.starts_with("subscription_sharing_") || code.starts_with("chatpass_")
}

/// A failure that the Responses API reports inside the stream.
fn responses_failure(code: &str, message: &str) -> anyhow::Error {
    if plan_code(code) {
        return CallError::Plan { code: code.to_string(), message: truncate(message, 300) }.into();
    }
    let text = if message.is_empty() { code.to_string() } else { format!("{} ({code})", truncate(message, 300)) };
    // A prompt or an image that the server refuses fails the same way again; the rest can pass.
    if code.starts_with("invalid_") || code.contains("image") || code.contains("unsupported") || code.contains("content") {
        return anyhow!("The provider refused the request: {text}");
    }
    CallError::Server(text).into()
}

/// An HTTP error of the ChatGPT plan route, before a stream opens.
fn plan_refusal(status: u16, code: &str, param: &str, message: String) -> anyhow::Error {
    match status {
        // A refused part of the request: the next try leaves it out (see Tweaks).
        400 | 422 => CallError::Http { status, message: if param.is_empty() { message } else { format!("{message} (parameter: {param})") } },
        _ if plan_code(code) => CallError::Plan { code: code.to_string(), message },
        // Without a code the answer is a text for a person.
        401 => CallError::Plan { code: "not_accepted".into(), message },
        403 => CallError::Plan { code: "blocked".into(), message },
        503 => CallError::Plan { code: "unavailable".into(), message },
        _ => CallError::Http { status, message },
    }
    .into()
}

pub fn is_transient(e: &anyhow::Error) -> bool {
    e.downcast_ref::<CallError>().map(|c| c.is_transient()).unwrap_or(false)
}

#[derive(Debug)]
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
    /// OpenAI Responses API, paid by a ChatGPT plan ("Sign in with ChatGPT").
    ChatGpt,
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
            "chatgpt" => Dialect::ChatGpt,
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
        // JSON mode is "response_format" in chat completions and "text.format" in the Responses API.
        let json_sent = has("response_format") || sent["text"]["format"]["type"] == "json_object";
        if (m.contains("response_format") || m.contains("json_object") || m.contains("json mode") || m.contains("text.format")) && json_sent {
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
        if json_sent && !self.no_json {
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

/// What a call carries to show who pays.
enum Auth {
    /// An API key. Empty for a server that needs none.
    Key(String),
    /// A signed-in ChatGPT account: its access token, renewed when it is old.
    Account(chatgpt::Session),
}

pub struct Provider {
    client: reqwest::Client,
    base_url: String,
    pub model: String,
    auth: Auth,
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

    if dialect == Dialect::ChatGpt {
        // The Responses API on the plan route: the system text is "instructions", nothing
        // is stored, and the route takes no output limit and no temperature.
        let content: Vec<Value> = parts
            .iter()
            .map(|p| match p {
                Part::Text(t) => json!({"type": "input_text", "text": t}),
                Part::Image(u) => json!({"type": "input_image", "image_url": u}),
            })
            .collect();
        let mut body = json!({
            "model": model,
            "instructions": system,
            "input": [{"role": "user", "content": content}],
            "store": false,
            "stream": true
        });
        if think {
            body["reasoning"] = json!({"effort": effort});
        }
        if json_mode && !tw.no_json {
            body["text"] = json!({"format": {"type": "json_object"}});
        }
        return body;
    }

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
    /// The Responses API sent its last event (the answer is whole, or cut off).
    completed: bool,
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
                // The Responses API (ChatGPT plan).
                "response.created" | "response.in_progress" => return Ok(Some(StreamEvent::Data)),
                "response.queued" => return Ok(Some(StreamEvent::KeepAlive)),
                "response.output_text.delta" => {
                    if let Some(c) = v["delta"].as_str().filter(|c| !c.is_empty()) {
                        self.content.push_str(c);
                        return Ok(Some(StreamEvent::Content(c.chars().count())));
                    }
                    return Ok(None);
                }
                "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                    if let Some(r) = v["delta"].as_str().filter(|r| !r.is_empty()) {
                        self.reasoning_chars += r.chars().count();
                        return Ok(Some(StreamEvent::Reasoning(r.chars().count())));
                    }
                    return Ok(None);
                }
                "response.completed" | "response.incomplete" => {
                    let r = &v["response"];
                    if let (Some(i), Some(o)) = (r["usage"]["input_tokens"].as_i64(), r["usage"]["output_tokens"].as_i64()) {
                        self.usage = Some((i, o));
                    }
                    self.finish = Some(if t == "response.completed" {
                        "stop".to_string()
                    } else {
                        match r["incomplete_details"]["reason"].as_str() {
                            Some("max_output_tokens") => "length".to_string(),
                            Some(other) => other.to_string(),
                            None => "incomplete".to_string(),
                        }
                    });
                    self.done = true;
                    self.completed = true;
                    return Ok(None);
                }
                "response.failed" => {
                    let err = &v["response"]["error"];
                    return Err(responses_failure(err["code"].as_str().unwrap_or("unknown_error"), err["message"].as_str().unwrap_or("")));
                }
                // Its error event has the fields at the top (Anthropic's has an "error" object, handled above).
                "error" => {
                    return Err(responses_failure(v["code"].as_str().unwrap_or("unknown_error"), v["message"].as_str().unwrap_or("")));
                }
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
        Self::build(base_url, model, Auth::Key(api_key.trim().to_string()), Dialect::detect(base_url), 64_000, Duration::from_secs(IDLE_SECS))
    }

    /// A provider for a profile. `api_key` is empty for providers without a key.
    pub fn from_profile(p: &ProviderProfile, api_key: &str) -> Result<Self> {
        if p.model.trim().is_empty() {
            anyhow::bail!("Choose a model for \"{}\" in Settings.", p.name);
        }
        // A local model can work on a long prompt for minutes before the first token.
        let idle = Duration::from_secs(if p.local() { 900 } else { IDLE_SECS });
        if p.uses_account() {
            // The tokens of the plan go to OpenAI only, whatever address the profile holds.
            return Self::build(chatgpt::API_BASE, &p.model, Auth::Account(chatgpt::Session::app()), Dialect::ChatGpt, p.max_output, idle);
        }
        Self::build(&p.base_url, &p.model, Auth::Key(api_key.trim().to_string()), Dialect::of_profile(p), p.max_output, idle)
    }

    /// A provider that pays with the ChatGPT account of `session`. The app uses
    /// `from_profile`, which calls OpenAI; this one calls the address that it is given
    /// (the tests give it a stand-in server).
    pub fn for_account(base_url: &str, model: &str, session: chatgpt::Session) -> Result<Self> {
        Self::build(base_url, model, Auth::Account(session), Dialect::ChatGpt, 64_000, Duration::from_secs(IDLE_SECS))
    }

    fn build(base_url: &str, model: &str, auth: Auth, dialect: Dialect, max_output: u32, idle: Duration) -> Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .user_agent(concat!("Corydoras/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            client,
            base_url: base_url.trim().trim_end_matches('/').to_string(),
            model: model.trim().to_string(),
            auth,
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

    /// Adds the headers that show who pays: the access token of the account (`bearer`),
    /// or the API key as a bearer token or in the Anthropic key headers.
    fn auth(&self, rb: reqwest::RequestBuilder, bearer: Option<&str>) -> reqwest::RequestBuilder {
        match (&self.auth, bearer) {
            (_, Some(token)) => rb.bearer_auth(token),
            (Auth::Key(key), None) if self.dialect == Dialect::Anthropic => rb.header("x-api-key", key).header("anthropic-version", "2023-06-01"),
            (Auth::Key(key), None) if !key.is_empty() => rb.bearer_auth(key),
            _ => rb,
        }
    }

    /// The access token of the account for the next request. None for a key provider.
    async fn bearer(&self) -> Result<Option<String>> {
        match &self.auth {
            Auth::Account(session) => Ok(Some(chatgpt::access_token(session).await?)),
            Auth::Key(_) => Ok(None),
        }
    }

    /// Sends the request and returns the answer when its headers arrive.
    async fn open(&self, body: &Value, bearer: Option<&str>, token: Option<&CancelToken>) -> Result<reqwest::Response> {
        let path = match self.dialect {
            Dialect::Anthropic => "messages",
            Dialect::ChatGpt => "responses",
            _ => "chat/completions",
        };
        let request = self
            .auth(self.client.post(format!("{}/{path}", self.base_url)), bearer)
            .header("Accept", "text/event-stream")
            .json(body)
            .send();
        Ok(guard(token, tokio::time::timeout(self.idle, request))
            .await?
            .map_err(|_| CallError::Stalled(format!("No answer from the server for {} s. The request was stopped.", self.idle.as_secs())))?
            .map_err(|e| CallError::Network(format!("cannot reach {}: {e}", self.base_url)))?)
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
        let bearer = guard(token, self.bearer()).await??;
        let mut resp = self.open(body, bearer.as_deref(), token).await?;
        if resp.status().as_u16() == 401 {
            if let (Auth::Account(session), Some(refused)) = (&self.auth, &bearer) {
                // The server refused a token that looked good: renew it and ask one more time.
                let renewed = guard(token, chatgpt::force_refresh(session, refused)).await??;
                resp = self.open(body, Some(&renewed), token).await?;
            }
        }

        let status = resp.status();
        if !status.is_success() {
            let text = guard(token, resp.text()).await?.unwrap_or_default();
            let parsed = serde_json::from_str::<Value>(text.trim()).ok();
            let detail = parsed
                .as_ref()
                .and_then(|v| {
                    v["error"]["message"]
                        .as_str()
                        .or_else(|| v["error"].as_str())
                        .or_else(|| v["message"].as_str())
                        .or_else(|| v["detail"].as_str())
                        .or_else(|| v[0]["error"]["message"].as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| truncate(text.trim(), 300));
            if self.dialect == Dialect::ChatGpt {
                let field = |name: &str| parsed.as_ref().and_then(|v| v["error"][name].as_str()).unwrap_or("");
                return Err(plan_refusal(status.as_u16(), field("code"), field("param"), detail));
            }
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
            if self.dialect == Dialect::ChatGpt {
                return Err(CallError::Server("the answer was not a stream".into()).into());
            }
            return self.parse_plain(&raw, body_len);
        }
        // The Responses API names its last event. A stream that ends without it is not an answer.
        if self.dialect == Dialect::ChatGpt && !st.completed {
            return Err(CallError::Network("the answer stopped before it was complete".into()).into());
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
    pub async fn models(&self) -> Result<Vec<ModelInfo>> {
        let bearer = self.bearer().await?;
        let resp = self
            .auth(self.client.get(format!("{}/models", self.base_url)), bearer.as_deref())
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|e| anyhow!("Cannot reach {}: {e}", self.base_url))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            let hint = match (status.as_u16(), &self.auth) {
                (401 | 403, Auth::Account(_)) => " Check the ChatGPT sign-in.",
                (401 | 403, Auth::Key(_)) => " Check the API key.",
                _ => "",
            };
            anyhow::bail!("The provider did not list its models (HTTP {}).{hint} {}", status.as_u16(), truncate(text.trim(), 200));
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| anyhow!("The model list is not JSON: {e}"))?;
        if self.dialect == Dialect::ChatGpt {
            // The models that ChatGPT lists for this account, in its order, with their names.
            let listed = v["models"].as_array().cloned().unwrap_or_default();
            return Ok(listed
                .iter()
                .filter(|m| m["visibility"] == "list")
                .filter_map(|m| Some(ModelInfo { id: m["slug"].as_str()?.to_string(), name: m["display_name"].as_str().unwrap_or("").to_string() }))
                .collect());
        }
        let list = v["data"].as_array().or_else(|| v["models"].as_array()).cloned().unwrap_or_default();
        let mut ids: Vec<String> = list
            .iter()
            .filter_map(|m| m["id"].as_str().or_else(|| m["name"].as_str()).or_else(|| m.as_str()))
            .map(|id| id.trim_start_matches("models/").to_string())
            .filter(|id| !id.is_empty())
            .collect();
        ids.sort();
        ids.dedup();
        Ok(ids.into_iter().map(|id| ModelInfo { id, name: String::new() }).collect())
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

/// A model of a provider. `name` is empty when the provider gives only IDs.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
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

    // ------------------------------------------------------------------ ChatGPT plan

    use crate::signin::chatgpt::{self, Account, Endpoints, Session, Store};
    use crate::testserver::{serve, Resp, Server};

    fn plan_body(json_mode: bool, t: Thinking, tw: &Tweaks) -> Value {
        let parts = [Part::Image("data:image/jpeg;base64,QUJD".into()), Part::Text("hi".into())];
        build_body(Dialect::ChatGpt, "gpt-x", "sys", &parts, json_mode, 4000, t, 64_000, tw)
    }

    #[test]
    fn chatgpt_plan_body_holds_only_what_the_plan_route_accepts() {
        let b = plan_body(true, Thinking::Low, &Tweaks::default());
        assert_eq!(b["model"], "gpt-x");
        assert_eq!(b["instructions"], "sys");
        assert_eq!((b["store"].clone(), b["stream"].clone()), (json!(false), json!(true)));
        assert_eq!(b["input"][0]["role"], "user");
        let content = &b["input"][0]["content"];
        assert_eq!(content[0], json!({"type": "input_image", "image_url": "data:image/jpeg;base64,QUJD"}));
        assert_eq!(content[1], json!({"type": "input_text", "text": "hi"}));
        assert_eq!(b["text"]["format"]["type"], "json_object");
        assert_eq!(b["reasoning"]["effort"], "low");
        for refused in [
            "max_output_tokens", "max_tokens", "max_completion_tokens", "temperature", "top_p", "stream_options", "messages",
            "response_format", "metadata", "user", "truncation", "previous_response_id", "system",
        ] {
            assert!(b.get(refused).is_none(), "{refused} must not be sent");
        }
        let plain = plan_body(false, Thinking::Off, &Tweaks::default());
        assert!(plain.get("text").is_none() && plain.get("reasoning").is_none());
        assert_eq!(plan_body(false, Thinking::High, &Tweaks::default())["reasoning"]["effort"], "high");
    }

    #[test]
    fn chatgpt_plan_refusals_take_the_named_part_away() {
        let mut tw = Tweaks::default();
        let b = plan_body(true, Thinking::Low, &tw);
        assert!(tw.adjust(Dialect::ChatGpt, "This capability is not supported (parameter: text.format)", &b).is_some());
        assert!(tw.no_json && !tw.no_reasoning);
        let b = plan_body(true, Thinking::Low, &tw);
        assert!(b.get("text").is_none());
        assert!(tw.adjust(Dialect::ChatGpt, "This capability is not supported (parameter: reasoning.effort)", &b).is_some());
        assert!(tw.no_reasoning);
        let b = plan_body(true, Thinking::Low, &tw);
        assert!(b.get("reasoning").is_none());
        // Nothing is left to change: the error goes to the person.
        assert!(tw.adjust(Dialect::ChatGpt, "This capability is not supported (parameter: model)", &b).is_none());
        // An unclear refusal with JSON mode on: JSON mode goes first, as for other servers.
        let mut tw = Tweaks::default();
        assert!(tw.adjust(Dialect::ChatGpt, "bad request", &plan_body(true, Thinking::Off, &tw)).is_some());
        assert!(tw.no_json);
    }

    #[test]
    fn chatgpt_plan_stream() {
        let mut st = StreamState::default();
        assert!(matches!(
            st.line(r#"data: {"type":"response.created","response":{"status":"in_progress","usage":null},"sequence_number":1}"#).unwrap(),
            Some(StreamEvent::Data)
        ));
        st.line("event: response.output_text.delta").unwrap();
        let r = st.line(r#"data: {"type":"response.reasoning_summary_text.delta","delta":"hmm","sequence_number":2}"#).unwrap();
        assert!(matches!(r, Some(StreamEvent::Reasoning(3))));
        let r = st.line(r#"data: {"type":"response.output_text.delta","delta":"{\"a\":","logprobs":[],"sequence_number":3}"#).unwrap();
        assert!(matches!(r, Some(StreamEvent::Content(5))));
        st.line(r#"data: {"type":"response.output_text.delta","delta":"1}"}"#).unwrap();
        st.line(r#"data: {"type":"response.output_text.done","text":"{\"a\":1}"}"#).unwrap();
        assert!(!st.done && !st.completed);
        st.line(r#"data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":120,"output_tokens":42,"total_tokens":162}}}"#).unwrap();
        assert_eq!(st.content, "{\"a\":1}");
        assert_eq!(st.reasoning_chars, 3);
        assert_eq!(st.usage, Some((120, 42)));
        assert_eq!(st.finish.as_deref(), Some("stop"));
        assert!(st.done && st.completed);
    }

    #[test]
    fn chatgpt_plan_answer_that_hits_the_limit_is_cut_off() {
        let mut st = StreamState::default();
        st.line(r#"data: {"type":"response.output_text.delta","delta":"{\"a\""}"#).unwrap();
        st.line(r#"data: {"type":"response.incomplete","response":{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"usage":{"input_tokens":10,"output_tokens":5}}}"#).unwrap();
        assert_eq!(st.finish.as_deref(), Some("length"));
        assert_eq!(st.usage, Some((10, 5)));
        assert!(st.done && st.completed);
    }

    #[test]
    fn chatgpt_plan_failures_in_the_stream() {
        let fail = |code: &str| {
            let line = format!(r#"data: {{"type":"response.failed","response":{{"status":"failed","error":{{"code":"{code}","message":"The server said so."}}}}}}"#);
            StreamState::default().line(&line).unwrap_err()
        };
        let limit = fail("subscription_sharing_usage_limit_exceeded");
        assert!(is_plan_limit(&limit) && !is_transient(&limit), "{limit}");
        assert!(limit.to_string().starts_with("Usage limit reached."), "{limit}");
        assert!(limit.to_string().contains("chatgpt.com/settings/usage"), "{limit}");
        for busy in ["subscription_sharing_usage_unavailable", "subscription_sharing_user_unavailable", "server_error", "rate_limit_exceeded"] {
            let e = fail(busy);
            assert!(is_transient(&e) && !is_plan_limit(&e), "{busy}: {e}");
        }
        for stays in ["subscription_sharing_user_not_eligible", "invalid_prompt", "invalid_base64_image", "image_too_large"] {
            let e = fail(stays);
            assert!(!is_transient(&e) && !is_plan_limit(&e), "{stays}: {e}");
            assert!(e.to_string().contains("The server said so."), "{e}");
        }
        // No error object at all.
        let none = StreamState::default().line(r#"data: {"type":"response.failed","response":{"status":"failed","error":null}}"#).unwrap_err();
        assert!(none.to_string().contains("unknown_error"), "{none}");
        // The error event of the Responses API has its fields at the top.
        let top = StreamState::default()
            .line(r#"data: {"type":"error","code":"rate_limit_exceeded","message":"Slow down","param":null,"sequence_number":1}"#)
            .unwrap_err();
        assert!(is_transient(&top) && top.to_string().contains("Slow down"), "{top}");
    }

    /// A signed-in account in a new folder, with an access token that is `age` seconds old.
    fn plan_session(server: &Server, age: i64) -> Session {
        let dir = std::env::temp_dir().join(format!("cory-plan-{}", uuid::Uuid::new_v4()));
        let sess = Session {
            store: Store { dir },
            name: "chatgpt-plan".into(),
            endpoints: Endpoints {
                authorize_url: format!("{}/authorize", server.base),
                token_url: format!("{}/token", server.base),
                revoke_url: format!("{}/revoke", server.base),
                jwks_url: format!("{}/jwks", server.base),
                issuer: "https://auth.test".into(),
                resource: chatgpt::API_BASE.into(),
            },
        };
        let account = Account {
            email: "ada@example.com".into(),
            subject: "user-1".into(),
            client_id: "oaiapp_1".into(),
            access_token: "at-old".into(),
            refresh_token: "rt-1".into(),
            expires_in: 3600,
            scopes: chatgpt::SCOPE.split(' ').map(str::to_string).collect(),
            saved_at: crate::util::now_ts() - age,
            ..Account::default()
        };
        sess.store.save_account(&sess.name, &account).unwrap();
        sess
    }

    fn plan_provider(server: &Server, sess: Session) -> Provider {
        Provider::for_account(&format!("{}/v1", server.base), "gpt-x", sess).unwrap()
    }

    fn ok_stream() -> Resp {
        Resp::sse(&[
            json!({"type": "response.created", "response": {"status": "in_progress"}}),
            json!({"type": "response.output_text.delta", "delta": "OK"}),
            json!({"type": "response.completed", "response": {"status": "completed", "usage": {"input_tokens": 7, "output_tokens": 2}}}),
        ])
    }

    fn renewed() -> Resp {
        Resp::json(200, json!({"access_token": "at-2", "refresh_token": "rt-2", "token_type": "Bearer", "expires_in": 3600, "scope": chatgpt::SCOPE}))
    }

    async fn ask(p: &Provider) -> Result<ChatResult> {
        p.chat("sys", &[Part::Text("hi".into())], false, 100, Thinking::Off, None).await
    }

    #[tokio::test]
    async fn a_plan_call_sends_the_access_token_and_reads_the_stream() {
        let server = serve(|req| if req.path == "/v1/responses" { ok_stream() } else { Resp::empty(404) }).await;
        let p = plan_provider(&server, plan_session(&server, 10));
        assert_eq!(p.dialect, Dialect::ChatGpt);
        let r = ask(&p).await.unwrap();
        assert_eq!((r.text.as_str(), r.input_tokens, r.output_tokens, r.usage_estimated), ("OK", 7, 2, false));
        assert_eq!(r.finish_reason, "stop");
        let sent = server.seen_at("/v1/responses");
        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0].method.as_str(), sent[0].header("authorization")), ("POST", "Bearer at-old"));
        assert_eq!(sent[0].json()["store"], false);
        assert_eq!(server.seen().len(), 1, "a fresh token needs no renewal");
    }

    #[tokio::test]
    async fn an_old_plan_token_is_renewed_before_the_call() {
        let server = serve(|req| match req.path.as_str() {
            "/v1/responses" => ok_stream(),
            "/token" => renewed(),
            _ => Resp::empty(404),
        })
        .await;
        let p = plan_provider(&server, plan_session(&server, 3590));
        ask(&p).await.unwrap();
        assert_eq!(server.seen_at("/v1/responses")[0].header("authorization"), "Bearer at-2");
    }

    #[tokio::test]
    async fn a_refused_plan_token_is_renewed_and_the_call_is_sent_again_one_time() {
        // The API refuses the saved token, although it is not old, and takes the renewed one.
        let server = serve(|req| match req.path.as_str() {
            "/v1/responses" if req.header("authorization") == "Bearer at-2" => ok_stream(),
            "/v1/responses" => Resp::json(401, json!({"detail": "The token is not accepted."})),
            "/token" => renewed(),
            _ => Resp::empty(404),
        })
        .await;
        let p = plan_provider(&server, plan_session(&server, 10));
        assert_eq!(ask(&p).await.unwrap().text, "OK");
        assert_eq!((server.seen_at("/v1/responses").len(), server.seen_at("/token").len()), (2, 1));

        // A server that refuses every token gets two requests, not a loop.
        let server = serve(|req| match req.path.as_str() {
            "/v1/responses" => Resp::json(401, json!({"error": {"code": "subscription_sharing_invalid_user", "message": "Not validated."}})),
            "/token" => renewed(),
            _ => Resp::empty(404),
        })
        .await;
        let p = plan_provider(&server, plan_session(&server, 10));
        let e = ask(&p).await.unwrap_err();
        assert!(e.to_string().contains("did not accept this sign-in"), "{e}");
        assert!(!is_transient(&e));
        assert_eq!(server.seen_at("/v1/responses").len(), 2);
    }

    #[tokio::test]
    async fn a_plan_stream_that_ends_early_is_a_failure_that_can_pass() {
        let server = serve(|_| {
            Resp::sse(&[json!({"type": "response.created", "response": {}}), json!({"type": "response.output_text.delta", "delta": "half an ans"})])
        })
        .await;
        let e = ask(&plan_provider(&server, plan_session(&server, 10))).await.unwrap_err();
        assert!(is_transient(&e), "{e}");
        assert!(e.to_string().contains("before it was complete"), "{e}");
    }

    #[tokio::test]
    async fn plan_refusals_before_the_stream_say_what_the_server_said() {
        let server = serve(|_| Resp::json(429, json!({"error": {"code": "subscription_sharing_usage_limit_exceeded", "message": "No usage left.", "param": null}}))).await;
        let e = ask(&plan_provider(&server, plan_session(&server, 10))).await.unwrap_err();
        assert!(is_plan_limit(&e) && !is_transient(&e), "{e}");

        // Before the stream opens, the answer can be a plain text for a person.
        let server = serve(|_| Resp::json(403, json!({"detail": "This region is not served."}))).await;
        let e = ask(&plan_provider(&server, plan_session(&server, 10))).await.unwrap_err();
        assert!(e.to_string().contains("This region is not served."), "{e}");
        assert!(!is_transient(&e) && !is_plan_limit(&e));

        let server = serve(|_| Resp::json(503, json!({"detail": "Not available."}))).await;
        let e = ask(&plan_provider(&server, plan_session(&server, 10))).await.unwrap_err();
        assert!(is_transient(&e), "{e}");
    }

    #[tokio::test]
    async fn a_plan_refusal_of_one_parameter_sends_the_call_again_without_it() {
        let server = serve(|req| {
            if req.json().get("text").is_some() {
                return Resp::json(400, json!({"error": {"code": "subscription_sharing_unsupported_capability", "message": "Not supported here.", "param": "text.format"}}));
            }
            ok_stream()
        })
        .await;
        let p = Provider::for_account(&format!("{}/v1", server.base), "gpt-json", plan_session(&server, 10)).unwrap();
        let r = p.chat("sys", &[Part::Text("Answer in JSON.".into())], true, 100, Thinking::Off, None).await.unwrap();
        assert_eq!(r.text, "OK");
        assert_eq!(server.seen().len(), 2);
    }

    #[tokio::test]
    async fn plan_models_are_the_listed_ones_with_their_names() {
        let server = serve(|req| match req.path.as_str() {
            "/v1/models" => Resp::json(
                200,
                json!({"models": [
                    {"slug": "gpt-b", "display_name": "GPT B", "visibility": "list"},
                    {"slug": "gpt-hidden", "display_name": "Hidden", "visibility": "hide"},
                    {"slug": "gpt-a", "display_name": "GPT A", "visibility": "list"}
                ]}),
            ),
            _ => Resp::empty(404),
        })
        .await;
        let p = plan_provider(&server, plan_session(&server, 10));
        let models = p.models().await.unwrap();
        // The order of the server stays.
        assert_eq!(models, vec![ModelInfo { id: "gpt-b".into(), name: "GPT B".into() }, ModelInfo { id: "gpt-a".into(), name: "GPT A".into() }]);
        assert_eq!(server.seen()[0].header("authorization"), "Bearer at-old");
    }

    #[tokio::test]
    async fn models_of_a_key_provider_are_sorted_ids() {
        let server = serve(|_| Resp::json(200, json!({"data": [{"id": "models/b"}, {"id": "a"}, {"id": "a"}]}))).await;
        let p = Provider::new(&server.base, "m", "k").unwrap();
        let models = p.models().await.unwrap();
        assert_eq!(models, vec![ModelInfo { id: "a".into(), name: String::new() }, ModelInfo { id: "b".into(), name: String::new() }]);
        assert_eq!(server.seen()[0].header("authorization"), "Bearer k");
    }
}
