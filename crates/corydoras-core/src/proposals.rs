//! Amendment proposals: changes to a language spec that an agent suggests through
//! the MCP server. A proposal never changes a language by itself. The designer
//! accepts it in the app (it then becomes a new edited version, after the same
//! validation as a manual edit) or rejects it.
//!
//! A proposal is a JSON Patch (RFC 6902) with the operations add, remove and
//! replace on tokens.json. A path segment into an array can be an index, "-"
//! (append, for add) or the "id", "name" or "term" of an element, so agents can
//! write `/families/blade-field/measures/blade pitch/value` without counting.

use crate::library::Library;
use crate::spec;
use crate::util::now_iso;
use anyhow::{anyhow, bail, Result};
use rusqlite::{params, OptionalExtension, Row};
use serde::Serialize;
use serde_json::Value;

/// Most operations in one proposal.
pub const MAX_OPS: usize = 50;
/// Most open proposals for one language. Older ones must be decided first.
pub const MAX_PENDING: i64 = 20;
/// Top-level fields that the app owns.
const PROTECTED: &[&str] = &["schema", "language", "version", "extends"];

#[derive(Serialize, Clone, Debug)]
pub struct Change {
    pub op: String,
    pub path: String,
    /// The value at the path before the change (null for a new value).
    pub before: Value,
    /// The new value (null for remove).
    pub after: Value,
}

#[derive(Serialize, Clone, Debug)]
pub struct Proposal {
    pub id: String,
    pub language_id: String,
    pub base_version: i64,
    pub summary: String,
    pub rationale: String,
    pub author: String,
    pub patch: Value,
    /// pending, accepted or rejected
    pub status: String,
    pub created_at: String,
    pub decided_at: Option<String>,
    pub result_version: Option<i64>,
    /// The changes against the latest version (pending proposals only).
    pub changes: Vec<Change>,
    /// Why the proposal cannot be accepted now, if it cannot.
    pub problem: Option<String>,
    /// Review notes that accepting would add (pending proposals only): notes of
    /// the amended spec that the latest version does not have already.
    pub new_notes: Vec<String>,
}

// ----------------------------------------------------------------------
// Patch
// ----------------------------------------------------------------------

fn segments(path: &str) -> Result<Vec<String>> {
    if path.is_empty() {
        bail!("The path must not be empty: a proposal cannot replace the whole spec.");
    }
    if !path.starts_with('/') {
        bail!("The path \"{path}\" must start with \"/\".");
    }
    Ok(path[1..].split('/').map(|s| s.replace("~1", "/").replace("~0", "~")).collect())
}

/// Index of `seg` in an array: a number, or the element whose id, name or term is `seg`.
fn array_index(arr: &[Value], seg: &str, path: &str) -> Result<usize> {
    if let Ok(i) = seg.parse::<usize>() {
        if i < arr.len() {
            return Ok(i);
        }
        bail!("\"{path}\": index {i} is past the end (the array has {} elements).", arr.len());
    }
    let key = seg.trim().to_lowercase();
    arr.iter()
        .position(|e| {
            ["id", "name", "term"].iter().any(|k| {
                e.get(*k).and_then(|v| v.as_str()).map(|v| v.trim().to_lowercase() == key).unwrap_or(false)
            })
        })
        .ok_or_else(|| anyhow!("\"{path}\": no element with the id, name or term \"{seg}\"."))
}

fn child_mut<'a>(v: &'a mut Value, seg: &str, path: &str) -> Result<&'a mut Value> {
    match v {
        Value::Object(o) => o.get_mut(seg).ok_or_else(|| anyhow!("\"{path}\": no field \"{seg}\".")),
        Value::Array(a) => {
            let i = array_index(a, seg, path)?;
            Ok(&mut a[i])
        }
        _ => bail!("\"{path}\": \"{seg}\" is inside a value that is not an object or an array."),
    }
}

