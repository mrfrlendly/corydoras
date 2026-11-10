//! Inheritance: a language can extend a base language and change a few rules.
//!
//! How it works:
//! - A language with a base has no synthesis of its own. Its document is the
//!   base document with the language's overrides applied.
//! - The overrides are a JSON Patch on the base spec (the same patch form as an
//!   amendment proposal: add, remove and replace, with array elements named by
//!   their id, name or term).
//! - The result is saved as a normal version of the language. Its tokens carry
//!   "extends": the base, the base version and each override with the value it
//!   replaced. So every reader (the app, a project pin, the MCP server) gets a
//!   complete, self-contained spec and can see where each change comes from.
//! - When the base gets a new version and the language follows the latest, the
//!   app builds a new version of the language. An override that no longer fits
//!   the base (its path is gone) is skipped and listed as a review note.
//! - An edit of the document (by hand or from an accepted proposal) becomes
//!   overrides: the difference between the base and the edited spec.

use crate::library::Library;
use crate::models::Language;
use crate::proposals::{apply_patch, Change};
use crate::spec;
use crate::util::fnv1a64;
use anyhow::{anyhow, bail, Result};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};

/// Top-level fields that are not part of the design content.
const APP_FIELDS: &[&str] = &["schema", "language", "version", "extends"];
/// Deepest chain of bases.
const MAX_DEPTH: usize = 6;

/// One override, as the app shows it.
#[derive(Serialize, Clone, Debug)]
pub struct OverrideView {
    pub op: String,
    pub path: String,
    /// The value of the base at the path (null for an added value).
    pub before: Value,
    pub after: Value,
    /// Why the override does not apply to the base now, if it does not.
    pub problem: Option<String>,
}

/// What a language inherits, for the app.
#[derive(Serialize, Clone, Debug)]
pub struct Inheritance {
    pub base_id: String,
    pub base_name: String,
    /// The base version that the language follows. None: the latest.
    pub pinned_version: Option<i64>,
    /// The base version that the latest version of the language was built from.
    pub built_from: Option<i64>,
    pub base_latest: Option<i64>,
    pub overrides: Vec<OverrideView>,
    /// The last build failed with this message.
    pub error: Option<String>,
}

fn esc(seg: &str) -> String {
    seg.replace('~', "~0").replace('/', "~1")
}

/// The field that names the elements of an array of objects: "id", "name" or
/// "term", when every element has it and no value repeats.
fn key_of(arr: &[Value]) -> Option<&'static str> {
    if arr.is_empty() {
        return None;
    }
    for k in ["id", "name", "term"] {
        let vals: Vec<String> = arr.iter().filter_map(|e| e.get(k).and_then(|v| v.as_str()).map(|s| s.trim().to_lowercase())).collect();
        if vals.len() == arr.len() && vals.iter().all(|v| !v.is_empty() && v.parse::<usize>().is_err()) {
            let mut sorted = vals.clone();
            sorted.sort();
            sorted.dedup();
            if sorted.len() == vals.len() {
                return Some(k);
            }
        }
    }
    None
}

fn diff_into(base: &Value, target: &Value, path: &str, out: &mut Vec<Value>) {
    if base == target {
        return;
    }
    match (base, target) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, av) in a {
                let p = format!("{path}/{}", esc(k));
                match b.get(k) {
                    Some(bv) => diff_into(av, bv, &p, out),
                    None => out.push(json!({"op": "remove", "path": p})),
                }
            }
            for (k, bv) in b {
                if !a.contains_key(k) {
                    out.push(json!({"op": "add", "path": format!("{path}/{}", esc(k)), "value": bv}));
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            let key = match (key_of(a), key_of(b)) {
                (Some(x), Some(y)) if x == y => Some(x),
                (Some(x), None) if b.is_empty() => Some(x),
                (None, Some(y)) if a.is_empty() => Some(y),
                _ => None,
            };
            let Some(key) = key else {
                out.push(json!({"op": "replace", "path": path, "value": target}));
                return;
            };
            let name = |e: &Value| e.get(key).and_then(|v| v.as_str()).map(|s| s.trim().to_lowercase()).unwrap_or_default();
            for ea in a {
                let p = format!("{path}/{}", esc(ea.get(key).and_then(|v| v.as_str()).unwrap_or("").trim()));
                match b.iter().find(|eb| name(eb) == name(ea)) {
                    Some(eb) => diff_into(ea, eb, &p, out),
                    None => out.push(json!({"op": "remove", "path": p})),
                }
            }
            for eb in b {
                if !a.iter().any(|ea| name(ea) == name(eb)) {
                    out.push(json!({"op": "add", "path": format!("{path}/-"), "value": eb}));
                }
            }
        }
        _ => out.push(json!({"op": "replace", "path": path, "value": target})),
    }
}

