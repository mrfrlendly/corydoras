use chrono::Utc;
use std::path::{Component, Path, PathBuf};

/// Current time as Unix seconds.
pub fn now_ts() -> i64 {
    Utc::now().timestamp()
}

/// Current time as Unix milliseconds.
pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// Current time as an RFC 3339 string (UTC, seconds).
pub fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Key for the usage table, for example "2026-11".
pub fn month_key() -> String {
    Utc::now().format("%Y-%m").to_string()
}

/// Lower-case ASCII slug: "Quiet Instrument!" -> "quiet-instrument".
pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in s.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "language".to_string()
    } else {
        out
    }
}

/// Stable 64-bit FNV-1a hash as hex. Used for duplicate checks and input fingerprints.
pub fn fnv1a64(data: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in data {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Decodes %XX sequences in a URL path.
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(a), Some(b)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(a * 16 + b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Joins a relative path to a root. Returns None for absolute paths, "..", or empty paths.
pub fn safe_join(root: &Path, rel: &str) -> Option<PathBuf> {
    let rel_path = Path::new(rel.trim_start_matches('/'));
    let mut out = root.to_path_buf();
    let mut parts = 0;
    for comp in rel_path.components() {
        match comp {
            Component::Normal(p) => {
                out.push(p);
                parts += 1;
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    if parts == 0 {
        None
    } else {
        Some(out)
    }
}

/// Cuts a string to at most `max` characters, on a char boundary.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug() {
        assert_eq!(slugify("Quiet Instrument!"), "quiet-instrument");
        assert_eq!(slugify("  --  "), "language");
        assert_eq!(slugify("Rams / Braun 1960s"), "rams-braun-1960s");
    }

    #[test]
    fn decode() {
        assert_eq!(percent_decode("a%20b%2Fc"), "a b/c");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%4"), "%4");
    }

    #[test]
    fn join() {
        let root = Path::new("/lib");
        assert!(safe_join(root, "../etc/passwd").is_none());
        assert!(safe_join(root, "").is_none());
        assert_eq!(
            safe_join(root, "/languages/a/thumbs/x.jpg").unwrap(),
            PathBuf::from("/lib/languages/a/thumbs/x.jpg")
        );
    }
}

/// File name of a screen capture, like macOS names screenshots:
/// "Screen capture 2026-11-08 at 09.15.03.png" (local time).
pub fn capture_name() -> String {
    chrono::Local::now().format("Screen capture %Y-%m-%d at %H.%M.%S.png").to_string()
}
