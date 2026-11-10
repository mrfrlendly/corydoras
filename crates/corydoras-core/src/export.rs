//! Export formats: a language version as CSS variables, as design tokens for
//! Figma (the W3C design-tokens form that Tokens Studio reads), or as a
//! parameter list for Grasshopper.
//!
//! A measure with a range gives two values (min and max). A process-dependent
//! measure takes its value from the hard rule with the same name, when the
//! rule sets that apply have one.

use crate::agent::match_rule;
use crate::models::RuleSet;
use crate::palette::parse_hex;
use crate::util::slugify;
use anyhow::{bail, Result};
use serde::Serialize;
use serde_json::{json, Map, Value};

pub const FORMATS: &[(&str, &str, &str)] = &[
    ("css", "CSS variables", "css"),
    ("figma", "Figma tokens (design tokens JSON)", "json"),
    ("grasshopper", "Grasshopper parameters (JSON)", "json"),
];

#[derive(Serialize, Clone, Debug)]
pub struct Exported {
    pub format: String,
    /// A file name to suggest, for example "wall-districts-v3.tokens.json".
    pub file_name: String,
    pub content: String,
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}

fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k).and_then(|x| x.as_array()).map(|a| a.as_slice()).unwrap_or(&[])
}

fn num(v: f64) -> String {
    let t = format!("{v:.4}");
    let t = t.trim_end_matches('0').trim_end_matches('.');
    if t.is_empty() || t == "-0" {
        "0".into()
    } else {
        t.to_string()
    }
}

/// One measure, ready to write.
struct Measure {
    family: String,
    family_name: String,
    name: String,
    unit: String,
    of: String,
    basis: String,
    confidence: Option<f64>,
    /// (low, high); equal for a single value. None: no value.
    range: Option<(f64, f64)>,
    /// Where the value comes from when it is not the language: "Minimum wall in FDM".
    from_rule: Option<String>,
}

fn measures(tokens: &Value, sets: &[RuleSet]) -> Vec<Measure> {
    let mut out = Vec::new();
    for f in arr(tokens, "families") {
        for m in arr(f, "measures") {
            let mut range = match m.get("value") {
                Some(Value::Number(n)) => n.as_f64().map(|x| (x, x)),
                Some(Value::Array(a)) if a.len() == 2 => a[0].as_f64().zip(a[1].as_f64()),
                _ => None,
            };
            let mut unit = s(m, "unit").to_string();
            let mut from_rule = None;
            if range.is_none() {
                if let Some((set, rule)) = match_rule(sets, s(m, "name")) {
                    if let Some(v) = rule.value {
                        range = Some((v, v));
                        unit = rule.unit.clone().unwrap_or(unit);
                        from_rule = Some(format!("\"{}\" in {}", rule.name, set.name));
                    }
                }
            }
            out.push(Measure {
                family: s(f, "id").to_string(),
                family_name: s(f, "name").to_string(),
                name: s(m, "name").to_string(),
                unit,
                of: s(m, "of").trim().to_string(),
                basis: s(m, "basis").to_string(),
                confidence: m.get("confidence").and_then(|v| v.as_f64()),
                range,
                from_rule,
            });
        }
    }
    out
}

struct Colour {
    name: String,
    hex: String,
    note: String,
    presentation: bool,
}

/// The colours with a unique name each: the role, plus a number when a role repeats.
fn colours(tokens: &Value) -> Vec<Colour> {
    let mut out: Vec<Colour> = Vec::new();
    let add = |role: &str, hex: &str, note: String, presentation: bool, out: &mut Vec<Colour>| {
        let base = slugify(if role.is_empty() { "colour" } else { role });
        let mut name = base.clone();
        let mut n = 2;
        while out.iter().any(|c| c.name == name && c.presentation == presentation) {
            name = format!("{base}-{n}");
            n += 1;
        }
        out.push(Colour { name, hex: hex.to_uppercase(), note, presentation });
    };
    for c in tokens.pointer("/cmf/object").and_then(|v| v.as_array()).into_iter().flatten() {
        let note = [s(c, "id"), s(c, "material"), s(c, "finish"), s(c, "ral")].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join(" · ");
        add(s(c, "role"), s(c, "hex"), note, false, &mut out);
    }
    for c in tokens.pointer("/cmf/presentation").and_then(|v| v.as_array()).into_iter().flatten() {
        add(s(c, "role"), s(c, "hex"), s(c, "note").trim().to_string(), true, &mut out);
    }
    out
}

