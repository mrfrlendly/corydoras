//! Print-feedback log: what happened when a design was printed.
//!
//! Each entry records a result (failed, warped, frosted, fine or other) against
//! a hard rule, a material, or both. The log feeds three places:
//! - the hard rules that agents get (a "Print feedback" table after the rules);
//! - the conformance check (a number that passes a rule but is at or past a
//!   value that failed in a real print becomes a warning);
//! - the rule set editor (counts for each rule, and a hint when the prints
//!   disagree with a limit).
//!
//! An agent can add an entry through the MCP server, for example when the
//! designer says in a chat how a print came out. The designer removes entries
//! in the app.

use crate::agent::names_match;
use crate::conformance::Finding;
use crate::library::Library;
use crate::models::{Rule, RuleSet};
use crate::util::now_iso;
use anyhow::{anyhow, bail, Result};
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

pub const RESULTS: &[&str] = &["failed", "warped", "frosted", "fine", "other"];
/// Most entries that one brief lists.
const BRIEF_LIMIT: usize = 40;

const COLS: &str = "p.id, p.result, p.material, p.rule, p.rule_set_id, (SELECT name FROM rule_sets r WHERE r.id = p.rule_set_id), \
    p.value, p.unit, p.note, p.language_id, p.output_id, p.author, p.created_at";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PrintResult {
    pub id: String,
    /// "failed", "warped", "frosted", "fine" or "other"
    pub result: String,
    pub material: String,
    /// The name of the hard rule that the result is about, or empty.
    pub rule: String,
    /// The rule set that the result belongs to. None: it applies everywhere.
    pub rule_set_id: Option<String>,
    pub rule_set_name: Option<String>,
    /// The value that was printed, for example a wall of 1.0 mm.
    pub value: Option<f64>,
    pub unit: Option<String>,
    pub note: String,
    pub language_id: Option<String>,
    pub output_id: Option<String>,
    /// Who wrote the entry: empty for the designer, else the agent's client name.
    pub author: String,
    pub created_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PrintResultInput {
    pub result: String,
    #[serde(default)]
    pub material: String,
    #[serde(default)]
    pub rule: String,
    /// Rule set ID or name (optional).
    #[serde(default)]
    pub rule_set: Option<String>,
    #[serde(default)]
    pub value: Option<f64>,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub note: String,
    /// Language ID or name (optional).
    #[serde(default)]
    pub language: Option<String>,
    /// Output ID or its first characters (optional).
    #[serde(default)]
    pub output: Option<String>,
}

/// What the log says about one rule of a rule set.
#[derive(Serialize, Clone, Debug, Default)]
pub struct RuleFeedback {
    pub rule: String,
    pub failed: usize,
    pub warped: usize,
    pub frosted: usize,
    pub fine: usize,
    pub other: usize,
    /// Suggestions where the prints disagree with the limit.
    pub hints: Vec<String>,
}

fn from_row(r: &Row) -> rusqlite::Result<PrintResult> {
    Ok(PrintResult {
        id: r.get(0)?,
        result: r.get(1)?,
        material: r.get(2)?,
        rule: r.get(3)?,
        rule_set_id: r.get(4)?,
        rule_set_name: r.get(5)?,
        value: r.get(6)?,
        unit: r.get(7)?,
        note: r.get(8)?,
        language_id: r.get(9)?,
        output_id: r.get(10)?,
        author: r.get(11)?,
        created_at: r.get(12)?,
    })
}

fn num(v: f64, unit: Option<&str>) -> String {
    let n = if (v - v.round()).abs() < 1e-9 { format!("{}", v.round() as i64) } else { format!("{:.3}", v).trim_end_matches('0').trim_end_matches('.').to_string() };
    match unit {
        Some(u) if !u.is_empty() => format!("{n} {u}"),
        _ => n,
    }
}

fn is_bad(result: &str) -> bool {
    matches!(result, "failed" | "warped")
}

impl Library {
    /// Adds an entry. It works on a query-only connection too (the MCP server).
    pub fn add_print_result(&self, input: &PrintResultInput, author: &str) -> Result<PrintResult> {
        let result = input.result.trim().to_lowercase();
        if !RESULTS.contains(&result.as_str()) {
            bail!("Unknown result \"{}\". Use one of: {}.", input.result, RESULTS.join(", "));
        }
        let (material, rule, note) = (input.material.trim(), input.rule.trim(), input.note.trim());
        if material.is_empty() && rule.is_empty() {
            bail!("Say which material or which rule the result is about.");
        }
        if result == "other" && note.is_empty() {
            bail!("The result \"other\" needs a note that says what happened.");
        }
        let set = match input.rule_set.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(key) => Some(self.get_rule_set(key)?),
            None => None,
        };
        if let (Some(set), false) = (&set, rule.is_empty()) {
            if !set.rules.iter().any(|r| names_match(&r.name, rule)) {
                bail!("\"{}\" has no rule \"{rule}\". Its rules: {}.", set.name, set.rules.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(", "));
            }
        }
        let (value, unit) = match input.value {
            Some(v) if !v.is_finite() => bail!("The value must be a number."),
            Some(v) => (Some(v), input.unit.as_deref().map(str::trim).filter(|u| !u.is_empty()).map(|u| u.to_string())),
            None => (None, None),
        };
        let language = match input.language.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(key) => Some(self.find_language(key)?.id),
            None => None,
        };
        let output = match input.output.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(key) => Some(self.find_output(key)?),
            None => None,
        };
        let language = language.or_else(|| output.as_ref().map(|o| o.language_id.clone()));
        let id = uuid::Uuid::new_v4().to_string();
        self.with_writes(|c| {
            c.execute(
                "INSERT INTO print_results (id, result, material, rule, rule_set_id, value, unit, note, language_id, output_id, author, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    id,
                    result,
                    material.chars().take(120).collect::<String>(),
                    rule.chars().take(120).collect::<String>(),
                    set.as_ref().map(|s| s.id.clone()),
                    value,
                    unit,
                    note.chars().take(1000).collect::<String>(),
                    language,
                    output.as_ref().map(|o| o.id.clone()),
                    author.trim().chars().take(60).collect::<String>(),
                    now_iso()
                ],
            )?;
            Ok(())
        })?;
        self.get_print_result(&id)
    }

    pub fn get_print_result(&self, id: &str) -> Result<PrintResult> {
        self.c()
            .query_row(&format!("SELECT {COLS} FROM print_results p WHERE p.id = ?1"), [id], from_row)
            .optional()?
            .ok_or_else(|| anyhow!("No print result with the ID {id}."))
    }

    /// Every entry, newest first.
    pub fn list_print_results(&self) -> Result<Vec<PrintResult>> {
        let c = self.c();
        let mut st = c.prepare(&format!("SELECT {COLS} FROM print_results p ORDER BY p.created_at DESC, p.rowid DESC"))?;
        let rows = st.query_map([], from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The entries that count for these rule sets: the ones of each set, and
    /// the ones without a rule set (they apply everywhere).
    pub fn print_results_for(&self, sets: &[RuleSet]) -> Result<Vec<PrintResult>> {
        Ok(self
            .list_print_results()?
            .into_iter()
            .filter(|p| p.rule_set_id.as_ref().map(|id| sets.iter().any(|s| &s.id == id)).unwrap_or(true))
            .collect())
    }

    pub fn remove_print_result(&self, id: &str) -> Result<()> {
        let n = self.c().execute("DELETE FROM print_results WHERE id = ?1", [id])?;
        if n == 0 {
            bail!("No print result with the ID {id}.");
        }
        Ok(())
    }

    /// Adds the first entry one time: clear PETG prints frosted.
    pub(crate) fn seed_print_log(&self) -> Result<()> {
        let done: Option<String> = self.c().query_row("SELECT value FROM settings WHERE key = 'print_log_seeded'", [], |r| r.get(0)).optional()?;
        if done.is_some() {
            return Ok(());
        }
        let set = self.list_rule_sets()?.into_iter().find(|r| r.name == Library::starter_rule_set().name);
        let input = PrintResultInput {
            result: "frosted".into(),
            material: "PETG, clear or translucent".into(),
            rule_set: set.map(|s| s.id),
            note: "Clear and translucent PETG prints frosted and almost opaque, not glass-clear. Do not design for a view through the wall. Put colour accents on the outside of a clear part, and show clear shells as frosted in renders.".into(),
            ..Default::default()
        };
        self.add_print_result(&input, "Corydoras")?;
        self.c().execute("INSERT INTO settings (key, value) VALUES ('print_log_seeded', '1') ON CONFLICT(key) DO NOTHING", [])?;
        Ok(())
    }

    /// What the log says about each rule of a rule set, plus hints.
    pub fn rule_feedback(&self, rule_set_id: &str) -> Result<Vec<RuleFeedback>> {
        let set = self.get_rule_set(rule_set_id)?;
        let log = self.print_results_for(std::slice::from_ref(&set))?;
        Ok(feedback(&set, &log))
    }
}

/// The entries about one rule.
fn about<'a>(log: &'a [PrintResult], set: &RuleSet, rule: &Rule) -> Vec<&'a PrintResult> {
    log.iter()
        .filter(|p| !p.rule.is_empty() && names_match(&p.rule, &rule.name) && p.rule_set_id.as_ref().map(|id| id == &set.id).unwrap_or(true))
        .collect()
}

