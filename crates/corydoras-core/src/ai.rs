//! Model providers. A profile is one provider plus one model, with its own key.
//! The analysis of images and sources uses one profile; the language document
//! (synthesis) can use another, for example a cheap vision model for the images
//! and a stronger model for the writing.
//!
//! Profiles are app settings (config.json), not library data, so they stay the same
//! when you switch or move the library. The keys are key files (see secrets.rs).

use crate::models::Settings;
use crate::secrets;
use crate::signin::chatgpt;
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};

/// The two wire formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Api {
    /// POST {base}/chat/completions (OpenAI, DeepSeek, Gemini, OpenRouter, Mistral, xAI, Ollama, LM Studio).
    Openai,
    /// POST {base}/messages (Anthropic).
    Anthropic,
}

/// A ready-made provider setting.
#[derive(Clone, Debug, Serialize)]
pub struct Preset {
    pub kind: &'static str,
    pub name: &'static str,
    pub api: Api,
    pub base_url: &'static str,
    /// A suggested model. "Load models" lists the models of your account.
    pub model: &'static str,
    pub needs_key: bool,
    /// Runs on this Mac: longer waits, no key.
    pub local: bool,
    /// Largest answer (output tokens) to ask for. Calls ask for less when they can.
    pub max_output: u32,
    /// Where to make a key.
    pub key_url: &'static str,
    pub note: &'static str,
    /// The account sign-in that the provider offers: "openrouter", "chatgpt", or "" for none.
    pub sign_in: &'static str,
}

pub fn presets() -> Vec<Preset> {
    vec![
        Preset {
            kind: "deepseek",
            name: "DeepSeek",
            api: Api::Openai,
            base_url: "https://api.deepseek.com",
            model: "deepseek-flash",
            needs_key: true,
            local: false,
            max_output: 64_000,
            key_url: "https://platform.deepseek.com/api_keys",
            sign_in: "",
            note: "Low cost. Thinking mode is available for the language document.",
        },
        Preset {
            kind: "openai",
            name: "OpenAI",
            api: Api::Openai,
            base_url: "https://api.openai.com/v1",
            model: "gpt-5-mini",
            needs_key: true,
            local: false,
            max_output: 64_000,
            key_url: "https://platform.openai.com/api-keys",
            sign_in: "",
            note: "Reasoning models use the Thinking setting as their reasoning effort.",
        },
        Preset {
            kind: "chatgpt",
            name: "ChatGPT plan (OpenAI)",
            api: Api::Openai,
            base_url: chatgpt::API_BASE,
            model: "",
            needs_key: false,
            local: false,
            max_output: 64_000,
            key_url: "",
            sign_in: "chatgpt",
            note: "Sign in with ChatGPT: the calls use the usage that is included in your ChatGPT plan or credits balance, not an API key. \
                   The plan route takes no output limit, and tokens have no price here.",
        },
        Preset {
            kind: "anthropic",
            name: "Anthropic (Claude)",
            api: Api::Anthropic,
            base_url: "https://api.anthropic.com/v1",
            model: "claude-sonnet-5-5",
            needs_key: true,
            local: false,
            max_output: 64_000,
            key_url: "https://platform.claude.com/settings/keys",
            sign_in: "",
            note: "Uses the Messages API. Thinking uses adaptive thinking, with a fallback for older models.",
        },
        Preset {
            kind: "gemini",
            name: "Google Gemini",
            api: Api::Openai,
            base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
            model: "gemini-2.5-flash",
            needs_key: true,
            local: false,
            max_output: 64_000,
            key_url: "https://aistudio.google.com/apikey",
            sign_in: "",
            note: "Uses the OpenAI-compatible endpoint of the Gemini API.",
        },
        Preset {
            kind: "openrouter",
            name: "OpenRouter",
            api: Api::Openai,
            base_url: "https://openrouter.ai/api/v1",
            model: "",
            needs_key: true,
            local: false,
            max_output: 32_000,
            key_url: "https://openrouter.ai/settings/keys",
            sign_in: "openrouter",
            note: "One account for many providers: sign in with OpenRouter, or paste a key. Choose a model that accepts images for the image analysis.",
        },
        Preset {
            kind: "mistral",
            name: "Mistral",
            api: Api::Openai,
            base_url: "https://api.mistral.ai/v1",
            model: "mistral-medium-latest",
            needs_key: true,
            local: false,
            max_output: 32_000,
            key_url: "https://console.mistral.ai/api-keys",
            sign_in: "",
            note: "",
        },
        Preset {
            kind: "xai",
            name: "xAI (Grok)",
            api: Api::Openai,
            base_url: "https://api.x.ai/v1",
            model: "",
            needs_key: true,
            local: false,
            max_output: 32_000,
            key_url: "https://console.x.ai",
            sign_in: "",
            note: "",
        },
        Preset {
            kind: "ollama",
            name: "Ollama (on this Mac)",
            api: Api::Openai,
            base_url: "http://localhost:11434/v1",
            model: "",
            needs_key: false,
            local: true,
            max_output: 16_000,
            key_url: "https://ollama.com/download",
            sign_in: "",
            note: "Free and private. Use a vision model (for example a Gemma, Qwen-VL or LLaVA model) for images. \
                   Set the context length in Ollama to 32 k or more: the language document needs a long prompt.",
        },
        Preset {
            kind: "lmstudio",
            name: "LM Studio (on this Mac)",
            api: Api::Openai,
            base_url: "http://localhost:1234/v1",
            model: "",
            needs_key: false,
            local: true,
            max_output: 16_000,
            key_url: "https://lmstudio.ai",
            sign_in: "",
            note: "Free and private. Start the local server in LM Studio and load a model with a long context.",
        },
        Preset {
            kind: "custom",
            name: "Other (OpenAI-compatible)",
            api: Api::Openai,
            base_url: "",
            model: "",
            needs_key: true,
            local: false,
            max_output: 16_000,
            key_url: "",
            sign_in: "",
            note: "Any server with an OpenAI-compatible /chat/completions endpoint.",
        },
    ]
}

