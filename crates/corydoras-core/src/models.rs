use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Language {
    pub id: String,
    pub name: String,
    pub description: String,
    pub created_at: String,
    pub updated_at: String,
    pub item_count: i64,
    pub latest_version: Option<i64>,
    /// Links and documents.
    pub source_count: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Item {
    pub id: String,
    pub language_id: String,
    pub kind: String,
    /// Path of the original file, relative to the library root.
    pub file_rel: String,
    /// Path of the thumbnail, relative to the library root.
    pub thumb_rel: String,
    pub original_name: String,
    pub source: Option<String>,
    pub width: i64,
    pub height: i64,
    pub added_at: String,
    pub tags: Vec<String>,
    pub note: String,
    pub weight: i64,
    pub anti: bool,
    pub analysis_status: String,
    pub analysis_error: Option<String>,
    pub region_count: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Region {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub note: String,
    pub tag: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RegionInput {
    pub id: Option<String>,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub tag: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ItemDetail {
    pub item: Item,
    pub regions: Vec<Region>,
    pub analysis: Option<Value>,
    pub analysis_model: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VersionInfo {
    pub language_id: String,
    pub version: i64,
    pub created_at: String,
    pub model: String,
    pub item_count: i64,
    pub edited: bool,
    /// 1: free markdown plus tokens. 2: structured spec; the app renders the markdown.
    pub schema: i64,
    /// Findings of the automatic review (soft checks).
    pub warnings: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VersionContent {
    pub info: VersionInfo,
    pub markdown: String,
    pub tokens: Value,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Pin {
    pub language_id: String,
    pub language_name: String,
    pub version: i64,
    /// "primary" or "secondary".
    pub role: String,
    /// "all", "cad", "cmf" or "prompts".
    pub scope: String,
    pub latest_version: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PinInput {
    pub language_id: String,
    pub version: i64,
    pub role: String,
    pub scope: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub pins: Vec<Pin>,
    /// Rule sets that this project selects. Rule sets marked "always" apply too.
    pub rule_set_ids: Vec<String>,
}

/// One hard rule. A rule with a value is a limit (minimum wall 1.2 mm);
/// a rule without a value is an instruction (orient load paths along the layers).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Rule {
    pub name: String,
    #[serde(default)]
    pub value: Option<f64>,
    /// "mm", "deg", "percent", "count" or "ratio". Only with a value.
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub note: String,
}

/// A set of hard rules, for example 3D-printing limits. Rule sets win over
/// every design language. They also supply the values that a language marks
/// as process-dependent.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RuleSet {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Applies to every project and to get_language without a project.
    pub always: bool,
    pub rules: Vec<Rule>,
    pub created_at: String,
    pub updated_at: String,
    /// Projects that select this rule set (not counting "always").
    pub project_count: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RuleSetInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub always: bool,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settings {
    pub base_url: String,
    pub model: String,
    /// Seconds without changes before the synthesis pass runs.
    pub debounce_secs: i64,
    /// Input + output tokens for each calendar month. 0 means no limit.
    pub monthly_token_budget: i64,
    /// Thinking mode of the synthesis pass: "off", "low" or "high".
    /// The item analysis and the connection test always run with thinking off.
    #[serde(default = "default_thinking")]
    pub thinking: String,
}

fn default_thinking() -> String {
    "low".into()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            base_url: "https://api.deepseek.com".into(),
            model: "deepseek-flash".into(),
            debounce_secs: 60,
            monthly_token_budget: 20_000_000,
            thinking: default_thinking(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Usage {
    pub month: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub requests: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Job {
    pub id: i64,
    pub kind: String,
    pub target: String,
    pub run_after: i64,
    pub attempts: i64,
    pub gen: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LanguageStatus {
    pub language_id: String,
    /// Items with an analysis job in the queue.
    pub pending_items: i64,
    /// Items whose analysis failed.
    pub error_items: i64,
    /// Seconds until the synthesis pass, if one is in the queue.
    pub synth_due_in: Option<i64>,
    /// Amendment proposals from agents that wait for a decision.
    #[serde(default)]
    pub open_proposals: i64,
}

/// One item and its inputs for the synthesis pass.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SynthInput {
    pub item: Item,
    pub regions: Vec<Region>,
    pub analysis: Value,
}

/// One job in the queue, for display.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct JobView {
    pub id: i64,
    /// "analyze_item" or "synthesize".
    pub kind: String,
    pub target: String,
    pub language_id: String,
    pub language_name: String,
    pub item_name: Option<String>,
    pub run_after_ms: i64,
    pub attempts: i64,
    pub last_error: Option<String>,
    /// Set by the app: true for the job that runs now.
    pub running: bool,
}

/// A link or a document that feeds the analysis of a language.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Source {
    pub id: String,
    pub language_id: String,
    /// "link" or "document".
    pub kind: String,
    pub title: String,
    pub url: Option<String>,
    pub site: String,
    pub description: String,
    /// "page", "video", "pdf", "text" or "doc".
    pub media: String,
    /// Stored file, relative to the library root (documents, and links to a PDF).
    pub file_rel: Option<String>,
    /// Preview image, relative to the library root.
    pub preview_rel: Option<String>,
    pub original_name: String,
    pub text_chars: i64,
    pub word_count: i64,
    pub added_at: String,
    pub note: String,
    pub weight: i64,
    pub analysis_status: String,
    pub analysis_error: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SourceDetail {
    pub source: Source,
    pub analysis: Option<Value>,
    pub analysis_model: Option<String>,
    /// The start of the extracted text.
    pub text_preview: String,
}

/// One source and its notes for the synthesis pass.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SynthSource {
    pub source: Source,
    pub analysis: Value,
}

/// The frames that a video import added.
#[derive(Serialize, Clone, Debug)]
pub struct VideoImport {
    pub items: Vec<Item>,
    /// Length of the video (seconds).
    pub duration: f64,
}
