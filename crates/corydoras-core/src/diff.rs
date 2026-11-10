//! Semantic diff: what changed between two versions of a language, in words
//! ("blade pitch: 8–10 mm → 9–11 mm, evidence added"), not in lines of JSON.
//! Also the rollback: an earlier version saved again as the newest one.

use crate::library::Library;
use crate::spec;
use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use serde_json::Value;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Change {
    /// "Summary", "Principles", "Family: Blade field", "Materials and CMF", ...
    pub section: String,
    /// "added", "removed" or "changed"
    pub kind: String,
    /// What changed: an ID or a name.
    pub subject: String,
    /// The change in words.
    pub text: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct VersionDiff {
    pub language: String,
    pub from: i64,
    pub to: i64,
    /// False when one of the versions uses the older format: there is no
    /// structure to compare, only the text.
    pub structured: bool,
    pub changes: Vec<Change>,
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}

fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k).and_then(|x| x.as_array()).map(|a| a.as_slice()).unwrap_or(&[])
}

fn ids(v: &Value, k: &str) -> Vec<String> {
    arr(v, k).iter().filter_map(|x| x.as_str().map(|s| s.chars().take(8).collect())).collect()
}

fn quote(text: &str) -> String {
    let t = text.trim();
    if t.chars().count() > 160 {
        format!("“{}…”", t.chars().take(160).collect::<String>())
    } else {
        format!("“{t}”")
    }
}

fn key(v: &Value, field: &str) -> String {
    s(v, field).trim().to_lowercase()
}

/// Pairs the elements of two lists by a field. Returns (only in a, only in b, in both).
fn pair<'a>(a: &'a [Value], b: &'a [Value], field: &str) -> (Vec<&'a Value>, Vec<&'a Value>, Vec<(&'a Value, &'a Value)>) {
    let gone = a.iter().filter(|x| !b.iter().any(|y| key(y, field) == key(x, field))).collect();
    let new = b.iter().filter(|y| !a.iter().any(|x| key(x, field) == key(y, field))).collect();
    let both = a.iter().filter_map(|x| b.iter().find(|y| key(y, field) == key(x, field)).map(|y| (x, y))).collect();
    (gone, new, both)
}

/// "evidence added `a` `b`, removed `c`", or nothing.
fn evidence_change(a: &Value, b: &Value, field: &str, word: &str) -> Option<String> {
    let (x, y) = (ids(a, field), ids(b, field));
    let added: Vec<String> = y.iter().filter(|i| !x.contains(i)).map(|i| format!("`{i}`")).collect();
    let removed: Vec<String> = x.iter().filter(|i| !y.contains(i)).map(|i| format!("`{i}`")).collect();
    match (added.is_empty(), removed.is_empty()) {
        (true, true) => None,
        (false, true) => Some(format!("{word} added {}", added.join(" "))),
        (true, false) => Some(format!("{word} removed {}", removed.join(" "))),
        (false, false) => Some(format!("{word} added {}, removed {}", added.join(" "), removed.join(" "))),
    }
}

/// The fields of two objects that differ, as "field: old → new".
fn field_changes(a: &Value, b: &Value, fields: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for f in fields {
        let (x, y) = (a.get(*f).unwrap_or(&Value::Null), b.get(*f).unwrap_or(&Value::Null));
        if x == y {
            continue;
        }
        let show = |v: &Value| match v {
            Value::String(t) => quote(t),
            Value::Null => "none".to_string(),
            other => other.to_string(),
        };
        out.push(format!("{f}: {} → {}", show(x), show(y)));
    }
    out
}

fn push(out: &mut Vec<Change>, section: &str, kind: &str, subject: &str, text: String) {
    out.push(Change { section: section.into(), kind: kind.into(), subject: subject.into(), text });
}

fn rules(out: &mut Vec<Change>, section: &str, a: &[Value], b: &[Value], label: &str) {
    let (gone, new, both) = pair(a, b, "id");
    for r in new {
        push(out, section, "added", s(r, "id"), format!("{label} {} added: {}", s(r, "id"), quote(s(r, "text"))));
    }
    for r in gone {
        push(out, section, "removed", s(r, "id"), format!("{label} {} removed: {}", s(r, "id"), quote(s(r, "text"))));
    }
    for (x, y) in both {
        let mut parts = field_changes(x, y, &["text", "priority", "basis", "overrides", "scope", "when"]);
        parts.extend(evidence_change(x, y, "evidence", "evidence"));
        if !parts.is_empty() {
            push(out, section, "changed", s(y, "id"), format!("{label} {}: {}", s(y, "id"), parts.join("; ")));
        }
    }
}