/// The patch that turns `base` into `target`, in the form that proposals use.
/// Elements of arrays are named by their id, name or term, so the patch still
/// fits when the base gets more elements. The fields that the app owns are left out.
pub fn diff_patch(base: &Value, target: &Value) -> Vec<Value> {
    let strip = |v: &Value| {
        let mut v = v.clone();
        if let Some(o) = v.as_object_mut() {
            for k in APP_FIELDS {
                o.remove(*k);
            }
        }
        v
    };
    let mut out = Vec::new();
    diff_into(&strip(base), &strip(target), "", &mut out);
    out
}

/// Applies the overrides one at a time. An override that does not fit is
/// skipped; the second list says why.
fn apply_overrides(base: &Value, overrides: &[Value]) -> (Value, Vec<Change>, Vec<(usize, String)>) {
    let mut doc = base.clone();
    let mut changes = Vec::new();
    let mut skipped = Vec::new();
    for (i, op) in overrides.iter().enumerate() {
        match apply_patch(&doc, &json!([op])) {
            Ok((next, mut c)) => {
                doc = next;
                changes.append(&mut c);
            }
            Err(e) => skipped.push((i, e.to_string().replace("Operation 1: ", ""))),
        }
    }
    (doc, changes, skipped)
}

impl Library {
    fn overrides_of(&self, language_id: &str) -> Result<Vec<Value>> {
        let text: String = self.c().query_row("SELECT overrides FROM languages WHERE id = ?1", [language_id], |r| r.get(0))?;
        Ok(serde_json::from_str::<Vec<Value>>(&text).unwrap_or_default())
    }

