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
}

/// ~/Library/Application Support/Corydoras on macOS.
pub fn config_dir() -> PathBuf {
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