pub fn feedback(set: &RuleSet, log: &[PrintResult]) -> Vec<RuleFeedback> {
    let mut out = Vec::new();
    for rule in &set.rules {
        let entries = about(log, set, rule);
        if entries.is_empty() {
            continue;
        }
        let count = |r: &str| entries.iter().filter(|p| p.result == r).count();
        let mut fb = RuleFeedback {
            rule: rule.name.clone(),
            failed: count("failed"),
            warped: count("warped"),
            frosted: count("frosted"),
            fine: count("fine"),
            other: count("other"),
            hints: Vec::new(),
        };
        if let (Some(limit), Some(floor)) = (rule.value, crate::conformance::direction(&rule.name)) {
            let unit = rule.unit.as_deref();
            let same_unit = |p: &PrintResult| p.unit.as_deref().map(|u| Some(u) == unit).unwrap_or(true);
            // A failure on the allowed side of the limit: the limit is too loose.
            let worst = entries
                .iter()
                .filter(|p| is_bad(&p.result) && same_unit(p))
                .filter_map(|p| p.value)
                .filter(|v| if floor { *v >= limit - 1e-9 } else { *v <= limit + 1e-9 })
                .fold(None, |acc: Option<f64>, v| Some(match acc {
                    Some(a) if floor => a.max(v),
                    Some(a) => a.min(v),
                    None => v,
                }));
            if let Some(v) = worst {
                fb.hints.push(format!(
                    "A print failed at {}, which the limit of {} allows. Set the limit {} {}.",
                    num(v, unit),
                    num(limit, unit),
                    if floor { "above" } else { "below" },
                    num(v, unit)
                ));
            }
            // A good print on the forbidden side: the limit may be too strict.
            let best = entries
                .iter()
                .filter(|p| p.result == "fine" && same_unit(p))
                .filter_map(|p| p.value)
                .filter(|v| if floor { *v < limit - 1e-9 } else { *v > limit + 1e-9 })
                .fold(None, |acc: Option<f64>, v| Some(match acc {
                    Some(a) if floor => a.min(v),
                    Some(a) => a.max(v),
                    None => v,
                }));
            if let (Some(v), None) = (best, worst) {
                fb.hints.push(format!("A print was fine at {}, which the limit of {} forbids. The limit can move to {} if that result repeats.", num(v, unit), num(limit, unit), num(v, unit)));
            }
        }
        out.push(fb);
    }
    out
}