/// Applies one operation. Returns the value that was at the path.
fn apply_op(doc: &mut Value, op: &str, path: &str, value: Option<&Value>) -> Result<Value> {
    let segs = segments(path)?;
    if segs.len() == 1 && PROTECTED.contains(&segs[0].as_str()) {
        bail!("\"{path}\": the app sets this field.");
    }
    let (last, parents) = segs.split_last().expect("non-empty");
    let mut parent = &mut *doc;
    for s in parents {
        parent = child_mut(parent, s, path)?;
    }
    let need_value = || value.cloned().ok_or_else(|| anyhow!("\"{path}\": the {op} operation needs a \"value\"."));
    match (op, parent) {
        ("add", Value::Object(o)) => Ok(o.insert(last.clone(), need_value()?).unwrap_or(Value::Null)),
        ("add", Value::Array(a)) => {
            let v = need_value()?;
            if last == "-" {
                a.push(v);
            } else {
                let i: usize = last
                    .parse()
                    .map_err(|_| anyhow!("\"{path}\": add into an array needs an index or \"-\"; use replace to change an element."))?;
                if i > a.len() {
                    bail!("\"{path}\": index {i} is past the end (the array has {} elements).", a.len());
                }
                a.insert(i, v);
            }
            Ok(Value::Null)
        }
        ("replace", p) => {
            let v = need_value()?;
            let slot = child_mut(p, last, path)?;
            Ok(std::mem::replace(slot, v))
        }
        ("remove", Value::Object(o)) => o.remove(last).ok_or_else(|| anyhow!("\"{path}\": no field \"{last}\".")),
        ("remove", Value::Array(a)) => {
            let i = array_index(a, last, path)?;
            Ok(a.remove(i))
        }
        ("add" | "remove", _) => bail!("\"{path}\": the parent is not an object or an array."),
        (other, _) => bail!("\"{path}\": the operation \"{other}\" is not supported (use add, remove or replace)."),
    }
}

/// Applies a patch to a copy of `doc`. Returns the new document and the changes.
pub fn apply_patch(doc: &Value, patch: &Value) -> Result<(Value, Vec<Change>)> {
    let ops = patch.as_array().ok_or_else(|| anyhow!("The patch must be an array of operations."))?;
    if ops.is_empty() {
        bail!("The patch has no operations.");
    }
    if ops.len() > MAX_OPS {
        bail!("The patch has {} operations; the limit is {MAX_OPS}.", ops.len());
    }
    let mut out = doc.clone();
    let mut changes = Vec::new();
    for (n, o) in ops.iter().enumerate() {
        let op = o.get("op").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("Operation {}: \"op\" is missing.", n + 1))?;
        let path = o.get("path").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("Operation {}: \"path\" is missing.", n + 1))?;
        let value = o.get("value");
        let before = apply_op(&mut out, op, path, value).map_err(|e| anyhow!("Operation {}: {e}", n + 1))?;
        changes.push(Change {
            op: op.to_string(),
            path: path.to_string(),
            before,
            after: if op == "remove" { Value::Null } else { value.cloned().unwrap_or(Value::Null) },
        });
    }
    Ok((out, changes))
}

// ----------------------------------------------------------------------
// Storage
// ----------------------------------------------------------------------

const COLS: &str = "id, language_id, base_version, summary, rationale, author, patch, status, created_at, decided_at, result_version";

fn from_row(r: &Row) -> rusqlite::Result<Proposal> {
    let patch: String = r.get(6)?;
    Ok(Proposal {
        id: r.get(0)?,
        language_id: r.get(1)?,
        base_version: r.get(2)?,
        summary: r.get(3)?,
        rationale: r.get(4)?,
        author: r.get(5)?,
        patch: serde_json::from_str(&patch).unwrap_or(Value::Null),
        status: r.get(7)?,
        created_at: r.get(8)?,
        decided_at: r.get(9)?,
        result_version: r.get(10)?,
        changes: Vec::new(),
        problem: None,
        new_notes: Vec::new(),
    })
}

/// The result of a submitted proposal.
#[derive(Debug)]
pub struct Submitted {
    pub proposal: Proposal,
    /// Review notes that the amendment adds (the latest version does not have them).
    pub warnings: Vec<String>,
}

