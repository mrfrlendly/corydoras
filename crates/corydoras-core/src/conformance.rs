//! Conformance check: an agent reports what it measured on a design, and this
//! module compares each number with the hard rules and with the measures of
//! the language. Hard rules come first and win: a number that breaks a hard
//! rule fails, whatever the language says.
//!
//! The check uses submitted numbers only. It does not look at images.

use crate::agent::match_rule;
use crate::models::RuleSet;
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One number that the agent measured.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Measurement {
    pub name: String,
    pub value: f64,
    #[serde(default)]
    pub unit: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Finding {
    pub measure: String,
    pub value: f64,
    pub unit: Option<String>,
    /// "pass", "fail", "warn" or "unknown"
    pub verdict: String,
    /// "hard rule", "language" or "none"
    pub source: String,
    /// What the number was compared with, in words.
    pub expected: String,
    pub detail: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Report {
    pub language: String,
    pub version: i64,
    pub family: Option<String>,
    pub findings: Vec<Finding>,
    /// Measures of the family that the agent did not report.
    pub not_reported: Vec<String>,
    /// Names of the rule sets that were applied. Empty: no hard rule judged anything.
    #[serde(default)]
    pub rule_sets: Vec<String>,
    pub pass: usize,
    pub fail: usize,
    pub warn: usize,
    pub unknown: usize,
}

fn fmt(v: f64, unit: Option<&str>) -> String {
    let n = if (v - v.round()).abs() < 1e-9 { format!("{}", v as i64) } else { format!("{:.3}", v).trim_end_matches('0').to_string() };
    match unit {
        Some(u) if !u.is_empty() => format!("{n} {u}"),
        _ => n,
    }
}

fn same_unit(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => x.eq_ignore_ascii_case(y),
        // A missing unit on either side cannot be compared safely.
        _ => false,
    }
}

fn norm(s: &str) -> String {
    s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// A limit's direction comes from its name: "minimum wall" is a floor,
/// "maximum overhang" a ceiling. Other names have no direction.
fn direction(rule_name: &str) -> Option<bool> {
    let n = norm(rule_name);
    let words: Vec<&str> = n.split(' ').collect();
    if words.iter().any(|w| matches!(*w, "minimum" | "min" | "minimal")) {
        Some(true)
    } else if words.iter().any(|w| matches!(*w, "maximum" | "max" | "maximal")) {
        Some(false)
    } else {
        None
    }
}

fn finding(m: &Measurement, verdict: &str, source: &str, expected: String, detail: String) -> Finding {
    Finding {
        measure: m.name.clone(),
        value: m.value,
        unit: m.unit.clone(),
        verdict: verdict.into(),
        source: source.into(),
        expected,
        detail,
    }
}

/// The measure range of a language measure: (low, high), or None when it has no value.
fn range_of(v: &Value) -> Option<(f64, f64)> {
    match v {
        Value::Number(n) => n.as_f64().map(|x| (x, x)),
        Value::Array(a) if a.len() == 2 => Some((a[0].as_f64()?, a[1].as_f64()?)),
        _ => None,
    }
}

fn check_one(m: &Measurement, sets: &[RuleSet], measures: &[&Value]) -> Finding {
    let unit = m.unit.as_deref();
    // 1. Hard rules win.
    if let Some((set, rule)) = match_rule(sets, &m.name) {
        let limit = rule.value.unwrap_or(0.0);
        let expected = format!("{} {} (\"{}\" in {})", match direction(&rule.name) { Some(true) => "at least", Some(false) => "at most", None => "about" }, fmt(limit, rule.unit.as_deref()), rule.name, set.name);
        if !same_unit(unit, rule.unit.as_deref()) {
            return finding(m, "unknown", "hard rule", expected, format!("The units differ or are missing (got {}). Report the number in {}.", unit.unwrap_or("no unit"), rule.unit.as_deref().unwrap_or("the rule's unit")));
        }
        return match direction(&rule.name) {
            Some(true) if m.value + 1e-9 < limit => finding(m, "fail", "hard rule", expected, format!("{} is below the minimum.", fmt(m.value, unit))),
            Some(false) if m.value - 1e-9 > limit => finding(m, "fail", "hard rule", expected, format!("{} is above the maximum.", fmt(m.value, unit))),
            Some(_) => finding(m, "pass", "hard rule", expected, String::new()),
            None => finding(m, "unknown", "hard rule", expected, "The rule has no direction (minimum or maximum), so the number cannot be judged.".into()),
        };
    }
    // 2. The language's own measures.
    let want = norm(&m.name);
    let hit = measures.iter().find(|x| norm(x.get("name").and_then(|v| v.as_str()).unwrap_or("")) == want);
    let Some(meas) = hit else {
        return finding(m, "unknown", "none", String::new(), "No hard rule or language measure has this name.".into());
    };
    let basis = meas.get("basis").and_then(|v| v.as_str()).unwrap_or("");
    let mu = meas.get("unit").and_then(|v| v.as_str());
    let of = meas.get("of").and_then(|v| v.as_str()).unwrap_or("");
    let Some((lo, hi)) = meas.get("value").and_then(range_of) else {
        return finding(m, "unknown", "language", String::new(), if basis == "process-dependent" {
            "This measure is process-dependent and no hard rule sets it. Ask the designer.".into()
        } else {
            "The language gives no value for this measure.".into()
        });
    };
    let expected = if lo == hi { format!("about {} (of {of})", fmt(lo, mu)) } else { format!("{} to {} (of {of})", fmt(lo, None), fmt(hi, mu)) };
    if !same_unit(unit, mu) {
        return finding(m, "unknown", "language", expected, format!("The units differ or are missing (got {}). Report the number in {}.", unit.unwrap_or("no unit"), mu.unwrap_or("the measure's unit")));
    }
    // A single value is a target: allow 10 % either way.
    let (a, b) = if lo == hi { (lo * 0.9, lo * 1.1) } else { (lo, hi) };
    let (a, b) = if a <= b { (a, b) } else { (b, a) };
    if m.value >= a - 1e-9 && m.value <= b + 1e-9 {
        return finding(m, "pass", "language", expected, String::new());
    }
    // An observed measure is seen in the references; a preferred one is a starting value.
    let (verdict, how) = if basis == "observed" { ("fail", "outside the range seen in the references") } else { ("warn", "outside the preferred range; adjust it only with a reason") };
    finding(m, verdict, "language", expected, format!("{} is {how}.", fmt(m.value, unit)))
}

/// Compares measurements with the hard rules and the language tokens (schema 2).
pub fn check(
    language_name: &str,
    version: i64,
    tokens: &Value,
    sets: &[RuleSet],
    family: Option<&str>,
    measurements: &[Measurement],
) -> Result<Report> {
    if measurements.is_empty() {
        bail!("Send at least one measurement: {{\"name\", \"value\", \"unit\"}}.");
    }
    if measurements.len() > 100 {
        bail!("Send at most 100 measurements per check.");
    }
    for m in measurements {
        if m.name.trim().is_empty() {
            bail!("Every measurement needs a name.");
        }
        if !m.value.is_finite() {
            bail!("The value of \"{}\" is not a number.", m.name);
        }
    }
    let families = tokens.get("families").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut picked: Vec<&Value> = Vec::new();
    let mut family_name = None;
    if let Some(f) = family.map(|f| f.trim()).filter(|f| !f.is_empty()) {
        let key = f.to_lowercase();
        let fam = families
            .iter()
            .find(|x| {
                x.get("id").and_then(|v| v.as_str()).map(|s| s.to_lowercase()) == Some(key.clone())
                    || x.get("name").and_then(|v| v.as_str()).map(|s| s.to_lowercase()) == Some(key.clone())
            })
            .ok_or_else(|| {
                let ids: Vec<&str> = families.iter().filter_map(|x| x.get("id").and_then(|v| v.as_str())).collect();
                anyhow!("No family \"{f}\" in {language_name} v{version}. Families: {}.", ids.join(", "))
            })?;
        family_name = fam.get("name").and_then(|v| v.as_str()).map(|s| s.to_string());
        picked.extend(fam.get("measures").and_then(|v| v.as_array()).map(|a| a.iter().collect::<Vec<_>>()).unwrap_or_default());
    }
    // Never apply one family's measures to another family: without a family, only
    // the hard rules can judge a number.
    let findings: Vec<Finding> = measurements.iter().map(|m| check_one(m, sets, &picked)).collect();
    let reported: Vec<String> = measurements.iter().map(|m| norm(&m.name)).collect();
    let not_reported = picked
        .iter()
        .filter_map(|m| m.get("name").and_then(|v| v.as_str()))
        .filter(|n| !reported.contains(&norm(n)))
        .map(|n| n.to_string())
        .collect();
    let count = |v: &str| findings.iter().filter(|f| f.verdict == v).count();
    Ok(Report {
        language: language_name.into(),
        version,
        family: family_name,
        pass: count("pass"),
        fail: count("fail"),
        warn: count("warn"),
        unknown: count("unknown"),
        findings,
        not_reported,
        rule_sets: sets.iter().map(|r| r.name.clone()).collect(),
    })
}

/// The report as text for an agent.
pub fn report_text(r: &Report) -> String {
    let mut s = format!(
        "CONFORMANCE of a design to {} v{}{}: {} pass, {} fail, {} warn, {} unknown.\n\n",
        r.language,
        r.version,
        r.family.as_ref().map(|f| format!(" (family {f})")).unwrap_or_default(),
        r.pass,
        r.fail,
        r.warn,
        r.unknown
    );
    for f in &r.findings {
        s.push_str(&format!(
            "- {} {} = {}{}{}{}\n",
            f.verdict.to_uppercase(),
            f.measure,
            fmt(f.value, f.unit.as_deref()),
            if f.expected.is_empty() { String::new() } else { format!(" · expected {} [{}]", f.expected, f.source) },
            if f.detail.is_empty() { String::new() } else { format!(" · {}", f.detail) },
            ""
        ));
    }
    if r.rule_sets.is_empty() {
        s.push_str("\nNo hard rule set applied, so no hard rule judged anything. Pass \"project\" (a project that selects rule sets), or ask the designer to mark a rule set \"Apply to every project\".\n");
    } else {
        s.push_str(&format!("\nHard rules from: {}.\n", r.rule_sets.join(", ")));
    }
    if !r.not_reported.is_empty() {
        s.push_str(&format!("\nNot reported (measures of the family): {}.\n", r.not_reported.join(", ")));
    }
    if r.fail > 0 {
        s.push_str("\nFix every FAIL: a hard-rule fail means the design value must change; the rule stays. A language fail means the design leaves what the references show.\n");
    }
    s
}

/// Runs the check against the latest version of a language. The hard rules are
/// those of `project` (a name or ID) plus the "always" sets; without a project,
/// only the "always" sets. With `output`, the report is saved with that output.
pub fn run(
    lib: &crate::Library,
    language_key: &str,
    family: Option<&str>,
    project: Option<&str>,
    measurements: &[Measurement],
    output: Option<&str>,
) -> Result<Report> {
    let lang = lib.find_language(language_key)?;
    let content = lib
        .read_latest(&lang.id)?
        .ok_or_else(|| anyhow!("\"{}\" has no version yet, so there is nothing to check against.", lang.name))?;
    if content.info.schema < 2 {
        bail!("Version {} of \"{}\" uses the older format. Synthesize a new version first.", content.info.version, lang.name);
    }
    let project = match project.map(|p| p.trim()).filter(|p| !p.is_empty()) {
        Some(p) => Some(lib.find_project(p)?),
        None => None,
    };
    let sets = lib.effective_rule_sets(project.as_ref())?;
    let report = check(&lang.name, content.info.version, &content.tokens, &sets, family, measurements)?;
    if let Some(key) = output.map(|o| o.trim()).filter(|o| !o.is_empty()) {
        let out = lib.find_output(key)?;
        if out.language_id != lang.id {
            bail!("That output belongs to another language.");
        }
        lib.save_output_check(&out.id, &report)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Rule;
    use serde_json::json;

    fn sets() -> Vec<RuleSet> {
        let r = |n: &str, v: f64| Rule { name: n.into(), value: Some(v), unit: Some("mm".into()), note: String::new() };
        vec![RuleSet {
            id: "s".into(),
            name: "FDM".into(),
            description: String::new(),
            always: true,
            rules: vec![r("Minimum wall", 1.2), r("Maximum bridge", 40.0)],
            created_at: String::new(),
            updated_at: String::new(),
            project_count: 0,
        }]
    }

    fn tokens() -> Value {
        json!({"families": [{"id": "shell", "name": "Laminated shell", "measures": [
            {"name": "edge fillet", "value": [0.3, 0.5], "unit": "ratio", "of": "edge", "basis": "observed"},
            {"name": "blade pitch", "value": 9, "unit": "mm", "of": "pitch", "basis": "preferred"},
            {"name": "minimum wall", "value": null, "unit": "mm", "of": "wall", "basis": "process-dependent"}
        ]}]})
    }

    fn m(n: &str, v: f64, u: &str) -> Measurement {
        Measurement { name: n.into(), value: v, unit: Some(u.into()) }
    }

    #[test]
    fn hard_rules_win() {
        let r = check("L", 1, &tokens(), &sets(), Some("shell"), &[m("min wall", 0.8, "mm"), m("minimum wall", 1.6, "mm"), m("maximum bridge", 55.0, "mm")]).unwrap();
        assert_eq!(r.findings[0].verdict, "fail");
        assert_eq!(r.findings[0].source, "hard rule");
        assert_eq!(r.findings[1].verdict, "pass");
        assert_eq!(r.findings[2].verdict, "fail");
        assert_eq!(r.fail, 2);
    }

    #[test]
    fn language_measures() {
        let r = check("L", 1, &tokens(), &sets(), Some("shell"), &[m("edge fillet", 0.4, "ratio"), m("edge fillet", 0.9, "ratio"), m("blade pitch", 12.0, "mm"), m("blade pitch", 9.5, "mm")]).unwrap();
        assert_eq!(r.findings[0].verdict, "pass");
        assert_eq!(r.findings[1].verdict, "fail"); // observed range
        assert_eq!(r.findings[2].verdict, "warn"); // preferred value, outside 10 %
        assert_eq!(r.findings[3].verdict, "pass");
        assert!(r.not_reported.contains(&"minimum wall".to_string()));
    }

    #[test]
    fn unknowns_are_not_passes() {
        let r = check("L", 1, &tokens(), &sets(), Some("shell"), &[m("minimum wall", 2.0, "inch"), m("colour temperature", 3000.0, "K"), Measurement { name: "edge fillet".into(), value: 0.4, unit: None }]).unwrap();
        assert!(r.findings.iter().all(|f| f.verdict == "unknown"));
        // Without a family, language measures are never applied.
        let r = check("L", 1, &tokens(), &sets(), None, &[m("edge fillet", 0.4, "ratio")]).unwrap();
        assert_eq!(r.findings[0].verdict, "unknown");
        assert!(check("L", 1, &tokens(), &sets(), Some("nope"), &[m("x", 1.0, "mm")]).is_err());
        assert!(check("L", 1, &tokens(), &sets(), None, &[]).is_err());
    }

    #[test]
    fn run_against_a_library() {
        let dir = tempfile::tempdir().unwrap();
        let lib = crate::Library::open(dir.path()).unwrap();
        let lang = lib.create_language("Organic", "").unwrap();
        let ms = [m("minimum wall", 0.8, "mm")];
        // No version yet.
        assert!(run(&lib, "organic", None, None, &ms, None).is_err());
        lib.save_spec_version(&lang.id, &tokens_for_run(), "m", &[], false, "h", &[]).unwrap();
        let r = run(&lib, "organic", Some("shell"), None, &ms, None).unwrap();
        // The starter rule set applies everywhere and has a minimum wall.
        assert_eq!(r.version, 1);
        assert_eq!(r.fail + r.pass + r.unknown + r.warn, 1);
        assert!(report_text(&r).contains("CONFORMANCE"));
        assert!(report_text(&r).contains("No hard rule set applied"));

        // The report is saved with an output, through a query-only connection.
        let img = image::RgbImage::from_fn(32, 24, |x, y| image::Rgb([x as u8, y as u8, 9]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img).write_to(&mut buf, image::ImageFormat::Png).unwrap();
        let ro = crate::Library::open_query_only(dir.path()).unwrap();
        let out = ro.submit_output("organic", "a.png", &buf.into_inner(), "shell", "", "t", "", &ms).unwrap();
        run(&ro, "organic", Some("shell"), None, &ms, Some(&out.id[..8])).unwrap();
        assert!(lib.get_output(&out.id).unwrap().check.is_some());
        assert!(run(&lib, "organic", None, Some("no such project"), &ms, None).is_err());
    }

    fn tokens_for_run() -> Value {
        crate::spec::tests::sample()
    }
}
