use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// App-level configuration. It lives outside the library, so that the app and
/// the MCP server can both find the library folder.
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct AppConfig {
    pub library_path: Option<PathBuf>,
    /// Libraries that the app opened before, newest first (for "Open another library").
    #[serde(default)]
    pub recent_libraries: Vec<PathBuf>,
    /// Model providers. None until you change them (then the 0.4 setting is migrated).
    #[serde(default)]
    pub ai: Option<crate::ai::AiConfig>,
    /// Screen capture from the menu bar and the keyboard shortcut.
    #[serde(default)]
    pub capture: CaptureConfig,
    /// The local bridge that the browser extension talks to.
    #[serde(default)]
    pub bridge: BridgeConfig,
}

/// The port of the bridge when nothing else is set.
pub const DEFAULT_BRIDGE_PORT: u16 = 47821;

/// The bridge listens on this computer only (127.0.0.1) and answers only
/// requests that carry the token. It is off until you turn it on.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BridgeConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_bridge_port")]
    pub port: u16,
    /// The secret that the extension sends with each request.
    #[serde(default)]
    pub token: String,
}

fn default_bridge_port() -> u16 {
    DEFAULT_BRIDGE_PORT
}

impl Default for BridgeConfig {
    fn default() -> Self {
        BridgeConfig { enabled: false, port: DEFAULT_BRIDGE_PORT, token: String::new() }
    }
}

/// The shortcut that captures when nothing else is set: Control-Option-Command-C.
pub const DEFAULT_CAPTURE_SHORTCUT: &str = "Control+Alt+Super+KeyC";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CaptureConfig {
    /// The language that captures go to. None (or a language that is gone):
    /// the language that changed last.
    #[serde(default)]
    pub language_id: Option<String>,
    /// Global shortcut in the form "Control+Alt+Super+KeyC". None: no shortcut.
    #[serde(default = "default_shortcut")]
    pub shortcut: Option<String>,
}

fn default_shortcut() -> Option<String> {
    Some(DEFAULT_CAPTURE_SHORTCUT.to_string())
}

impl Default for CaptureConfig {
    fn default() -> Self {
        CaptureConfig { language_id: None, shortcut: default_shortcut() }
    }
}

/// ~/Library/Application Support/Corydoras on macOS. CORYDORAS_CONFIG_DIR
/// names another folder (development: a run that must not touch your settings
/// and keys).
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CORYDORAS_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Corydoras")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

/// The library folder that the app suggests at first launch.
pub fn default_library_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Corydoras")
}

impl AppConfig {
    pub fn load() -> Self {
        std::fs::read_to_string(config_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Sets the library and keeps the previous one in the recent list.
    pub fn set_library(&mut self, path: PathBuf) {
        if let Some(old) = self.library_path.take() {
            if old != path {
                self.recent_libraries.retain(|p| p != &old);
                self.recent_libraries.insert(0, old);
            }
        }
        self.recent_libraries.retain(|p| p != &path);
        self.recent_libraries.truncate(8);
        self.library_path = Some(path);
    }

    pub fn save(&self) -> Result<()> {
        std::fs::create_dir_all(config_dir())?;
        std::fs::write(config_path(), serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

/// Library folder: command-line value, then CORYDORAS_LIBRARY, then the app config.
pub fn resolve_library_path(cli: Option<PathBuf>) -> Option<PathBuf> {
    cli.or_else(|| std::env::var_os("CORYDORAS_LIBRARY").map(PathBuf::from))
        .or_else(|| AppConfig::load().library_path)
}