impl Library {
    /// The amended spec: the patch applied to the latest version, checked.
    /// The warnings are only the new ones: notes that the latest version has
    /// already do not count against the proposal.
    fn amended(&self, language_id: &str, patch: &Value) -> Result<(i64, Value, Vec<Change>, Vec<String>)> {
        let latest = self
            .read_latest(language_id)?
            .ok_or_else(|| anyhow!("The language has no version yet, so there is nothing to amend."))?;
        if latest.info.schema < 2 {
            bail!("The latest version uses the older format; amendments need a schema 2 spec. Synthesize a new version first.");
        }
        let (doc, changes) = apply_patch(&latest.tokens, patch)?;
        let known = self.known_evidence_ids(language_id)?;
        let check = spec::validate(&doc, &known);
        if !check.errors.is_empty() {
            bail!("The amended spec is not valid: {}", check.errors.join(" "));
        }
        let before = spec::validate(&latest.tokens, &known).warnings;
        let new_notes = check.warnings.into_iter().filter(|w| !before.contains(w)).collect();
        Ok((latest.info.version, doc, changes, new_notes))
    }

    /// Checks and stores a proposal. It works on a query-only connection too
    /// (the MCP server): this is the one write that the server makes.
    pub fn submit_proposal(
        &self,
        language_key: &str,
        base_version: Option<i64>,
        summary: &str,
        rationale: &str,
        patch: &Value,
        author: &str,
    ) -> Result<Submitted> {
        let lang = self.find_language(language_key)?;
        let summary = summary.trim();
        if summary.is_empty() {
            bail!("The summary must not be empty: say in one sentence what the amendment changes.");
        }
        let (latest, _doc, changes, warnings) = self.amended(&lang.id, patch)?;
        if let Some(b) = base_version {
            if b != latest {
                bail!("The proposal is based on v{b}, but \"{}\" is now at v{latest}. Read the language again and send the patch for v{latest}.", lang.name);
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        let proposal = {
            let c = self.c();
            let pending: i64 = c.query_row(
                "SELECT COUNT(*) FROM proposals WHERE language_id = ?1 AND status = 'pending'",
                [&lang.id],
                |r| r.get(0),
            )?;
            if pending >= MAX_PENDING {
                bail!("\"{}\" has {pending} open proposals. The designer must decide on them before new ones can come in.", lang.name);
            }
            let query_only: i64 = c.query_row("PRAGMA query_only", [], |r| r.get(0))?;
            c.execute_batch("PRAGMA query_only=OFF;")?;
            let res = c.execute(
                &format!("INSERT INTO proposals ({COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8, NULL, NULL)"),
                params![id, lang.id, latest, summary, rationale.trim(), author.trim(), serde_json::to_string(patch)?, now_iso()],
            );
            if query_only != 0 {
                c.execute_batch("PRAGMA query_only=ON;")?;
            }
            res?;
            c.query_row(&format!("SELECT {COLS} FROM proposals WHERE id = ?1"), [&id], from_row)?
        };
        Ok(Submitted {
            proposal: Proposal { changes, new_notes: warnings.clone(), ..proposal },
            warnings,
        })
    }

    fn get_proposal(&self, id: &str) -> Result<Proposal> {
        self.c()
            .query_row(&format!("SELECT {COLS} FROM proposals WHERE id = ?1"), [id], from_row)
            .optional()?
            .ok_or_else(|| anyhow!("No proposal with the ID {id}."))
    }

    /// Proposals of a language, newest first. Pending ones carry their changes
    /// against the latest version, or the problem that blocks them.
    pub fn list_proposals(&self, language_id: &str, include_decided: bool) -> Result<Vec<Proposal>> {
        let rows = {
            let c = self.c();
            let sql = format!(
                "SELECT {COLS} FROM proposals WHERE language_id = ?1 {} ORDER BY created_at DESC LIMIT 100",
                if include_decided { "" } else { "AND status = 'pending'" }
            );
            let mut st = c.prepare(&sql)?;
            let rows = st.query_map([language_id], from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        Ok(rows
            .into_iter()
            .map(|mut p| {
                if p.status == "pending" {
                    match self.amended(language_id, &p.patch) {
                        Ok((_, _, changes, notes)) => {
                            p.changes = changes;
                            p.new_notes = notes;
                        }
                        Err(e) => p.problem = Some(e.to_string()),
                    }
                }
                p
            })
            .collect())
    }

    pub fn pending_proposal_count(&self, language_id: &str) -> Result<i64> {
        Ok(self.c().query_row(
            "SELECT COUNT(*) FROM proposals WHERE language_id = ?1 AND status = 'pending'",
            [language_id],
            |r| r.get(0),
        )?)
    }

    /// Applies a pending proposal to the latest version and saves the result as
    /// a new edited version. Returns the version number.
    pub fn accept_proposal(&self, id: &str) -> Result<i64> {
        let p = self.get_proposal(id)?;
        if p.status != "pending" {
            bail!("This proposal was {} already.", p.status);
        }
        let (_, doc, _, _) = self.amended(&p.language_id, &p.patch)?;
        let label = if p.author.is_empty() { "proposal accepted by you".to_string() } else { format!("{} · accepted by you", p.author) };
        let version = self.save_manual_spec(&p.language_id, &doc, &label)?;
        self.c().execute(
            "UPDATE proposals SET status = 'accepted', decided_at = ?2, result_version = ?3 WHERE id = ?1",
            params![id, now_iso(), version],
        )?;
        Ok(version)
    }

    pub fn reject_proposal(&self, id: &str) -> Result<()> {
        let n = self.c().execute(
            "UPDATE proposals SET status = 'rejected', decided_at = ?2 WHERE id = ?1 AND status = 'pending'",
            params![id, now_iso()],
        )?;
        if n == 0 {
            bail!("This proposal is not open.");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn patch_by_id_and_index() {
        let doc = crate::spec::tests::sample();
        let patch = json!([
            {"op": "replace", "path": "/families/blade-field/measures/blade pitch/value", "value": [9, 11]},
            {"op": "add", "path": "/principles/-", "value": {"id": "P3", "text": "Soft light only.", "priority": 3, "basis": "designer", "evidence": []}},
            {"op": "remove", "path": "/dont/0"}
        ]);
        let (out, changes) = apply_patch(&doc, &patch).unwrap();
        assert_eq!(out["families"][1]["measures"][0]["value"], json!([9, 11]));
        assert_eq!(out["principles"][2]["id"], "P3");
        assert_eq!(out["dont"].as_array().unwrap().len(), 0);
        assert_eq!(changes[0].before, json!([8, 10]));
        assert_eq!(changes[2].after, Value::Null);
        // The original is not changed.
        assert_eq!(doc["families"][1]["measures"][0]["value"], json!([8, 10]));
    }

    #[test]
    fn patch_errors() {
        let doc = crate::spec::tests::sample();
        for (patch, want) in [
            (json!([{"op": "replace", "path": "/version", "value": 9}]), "app sets"),
            (json!([{"op": "replace", "path": "/families/nope/name", "value": "x"}]), "no element"),
            (json!([{"op": "move", "path": "/summary", "from": "/x"}]), "not supported"),
            (json!([{"op": "replace", "path": "/summary"}]), "needs a \"value\""),
            (json!([{"op": "add", "path": "/principles/P1", "value": {}}]), "use replace"),
            (json!([]), "no operations"),
            (json!({"op": "remove"}), "array of operations"),
        ] {
            let e = apply_patch(&doc, &patch).unwrap_err().to_string();
            assert!(e.contains(want), "{e}");
        }
    }

    fn png() -> Vec<u8> {
        let img = image::RgbImage::from_fn(32, 24, |x, y| image::Rgb([x as u8, y as u8, 90]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img).write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    #[test]
    fn search_propose_accept_reject() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path()).unwrap();
        let lang = lib.create_language("Organic", "").unwrap();
        let a = lib.import_bytes(&lang.id, "bench.png", &png(), None).unwrap();
        lib.update_item(&a.id, &["shell".into()], "soft oak bench", 5, false).unwrap();
        lib.save_analysis(&a.id, &json!({"summary": "A laminated shell bench.", "object_family": "bench", "features": ["laminated shell"]}), "m").unwrap();
        let mut px = png();
        px.push(0); // different hash, so a second item
        let b = lib.import_bytes(&lang.id, "louvre.png", &px, None).unwrap();
        lib.save_analysis(&b.id, &json!({"summary": "Constant blades.", "object_family": "louvre facade"}), "m").unwrap();

        let hits = crate::agent::search_items(&lib, "laminated bench", None, false, 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].item.id, a.id);
        assert_eq!(hits[0].matched, 2);
        let hits = crate::agent::search_items(&lib, "louv", Some("organic"), false, 10).unwrap();
        assert_eq!(hits[0].item.id, b.id);
        assert!(crate::agent::search_items(&lib, "  ", None, false, 10).is_err());

        // No version yet: nothing to amend.
        let patch = json!([{"op": "replace", "path": "/families/blade-field/measures/blade pitch/value", "value": [9, 11]}]);
        assert!(lib.submit_proposal(&lang.id, None, "x", "", &patch, "t").is_err());
        lib.save_spec_version(&lang.id, &crate::spec::tests::sample(), "m", &[], false, "h", &[]).unwrap();

        // The MCP server submits on a query-only connection; the connection stays query-only.
        let ro = Library::open_query_only(dir.path()).unwrap();
        assert!(ro.submit_proposal(&lang.id, None, " ", "", &patch, "t").is_err());
        assert!(ro.submit_proposal(&lang.id, Some(7), "Wider pitch", "", &patch, "t").unwrap_err().to_string().contains("now at v1"));
        let bad = json!([{"op": "replace", "path": "/families/shell/measures/minimum wall/value", "value": 2.0}]);
        assert!(ro.submit_proposal(&lang.id, None, "Wall", "", &bad, "t").unwrap_err().to_string().contains("not valid"));
        let sub = ro.submit_proposal(&lang.id, Some(1), "Wider pitch", "Seen in the newest refs.", &patch, "Claude Code").unwrap();
        assert_eq!(sub.proposal.changes[0].before, json!([8, 10]));
        assert!(ro.reject_proposal(&sub.proposal.id).is_err(), "the server connection must stay query-only");

        // The app lists, accepts and rejects.
        let open = lib.list_proposals(&lang.id, false).unwrap();
        assert_eq!(open.len(), 1);
        assert!(open[0].problem.is_none());
        assert_eq!(lib.pending_proposal_count(&lang.id).unwrap(), 1);
        let v = lib.accept_proposal(&sub.proposal.id).unwrap();
        assert_eq!(v, 2);
        let c = lib.read_version(&lang.id, 2).unwrap();
        assert_eq!(c.tokens["families"][1]["measures"][0]["value"], json!([9, 11]));
        assert!(c.info.edited);
        assert!(c.info.model.contains("Claude Code"));
        assert!(lib.accept_proposal(&sub.proposal.id).is_err());

        // A proposal that no longer applies to the latest version shows its problem.
        let rm = json!([{"op": "remove", "path": "/principles/P2"}]);
        let p2 = lib.submit_proposal(&lang.id, None, "Drop P2", "", &rm, "").unwrap();
        let p3 = lib.submit_proposal(&lang.id, None, "Drop P2 again", "", &rm, "").unwrap();
        assert!(p2.warnings.iter().any(|w| w.contains("P2")), "{:?}", p2.warnings);
        assert!(sub.warnings.is_empty(), "{:?}", sub.warnings);
        assert!(lib.list_proposals(&lang.id, false).unwrap()[0].new_notes.iter().any(|w| w.contains("P2")));
        lib.accept_proposal(&p2.proposal.id).unwrap();
        let open = lib.list_proposals(&lang.id, false).unwrap();
        assert!(open[0].problem.as_deref().unwrap_or("").contains("P2"));
        assert!(lib.accept_proposal(&p3.proposal.id).is_err());
        lib.reject_proposal(&p3.proposal.id).unwrap();
        assert_eq!(lib.pending_proposal_count(&lang.id).unwrap(), 0);
        assert_eq!(lib.list_proposals(&lang.id, true).unwrap().len(), 3);
    }
}