pub fn preset(kind: &str) -> Preset {
    let all = presets();
    let custom = all.iter().find(|p| p.kind == "custom").cloned().expect("custom preset");
    all.into_iter().find(|p| p.kind == kind).unwrap_or(custom)
}

/// One provider and one model.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProviderProfile {
    pub id: String,
    pub name: String,
    /// Preset kind: deepseek, openai, chatgpt, anthropic, gemini, openrouter, mistral, xai, ollama, lmstudio, custom.
    pub kind: String,
    pub api: Api,
    pub base_url: String,
    pub model: String,
    /// Thinking for the language document: "off", "low" or "high". The analyses always run without it.
    pub thinking: String,
    /// Largest answer to ask for (output tokens).
    pub max_output: u32,
    /// The result of the last connection test: does the model read images?
    #[serde(default)]
    pub vision: Option<bool>,
    /// The account (key file name) of the key.
    pub key_account: String,
    pub needs_key: bool,
    /// Price of one million input tokens in US dollars. 0: not set, so cost
    /// estimates show tokens only.
    #[serde(default)]
    pub price_in: f64,
    /// Price of one million output tokens in US dollars.
    #[serde(default)]
    pub price_out: f64,
}

impl ProviderProfile {
    pub fn from_preset(kind: &str) -> Self {
        let p = preset(kind);
        let id = format!("p-{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
        Self {
            key_account: format!("provider-{id}"),
            id,
            name: p.name.to_string(),
            kind: p.kind.to_string(),
            api: p.api,
            base_url: p.base_url.to_string(),
            model: p.model.to_string(),
            thinking: if kind == "deepseek" { "low".into() } else { "off".into() },
            max_output: p.max_output,
            vision: None,
            needs_key: p.needs_key,
            price_in: 0.0,
            price_out: 0.0,
        }
    }

    /// True when a signed-in account pays for the calls, not an API key.
    pub fn uses_account(&self) -> bool {
        self.kind == "chatgpt"
    }

    pub fn local(&self) -> bool {
        preset(&self.kind).local
            || ["localhost", "127.0.0.1", "[::1]"].iter().any(|h| self.base_url.contains(h))
    }

    /// True when the provider can pay for a call: it has its key, needs none, or (for the
    /// ChatGPT plan) the app is signed in and the plan is allowed.
    pub fn has_key(&self) -> bool {
        if self.uses_account() {
            return chatgpt::Session::app().account().is_some_and(|a| a.signed_in() && a.plan_enabled());
        }
        !self.needs_key || secrets::has_key(&self.key_account)
    }

    /// The key, or an empty string for a provider without a key.
    pub fn key(&self) -> Result<String> {
        if self.uses_account() {
            return if self.has_key() { Ok(String::new()) } else { Err(anyhow!("Sign in to ChatGPT for \"{}\" in Settings.", self.name)) };
        }
        match secrets::get_key(&self.key_account) {
            Some(k) => Ok(k),
            None if !self.needs_key => Ok(String::new()),
            None => Err(anyhow!("Add the API key of \"{}\" in Settings.", self.name)),
        }
    }

    /// "DeepSeek · deepseek-flash"
    pub fn label(&self) -> String {
        if self.model.is_empty() {
            self.name.clone()
        } else {
            format!("{} · {}", self.name, self.model)
        }
    }

    /// Cleans the fields. Errors for values that cannot work.
    pub fn normalized(mut self) -> Result<Self> {
        self.name = self.name.trim().to_string();
        self.base_url = self.base_url.trim().trim_end_matches('/').to_string();
        self.model = self.model.trim().to_string();
        let t = self.thinking.trim().to_ascii_lowercase();
        self.thinking = if ["off", "low", "high"].contains(&t.as_str()) { t } else { "off".into() };
        self.max_output = self.max_output.clamp(1_000, 200_000);
        let price = |v: f64| if v.is_finite() && v > 0.0 { v.min(100_000.0) } else { 0.0 };
        self.price_in = price(self.price_in);
        self.price_out = price(self.price_out);
        if self.name.is_empty() {
            self.name = preset(&self.kind).name.to_string();
        }
        if self.uses_account() {
            // The tokens of the plan go to OpenAI only.
            self.base_url = chatgpt::API_BASE.to_string();
            self.api = Api::Openai;
            self.needs_key = false;
        }
        if !(self.base_url.starts_with("http://") || self.base_url.starts_with("https://")) {
            bail!("The address of \"{}\" must start with http:// or https://.", self.name);
        }
        if self.key_account.trim().is_empty() {
            self.key_account = format!("provider-{}", self.id);
        }
        Ok(self)
    }
}

/// The providers and which profile does which job.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct AiConfig {
    pub profiles: Vec<ProviderProfile>,
    /// Profile for image and source analysis (needs a model that reads images).
    pub analysis_profile: String,
    /// Profile for the language document.
    pub synthesis_profile: String,
}

