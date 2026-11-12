//! API key storage. Each key is a file in `<config dir>/keys/` that only the user can
//! read (mode 0600, folder 0700). On macOS that is
//! `~/Library/Application Support/Corydoras/keys/`.
//! Each provider profile has its own account (one file), so every provider keeps its own key.
//! The environment variable CORYDORAS_API_KEY overrides every key (development and tests).
//!
//! Up to 0.x the keys were in the macOS login Keychain. Because the app is ad-hoc signed,
//! every new build made macOS ask for the login password again. The first time an account
//! is used, its Keychain item is moved into a key file and deleted from the Keychain (that
//! can show one last Keychain prompt). The names of moved accounts are kept in
//! `keys/.from-keychain`, so the Keychain is never read again for them.
//!
//! The app reads each key file at most one time per launch and keeps the key in memory.

use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// The account of the DeepSeek key from 0.1 to 0.4. The migrated
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

/// The key of one account.
pub fn get_key(account: &str) -> Option<String> {
    if let Some(k) = env_key() {
        return Some(k);
    }
    if let Some(cached) = with_cache(|c| c.get(account).cloned()) {
        return cached;
    }
    // First read of this launch. A failed read counts as "no key".
    let value = store::get(account);
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
    store::set(account, key)?;
    with_cache(|c| c.insert(account.to_string(), Some(key.to_string())));
    Ok(())
}

pub fn clear_key(account: &str) -> Result<()> {
    store::clear(account)?;
    with_cache(|c| c.insert(account.to_string(), None));
    Ok(())
}

/// Forgets all cached values, so the next reads go to the key files again.
pub fn reload() {
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The folder of the key files. CORYDORAS_KEYS_DIR replaces it (tests).
pub fn keys_dir() -> PathBuf {
    match std::env::var_os("CORYDORAS_KEYS_DIR") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => crate::config::config_dir().join("keys"),
    }
}

/// A name that is safe as a file name in the keys folder.
pub(crate) fn safe_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

/// Writes a file that only the user can read, in a folder that only the user can open
/// (folder mode 0700, file mode 0600). The text goes through a temporary file that has
/// mode 0600 from the start, so it is never readable by others, not even for a moment,
/// and a crash leaves the old file whole.
pub(crate) fn write_private_file(path: &std::path::Path, contents: &str) -> Result<()> {
    use std::fs;
    use std::io::Write;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        }
    }
    let tmp = path.with_extension("tmp");
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    #[cfg(unix)]
    {
        // The mode of open() applies only to new files; make sure of it.
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    f.write_all(contents.as_bytes())?;
    f.sync_all()?;
    drop(f);
    fs::rename(&tmp, path)?;
    Ok(())
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

/// Key files, plus the one-time move out of the macOS Keychain.
mod store {
    use super::keys_dir;
    use anyhow::Result;
    use std::fs;
    use std::path::PathBuf;

    /// File of the names of the accounts whose Keychain item was already handled.
    const MOVED_LIST: &str = ".from-keychain";

    fn path(account: &str) -> PathBuf {
        keys_dir().join(super::safe_name(account))
    }

    fn write_private(path: &PathBuf, contents: &str) -> Result<()> {
        super::write_private_file(path, contents)
    }

    fn read_file(account: &str) -> Option<String> {
        fs::read_to_string(path(account))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    pub fn get(account: &str) -> Option<String> {
        if let Some(k) = read_file(account) {
            return Some(k);
        }
        move_from_keychain(account)
    }

    pub fn set(account: &str, key: &str) -> Result<()> {
        // Take the old Keychain item away first, so it cannot linger there.
        let _ = move_from_keychain(account);
        write_private(&path(account), key)
    }

    pub fn clear(account: &str) -> Result<()> {
        let _ = move_from_keychain(account);
        match fs::remove_file(path(account)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    fn moved_accounts() -> Vec<String> {
        fs::read_to_string(keys_dir().join(MOVED_LIST))
            .map(|s| s.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
            .unwrap_or_default()
    }

    fn mark_moved(account: &str) -> Result<()> {
        let mut list = moved_accounts();
        if !list.iter().any(|a| a == account) {
            list.push(account.to_string());
            write_private(&keys_dir().join(MOVED_LIST), &(list.join("\n") + "\n"))?;
        }
        Ok(())
    }

    /// Moves the account's Keychain item (if any) into its key file, one time per
    /// account. Returns the moved key. Does nothing off macOS.
    fn move_from_keychain(account: &str) -> Option<String> {
        if moved_accounts().iter().any(|a| a == account) {
            return None;
        }
        let key = keychain::take(account);
        if let Some(k) = &key {
            // Without the file, keep the Keychain item: better a prompt than a lost key.
            if write_private(&path(account), k).is_err() {
                return key;
            }
        }
        if mark_moved(account).is_err() {
            return key;
        }
        if key.is_some() {
            keychain::delete(account);
        }
        key
    }

    #[cfg(target_os = "macos")]
    mod keychain {
        const SERVICE: &str = "com.travisforsyth.corydoras";

        pub fn take(account: &str) -> Option<String> {
            keyring::Entry::new(SERVICE, account)
                .ok()?
                .get_password()
                .ok()
                .map(|k| k.trim().to_string())
                .filter(|k| !k.is_empty())
        }

        pub fn delete(account: &str) {
            if let Ok(e) = keyring::Entry::new(SERVICE, account) {
                let _ = e.delete_credential();
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    mod keychain {
        pub fn take(_account: &str) -> Option<String> {
            None
        }
        pub fn delete(_account: &str) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_files_are_private_and_cached() {
        std::env::remove_var("CORYDORAS_API_KEY");
        std::env::remove_var("DEEPSEEK_API_KEY");
        let dir = std::env::temp_dir().join(format!("cory-secrets-{}", std::process::id()));
        std::env::set_var("CORYDORAS_KEYS_DIR", dir.join("keys"));
        // Accounts that have no Keychain item, so the test never touches a real one.
        let a = "test-legacy-account-x";
        let b = "provider-test-x";
        reload();
        set_key(a, "sk-abc").unwrap();
        set_key(b, "sk-x").unwrap();
        assert_eq!(get_key(a).as_deref(), Some("sk-abc"));
        assert_eq!(get_key(b).as_deref(), Some("sk-x"));
        assert!(!has_key("provider-none"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: std::path::PathBuf| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(keys_dir()), 0o700);
            assert_eq!(mode(keys_dir().join(b)), 0o600);
        }

        // Change the file behind the cache: the cache still answers.
        std::fs::write(keys_dir().join(a), "sk-other").unwrap();
        assert_eq!(get_key(a).as_deref(), Some("sk-abc"));
        reload();
        assert_eq!(get_key(a).as_deref(), Some("sk-other"));
        clear_key(a).unwrap();
        clear_key(b).unwrap();
        reload();
        assert!(!has_key(a));
        assert!(!has_key(b));
        let _ = std::fs::remove_dir_all(dir);
    }
}
