//! Coverage report: what a language cannot answer yet, and what to add.
//!
//! The report is worked out from the latest version and the analyses of the
//! references. It needs no model call. It lists families that rest on one
//! reference, views that no reference shows, measures with weak evidence,
//! process-dependent measures that no hard rule fills, and references that
//! disagree with each other.

use crate::agent::match_rule;
use crate::library::Library;
use crate::models::Item;
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

/// The views that a family needs so that an agent can model it.
pub const WANTED_VIEWS: &[&str] = &["three-quarter", "front or side", "detail"];

#[derive(Serialize, Clone, Debug)]
pub struct Gap {
    /// "thin-family", "missing-views", "weak-measure", "no-hard-rule", "contradiction", ...
    pub kind: String,
    /// "high", "medium" or "low"
    pub severity: String,
    /// The family that the gap is in, if it is in one.
    pub family: Option<String>,
    pub title: String,
    pub detail: String,
    /// What to add or change.
    pub suggestion: String,
    /// The 8-character IDs of the items that the gap is about.
    pub evidence: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct FamilyCoverage {
    pub id: String,
    pub name: String,
    pub references: usize,
    pub anti_references: usize,
    pub measures: usize,
    pub rules: usize,
    /// Views that the references of the family show.
    pub views: Vec<String>,
    pub missing_views: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Coverage {
    pub language: String,
    pub version: Option<i64>,
    pub families: Vec<FamilyCoverage>,
    pub gaps: Vec<Gap>,
    /// Analyzed references that no family uses.
    pub unused_references: usize,
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}

fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k).and_then(|x| x.as_array()).map(|a| a.as_slice()).unwrap_or(&[])
}

fn short(id: &str) -> String {
    id.chars().take(8).collect()
}

fn norm(text: &str) -> String {
    text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// The views that an item shows. The analysis names the view in "view"; older
/// analyses have no such field, so the words of the description decide.
pub fn views_of(item: &Item, analysis: Option<&Value>) -> Vec<&'static str> {
    fn add(out: &mut Vec<&'static str>, v: &'static str) {
        if !out.contains(&v) {
            out.push(v);
        }
    }
    let mut out: Vec<&'static str> = Vec::new();
    if item.kind == "model" {
        // The render sheet has a three-quarter view, the front and the top.
        for v in ["three-quarter", "front", "top"] {
            add(&mut out, v);
        }
    }
    if item.region_count > 0 {
        add(&mut out, "detail");
    }
    let Some(a) = analysis else { return out };
    let named = s(a, "view").to_lowercase();
    let text = format!("{} {} {} {}", named, s(a, "light_and_presentation"), s(a, "summary"), s(a, "subject")).to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| text.contains(w));
    if has(&["three-quarter", "three quarter", "3/4", "perspective", "isometric", "oblique", "angled view", "at an angle"]) {
        add(&mut out, "three-quarter");
    }
    if has(&["front view", "frontal", "front elevation", "from the front", "head-on", "straight-on"]) || named == "front" {
        add(&mut out, "front");
    }
    if has(&["side view", "side elevation", "from the side", "in profile", "profile view"]) || named == "side" {
        add(&mut out, "side");
    }
    if has(&["top view", "top-down", "plan view", "from above", "overhead"]) || named == "top" {
        add(&mut out, "top");
    }
    if has(&["back view", "rear view", "from behind"]) || named == "back" {
        add(&mut out, "back");
    }
    if has(&["close-up", "closeup", "close up", "macro", "detail shot", "detail view"]) || named == "detail" {
        add(&mut out, "detail");
    }
    if has(&["section", "cutaway", "cut-away", "exploded"]) {
        add(&mut out, "section");
    }
    if has(&["in use", "in context", "lifestyle", "in situ", "interior scene", "in a room"]) || named == "context" {
        add(&mut out, "context");
    }
    out
}

fn finish_class(text: &str) -> Option<&'static str> {
    let t = text.to_lowercase();
    if t.contains("gloss") || t.contains("polished") || t.contains("mirror") {
        Some("gloss")
    } else if t.contains("matte") || t.contains("matt ") || t == "matt" || t.contains("flat finish") {
        Some("matte")
    } else {
        None
    }
}

