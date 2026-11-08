//! API key storage. On macOS each key is an item in the login Keychain.
//! Each provider profile has its own Keychain account, so every provider keeps its own key.
//! The environment variable CORYDORAS_API_KEY overrides every key (development and tests).
//!
//! The app reads each Keychain item at most one time per launch and keeps the key in
//! memory. Each Keychain read can show a macOS access prompt, so the status poll and the
//! worker must never read the Keychain directly.

use anyhow::Result;
use std::collections::HashMap;
use std::sync::Mutex;

#[cfg(target_os = "macos")]
const SERVICE: &str = "com.travisforsyth.corydoras";

/// The Keychain account of the DeepSeek key from 0.1 to 0.4. The migrated
/// DeepSeek profile keeps this account, so the key stays valid.
pub const LEGACY_ACCOUNT: &str = "deepseek-api-key";

/// Account name -> key (None: no key). A missing entry means "not read yet".
static CACHE: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);

fn env_key() -> Option<String> {
    for var in ["CORYDORAS_API_KEY", "DEEPSEEK_API_KEY"] {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

fn with_cache<T>(f: impl FnOnce(&mut HashMap<String, Option<String>>) -> T) -> T {
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(HashMap::new))
}

/// The key of one Keychain account.
pub fn get_key(account: &str) -> Option<String> {
    if let Some(k) = env_key() {
        return Some(k);
    }
    if let Some(cached) = with_cache(|c| c.get(account).cloned()) {
        return cached;
    }
    // First read of this launch. A refused or failed read counts as "no key",
    // so a refusal does not make the prompt come back every few seconds.
    let value = platform::get(account);
    with_cache(|c| c.insert(account.to_string(), value.clone()));
    value
}

/// True when the account has a key, from the cache. Accounts that were not read yet
/// are read one time.
pub fn has_key(account: &str) -> bool {
    get_key(account).is_some()
}

pub fn set_key(account: &str, key: &str) -> Result<()> {
    let key = key.trim();
    if key.is_empty() {
        anyhow::bail!("The key is empty.");
    }
    platform::set(account, key)?;
    with_cache(|c| c.insert(account.to_string(), Some(key.to_string())));
    Ok(())
}

pub fn clear_key(account: &str) -> Result<()> {
    platform::clear(account)?;
    with_cache(|c| c.insert(account.to_string(), None));
    Ok(())
}

/// Forgets all cached values, so the next reads go to the Keychain again.
pub fn reload() {
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

// The single-key functions of 0.4, for the legacy account.
pub fn get_api_key() -> Option<String> {
    get_key(LEGACY_ACCOUNT)
}
pub fn has_api_key() -> bool {
    has_key(LEGACY_ACCOUNT)
}
pub fn set_api_key(key: &str) -> Result<()> {
    set_key(LEGACY_ACCOUNT, key)
}
pub fn clear_api_key() -> Result<()> {
    clear_key(LEGACY_ACCOUNT)
}

#[cfg(target_os = "macos")]
mod platform {
    use super::SERVICE;
    use anyhow::Result;

    pub fn get(account: &str) -> Option<String> {
        keyring::Entry::new(SERVICE, account)
            .ok()?
            .get_password()
            .ok()
            .filter(|k| !k.trim().is_empty())
    }

    /// Deletes the old item first, so the new item belongs to this build of the
    /// app. Then macOS does not ask for the login password on later reads.
    pub fn set(account: &str, key: &str) -> Result<()> {
        let entry = keyring::Entry::new(SERVICE, account)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(e.into()),
        }
        entry.set_password(key)?;
        Ok(())
    }

    pub fn clear(account: &str) -> Result<()> {
        match keyring::Entry::new(SERVICE, account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// Development fallback for other systems: files with mode 0600 in the config folder.
#[cfg(not(target_os = "macos"))]
mod platform {
    use anyhow::Result;

    fn path(account: &str) -> std::path::PathBuf {
        let name = if account == super::LEGACY_ACCOUNT {
            "api_key".to_string()
        } else {
            format!("key-{}", account.replace(['/', '\\', '.'], "_"))
        };
        crate::config::config_dir().join(name)
    }

    pub fn get(account: &str) -> Option<String> {
        std::fs::read_to_string(path(account))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    pub fn set(account: &str, key: &str) -> Result<()> {
        std::fs::create_dir_all(crate::config::config_dir())?;
        let p = path(account);
        std::fs::write(&p, key)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    pub fn clear(account: &str) -> Result<()> {
        let p = path(account);
        if p.exists() {
            std::fs::remove_file(p)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_reads_once_per_account() {
        // Non-macOS test path: the file store. The cache must return the set value
        // without a second store read, and clear must stick.
        std::env::remove_var("CORYDORAS_API_KEY");
        std::env::remove_var("DEEPSEEK_API_KEY");
        let dir = std::env::temp_dir().join(format!("cory-secrets-{}", std::process::id()));
        std::env::set_var("XDG_CONFIG_HOME", &dir);
        reload();
        set_api_key("sk-abc").unwrap();
        set_key("provider-x", "sk-x").unwrap();
        assert_eq!(get_api_key().as_deref(), Some("sk-abc"));
        assert_eq!(get_key("provider-x").as_deref(), Some("sk-x"));
        assert!(!has_key("provider-none"));
        // Change the store behind the cache: the cache still answers.
        std::fs::write(crate::config::config_dir().join("api_key"), "sk-other").unwrap();
        assert_eq!(get_api_key().as_deref(), Some("sk-abc"));
        reload();
        assert_eq!(get_api_key().as_deref(), Some("sk-other"));
        clear_api_key().unwrap();
        clear_key("provider-x").unwrap();
        assert!(!has_api_key());
        assert!(!has_key("provider-x"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