/// Which job a profile does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Analysis,
    Synthesis,
}

impl AiConfig {
    /// The 0.4 setting (one DeepSeek-compatible provider in the library settings) as a
    /// profile. It keeps the old key account, so the saved key stays valid.
    pub fn migrated(s: &Settings) -> Self {
        let lower = s.base_url.to_ascii_lowercase();
        let kind = if lower.contains("deepseek") {
            "deepseek"
        } else if lower.contains("anthropic.com") {
            "anthropic"
        } else if lower.contains("api.openai.com") {
            "openai"
        } else if lower.contains("localhost:11434") {
            "ollama"
        } else {
            "custom"
        };
        let p = preset(kind);
        let profile = ProviderProfile {
            id: "p-default".into(),
            name: p.name.to_string(),
            kind: kind.into(),
            api: p.api,
            base_url: s.base_url.trim().trim_end_matches('/').to_string(),
            model: s.model.trim().to_string(),
            thinking: s.thinking.clone(),
            max_output: p.max_output,
            vision: None,
            key_account: secrets::LEGACY_ACCOUNT.into(),
            needs_key: p.needs_key,
            price_in: 0.0,
            price_out: 0.0,
        };
        Self {
            profiles: vec![profile],
            analysis_profile: "p-default".into(),
            synthesis_profile: "p-default".into(),
        }
    }

    /// The saved configuration, or the migrated 0.4 setting.
    pub fn effective(saved: Option<&AiConfig>, library_settings: &Settings) -> Self {
        match saved {
            Some(c) if !c.profiles.is_empty() => c.clone(),
            _ => Self::migrated(library_settings),
        }
    }