/// The changes from spec `a` to spec `b` (both schema 2).
pub fn semantic(a: &Value, b: &Value) -> Vec<Change> {
    let mut out: Vec<Change> = Vec::new();
    if s(a, "summary").trim() != s(b, "summary").trim() {
        push(&mut out, "Summary", "changed", "summary", format!("The summary is now: {}", quote(s(b, "summary"))));
    }
    // Inheritance
    let (ea, eb) = (a.get("extends").filter(|e| e.is_object()), b.get("extends").filter(|e| e.is_object()));
    match (ea, eb) {
        (None, Some(e)) => push(&mut out, "Inheritance", "added", s(e, "language"), format!("The language now extends {} v{}.", s(e, "name"), e["version"].as_i64().unwrap_or(0))),
        (Some(e), None) => push(&mut out, "Inheritance", "removed", s(e, "language"), format!("The language no longer extends {}.", s(e, "name"))),
        (Some(x), Some(y)) => {
            if s(x, "language") != s(y, "language") {
                push(&mut out, "Inheritance", "changed", s(y, "language"), format!("The base changed from {} to {}.", s(x, "name"), s(y, "name")));
            } else if x["version"] != y["version"] {
                push(
                    &mut out,
                    "Inheritance",
                    "changed",
                    s(y, "language"),
                    format!("The base {} moved from v{} to v{}.", s(y, "name"), x["version"].as_i64().unwrap_or(0), y["version"].as_i64().unwrap_or(0)),
                );
            }
            let (n, m) = (arr(x, "overrides").len(), arr(y, "overrides").len());
            if n != m {
                push(&mut out, "Inheritance", "changed", "overrides", format!("The language changes {m} thing{} of its base (before: {n}).", if m == 1 { "" } else { "s" }));
            }
        }
        _ => {}
    }

    rules(&mut out, "Principles", arr(a, "principles"), arr(b, "principles"), "Principle");

    let (gone, new, both) = pair(arr(a, "definitions"), arr(b, "definitions"), "term");
    for d in new {
        push(&mut out, "Definitions", "added", s(d, "term"), format!("Definition of {} added: {}", s(d, "term"), quote(s(d, "meaning"))));
    }
    for d in gone {
        push(&mut out, "Definitions", "removed", s(d, "term"), format!("Definition of {} removed.", s(d, "term")));
    }
    for (x, y) in both {
        if s(x, "meaning").trim() != s(y, "meaning").trim() {
            push(&mut out, "Definitions", "changed", s(y, "term"), format!("{} now means: {}", s(y, "term"), quote(s(y, "meaning"))));
        }
    }

    // Families
    let (gone, new, both) = pair(arr(a, "families"), arr(b, "families"), "id");
    for f in new {
        push(
            &mut out,
            "Families",
            "added",
            s(f, "id"),
            format!("Family {} (`{}`) added, for {}: {} measures, {} rules.", s(f, "name"), s(f, "id"), s(f, "applies_to").trim(), arr(f, "measures").len(), arr(f, "rules").len()),
        );
    }
    for f in gone {
        push(&mut out, "Families", "removed", s(f, "id"), format!("Family {} (`{}`) removed.", s(f, "name"), s(f, "id")));
    }
    for (x, y) in both {
        let section = format!("Family: {}", s(y, "name"));
        let mut parts = field_changes(x, y, &["name", "applies_to", "intent"]);
        parts.extend(evidence_change(x, y, "items", "references"));
        if !parts.is_empty() {
            push(&mut out, &section, "changed", s(y, "id"), parts.join("; "));
        }
        let (mgone, mnew, mboth) = pair(arr(x, "measures"), arr(y, "measures"), "name");
        for m in mnew {
            push(&mut out, &section, "added", s(m, "name"), format!("{} added: {} ({}).", s(m, "name"), spec::fmt_value(m), s(m, "basis")));
        }
        for m in mgone {
            push(&mut out, &section, "removed", s(m, "name"), format!("{} removed (it was {}).", s(m, "name"), spec::fmt_value(m)));
        }
        for (p, q) in mboth {
            let mut parts = Vec::new();
            if p.get("value") != q.get("value") || s(p, "unit") != s(q, "unit") {
                parts.push(format!("{} → {}", spec::fmt_value(p), spec::fmt_value(q)));
            }
            if s(p, "basis") != s(q, "basis") {
                parts.push(format!("basis {} → {}", s(p, "basis"), s(q, "basis")));
            }
            if s(p, "of").trim() != s(q, "of").trim() {
                parts.push(format!("now measures {}", quote(s(q, "of"))));
            }
            let conf = |m: &Value| m.get("confidence").and_then(|v| v.as_f64()).map(|c| (c * 100.0).round() as i64);
            if conf(p) != conf(q) {
                parts.push(format!("confidence {} % → {} %", conf(p).unwrap_or(0), conf(q).unwrap_or(0)));
            }
            parts.extend(evidence_change(p, q, "evidence", "evidence"));
            if !parts.is_empty() {
                push(&mut out, &section, "changed", s(q, "name"), format!("{}: {}", s(q, "name"), parts.join(", ")));
            }
        }
        rules(&mut out, &section, arr(x, "rules"), arr(y, "rules"), "Rule");
    }

    rules(&mut out, "Exceptions", arr(a, "exceptions"), arr(b, "exceptions"), "Exception");

    // Product colours: by ID when both sides have IDs, else by role and colour.
    let colours = |t: &'_ Value| -> Vec<Value> { t.pointer("/cmf/object").and_then(|v| v.as_array()).cloned().unwrap_or_default() };
    let (ca, cb) = (colours(a), colours(b));
    let label = |c: &Value| format!("{}{} {}", if s(c, "id").is_empty() { String::new() } else { format!("{} ", s(c, "id")) }, s(c, "role"), s(c, "hex"));
    let field = if ca.iter().chain(cb.iter()).all(|c| !s(c, "id").is_empty()) { "id" } else { "hex" };
    let (gone, new, both) = pair(&ca, &cb, field);
    for c in new {
        push(&mut out, "Materials and CMF", "added", s(c, field), format!("Colour {} added{}.", label(c), if s(c, "material").is_empty() { String::new() } else { format!(" ({})", s(c, "material")) }));
    }
    for c in gone {
        push(&mut out, "Materials and CMF", "removed", s(c, field), format!("Colour {} removed.", label(c)));
    }
    for (x, y) in both {
        let mut parts = field_changes(x, y, &["hex", "role", "material", "finish", "ral", "pantone_approx", "scope", "basis"]);
        parts.extend(evidence_change(x, y, "evidence", "evidence"));
        if !parts.is_empty() {
            push(&mut out, "Materials and CMF", "changed", s(y, field), format!("Colour {}: {}", label(y), parts.join("; ")));
        }
    }
    let pres = |t: &Value| -> Vec<String> {
        t.pointer("/cmf/presentation").and_then(|v| v.as_array()).map(|p| p.iter().map(|c| format!("{} {}", s(c, "role"), s(c, "hex"))).collect()).unwrap_or_default()
    };
    let (pa, pb) = (pres(a), pres(b));
    for c in pb.iter().filter(|c| !pa.contains(c)) {
        push(&mut out, "Presentation colours", "added", c, format!("Presentation colour {c} added."));
    }
    for c in pa.iter().filter(|c| !pb.contains(c)) {
        push(&mut out, "Presentation colours", "removed", c, format!("Presentation colour {c} removed."));
    }

    // Prompts
    for k in ["positive", "negative"] {
        let (x, y) = (a.pointer(&format!("/prompts/{k}")).and_then(|v| v.as_str()).unwrap_or(""), b.pointer(&format!("/prompts/{k}")).and_then(|v| v.as_str()).unwrap_or(""));
        if x.trim() != y.trim() {
            push(&mut out, "Image-generation tokens", "changed", k, format!("The {k} prompt is now: {}", quote(y)));
        }
    }
    let fam = |t: &Value| t.pointer("/prompts/families").and_then(|v| v.as_object()).cloned().unwrap_or_default();
    let (fa, fb) = (fam(a), fam(b));
    for (k, v) in &fb {
        match fa.get(k) {
            None => push(&mut out, "Image-generation tokens", "added", k, format!("Prompt for family {k} added: {}", quote(v.as_str().unwrap_or("")))),
            Some(old) if old != v => push(&mut out, "Image-generation tokens", "changed", k, format!("The prompt for family {k} is now: {}", quote(v.as_str().unwrap_or("")))),
            _ => {}
        }
    }
    for k in fa.keys().filter(|k| !fb.contains_key(*k)) {
        push(&mut out, "Image-generation tokens", "removed", k, format!("Prompt for family {k} removed."));
    }

    // Do and do not, by text.
    for (k, word) in [("do", "Do"), ("dont", "Do not")] {
        let list = |t: &Value| -> Vec<String> { arr(t, k).iter().map(|d| s(d, "text").trim().to_string()).collect() };
        let (x, y) = (list(a), list(b));
        for t in y.iter().filter(|t| !x.contains(t)) {
            push(&mut out, "Do and do not", "added", word, format!("{word} added: {}", quote(t)));
        }
        for t in x.iter().filter(|t| !y.contains(t)) {
            push(&mut out, "Do and do not", "removed", word, format!("{word} removed: {}", quote(t)));
        }
    }

    for (k, section, field) in [("anti_references", "Anti-references", "lesson"), ("key_references", "Key references", "why")] {
        let short = |v: &Value| s(v, "item").chars().take(8).collect::<String>();
        let (x, y) = (arr(a, k), arr(b, k));
        for r in y.iter().filter(|r| !x.iter().any(|o| short(o) == short(r))) {
            push(&mut out, section, "added", &short(r), format!("`{}` added: {}", short(r), quote(s(r, field))));
        }
        for r in x.iter().filter(|r| !y.iter().any(|o| short(o) == short(r))) {
            push(&mut out, section, "removed", &short(r), format!("`{}` removed.", short(r)));
        }
        for r in y {
            if let Some(o) = x.iter().find(|o| short(o) == short(r)) {
                if s(o, field).trim() != s(r, field).trim() {
                    push(&mut out, section, "changed", &short(r), format!("`{}` now: {}", short(r), quote(s(r, field))));
                }
            }
        }
    }
    out
}