/// Adds what the print log knows to a finding that a hard rule judged.
/// A number that passes the rule but is at or past a value that failed in a
/// real print becomes a warning.
pub fn annotate(f: &mut Finding, set: &RuleSet, rule: &Rule, log: &[PrintResult]) {
    let entries = about(log, set, rule);
    if entries.is_empty() {
        return;
    }
    let count = |r: &str| entries.iter().filter(|p| p.result == r).count();
    let mut parts = Vec::new();
    for r in RESULTS {
        let n = count(r);
        if n > 0 {
            parts.push(format!("{n} {r}"));
        }
    }
    let mut text = format!("Print log for this rule: {}.", parts.join(", "));
    if f.verdict == "pass" {
        if let Some(floor) = crate::conformance::direction(&rule.name) {
            let hit = entries.iter().find(|p| {
                is_bad(&p.result)
                    && p.unit.as_deref().map(|u| Some(u) == f.unit.as_deref()).unwrap_or(true)
                    && p.value.map(|v| if floor { f.value <= v + 1e-9 } else { f.value >= v - 1e-9 }).unwrap_or(false)
            });
            if let Some(p) = hit {
                f.verdict = "warn".into();
                text = format!(
                    "The number passes the rule, but a print {} at {}{}. {text}",
                    p.result,
                    num(p.value.unwrap_or(0.0), p.unit.as_deref().or(f.unit.as_deref())),
                    if p.note.is_empty() { String::new() } else { format!(" ({})", p.note) }
                );
            }
        }
    }
    f.detail = if f.detail.is_empty() { text } else { format!("{} {text}", f.detail) };
}