fn fmt(v: f64) -> String {
    let t = format!("{v:.2}");
    t.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The coverage of a language.
pub fn report(lib: &Library, language_key: &str) -> Result<Coverage> {
    let lang = lib.find_language(language_key)?;
    let mut gaps: Vec<Gap> = Vec::new();
    let mut gap = |kind: &str, severity: &str, family: Option<&str>, title: String, detail: String, suggestion: String, evidence: Vec<String>| {
        gaps.push(Gap { kind: kind.into(), severity: severity.into(), family: family.map(|f| f.to_string()), title, detail, suggestion, evidence });
    };

    // The references: the items of this language and, for a language that extends a base, of the base.
    let mut items: Vec<Item> = Vec::new();
    for l in lib.base_chain(&lang.id)? {
        items.extend(lib.list_items(&l.id)?);
    }
    let own: Vec<&Item> = items.iter().filter(|i| i.language_id == lang.id).collect();
    let mut analyses: HashMap<String, Value> = HashMap::new();
    for i in &items {
        if let Some((a, _)) = lib.get_analysis(&i.id)? {
            analyses.insert(i.id.clone(), a);
        }
    }
    let waiting = own.iter().filter(|i| i.analysis_status == "pending" || i.analysis_status == "analyzing").count();
    let failed = own.iter().filter(|i| i.analysis_status == "error").count();
    if failed > 0 {
        gap(
            "not-analyzed",
            "medium",
            None,
            format!("{failed} reference{} failed in the analysis", if failed == 1 { "" } else { "s" }),
            "The language document does not use a reference without an analysis.".into(),
            "Open the Board and click Retry, or check the provider in Settings.".into(),
            own.iter().filter(|i| i.analysis_status == "error").map(|i| short(&i.id)).collect(),
        );
    }
    if waiting > 0 {
        gap(
            "not-analyzed",
            "low",
            None,
            format!("{waiting} reference{} wait for the analysis", if waiting == 1 { "" } else { "s" }),
            "The report below does not count them yet.".into(),
            "Wait for the queue, or click Run queue in manual mode.".into(),
            Vec::new(),
        );
    }

    let content = lib.read_latest(&lang.id)?;
    let Some(content) = content.filter(|c| c.info.schema >= 2) else {
        let has_version = lang.latest_version.is_some();
        gap(
            "no-document",
            "high",
            None,
            if has_version { "The latest version uses the older format".into() } else { "No language document yet".into() },
            if has_version { "The older format has no families and no measures, so its coverage cannot be checked.".into() } else { "Agents get nothing from this language until it has a version.".into() },
            if own.is_empty() { "Add reference images on the Board.".into() } else { "Click Synthesize now.".into() },
            Vec::new(),
        );
        return Ok(Coverage { language: lang.name, version: lang.latest_version, families: Vec::new(), gaps, unused_references: 0 });
    };
    let t = &content.tokens;
    let find = |id8: &str| items.iter().find(|i| i.id.starts_with(id8));
    let sets = lib.list_rule_sets()?;

    // Every ID that the document cites.
    let mut cited: Vec<String> = Vec::new();
    fn collect(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(o) => {
                for (k, x) in o {
                    if k == "evidence" || k == "items" {
                        out.extend(x.as_array().into_iter().flatten().filter_map(|e| e.as_str().map(|s| s.chars().take(8).collect::<String>())));
                    } else if k == "item" {
                        out.extend(x.as_str().map(|s| s.chars().take(8).collect::<String>()));
                    } else {
                        collect(x, out);
                    }
                }
            }
            Value::Array(a) => a.iter().for_each(|x| collect(x, out)),
            _ => {}
        }
    }
    collect(t, &mut cited);
    cited.sort();
    cited.dedup();
    let known = lib.known_evidence_ids(&lang.id)?;
    let stale: Vec<String> = cited.iter().filter(|c| !known.contains(c)).cloned().collect();
    if !stale.is_empty() {
        gap(
            "stale-evidence",
            "medium",
            None,
            format!("The document cites {} reference{} no longer on the board", stale.len(), if stale.len() == 1 { " that is" } else { "s that are" }),
            "A rule that cites a removed reference has no evidence that an agent can look at.".into(),
            "Click Synthesize now, so that the next version cites the references that exist.".into(),
            stale,
        );
    }

    let mut families: Vec<FamilyCoverage> = Vec::new();
    for f in arr(t, "families") {
        let (fid, fname) = (s(f, "id"), s(f, "name"));
        let ids: Vec<String> = arr(f, "items").iter().filter_map(|x| x.as_str().map(short)).collect();
        let members: Vec<&Item> = ids.iter().filter_map(|id| find(id)).collect();
        let refs: Vec<&Item> = members.iter().copied().filter(|i| !i.anti).collect();
        let anti = members.len() - refs.len();
        let measures = arr(f, "measures");

        if refs.len() <= 1 {
            gap(
                "thin-family",
                "high",
                Some(fid),
                format!("{fname} rests on {}", if refs.is_empty() { "no reference".to_string() } else { "one reference".to_string() }),
                "One image cannot show what is constant in a family and what is chance. Every measure of this family is a guess from a single view.".into(),
                format!("Add two or three more references of {}, from other angles and of other objects.", s(f, "applies_to").trim().trim_end_matches('.')),
                refs.iter().map(|i| short(&i.id)).collect(),
            );
        }

        // Views
        let mut views: Vec<&'static str> = Vec::new();
        for i in &refs {
            for v in views_of(i, analyses.get(&i.id)) {
                if !views.contains(&v) {
                    views.push(v);
                }
            }
        }
        let mut missing: Vec<String> = Vec::new();
        if !views.contains(&"three-quarter") {
            missing.push("three-quarter".into());
        }
        if !views.contains(&"front") && !views.contains(&"side") {
            missing.push("front or side".into());
        }
        if !views.contains(&"detail") {
            missing.push("detail".into());
        }
        if !missing.is_empty() && !refs.is_empty() {
            gap(
                "missing-views",
                if missing.len() >= 2 { "medium" } else { "low" },
                Some(fid),
                format!("{fname} has no {} view", missing.join(", no ")),
                format!(
                    "The references of this family show: {}. Without {} an agent must guess {}.",
                    if views.is_empty() { "no named view".to_string() } else { views.join(", ") },
                    if missing.len() == 1 { "that view" } else { "those views" },
                    if missing.iter().any(|m| m == "detail") { "how edges and joints are made" } else { "the proportions" }
                ),
                format!(
                    "Add {}{}.",
                    missing.iter().map(|m| format!("a {m} view")).collect::<Vec<_>>().join(" and "),
                    if missing.iter().any(|m| m == "detail") { ", or mark a detail region on an image that shows one" } else { "" }
                ),
                refs.iter().map(|i| short(&i.id)).collect(),
            );
        }

        // Measures and rules
        if measures.is_empty() {
            gap(
                "no-measures",
                "medium",
                Some(fid),
                format!("{fname} has no measures"),
                "The family describes a look, but it gives an agent no number to model with.".into(),
                "Add references in which a size or a ratio is visible, or add a source that states values.".into(),
                Vec::new(),
            );
        }
        if arr(f, "rules").is_empty() {
            gap("no-rules", "low", Some(fid), format!("{fname} has no rules of its own"), "Only the principles of the language apply to it.".into(), "Write a note on a reference of this family that says what must always be true.".into(), Vec::new());
        }
        for m in measures {
            let (name, basis) = (s(m, "name"), s(m, "basis"));
            let evidence: Vec<String> = arr(m, "evidence").iter().filter_map(|x| x.as_str().map(short)).collect();
            let conf = m.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if basis == "process-dependent" {
                if match_rule(&sets, name).is_none() {
                    gap(
                        "no-hard-rule",
                        "high",
                        Some(fid),
                        format!("No hard rule sets \"{name}\""),
                        format!("{fname} leaves \"{name}\" to the process, but no rule set has a rule with that name. An agent gets no value."),
                        format!("Add a rule \"{name}\" to a rule set, and select that rule set in the project."),
                        Vec::new(),
                    );
                }
                continue;
            }
            let mut why: Vec<String> = Vec::new();
            if conf < 0.5 {
                why.push(format!("the confidence is {:.0} %", conf * 100.0));
            }
            if basis == "observed" && evidence.len() == 1 {
                why.push("one reference shows it".into());
            }
            if evidence.is_empty() {
                why.push("no reference backs it".into());
            }
            if !why.is_empty() {
                gap(
                    "weak-measure",
                    if evidence.is_empty() || conf < 0.35 { "medium" } else { "low" },
                    Some(fid),
                    format!("\"{name}\" in {fname} is weak"),
                    format!("{} ({}): {}.", crate::spec::fmt_value(m), basis, why.join(", ")),
                    format!("Add a reference where the measure is clear to see ({}), or state the value in a note or a source.", s(m, "of").trim().trim_end_matches('.')),
                    evidence,
                );
            }
        }

        // References that disagree: the same measurement with far-apart values.
        let mut by_name: BTreeMap<(String, String), Vec<(f64, String)>> = BTreeMap::new();
        let mut finishes: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
        for i in &refs {
            let Some(a) = analyses.get(&i.id) else { continue };
            for m in arr(a, "measurements") {
                if let (Some(v), unit) = (m.get("value").and_then(|v| v.as_f64()), s(m, "unit")) {
                    if !s(m, "name").is_empty() {
                        by_name.entry((norm(s(m, "name")), unit.to_string())).or_default().push((v, short(&i.id)));
                    }
                }
            }
            if let Some(c) = a.pointer("/surfaces/finish").and_then(|v| v.as_str()).and_then(finish_class) {
                finishes.entry(c).or_default().push(short(&i.id));
            }
        }
        for ((name, unit), vals) in &by_name {
            if vals.len() < 2 {
                continue;
            }
            let lo = vals.iter().cloned().fold((f64::MAX, String::new()), |a, b| if b.0 < a.0 { b } else { a });
            let hi = vals.iter().cloned().fold((f64::MIN, String::new()), |a, b| if b.0 > a.0 { b } else { a });
            let apart = if unit == "deg" { hi.0 - lo.0 >= 20.0 } else { lo.0 > 0.0 && hi.0 / lo.0 >= 1.6 };
            if apart {
                gap(
                    "contradiction",
                    "medium",
                    Some(fid),
                    format!("References of {fname} disagree on \"{name}\""),
                    format!("`{}` shows {} {unit}, `{}` shows {} {unit}. The document can give only one value or a wide range.", lo.1, fmt(lo.0), hi.1, fmt(hi.0)),
                    "Raise the weight of the reference that shows what you want, write the intended value in a note, or split the family in two.".into(),
                    vec![lo.1.clone(), hi.1.clone()],
                );
            }
        }
        if let (Some(m), Some(g)) = (finishes.get("matte"), finishes.get("gloss")) {
            gap(
                "contradiction",
                "medium",
                Some(fid),
                format!("References of {fname} disagree on the finish"),
                format!("{} matte ({}), {} gloss ({}).", m.len(), m.iter().map(|x| format!("`{x}`")).collect::<Vec<_>>().join(" "), g.len(), g.iter().map(|x| format!("`{x}`")).collect::<Vec<_>>().join(" ")),
                "Mark the references with the wrong finish as \"not this\", or say in a note which finish the language wants.".into(),
                m.iter().chain(g.iter()).cloned().collect(),
            );
        }

        families.push(FamilyCoverage {
            id: fid.to_string(),
            name: fname.to_string(),
            references: refs.len(),
            anti_references: anti,
            measures: measures.len(),
            rules: arr(f, "rules").len(),
            views: views.iter().map(|v| v.to_string()).collect(),
            missing_views: missing,
        });
    }

    // References that no part of the document uses.
    let unused: Vec<&Item> = own.iter().copied().filter(|i| i.analysis_status == "done" && !cited.contains(&short(&i.id))).collect();
    if !unused.is_empty() {
        gap(
            "unused-reference",
            "low",
            None,
            format!("{} reference{} on the board are not used by the document", unused.len(), if unused.len() == 1 { "" } else { "s" }),
            "No family, rule or measure cites them. They may be newer than the latest version, or the synthesis found no place for them.".into(),
            "Click Synthesize now. If they stay unused, add a note that says what each one shows.".into(),
            unused.iter().take(12).map(|i| short(&i.id)).collect(),
        );
    }
    if !own.is_empty() && !items.iter().any(|i| i.anti) {
        gap(
            "no-anti-reference",
            "low",
            None,
            "The language has no \"not this\" example".into(),
            "Without one, the document says what to do but not where the limit is.".into(),
            "Mark an image as \"not this\", or reject an agent output that misses the intent.".into(),
            Vec::new(),
        );
    }
    if t.pointer("/cmf/object").and_then(|v| v.as_array()).map(|a| a.is_empty()).unwrap_or(true) {
        gap(
            "no-colours",
            "low",
            None,
            "The language has no product colours".into(),
            "An agent that picks colours, materials or finishes gets nothing to go on.".into(),
            "Add references that show the product in its real colours, or state them in the description.".into(),
            Vec::new(),
        );
    }
    if !content.info.warnings.is_empty() {
        gap(
            "review-notes",
            "medium",
            None,
            format!("The automatic review left {} note{} on v{}", content.info.warnings.len(), if content.info.warnings.len() == 1 { "" } else { "s" }, content.info.version),
            content.info.warnings.iter().take(3).cloned().collect::<Vec<_>>().join(" "),
            "Open the Document tab, read the notes at the top, and edit tokens.json or synthesize again.".into(),
            Vec::new(),
        );
    }

    let rank = |g: &Gap| match g.severity.as_str() {
        "high" => 0,
        "medium" => 1,
        _ => 2,
    };
    gaps.sort_by_key(rank);
    Ok(Coverage { language: lang.name, version: Some(content.info.version), families, gaps, unused_references: unused.len() })
}