/// The diff between two versions of a language.
pub fn versions(lib: &Library, language_key: &str, from: i64, to: i64) -> Result<VersionDiff> {
    let lang = lib.find_language(language_key)?;
    let (a, b) = (lib.read_version(&lang.id, from)?, lib.read_version(&lang.id, to)?);
    let structured = a.info.schema >= 2 && b.info.schema >= 2;
    Ok(VersionDiff {
        language: lang.name,
        from,
        to,
        structured,
        changes: if structured { semantic(&a.tokens, &b.tokens) } else { Vec::new() },
    })
}

/// The diff as text for an agent.
pub fn diff_text(d: &VersionDiff) -> String {
    if !d.structured {
        return format!(
            "{} v{} or v{} uses the older format, which has no structure to compare. Read both versions with get_language.",
            d.language, d.from, d.to
        );
    }
    if d.changes.is_empty() {
        return format!("{}: v{} and v{} have the same content.", d.language, d.from, d.to);
    }
    let mut out = format!("CHANGES in {} from v{} to v{}: {} change{}.\n", d.language, d.from, d.to, d.changes.len(), if d.changes.len() == 1 { "" } else { "s" });
    let mut section = "";
    for c in &d.changes {
        if c.section != section {
            out.push_str(&format!("\n## {}\n", c.section));
            section = &c.section;
        }
        out.push_str(&format!("- [{}] {}\n", c.kind, c.text));
    }
    out
}