fn css_unit(unit: &str) -> &'static str {
    match unit {
        "mm" => "mm",
        "deg" => "deg",
        "percent" => "%",
        _ => "",
    }
}

fn comment(text: &str) -> String {
    text.replace("*/", "* /").replace('\n', " ")
}

fn css(name: &str, tokens: &Value, sets: &[RuleSet]) -> String {
    let prefix = slugify(s(tokens, "language"));
    let version = tokens.get("version").and_then(|v| v.as_i64()).unwrap_or(0);
    let mut out = format!(
        "/* {} v{} — exported by Corydoras.\n   Lengths are millimetres (mm), angles degrees (deg). A ratio or a count has no unit.\n   A measure with a range has a -min and a -max variable. */\n:root {{\n",
        comment(name),
        version
    );
    let cols = colours(tokens);
    if cols.iter().any(|c| !c.presentation) {
        out.push_str("  /* Product colours */\n");
    }
    for c in cols.iter().filter(|c| !c.presentation) {
        out.push_str(&format!("  --{prefix}-color-{}: {};{}\n", c.name, c.hex, if c.note.is_empty() { String::new() } else { format!(" /* {} */", comment(&c.note)) }));
    }
    if cols.iter().any(|c| c.presentation) {
        out.push_str("  /* Presentation colours: for renders only, never product colours */\n");
    }
    for c in cols.iter().filter(|c| c.presentation) {
        out.push_str(&format!("  --{prefix}-presentation-{}: {};\n", c.name, c.hex));
    }
    let mut family = String::new();
    for m in measures(tokens, sets) {
        if m.family != family {
            out.push_str(&format!("  /* {} ({}) */\n", comment(&m.family_name), m.family));
            family = m.family.clone();
        }
        let var = format!("--{prefix}-{}-{}", slugify(&m.family), slugify(&m.name));
        let note = format!(
            "{}{}",
            comment(&m.of),
            match &m.from_rule {
                Some(r) => format!(" · from the hard rule {}", comment(r)),
                None => format!(" · {}", m.basis),
            }
        );
        match m.range {
            None => out.push_str(&format!("  /* {var}: set by the process; no hard rule gives a value. {} */\n", comment(&m.of))),
            Some((lo, hi)) if lo == hi => out.push_str(&format!("  {var}: {}{}; /* {note} */\n", num(lo), css_unit(&m.unit))),
            Some((lo, hi)) => {
                out.push_str(&format!("  {var}-min: {}{}; /* {note} */\n", num(lo), css_unit(&m.unit)));
                out.push_str(&format!("  {var}-max: {}{};\n", num(hi), css_unit(&m.unit)));
            }
        }
    }
    out.push_str("}\n");
    out
}

fn figma(name: &str, tokens: &Value, sets: &[RuleSet]) -> Result<String> {
    let version = tokens.get("version").and_then(|v| v.as_i64()).unwrap_or(0);
    let mut root = Map::new();
    root.insert("$description".into(), json!(format!("{name} v{version}, exported by Corydoras. Numbers are millimetres unless the description of a token says otherwise.")));
    let cols = colours(tokens);
    let mut color = Map::new();
    let mut presentation = Map::new();
    for c in &cols {
        let token = json!({"$type": "color", "$value": c.hex, "$description": c.note});
        if c.presentation {
            presentation.insert(c.name.clone(), token);
        } else {
            color.insert(c.name.clone(), token);
        }
    }
    if !color.is_empty() {
        root.insert("color".into(), Value::Object(color));
    }
    if !presentation.is_empty() {
        root.insert("presentation".into(), Value::Object(presentation));
    }
    for m in measures(tokens, sets) {
        let family = root.entry(m.family.clone()).or_insert_with(|| json!({"$description": m.family_name})).as_object_mut().expect("family group");
        let ext = json!({"corydoras": {"unit": m.unit, "of": m.of, "basis": m.basis, "confidence": m.confidence, "from_rule": m.from_rule}});
        let describe = |part: &str| format!("{}{}{}", m.of, if m.unit.is_empty() { String::new() } else { format!(" ({})", m.unit) }, part);
        let token = |v: f64, part: &str| json!({"$type": "number", "$value": v, "$description": describe(part), "$extensions": ext});
        match m.range {
            None => {}
            Some((lo, hi)) if lo == hi => {
                family.insert(m.name.clone(), token(lo, ""));
            }
            Some((lo, hi)) => {
                family.insert(m.name.clone(), json!({"min": token(lo, ", lower end"), "max": token(hi, ", upper end")}));
            }
        }
    }
    Ok(serde_json::to_string_pretty(&json!({ name: Value::Object(root) }))?)
}