    pub fn profile(&self, id: &str) -> Option<&ProviderProfile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    /// The profile for a job. Falls back to the first profile.
    pub fn for_role(&self, role: Role) -> Option<&ProviderProfile> {
        let id = match role {
            Role::Analysis => &self.analysis_profile,
            Role::Synthesis => &self.synthesis_profile,
        };
        self.profile(id).or_else(|| self.profiles.first())
    }

    /// Cleans the profiles and repairs the job assignments.
    pub fn normalized(self) -> Result<Self> {
        let mut profiles = Vec::new();
        for p in self.profiles {
            if profiles.iter().any(|q: &ProviderProfile| q.id == p.id) {
                bail!("Two providers have the same ID \"{}\".", p.id);
            }
            profiles.push(p.normalized()?);
        }
        if profiles.is_empty() {
            bail!("Keep at least one provider.");
        }
        let first = profiles[0].id.clone();
        let fix = |id: String| if profiles.iter().any(|p| p.id == id) { id } else { first.clone() };
        Ok(Self {
            analysis_profile: fix(self.analysis_profile),
            synthesis_profile: fix(self.synthesis_profile),
            profiles,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_keeps_the_old_key_account() {
        let c = AiConfig::effective(None, &Settings::default());
        assert_eq!(c.profiles.len(), 1);
        let p = c.for_role(Role::Synthesis).unwrap();
        assert_eq!(p.kind, "deepseek");
        assert_eq!(p.key_account, secrets::LEGACY_ACCOUNT);
        assert_eq!(p.model, "deepseek-flash");
        assert_eq!(c.for_role(Role::Analysis).unwrap().id, p.id);
    }

    #[test]
    fn normalize_repairs_roles() {
        let mut a = ProviderProfile::from_preset("anthropic");
        a.base_url = "https://api.anthropic.com/v1/".into();
        a.thinking = "HIGH".into();
        let o = ProviderProfile::from_preset("ollama");
        assert!(!o.needs_key && o.local());
        let c = AiConfig {
            profiles: vec![a.clone(), o.clone()],
            analysis_profile: o.id.clone(),
            synthesis_profile: "gone".into(),
        }
        .normalized()
        .unwrap();
        assert_eq!(c.profiles[0].base_url, "https://api.anthropic.com/v1");
        assert_eq!(c.profiles[0].thinking, "high");
        assert_eq!(c.analysis_profile, o.id);
        assert_eq!(c.synthesis_profile, a.id);
        let bad = AiConfig { profiles: vec![], ..Default::default() };
        assert!(bad.normalized().is_err());
        let mut x = ProviderProfile::from_preset("custom");
        x.base_url = "api.example.com".into();
        assert!(x.normalized().is_err());
    }

    #[test]
    fn the_chatgpt_plan_provider_needs_a_sign_in_and_no_key() {
        use crate::provider::Dialect;
        use crate::signin::chatgpt;
        let p = ProviderProfile::from_preset("chatgpt");
        assert!(p.uses_account() && !p.needs_key);
        assert_eq!(p.base_url, chatgpt::API_BASE);
        assert_eq!(Dialect::of_profile(&p), Dialect::ChatGpt);
        // The tokens of the plan go to OpenAI only: another address does not stay.
        let mut moved = p.clone();
        moved.base_url = "https://elsewhere.example/v1".into();
        moved.needs_key = true;
        let n = moved.normalized().unwrap();
        assert_eq!(n.base_url, chatgpt::API_BASE);
        assert!(!n.needs_key);
        // A custom provider with the same address is a key provider.
        let mut custom = ProviderProfile::from_preset("custom");
        custom.base_url = chatgpt::API_BASE.into();
        assert!(!custom.uses_account());
        assert_eq!(Dialect::of_profile(&custom), Dialect::OpenAi);
    }

    #[test]
    fn two_providers_offer_a_sign_in() {
        assert_eq!(preset("chatgpt").sign_in, "chatgpt");
        assert_eq!(preset("openrouter").sign_in, "openrouter");
        let with: Vec<&str> = presets().iter().filter(|p| !p.sign_in.is_empty()).map(|p| p.kind).collect();
        assert_eq!(with, vec!["chatgpt", "openrouter"]);
        // OpenRouter's sign-in gives a key, so the provider stays a key provider.
        let o = ProviderProfile::from_preset("openrouter");
        assert!(o.needs_key && !o.uses_account());
    }
}