/// The report as text for an agent.
pub fn coverage_text(c: &Coverage) -> String {
    let mut out = format!(
        "COVERAGE of {}{}: {} gap{}.\n",
        c.language,
        c.version.map(|v| format!(" v{v}")).unwrap_or_default(),
        c.gaps.len(),
        if c.gaps.len() == 1 { "" } else { "s" }
    );
    if c.gaps.is_empty() {
        out.push_str("\nNo gaps found: every family has several references, the wanted views, and measures with evidence.\n");
    }
    if !c.families.is_empty() {
        out.push_str("\n| Family | References | Not this | Measures | Rules | Views shown | Views missing |\n| --- | --- | --- | --- | --- | --- | --- |\n");
        for f in &c.families {
            out.push_str(&format!(
                "| {} (`{}`) | {} | {} | {} | {} | {} | {} |\n",
                f.name,
                f.id,
                f.references,
                f.anti_references,
                f.measures,
                f.rules,
                if f.views.is_empty() { "—".to_string() } else { f.views.join(", ") },
                if f.missing_views.is_empty() { "—".to_string() } else { f.missing_views.join(", ") }
            ));
        }
    }
    for sev in ["high", "medium", "low"] {
        let list: Vec<&Gap> = c.gaps.iter().filter(|g| g.severity == sev).collect();
        if list.is_empty() {
            continue;
        }
        out.push_str(&format!("\n## {} gaps\n", match sev {
            "high" => "High: the language cannot answer this",
            "medium" => "Medium: the answer is weak",
            _ => "Low",
        }));
        for g in list {
            out.push_str(&format!("- {}. {}{}\n", g.title, g.detail, if g.evidence.is_empty() { String::new() } else { format!(" Items: {}.", g.evidence.iter().map(|e| format!("`{e}`")).collect::<Vec<_>>().join(" ")) }));
        }
    }
    out.push_str("\nWhere the language has a gap, do not invent a value: ask the designer, or use propose_amendment when the references or the hard rules show the answer.\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn png(seed: u8) -> Vec<u8> {
        let img = image::RgbImage::from_fn(40, 30, |x, y| image::Rgb([(x as u8).wrapping_mul(seed), y as u8, seed]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img).write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    #[test]
    fn gaps_of_a_small_language() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path()).unwrap();
        let lang = lib.create_language("Organic", "").unwrap();
        // Nothing yet.
        let c = report(&lib, "organic").unwrap();
        assert_eq!(c.gaps.len(), 1);
        assert_eq!(c.gaps[0].kind, "no-document");

        let add = |seed: u8, analysis: Value| {
            let i = lib.import_bytes(&lang.id, &format!("{seed}.png"), &png(seed), None).unwrap();
            lib.save_analysis(&i.id, &analysis, "m").unwrap();
            i
        };
        let a = add(3, json!({"summary": "A three-quarter view of a bench.", "surfaces": {"finish": "matte"}, "measurements": [{"name": "Edge fillet", "value": 0.3, "unit": "ratio"}]}));
        let b = add(5, json!({"summary": "Side view of a shell.", "view": "side", "surfaces": {"finish": "high gloss"}, "measurements": [{"name": "edge fillet", "value": 0.8, "unit": "ratio"}]}));
        let l = add(7, json!({"summary": "Blades.", "light_and_presentation": "Front view, studio light."}));
        let unused = add(9, json!({"summary": "Something else."}));
        let (a8, b8, l8) = (short(&a.id), short(&b.id), short(&l.id));

        let mut spec = crate::spec::tests::sample();
        spec["families"][0]["items"] = json!([a8, b8]);
        spec["families"][0]["measures"][0]["evidence"] = json!([a8]);
        spec["families"][1]["items"] = json!([l8]);
        spec["families"][1]["measures"][0]["evidence"] = json!([l8]);
        spec["principles"][0]["evidence"] = json!([a8]);
        spec["families"][0]["rules"][0]["evidence"] = json!([a8]);
        spec["families"][1]["rules"][0]["evidence"] = json!([l8]);
        spec["cmf"]["object"][0]["evidence"] = json!([a8]);
        spec["key_references"] = json!([{"item": a8, "why": "Best."}]);
        spec["anti_references"] = json!([{"item": "cccccccc", "lesson": "Gloss."}]);
        lib.save_spec_version(&lang.id, &spec, "m", &[], false, "h", &["check this".into()]).unwrap();

        let c = report(&lib, "organic").unwrap();
        let kinds: Vec<(&str, Option<&str>)> = c.gaps.iter().map(|g| (g.kind.as_str(), g.family.as_deref())).collect();
        // The blade field rests on one reference and has only a front view.
        assert!(kinds.contains(&("thin-family", Some("blade-field"))), "{kinds:?}");
        let views = c.gaps.iter().find(|g| g.kind == "missing-views" && g.family.as_deref() == Some("blade-field")).unwrap();
        assert!(views.title.contains("three-quarter") && views.title.contains("detail"), "{}", views.title);
        // The shell has a three-quarter and a side view, but no detail.
        let shell = c.families.iter().find(|f| f.id == "shell").unwrap();
        assert_eq!((shell.references, shell.missing_views.clone()), (2, vec!["detail".to_string()]));
        // The two shell references disagree on the fillet and on the finish.
        let contra: Vec<&Gap> = c.gaps.iter().filter(|g| g.kind == "contradiction").collect();
        assert_eq!(contra.len(), 2, "{contra:#?}");
        assert!(contra.iter().any(|g| g.title.contains("edge fillet") && g.detail.contains("0.3") && g.detail.contains("0.8")));
        assert!(contra.iter().any(|g| g.title.contains("finish")));
        // "blade pitch" is preferred at 40 %: weak. "edge fillet" is observed in one reference: weak.
        assert_eq!(c.gaps.iter().filter(|g| g.kind == "weak-measure").count(), 2);
        // The starter rule set fills "minimum wall": no gap for the process value.
        assert!(!kinds.iter().any(|k| k.0 == "no-hard-rule"));
        assert!(kinds.contains(&("stale-evidence", None)) && kinds.contains(&("review-notes", None)) && kinds.contains(&("no-anti-reference", None)));
        let un = c.gaps.iter().find(|g| g.kind == "unused-reference").unwrap();
        assert_eq!(un.evidence, vec![short(&unused.id)]);
        assert_eq!(c.unused_references, 1);
        // High gaps come first.
        assert_eq!(c.gaps[0].severity, "high");
        let text = coverage_text(&c);
        assert!(text.contains("COVERAGE of Organic v1") && text.contains("| Blade field (`blade-field`) | 1 |"), "{text}");

        // Without any rule set, the process value has no source.
        for r in lib.list_rule_sets().unwrap() {
            lib.delete_rule_set(&r.id).unwrap();
        }
        let c = report(&lib, "organic").unwrap();
        assert!(c.gaps.iter().any(|g| g.kind == "no-hard-rule" && g.title.contains("minimum wall")));
    }
}