/// One line for each entry that is about a material, not a rule.
pub fn material_notes(log: &[PrintResult]) -> Vec<String> {
    log.iter()
        .filter(|p| p.rule.is_empty())
        .take(12)
        .map(|p| format!("{} · {}: {}", p.result, p.material, p.note))
        .collect()
}

/// The log as a markdown block for agents (empty when there are no entries).
pub fn block(log: &[PrintResult]) -> String {
    if log.is_empty() {
        return String::new();
    }
    let mut s = String::from(
        "## Print feedback\n\nResults of real prints that the designer logged. Treat a failed or warped result as a limit: do not repeat it. A frosted or other result tells you how the material looks.\n\n| Result | Material | Rule | Value | Note | Date |\n| --- | --- | --- | --- | --- | --- |\n",
    );
    let cell = |x: &str| if x.is_empty() { "—".to_string() } else { x.replace('|', "\\|").replace('\n', " ") };
    for p in log.iter().take(BRIEF_LIMIT) {
        s.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            p.result,
            cell(&p.material),
            cell(&p.rule),
            p.value.map(|v| num(v, p.unit.as_deref())).unwrap_or_else(|| "—".into()),
            cell(&p.note),
            p.created_at.chars().take(10).collect::<String>()
        ));
    }
    if log.len() > BRIEF_LIMIT {
        s.push_str(&format!("\n{} older entries are not shown.\n", log.len() - BRIEF_LIMIT));
    }
    s.push('\n');
    s
}