fn grasshopper(name: &str, tokens: &Value, sets: &[RuleSet]) -> Result<String> {
    let mut params = Vec::new();
    for m in measures(tokens, sets) {
        let Some((lo, hi)) = m.range else { continue };
        let value = (lo + hi) / 2.0;
        // A slider needs a span: the range of the language, or half and one-and-a-half times a single value.
        let (smin, smax) = if lo == hi { (lo * 0.5, lo * 1.5) } else { (lo, hi) };
        params.push(json!({
            "name": format!("{} · {}", m.family_name, m.name),
            "nickname": slugify(&m.name).replace('-', "_"),
            "family": m.family,
            "measure": m.name,
            "value": value,
            "min": lo,
            "max": hi,
            "slider_min": smin.min(smax),
            "slider_max": smin.max(smax),
            "integer": m.unit == "count",
            "unit": m.unit,
            "of": m.of,
            "basis": m.basis,
            "confidence": m.confidence,
            "from_rule": m.from_rule,
        }));
    }
    let cols: Vec<Value> = colours(tokens)
        .iter()
        .map(|c| {
            let rgb = parse_hex(&c.hex).unwrap_or([0, 0, 0]);
            json!({"name": c.name, "hex": c.hex, "rgb": rgb, "presentation": c.presentation, "note": c.note})
        })
        .collect();
    let rules: Vec<Value> = sets
        .iter()
        .flat_map(|set| set.rules.iter().filter(|r| r.value.is_some()).map(move |r| json!({"name": r.name, "value": r.value, "unit": r.unit, "rule_set": set.name})))
        .collect();
    Ok(serde_json::to_string_pretty(&json!({
        "language": s(tokens, "language"),
        "name": name,
        "version": tokens.get("version"),
        "length_unit": "mm",
        "note": "Each parameter is one measure of the language. Make a number slider from slider_min to slider_max, set to value. hard_rules are limits that win over every parameter.",
        "parameters": params,
        "colours": cols,
        "hard_rules": rules,
    }))?)
}

/// Writes a language version (schema 2 tokens) in one of the export formats.
/// `sets` are the hard rule sets that fill process-dependent measures.
pub fn export(name: &str, tokens: &Value, sets: &[RuleSet], format: &str) -> Result<Exported> {
    if crate::spec::schema_of(tokens) < 2 {
        bail!("This version uses the older format, which has no measures to export. Synthesize a new version first.");
    }
    let version = tokens.get("version").and_then(|v| v.as_i64()).unwrap_or(0);
    let stem = format!("{}-v{version}", slugify(s(tokens, "language")));
    let (content, file_name) = match format {
        "css" => (css(name, tokens, sets), format!("{stem}.css")),
        "figma" => (figma(name, tokens, sets)?, format!("{stem}.tokens.json")),
        "grasshopper" => (grasshopper(name, tokens, sets)?, format!("{stem}.grasshopper.json")),
        other => bail!("Unknown export format \"{other}\". Use css, figma or grasshopper."),
    };
    Ok(Exported { format: format.to_string(), file_name, content })
}

