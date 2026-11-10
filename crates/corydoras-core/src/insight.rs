//! Usage insight: which parts of a language agents fetch through the MCP
//! server, which parts nobody uses, and which changes agents propose again
//! and again.
//!
//! The MCP server writes one small record for each tool call. The records stay
//! in the library on this Mac.

use crate::library::Library;
use crate::util::now_iso;
use anyhow::Result;
use rusqlite::params;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// Records to keep. Older ones are removed.
const KEEP_EVENTS: i64 = 20_000;

/// One tool call of an agent.
#[derive(Clone, Debug, Default)]
pub struct Event<'a> {
    pub tool: &'a str,
    pub language_id: &'a str,
    pub version: Option<i64>,
    pub family: &'a str,
    pub project: &'a str,
    pub client: &'a str,
    /// What the call named: measure names, item IDs, query words.
    pub detail: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct FamilyUse {
    pub id: String,
    pub name: String,
    /// Calls that asked for this family by name.
    pub fetches: i64,
    /// Conformance and geometry checks for this family.
    pub checks: i64,
    pub last_used: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct MeasureUse {
    pub family: String,
    pub name: String,
    pub checks: i64,
}

#[derive(Serialize, Clone, Debug)]
pub struct Recurring {
    /// The place in the spec that the proposals change.
    pub path: String,
    pub proposals: i64,
    pub pending: i64,
    pub accepted: i64,
    pub rejected: i64,
    /// What the proposals said (newest first).
    pub summaries: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Usage {
    pub language: String,
    /// The first day of the period.
    pub since: String,
    pub days: i64,
    pub calls: i64,
    /// Calls that read the whole document (every family goes with it).
    pub document_reads: i64,
    /// Briefs of projects that pin this language.
    pub briefs: i64,
    pub last_call: Option<String>,
    /// (tool, calls), most used first.
    pub tools: Vec<(String, i64)>,
    /// (client, calls), most active first.
    pub clients: Vec<(String, i64)>,
    pub families: Vec<FamilyUse>,
    /// Families that no call named and no check used.
    pub unused_families: Vec<String>,
    /// Measures that a check compared a number with.
    pub measures: Vec<MeasureUse>,
    /// Measures that no check used ("family · measure").
    pub unchecked_measures: Vec<String>,
    /// (8-character item ID, times that an agent asked to see it).
    pub references: Vec<(String, i64)>,
    pub recurring: Vec<Recurring>,
}

fn norm(text: &str) -> String {
    text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

impl Library {
    /// Stores one tool call. It works on a query-only connection (the MCP
    /// server). A failure must not break the tool call, so the caller can
    /// ignore the result.
    pub fn record_event(&self, e: &Event) -> Result<()> {
        let detail = serde_json::to_string(&e.detail.iter().take(40).collect::<Vec<_>>())?;
        self.with_writes(|c| {
            c.execute(
                "INSERT INTO agent_events (at, tool, language_id, version, family, project, client, detail) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![now_iso(), e.tool, e.language_id, e.version, e.family.trim(), e.project.trim(), e.client.trim(), detail],
            )?;
            // Now and then, drop the oldest records.
            let id = c.last_insert_rowid();
            if id % 500 == 0 {
                c.execute("DELETE FROM agent_events WHERE id <= ?1", [id - KEEP_EVENTS])?;
            }
            Ok(())
        })
    }
}

/// How agents used a language in the last `days` days.
pub fn usage(lib: &Library, language_key: &str, days: i64) -> Result<Usage> {
    let lang = lib.find_language(language_key)?;
    let days = days.clamp(1, 3650);
    let since = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    // (at, tool, family, client, detail)
    let events: Vec<(String, String, String, String, Vec<String>)> = {
        let c = lib.c();
        let mut st = c.prepare("SELECT at, tool, family, client, detail FROM agent_events WHERE language_id = ?1 AND at >= ?2 ORDER BY at")?;
        let rows = st
            .query_map(params![lang.id, since], |r| {
                let detail: String = r.get(4)?;
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, serde_json::from_str(&detail).unwrap_or_default()))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let tokens = lib.read_latest(&lang.id)?.filter(|c| c.info.schema >= 2).map(|c| c.tokens).unwrap_or(Value::Null);
    let families: Vec<&Value> = tokens.get("families").and_then(|v| v.as_array()).map(|a| a.iter().collect()).unwrap_or_default();
    let fid = |f: &Value| f.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let fname = |f: &Value| f.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let is_check = |tool: &str| tool == "check_against_language" || tool == "check_geometry";
    let same_family = |f: &Value, key: &str| !key.is_empty() && (fid(f).eq_ignore_ascii_case(key) || fname(f).eq_ignore_ascii_case(key));

    let mut tools: BTreeMap<String, i64> = BTreeMap::new();
    let mut clients: BTreeMap<String, i64> = BTreeMap::new();
    let mut references: BTreeMap<String, i64> = BTreeMap::new();
    let (mut document_reads, mut briefs) = (0, 0);
    for (_, tool, family, client, detail) in &events {
        *tools.entry(tool.clone()).or_default() += 1;
        *clients.entry(if client.is_empty() { "unknown client".to_string() } else { client.clone() }).or_default() += 1;
        match tool.as_str() {
            "get_language" if family.is_empty() => document_reads += 1,
            "get_project_brief" => briefs += 1,
            "get_reference_images" => {
                for id in detail {
                    *references.entry(id.chars().take(8).collect()).or_default() += 1;
                }
            }
            _ => {}
        }
    }
    let by_count = |m: BTreeMap<String, i64>| {
        let mut v: Vec<(String, i64)> = m.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    };

    let mut family_use = Vec::new();
    let mut measures = Vec::new();
    let mut unchecked = Vec::new();
    for f in &families {
        let mine: Vec<&(String, String, String, String, Vec<String>)> = events.iter().filter(|e| same_family(f, &e.2)).collect();
        family_use.push(FamilyUse {
            id: fid(f),
            name: fname(f),
            fetches: mine.iter().filter(|e| !is_check(&e.1)).count() as i64,
            checks: mine.iter().filter(|e| is_check(&e.1)).count() as i64,
            last_used: mine.last().map(|e| e.0.clone()),
        });
        for m in f.get("measures").and_then(|v| v.as_array()).into_iter().flatten() {
            let name = m.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let checks = mine.iter().filter(|e| is_check(&e.1) && e.4.iter().any(|d| norm(d) == norm(name))).count() as i64;
            if checks > 0 {
                measures.push(MeasureUse { family: fid(f), name: name.to_string(), checks });
            } else {
                unchecked.push(format!("{} · {name}", fname(f)));
            }
        }
    }
    let unused_families = family_use.iter().filter(|f| f.fetches == 0 && f.checks == 0).map(|f| f.name.clone()).collect();
    measures.sort_by(|a, b| b.checks.cmp(&a.checks));

    // Proposals that change the same place more than one time.
    let mut by_path: BTreeMap<String, Recurring> = BTreeMap::new();
    for p in lib.list_proposals(&lang.id, true)? {
        let mut paths: Vec<String> = p
            .patch
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|op| op.get("path").and_then(|v| v.as_str()))
            .map(|path| {
                // An added element has no name yet: count it for its list.
                let path = path.trim_end_matches("/-").to_lowercase();
                let parts: Vec<&str> = path.split('/').collect();
                if parts.last().map(|l| l.parse::<usize>().is_ok()).unwrap_or(false) { parts[..parts.len() - 1].join("/") } else { path.clone() }
            })
            .collect();
        paths.sort();
        paths.dedup();
        for path in paths {
            let r = by_path.entry(path.clone()).or_insert_with(|| Recurring { path, proposals: 0, pending: 0, accepted: 0, rejected: 0, summaries: Vec::new() });
            r.proposals += 1;
            match p.status.as_str() {
                "accepted" => r.accepted += 1,
                "rejected" => r.rejected += 1,
                _ => r.pending += 1,
            }
            if r.summaries.len() < 4 {
                r.summaries.push(p.summary.clone());
            }
        }
    }
    let mut recurring: Vec<Recurring> = by_path.into_values().filter(|r| r.proposals >= 2).collect();
    recurring.sort_by(|a, b| b.proposals.cmp(&a.proposals).then(a.path.cmp(&b.path)));

    Ok(Usage {
        language: lang.name,
        since: since.chars().take(10).collect(),
        days,
        calls: events.len() as i64,
        document_reads,
        briefs,
        last_call: events.last().map(|e| e.0.clone()),
        tools: by_count(tools),
        clients: by_count(clients),
        families: family_use,
        unused_families,
        measures,
        unchecked_measures: unchecked,
        references: by_count(references).into_iter().take(12).collect(),
        recurring,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn what_agents_use() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path()).unwrap();
        let lang = lib.create_language("Organic", "").unwrap();
        lib.save_spec_version(&lang.id, &crate::spec::tests::sample(), "m", &[], false, "h", &[]).unwrap();
        let empty = usage(&lib, "organic", 90).unwrap();
        assert_eq!((empty.calls, empty.unused_families.len(), empty.unchecked_measures.len()), (0, 2, 3));

        // The MCP server records on a query-only connection.
        let ro = Library::open_query_only(dir.path()).unwrap();
        let ev = |tool: &'static str, family: &'static str, detail: &[&str]| Event {
            tool,
            language_id: "organic",
            version: Some(1),
            family,
            project: "",
            client: "Claude Code",
            detail: detail.iter().map(|d| d.to_string()).collect(),
        };
        ro.record_event(&ev("get_language", "", &[])).unwrap();
        ro.record_event(&ev("get_language", "shell", &[])).unwrap();
        ro.record_event(&ev("get_language", "Laminated shell", &[])).unwrap();
        ro.record_event(&ev("check_against_language", "shell", &["Edge fillet", "wall"])).unwrap();
        ro.record_event(&ev("get_reference_images", "", &["aaaaaaaa", "aaaaaaaa-long-id"])).unwrap();
        ro.record_event(&Event { tool: "get_language", language_id: "another", client: "x", ..Default::default() }).unwrap();

        let u = usage(&lib, "organic", 90).unwrap();
        assert_eq!((u.calls, u.document_reads, u.briefs), (5, 1, 0));
        assert_eq!(u.tools[0], ("get_language".to_string(), 3));
        assert_eq!(u.clients, vec![("Claude Code".to_string(), 5)]);
        let shell = u.families.iter().find(|f| f.id == "shell").unwrap();
        assert_eq!((shell.fetches, shell.checks), (2, 1));
        assert!(shell.last_used.is_some());
        assert_eq!(u.unused_families, vec!["Blade field".to_string()]);
        assert_eq!(u.measures.len(), 1);
        assert_eq!((u.measures[0].name.as_str(), u.measures[0].checks), ("edge fillet", 1));
        assert_eq!(u.unchecked_measures, vec!["Laminated shell · minimum wall".to_string(), "Blade field · blade pitch".to_string()]);
        assert_eq!(u.references, vec![("aaaaaaaa".to_string(), 2)]);
        assert!(u.recurring.is_empty());

        // Two proposals on the same measure are a recurring change.
        let patch = |v: i64| json!([{"op": "replace", "path": "/families/blade-field/measures/blade pitch/value", "value": [v, v + 2]}]);
        let first = lib.submit_proposal(&lang.id, None, "Wider pitch", "", &patch(9), "a").unwrap();
        lib.reject_proposal(&first.proposal.id).unwrap();
        lib.submit_proposal(&lang.id, None, "Wider pitch again", "", &patch(10), "b").unwrap();
        lib.submit_proposal(&lang.id, None, "New principle", "", &json!([{"op": "add", "path": "/principles/-", "value": {"id": "P9", "text": "Soft.", "priority": 9, "basis": "designer", "evidence": []}}]), "b").unwrap();
        let u = usage(&lib, "organic", 90).unwrap();
        assert_eq!(u.recurring.len(), 1);
        let r = &u.recurring[0];
        assert_eq!((r.path.as_str(), r.proposals, r.pending, r.rejected), ("/families/blade-field/measures/blade pitch/value", 2, 1, 1));
    }
}