/// The answer of the MCP tool that lists the log.
pub fn log_text(log: &[PrintResult]) -> String {
    if log.is_empty() {
        return "The print log is empty.".into();
    }
    block(log)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conformance::{check_with_log, Measurement};
    use serde_json::Value;

    #[test]
    fn log_feeds_rules_and_check() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path()).unwrap();
        let set = lib.list_rule_sets().unwrap().remove(0);
        // The library starts with the PETG finding, one time.
        let log = lib.list_print_results().unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!((log[0].result.as_str(), log[0].rule_set_id.as_deref()), ("frosted", Some(set.id.as_str())));
        assert!(log[0].material.contains("PETG"));
        drop(lib);
        let lib = Library::open(dir.path()).unwrap();
        assert_eq!(lib.list_print_results().unwrap().len(), 1);

        // An agent adds an entry on a query-only connection.
        let ro = Library::open_query_only(dir.path()).unwrap();
        let input = |result: &str, value: f64| PrintResultInput {
            result: result.into(),
            material: "PETG".into(),
            rule: "minimum wall".into(),
            rule_set: Some(set.name.clone()),
            value: Some(value),
            unit: Some("mm".into()),
            note: "split along the layers".into(),
            ..Default::default()
        };
        let e = ro.add_print_result(&input("Failed", 1.3), "Claude Code").unwrap();
        assert_eq!((e.result.as_str(), e.author.as_str(), e.rule_set_name.as_deref()), ("failed", "Claude Code", Some(set.name.as_str())));
        assert!(ro.remove_print_result(&e.id).is_err(), "the server connection stays query-only");
        assert!(ro.add_print_result(&PrintResultInput { result: "melted".into(), material: "PLA".into(), ..Default::default() }, "").is_err());
        assert!(ro.add_print_result(&PrintResultInput { result: "fine".into(), ..Default::default() }, "").is_err());
        assert!(ro.add_print_result(&PrintResultInput { result: "failed".into(), rule: "no such rule".into(), rule_set: Some(set.id.clone()), ..Default::default() }, "").is_err());
        lib.add_print_result(&input("fine", 1.6), "").unwrap();

        // The rule set editor gets counts and a hint: the limit of 1.2 allows 1.3, which failed.
        let fb = lib.rule_feedback(&set.id).unwrap();
        assert_eq!(fb.len(), 1);
        assert_eq!((fb[0].rule.as_str(), fb[0].failed, fb[0].fine), ("Minimum wall", 1, 1));
        assert!(fb[0].hints[0].contains("above 1.3 mm"), "{:?}", fb[0].hints);

        // The check: 1.25 mm passes the rule of 1.2 mm, but a print failed at 1.3 mm.
        let sets = vec![set.clone()];
        let log = lib.print_results_for(&sets).unwrap();
        let m = |v: f64| [Measurement { name: "minimum wall".into(), value: v, unit: Some("mm".into()) }];
        let r = check_with_log("", 0, &Value::Null, &sets, None, &m(1.25), &log).unwrap();
        assert_eq!(r.findings[0].verdict, "warn");
        assert!(r.findings[0].detail.contains("a print failed at 1.3 mm"), "{}", r.findings[0].detail);
        assert_eq!(r.warn, 1);
        let r = check_with_log("", 0, &Value::Null, &sets, None, &m(1.8), &log).unwrap();
        assert_eq!(r.findings[0].verdict, "pass");
        assert!(r.findings[0].detail.contains("1 failed, 1 fine"));
        // The material finding goes with every report and every brief.
        assert!(r.print_log.iter().any(|n| n.contains("frosted")));
        let text = block(&log);
        assert!(text.contains("| failed | PETG | minimum wall | 1.3 mm |") && text.contains("frosted"), "{text}");

        // An entry of another rule set does not count here.
        let other = lib.create_rule_set(&crate::models::RuleSetInput { name: "SLA".into(), description: String::new(), always: false, rules: vec![] }).unwrap();
        lib.add_print_result(&PrintResultInput { result: "warped".into(), material: "resin".into(), rule_set: Some(other.id.clone()), ..Default::default() }, "").unwrap();
        assert_eq!(lib.print_results_for(&sets).unwrap().len(), 3);
        assert_eq!(lib.list_print_results().unwrap().len(), 4);
        lib.remove_print_result(&e.id).unwrap();
        // Only the good print at 1.6 mm is left; the limit of 1.2 mm allows it, so there is no hint.
        assert!(lib.rule_feedback(&set.id).unwrap()[0].hints.is_empty());
    }
}