/// Exports a version of a language. With a project, the rule sets of the
/// project fill the process-dependent measures; without one, the sets that
/// apply everywhere.
pub fn export_language(lib: &crate::Library, language_key: &str, version: Option<i64>, format: &str, project: Option<&str>) -> Result<Exported> {
    let lang = lib.find_language(language_key)?;
    let content = match version {
        Some(v) => lib.read_version(&lang.id, v)?,
        None => lib.read_latest(&lang.id)?.ok_or_else(|| anyhow::anyhow!("\"{}\" has no version yet.", lang.name))?,
    };
    let project = match project.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => Some(lib.find_project(p)?),
        None => None,
    };
    let sets = lib.effective_rule_sets(project.as_ref())?;
    export(&lang.name, &content.tokens, &sets, format)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Rule;

    fn tokens() -> Value {
        let mut t = crate::spec::tests::sample();
        crate::spec::finalize(&mut t, "organic", 3);
        t["cmf"]["object"].as_array_mut().unwrap().push(json!({"id": "C2", "role": "body", "hex": "#30363a", "material": "", "finish": "satin", "scope": "global"}));
        t
    }

    fn sets() -> Vec<RuleSet> {
        vec![RuleSet {
            id: "s".into(),
            name: "FDM".into(),
            description: String::new(),
            always: true,
            rules: vec![Rule { name: "Minimum wall".into(), value: Some(1.2), unit: Some("mm".into()), note: String::new() }],
            created_at: String::new(),
            updated_at: String::new(),
            project_count: 0,
        }]
    }

    #[test]
    fn css_variables() {
        let x = export("Organic", &tokens(), &sets(), "css").unwrap();
        assert_eq!(x.file_name, "organic-v3.css");
        let c = &x.content;
        assert!(c.starts_with("/* Organic v3"), "{c}");
        assert!(c.contains("  --organic-color-body: #E8E4DC; /* C1 · oak veneer · matte · RAL 9001 */"), "{c}");
        assert!(c.contains("  --organic-color-body-2: #30363A; /* C2 · satin */"));
        assert!(c.contains("  --organic-presentation-background: #F4F2EE;"));
        assert!(c.contains("  --organic-shell-edge-fillet-min: 0.3; /* shell thickness at the edge · observed */"));
        assert!(c.contains("  --organic-shell-edge-fillet-max: 0.5;"));
        assert!(c.contains("  --organic-blade-field-blade-pitch-min: 8mm;"));
        // The process value comes from the hard rule.
        assert!(c.contains("  --organic-shell-minimum-wall: 1.2mm; /* shell wall thickness · from the hard rule \"Minimum wall\" in FDM */"), "{c}");
        // Without a rule set, the variable is left out with a comment.
        let c = export("Organic", &tokens(), &[], "css").unwrap().content;
        assert!(c.contains("/* --organic-shell-minimum-wall: set by the process"));
        assert!(c.trim_end().ends_with('}'));
    }

    #[test]
    fn figma_and_grasshopper() {
        let x = export("Organic", &tokens(), &sets(), "figma").unwrap();
        assert_eq!(x.file_name, "organic-v3.tokens.json");
        let v: Value = serde_json::from_str(&x.content).unwrap();
        assert_eq!(v["Organic"]["color"]["body"]["$value"], "#E8E4DC");
        assert_eq!(v["Organic"]["color"]["body"]["$type"], "color");
        assert_eq!(v["Organic"]["shell"]["edge fillet"]["min"]["$value"], 0.3);
        assert_eq!(v["Organic"]["shell"]["minimum wall"]["$value"], 1.2);
        assert_eq!(v["Organic"]["shell"]["minimum wall"]["$extensions"]["corydoras"]["unit"], "mm");

        let x = export("Organic", &tokens(), &sets(), "grasshopper").unwrap();
        let v: Value = serde_json::from_str(&x.content).unwrap();
        let p = v["parameters"].as_array().unwrap();
        assert_eq!(p.len(), 3);
        let pitch = p.iter().find(|x| x["measure"] == "blade pitch").unwrap();
        assert_eq!((pitch["value"].as_f64(), pitch["slider_min"].as_f64(), pitch["slider_max"].as_f64()), (Some(9.0), Some(8.0), Some(10.0)));
        assert_eq!(pitch["nickname"], "blade_pitch");
        let wall = p.iter().find(|x| x["measure"] == "minimum wall").unwrap();
        assert_eq!((wall["value"].as_f64(), wall["slider_min"].as_f64(), wall["slider_max"].as_f64()), (Some(1.2), Some(0.6), Some(1.2 * 1.5)));
        assert_eq!(v["colours"][0]["rgb"], json!([232, 228, 220]));
        assert_eq!(v["hard_rules"][0]["name"], "Minimum wall");

        assert!(export("Organic", &tokens(), &sets(), "pdf").is_err());
        assert!(export("Organic", &json!({"geometry": {}}), &sets(), "css").is_err());
    }
}