impl Library {
    /// Rolls a language back: saves an earlier version again as the newest
    /// version. The versions in between stay in the history.
    pub fn restore_version(&self, language_id: &str, version: i64) -> Result<i64> {
        let latest = self.latest_version(language_id)?.ok_or_else(|| anyhow!("The language has no version."))?;
        if version == latest {
            bail!("Version {version} is the latest version already.");
        }
        let old = self.read_version(language_id, version)?;
        let label = format!("restored from v{version}");
        if old.info.schema >= 2 {
            let mut tokens = old.tokens.clone();
            if let Some(o) = tokens.as_object_mut() {
                // A restored version of a language without a base must not claim one.
                if self.get_language(language_id)?.base_id.is_none() {
                    o.remove("extends");
                }
            }
            return self.save_manual_spec(language_id, &tokens, &label);
        }
        let ids: Vec<String> = self.list_items(language_id)?.into_iter().map(|i| i.id).collect();
        let hash = self.latest_input_hash(language_id)?.unwrap_or_default();
        self.save_version(language_id, &old.markdown, &old.tokens, &label, &ids, true, &hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn changes_in_words() {
        let a = crate::spec::tests::sample();
        assert!(semantic(&a, &a).is_empty());
        let mut b = a.clone();
        b["families"][1]["measures"][0]["value"] = json!([9, 11]);
        b["families"][1]["measures"][0]["evidence"] = json!(["bbbbbbbb", "dddddddd"]);
        b["families"][0]["measures"].as_array_mut().unwrap().push(json!({"name": "seam offset", "value": 4, "unit": "mm", "of": "x", "basis": "preferred", "confidence": 0.5, "evidence": []}));
        b["principles"][0]["text"] = json!("Keep the mass continuous.");
        b["principles"][1]["priority"] = json!(3);
        b["principles"].as_array_mut().unwrap().push(json!({"id": "P3", "text": "Soft light only.", "priority": 4, "basis": "designer", "evidence": []}));
        b["cmf"]["object"][0]["hex"] = json!("#E0DCD2");
        b["dont"] = json!([{"text": "Chrome.", "scope": "global"}]);
        b["exceptions"] = json!([]);
        b["prompts"]["negative"] = json!("chrome");
        b["families"].as_array_mut().unwrap().push(json!({"id": "grip", "name": "Grip", "applies_to": "handles", "measures": [], "rules": []}));
        let c = semantic(&a, &b);
        let has = |needle: &str| c.iter().any(|x| x.text.contains(needle));
        assert!(has("blade pitch: 8–10 mm → 9–11 mm, evidence added `dddddddd`"), "{c:#?}");
        assert!(has("seam offset added: 4 mm (preferred)."));
        assert!(has("Principle P1: text: “Keep the mass continuous; no visible frame.” → “Keep the mass continuous.”"));
        assert!(has("Principle P2: priority: 2 → 3"));
        assert!(has("Principle P3 added"));
        assert!(has("Colour C1 body #E0DCD2: hex: “#E8E4DC” → “#E0DCD2”"));
        assert!(has("Do not added: “Chrome.”") && has("Do not removed: “Visible fasteners.”"));
        assert!(has("Exception X1 removed"));
        assert!(has("The negative prompt is now: “chrome”"));
        assert!(has("Family Grip (`grip`) added, for handles"));
        assert_eq!(c.iter().filter(|x| x.section == "Family: Blade field").count(), 1);
    }

    #[test]
    fn version_diff_and_restore() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path()).unwrap();
        let lang = lib.create_language("Organic", "").unwrap();
        let v1 = crate::spec::tests::sample();
        lib.save_spec_version(&lang.id, &v1, "m", &[], false, "h1", &[]).unwrap();
        let mut v2 = v1.clone();
        v2["families"][1]["measures"][0]["value"] = json!(12);
        lib.save_spec_version(&lang.id, &v2, "m", &[], false, "h2", &[]).unwrap();

        let d = versions(&lib, "organic", 1, 2).unwrap();
        assert!(d.structured);
        assert_eq!(d.changes.len(), 1);
        assert!(diff_text(&d).contains("blade pitch: 8–10 mm → 12 mm"), "{}", diff_text(&d));
        assert!(diff_text(&versions(&lib, "organic", 2, 2).unwrap()).contains("same content"));
        assert!(versions(&lib, "organic", 1, 9).is_err());

        // Rollback: v1 again as v3; v2 stays in the history.
        assert!(lib.restore_version(&lang.id, 2).is_err());
        let v3 = lib.restore_version(&lang.id, 1).unwrap();
        assert_eq!(v3, 3);
        let c = lib.read_version(&lang.id, 3).unwrap();
        assert_eq!(c.tokens["families"][1]["measures"][0]["value"], json!([8, 10]));
        assert_eq!((c.info.model.as_str(), c.info.edited), ("restored from v1", true));
        assert!(versions(&lib, "organic", 1, 3).unwrap().changes.is_empty());
        assert_eq!(lib.list_versions(&lang.id).unwrap().len(), 3);
    }
}