    fn set_inherit_error(&self, language_id: &str, error: Option<&str>) -> Result<()> {
        let key = format!("inherit_error.{language_id}");
        match error {
            Some(e) => self.c().execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, e],
            )?,
            None => self.c().execute("DELETE FROM settings WHERE key = ?1", [key])?,
        };
        Ok(())
    }

    /// The language and its bases, nearest first. Evidence IDs of a derived
    /// language can name items of any of them.
    pub fn base_chain(&self, language_id: &str) -> Result<Vec<Language>> {
        let all = self.list_languages()?;
        let mut out: Vec<Language> = Vec::new();
        let mut next = Some(language_id.to_string());
        while let Some(id) = next {
            if out.iter().any(|l| l.id == id) || out.len() > MAX_DEPTH {
                break;
            }
            let Some(l) = all.iter().find(|l| l.id == id) else { break };
            next = l.base_id.clone();
            out.push(l.clone());
        }
        Ok(out)
    }

    /// The base version that a derived language is built from now.
    fn base_content(&self, lang: &Language) -> Result<(Language, crate::models::VersionContent)> {
        let base_id = lang.base_id.as_deref().ok_or_else(|| anyhow!("\"{}\" extends no language.", lang.name))?;
        let base = self.get_language(base_id)?;
        let content = match lang.base_version {
            Some(v) => self.read_version(&base.id, v)?,
            None => self
                .read_latest(&base.id)?
                .ok_or_else(|| anyhow!("\"{}\" has no version yet, so \"{}\" cannot extend it.", base.name, lang.name))?,
        };
        if content.info.schema < 2 {
            bail!("Version {} of \"{}\" uses the older format. Synthesize a new version of it first.", content.info.version, base.name);
        }
        Ok((base, content))
    }

    /// Sets or removes the base of a language. `base` is a language ID or name;
    /// `version` pins a base version (None follows the latest). A new base
    /// starts without overrides. Removing the base keeps the versions.
    pub fn set_base(&self, language_id: &str, base: Option<&str>, version: Option<i64>) -> Result<Language> {
        let lang = self.get_language(language_id)?;
        let Some(key) = base.map(str::trim).filter(|b| !b.is_empty()) else {
            self.c().execute("UPDATE languages SET base_id = NULL, base_version = NULL, overrides = '[]' WHERE id = ?1", [language_id])?;
            self.set_inherit_error(language_id, None)?;
            // The mood board writes the document again.
            if lang.base_id.is_some() && lang.item_count > 0 {
                self.enqueue_synth(language_id)?;
            }
            return self.get_language(language_id);
        };
        let b = self.find_language(key)?;
        if b.id == lang.id {
            bail!("A language cannot extend itself.");
        }
        if self.base_chain(&b.id)?.iter().any(|l| l.id == lang.id) {
            bail!("\"{}\" extends \"{}\" already, so \"{}\" cannot extend it.", b.name, lang.name, lang.name);
        }
        if self.base_chain(&b.id)?.len() >= MAX_DEPTH {
            bail!("The chain of base languages is too long.");
        }
        let versions = self.list_versions(&b.id)?;
        match version {
            Some(v) => {
                let info = versions.iter().find(|x| x.version == v).ok_or_else(|| anyhow!("\"{}\" has no version {v}.", b.name))?;
                if info.schema < 2 {
                    bail!("Version {v} of \"{}\" uses the older format.", b.name);
                }
            }
            None => {
                let latest = versions.first().ok_or_else(|| anyhow!("\"{}\" has no version yet. A base needs a document.", b.name))?;
                if latest.schema < 2 {
                    bail!("The latest version of \"{}\" uses the older format. Synthesize a new version of it first.", b.name);
                }
            }
        }
        let same_base = lang.base_id.as_deref() == Some(b.id.as_str());
        self.c().execute(
            &format!("UPDATE languages SET base_id = ?2, base_version = ?3{} WHERE id = ?1", if same_base { "" } else { ", overrides = '[]'" }),
            params![language_id, b.id, version],
        )?;
        // A language with a base has no synthesis of its own.
        self.c().execute("DELETE FROM jobs WHERE kind = 'synthesize' AND target = ?1", [language_id])?;
        self.rebuild_derived(language_id)?;
        self.get_language(language_id)
    }

    /// Builds the document of a derived language from its base and its
    /// overrides, and saves it as a new version when it changed. Returns the
    /// new version number, or None when nothing changed.
    pub fn rebuild_derived(&self, language_id: &str) -> Result<Option<i64>> {
        let result = self.rebuild_inner(language_id);
        self.set_inherit_error(language_id, result.as_ref().err().map(|e| e.to_string()).as_deref())?;
        result
    }

    fn rebuild_inner(&self, language_id: &str) -> Result<Option<i64>> {
        let lang = self.get_language(language_id)?;
        let (base, content) = self.base_content(&lang)?;
        let overrides = self.overrides_of(language_id)?;
        let hash = fnv1a64(serde_json::to_string(&json!(["derived", base.id, content.info.version, overrides]))?.as_bytes());
        if self.latest_input_hash(language_id)?.as_deref() == Some(hash.as_str()) {
            return Ok(None);
        }
        let mut base_doc = content.tokens.clone();
        if let Some(o) = base_doc.as_object_mut() {
            for k in APP_FIELDS {
                o.remove(*k);
            }
        }
        let (mut doc, changes, skipped) = apply_overrides(&base_doc, &overrides);
        if let Some(o) = doc.as_object_mut() {
            o.insert(
                "extends".into(),
                json!({
                    "language": base.id,
                    "name": base.name,
                    "version": content.info.version,
                    "overrides": changes.iter().map(|c| json!({"op": c.op, "path": c.path, "before": c.before, "after": c.after})).collect::<Vec<_>>(),
                }),
            );
        }
        let check = spec::validate(&doc, &self.known_evidence_ids(language_id)?);
        if !check.errors.is_empty() {
            bail!("The overrides of \"{}\" do not give a valid document on {} v{}: {}", lang.name, base.name, content.info.version, check.errors.join(" "));
        }
        let mut warnings: Vec<String> = skipped
            .iter()
            .map(|(i, why)| {
                let op = &overrides[*i];
                format!(
                    "Override {} ({} {}) does not fit {} v{} and was skipped: {why}",
                    i + 1,
                    op.get("op").and_then(|v| v.as_str()).unwrap_or("?"),
                    op.get("path").and_then(|v| v.as_str()).unwrap_or("?"),
                    base.name,
                    content.info.version
                )
            })
            .collect();
        warnings.extend(check.warnings);
        let label = format!(
            "{} v{} + {} change{}",
            base.name,
            content.info.version,
            changes.len(),
            if changes.len() == 1 { "" } else { "s" }
        );
        let ids: Vec<String> = self.list_items(language_id)?.into_iter().map(|i| i.id).collect();
        let v = self.save_spec_version(language_id, &doc, &label, &ids, true, &hash, &warnings)?;
        Ok(Some(v))
    }

    /// After a language got a new version: builds the languages that extend it
    /// and follow its latest version.
    pub(crate) fn rebuild_children(&self, base_id: &str) -> Result<()> {
        let children: Vec<String> = {
            let c = self.c();
            let mut st = c.prepare("SELECT id FROM languages WHERE base_id = ?1 AND base_version IS NULL")?;
            let rows = st.query_map([base_id], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            rows
        };
        for id in children {
            // A failure is kept with the language and shown in the app; it must
            // not stop the version of the base.
            let _ = self.rebuild_derived(&id);
        }
        Ok(())
    }

    /// Saves an edited spec of a derived language: the difference to the base
    /// becomes the overrides, and the document is built again.
    pub(crate) fn save_derived_edit(&self, language_id: &str, tokens: &Value) -> Result<i64> {
        let lang = self.get_language(language_id)?;
        let (base, content) = self.base_content(&lang)?;
        let check = spec::validate(tokens, &self.known_evidence_ids(language_id)?);
        if !check.errors.is_empty() {
            bail!("tokens.json: {}", check.errors.join(" "));
        }
        let patch = diff_patch(&content.tokens, tokens);
        let before = self.overrides_of(language_id)?;
        self.c().execute("UPDATE languages SET overrides = ?2 WHERE id = ?1", params![language_id, serde_json::to_string(&patch)?])?;
        match self.rebuild_derived(language_id) {
            Ok(Some(v)) => Ok(v),
            Ok(None) => bail!("The edit changes nothing: the document is the same as {} v{} with the overrides that it has.", base.name, content.info.version),
            Err(e) => {
                self.c().execute("UPDATE languages SET overrides = ?2 WHERE id = ?1", params![language_id, serde_json::to_string(&before)?])?;
                let _ = self.set_inherit_error(language_id, None);
                Err(e)
            }
        }
    }

    /// Removes one override (by its place in the list) and builds the document again.
    pub fn remove_override(&self, language_id: &str, index: usize) -> Result<Option<i64>> {
        let mut list = self.overrides_of(language_id)?;
        if index >= list.len() {
            bail!("This language has no override {}.", index + 1);
        }
        list.remove(index);
        self.c().execute("UPDATE languages SET overrides = ?2 WHERE id = ?1", params![language_id, serde_json::to_string(&list)?])?;
        self.rebuild_derived(language_id)
    }

    /// What a language inherits and changes. None for a language without a base.
    pub fn inheritance(&self, language_id: &str) -> Result<Option<Inheritance>> {
        let lang = self.get_language(language_id)?;
        let Some(base_id) = lang.base_id.clone() else { return Ok(None) };
        let base = self.get_language(&base_id)?;
        let overrides = self.overrides_of(language_id)?;
        let mut views = Vec::new();
        if let Ok((_, content)) = self.base_content(&lang) {
            let mut doc = content.tokens.clone();
            for op in &overrides {
                let (o, p) = (op.get("op").and_then(|v| v.as_str()).unwrap_or("").to_string(), op.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string());
                match apply_patch(&doc, &json!([op])) {
                    Ok((next, c)) => {
                        doc = next;
                        let c = c.into_iter().next();
                        views.push(OverrideView {
                            op: o,
                            path: p,
                            before: c.as_ref().map(|x| x.before.clone()).unwrap_or(Value::Null),
                            after: c.map(|x| x.after).unwrap_or(Value::Null),
                            problem: None,
                        });
                    }
                    Err(e) => views.push(OverrideView {
                        op: o,
                        path: p,
                        before: Value::Null,
                        after: op.get("value").cloned().unwrap_or(Value::Null),
                        problem: Some(e.to_string().replace("Operation 1: ", "")),
                    }),
                }
            }
        }
        let built_from = self
            .read_latest(language_id)?
            .and_then(|c| c.tokens.pointer("/extends/version").and_then(|v| v.as_i64()));
        let error: Option<String> = self
            .c()
            .query_row("SELECT value FROM settings WHERE key = ?1", [format!("inherit_error.{language_id}")], |r| r.get(0))
            .optional()?;
        Ok(Some(Inheritance {
            base_id,
            base_name: base.name,
            pinned_version: lang.base_version,
            built_from,
            base_latest: base.latest_version,
            overrides: views,
            error,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        crate::spec::tests::sample()
    }

    #[test]
    fn patch_from_a_difference() {
        let base = sample();
        let mut target = base.clone();
        target["families"][1]["measures"][0]["value"] = json!([9, 11]);
        target["principles"].as_array_mut().unwrap().push(json!({"id": "P3", "text": "Soft light only.", "priority": 3, "basis": "designer", "evidence": []}));
        target["families"][0]["rules"].as_array_mut().unwrap().remove(0);
        target["prompts"]["negative"] = json!("visible frame");
        target["dont"] = json!([]);
        target["version"] = json!(99);
        let patch = diff_patch(&base, &target);
        let paths: Vec<String> = patch.iter().map(|o| format!("{} {}", o["op"].as_str().unwrap(), o["path"].as_str().unwrap())).collect();
        assert!(paths.contains(&"replace /families/blade-field/measures/blade pitch/value".to_string()), "{paths:?}");
        assert!(paths.contains(&"add /principles/-".to_string()));
        assert!(paths.contains(&"remove /families/shell/rules/S1".to_string()));
        assert!(paths.contains(&"replace /prompts/negative".to_string()));
        assert!(paths.contains(&"replace /dont".to_string()));
        assert_eq!(patch.len(), 5, "{paths:?}");
        // The patch gives the target back (without the fields of the app).
        let (out, _) = apply_patch(&base, &Value::Array(patch)).unwrap();
        let mut want = target.clone();
        want.as_object_mut().unwrap().remove("version");
        assert_eq!(out, want);
        assert!(diff_patch(&base, &base).is_empty());
    }

    #[test]
    fn derived_language_follows_its_base() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path()).unwrap();
        let base = lib.create_language("Wall", "").unwrap();
        let child = lib.create_language("Wall PETG", "").unwrap();
        // A base needs a document.
        assert!(lib.set_base(&child.id, Some("wall"), None).is_err());
        lib.save_spec_version(&base.id, &sample(), "m", &[], false, "h1", &[]).unwrap();
        assert!(lib.set_base(&base.id, Some("wall"), None).is_err());

        // The first version of the child is the base document.
        let c = lib.set_base(&child.id, Some("Wall"), None).unwrap();
        assert_eq!((c.base_id.as_deref(), c.latest_version, c.override_count), (Some("wall"), Some(1), 0));
        assert!(lib.set_base(&base.id, Some(&child.id), None).unwrap_err().to_string().contains("already"));
        let v1 = lib.read_version(&child.id, 1).unwrap();
        assert_eq!(v1.tokens["extends"]["version"], 1);
        assert_eq!(v1.tokens["language"], "wall-petg");
        assert!(v1.markdown.contains("## Inheritance") && v1.markdown.contains("Wall v1"), "{}", v1.markdown);
        // No synthesis for a derived language.
        assert!(lib.synthesize_now(&child.id).is_err());

        // An edit becomes overrides.
        let mut edited = v1.tokens.clone();
        edited["families"][1]["measures"][0]["value"] = json!([9, 11]);
        let v2 = lib.save_edited_version(&child.id, "", &edited.to_string()).unwrap();
        assert_eq!(v2, 2);
        let inh = lib.inheritance(&child.id).unwrap().unwrap();
        assert_eq!(inh.overrides.len(), 1);
        assert_eq!((inh.overrides[0].path.as_str(), &inh.overrides[0].before), ("/families/blade-field/measures/blade pitch/value", &json!([8, 10])));
        assert_eq!(lib.get_language(&child.id).unwrap().override_count, 1);
        assert_eq!(lib.read_version(&child.id, 2).unwrap().tokens["extends"]["overrides"][0]["after"], json!([9, 11]));
        // The same edit again changes nothing.
        assert!(lib.save_edited_version(&child.id, "", &edited.to_string()).is_err());

        // A new base version flows into the child; the override stays.
        let mut next = sample();
        next["summary"] = json!("A new summary of the base.");
        lib.save_spec_version(&base.id, &next, "m", &[], false, "h2", &[]).unwrap();
        let c3 = lib.read_latest(&child.id).unwrap().unwrap();
        assert_eq!(c3.info.version, 3);
        assert_eq!(c3.tokens["summary"], "A new summary of the base.");
        assert_eq!(c3.tokens["families"][1]["measures"][0]["value"], json!([9, 11]));
        assert_eq!(c3.tokens["extends"]["version"], 2);
        assert!(c3.info.model.contains("Wall v2 + 1 change"), "{}", c3.info.model);

        // The base drops the measure: the override is skipped with a review note.
        let mut gone = next.clone();
        gone["families"][1]["measures"] = json!([]);
        lib.save_spec_version(&base.id, &gone, "m", &[], false, "h3", &[]).unwrap();
        let c4 = lib.read_latest(&child.id).unwrap().unwrap();
        assert_eq!(c4.info.version, 4);
        assert!(c4.info.warnings.iter().any(|w| w.contains("Override 1") && w.contains("skipped")), "{:?}", c4.info.warnings);
        assert!(lib.inheritance(&child.id).unwrap().unwrap().overrides[0].problem.is_some());

        // A pinned base version does not follow.
        lib.set_base(&child.id, Some("wall"), Some(2)).unwrap();
        let pinned = lib.read_latest(&child.id).unwrap().unwrap();
        assert_eq!(pinned.tokens["extends"]["version"], 2);
        assert_eq!(pinned.tokens["families"][1]["measures"][0]["value"], json!([9, 11]));
        let n = pinned.info.version;
        lib.save_spec_version(&base.id, &sample(), "m", &[], false, "h4", &[]).unwrap();
        assert_eq!(lib.latest_version(&child.id).unwrap(), Some(n));
        assert_eq!(lib.remove_override(&child.id, 0).unwrap(), Some(n + 1));
        assert!(lib.remove_override(&child.id, 0).is_err());

        // Without a base, the versions stay and the language is its own again.
        let free = lib.set_base(&child.id, None, None).unwrap();
        assert_eq!((free.base_id, free.latest_version), (None, Some(n + 1)));
        assert!(lib.inheritance(&child.id).unwrap().is_none());
    }
}
