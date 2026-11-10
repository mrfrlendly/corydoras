//! Schema 2 of tokens.json: the structured design-language spec.
//!
//! The model writes only this JSON. The app renders language.md from it, so
//! the prose and the values cannot disagree. Every numeric value is a
//! "measure" with a unit, the exact thing it measures, a basis
//! (observed / preferred / process-dependent), a confidence, and evidence IDs.

use serde_json::{json, Value};
use std::collections::HashSet;

pub const SCHEMA_VERSION: i64 = 2;

pub const MEASURE_UNITS: &[&str] = &["mm", "ratio", "deg", "count", "percent"];
pub const MEASURE_BASES: &[&str] = &["observed", "preferred", "process-dependent"];
pub const RULE_BASES: &[&str] = &["observed", "designer", "inferred", "source"];
pub const PRESENTATION_ROLES: &[&str] = &["background", "light", "shadow", "surface", "other"];

/// The shape that the synthesis pass must return (example values).
pub fn output_shape() -> Value {
    json!({
        "summary": "Two to four sentences of intent. No numbers.",
        "principles": [
            {"id": "P1", "text": "A rule that holds across every family. No numbers.", "priority": 1, "basis": "observed | designer | inferred", "evidence": ["item id"]}
        ],
        "definitions": [
            {"term": "pitch", "meaning": "Centre-to-centre distance between adjacent blades."}
        ],
        "families": [
            {
                "id": "blade-field",
                "name": "Blade field",
                "applies_to": "Which objects or elements use this family (for example louvres, screens, grilles).",
                "intent": "What this family must look and feel like. No numbers.",
                "items": ["item id"],
                "measures": [
                    {"name": "blade pitch", "value": [8, 10], "unit": "mm | ratio | deg | count | percent", "of": "Exactly what is measured, and on which feature.", "basis": "observed | preferred | process-dependent", "confidence": 0.6, "evidence": ["item id"]}
                ],
                "rules": [
                    {"id": "BF1", "text": "A rule for this family only. No numbers.", "basis": "observed | designer | inferred", "evidence": ["item id"]}
                ]
            }
        ],
        "exceptions": [
            {"id": "X1", "overrides": "ID of the principle or rule that this exception overrides", "scope": "global | family id", "when": "The condition", "text": "What applies instead. No numbers.", "priority": 1}
        ],
        "cmf": {
            "object": [
                {"id": "C1", "role": "body | accent | detail | structure", "hex": "#RRGGBB", "ral": null, "pantone_approx": null, "material": "", "finish": "matte", "scope": "global | family id", "basis": "observed | designer | inferred", "evidence": ["item id"]}
            ],
            "presentation": [
                {"role": "background | light | shadow | surface | other", "hex": "#RRGGBB", "note": "For renders and concept images only. Never a product colour."}
            ]
        },
        "prompts": {
            "positive": "Comma-separated prompt fragments.",
            "negative": "Comma-separated negative prompt fragments.",
            "families": {"blade-field": "Prompt fragments for this family only."}
        },
        "do": [{"text": "", "scope": "global | family id"}],
        "dont": [{"text": "", "scope": "global | family id"}],
        "anti_references": [{"item": "item id", "lesson": "What the language must avoid, and why."}],
        "key_references": [{"item": "item id", "why": ""}]
    })
}

// ----------------------------------------------------------------------
// Validation
// ----------------------------------------------------------------------

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}

fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k).and_then(|x| x.as_array()).map(|a| a.as_slice()).unwrap_or(&[])
}

fn hex_ok(h: &str) -> bool {
    h.len() == 7 && h.starts_with('#') && h[1..].chars().all(|c| c.is_ascii_hexdigit())
}

fn ids_of(v: &Value, k: &str) -> Vec<String> {
    arr(v, k)
        .iter()
        .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
        .collect()
}

/// Finds a number with a unit (12 mm, 0.4×, 45°, 30 %, 1:1.6) in a text.
/// Numbers belong in measures; prose with numbers can drift from the tokens.
pub fn find_numeric_value(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        if chars[i].is_ascii_digit() && (i == 0 || !chars[i - 1].is_alphanumeric()) {
            let start = i;
            while i < n && (chars[i].is_ascii_digit() || ((chars[i] == '.' || chars[i] == ',') && i + 1 < n && chars[i + 1].is_ascii_digit())) {
                i += 1;
            }
            let mut j = i;
            while j < n && chars[j] == ' ' {
                j += 1;
            }
            let rest: String = chars[j..n.min(j + 4)].iter().collect();
            let lower = rest.to_lowercase();
            let after = |k: usize| chars.get(j + k).copied().unwrap_or(' ');
            let unit_hit = lower.starts_with("mm")
                || lower.starts_with("cm")
                || lower.starts_with("deg")
                || rest.starts_with('°')
                || rest.starts_with('%')
                || rest.starts_with('×')
                || (lower.starts_with('x') && !after(1).is_alphanumeric())
                || (rest.starts_with(':') && after(1).is_ascii_digit())
                || (rest.starts_with('–') || rest.starts_with('-')) && after(1).is_ascii_digit() && {
                    // A range such as "8–10 mm".
                    let mut k = j + 1;
                    while k < n && (chars[k].is_ascii_digit() || chars[k] == '.') {
                        k += 1;
                    }
                    while k < n && chars[k] == ' ' {
                        k += 1;
                    }
                    let r: String = chars[k..n.min(k + 2)].iter().collect::<String>().to_lowercase();
                    r.starts_with("mm") || r.starts_with('°') || r.starts_with('%') || r.starts_with("cm")
                };
            if unit_hit {
                let end = n.min(j + 6);
                let snippet: String = chars[start.saturating_sub(12)..end].iter().collect();
                return Some(snippet.trim().to_string());
            }
            continue;
        }
        i += 1;
    }
    None
}

// ----------------------------------------------------------------------
// Repair
// ----------------------------------------------------------------------

/// What `repair` changed. `fixed` are format corrections (logged only);
/// `flagged` change the meaning and are saved as review notes.
#[derive(Default, Debug)]
pub struct Repair {
    pub fixed: Vec<String>,
    pub flagged: Vec<String>,
}

fn parse_num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let t: String = s.trim().trim_end_matches(|c: char| c.is_alphabetic() || c == '%' || c == '°' || c == '×' || c == ' ').to_string();
            t.trim().replace(',', ".").parse::<f64>().ok()
        }
        _ => None,
    }
}

/// "8-10 mm", "8–10", "8 to 10" -> (8, 10).
fn parse_range_str(s: &str) -> Option<(f64, f64)> {
    let cleaned = s.replace(" to ", "-").replace('–', "-").replace('—', "-");
    let parts: Vec<&str> = cleaned.splitn(2, '-').collect();
    if parts.len() != 2 || parts[0].trim().is_empty() {
        return None;
    }
    let a = parse_num(&Value::String(parts[0].to_string()))?;
    let b = parse_num(&Value::String(parts[1].to_string()))?;
    Some((a, b))
}

fn norm_unit(u: &str) -> Option<&'static str> {
    let u = u.trim().to_lowercase();
    let u = u.as_str();
    Some(match u {
        "mm" | "millimetre" | "millimetres" | "millimeter" | "millimeters" => "mm",
        "ratio" | "x" | "×" | "fraction" | "proportion" | "factor" | "multiple" => "ratio",
        "deg" | "degree" | "degrees" | "°" => "deg",
        "count" | "number" | "n" | "pcs" | "pieces" | "qty" => "count",
        "percent" | "%" | "pct" | "percentage" => "percent",
        _ if u.starts_with("ratio") || u.contains(':') || u.contains(" / ") || u.contains(" of ") => "ratio",
        _ => return None,
    })
}

fn norm_measure_basis(b: &str) -> Option<&'static str> {
    let b = b.trim().to_lowercase();
    Some(match b.as_str() {
        "observed" | "measured" | "seen" => "observed",
        "preferred" | "estimated" | "estimate" | "inferred" | "suggested" | "recommended" | "designer" => "preferred",
        "process-dependent" | "process dependent" | "process_dependent" | "process" | "manufacturing" => "process-dependent",
        _ => return None,
    })
}

fn norm_rule_basis(b: &str) -> &'static str {
    match b.trim().to_lowercase().as_str() {
        "observed" | "measured" | "seen" => "observed",
        "designer" | "designer note" | "stated" | "intent" => "designer",
        "source" | "tutorial" | "document" | "article" | "reference" => "source",
        _ => "inferred",
    }
}

fn norm_hex(h: &str) -> Option<String> {
    let t = h.trim().trim_start_matches('#');
    let t = if t.len() == 3 && t.chars().all(|c| c.is_ascii_hexdigit()) {
        t.chars().flat_map(|c| [c, c]).collect::<String>()
    } else {
        t.to_string()
    };
    if t.len() == 6 && t.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(format!("#{}", t.to_uppercase()))
    } else {
        None
    }
}

fn ensure_array(obj: &mut serde_json::Map<String, Value>, key: &str, r: &mut Repair) {
    match obj.get(key) {
        Some(Value::Array(_)) => {}
        Some(Value::Null) | None => {
            obj.insert(key.into(), json!([]));
        }
        Some(other) => {
            let one = other.clone();
            obj.insert(key.into(), json!([one]));
            r.fixed.push(format!("\"{key}\" was not a list"));
        }
    }
}

fn string_list(v: &mut Value) {
    match v {
        Value::String(s) => *v = json!([s.clone()]),
        Value::Array(a) => {
            a.retain(|x| x.is_string());
        }
        _ => *v = json!([]),
    }
}

fn text_object(v: &mut Value, key: &str) {
    if let Value::String(s) = v {
        *v = json!({ key: s.clone() });
    }
}

fn get_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.round() as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Fixes small format problems in a model answer, in place.
pub fn repair(t: &mut Value) -> Repair {
    let mut r = Repair::default();
    let Some(obj) = t.as_object_mut() else { return r };

    for key in ["principles", "definitions", "families", "exceptions", "do", "dont", "anti_references", "key_references"] {
        ensure_array(obj, key, &mut r);
    }

    // CMF: schema 1 used a flat list.
    match obj.get("cmf").cloned() {
        Some(Value::Array(a)) => {
            obj.insert("cmf".into(), json!({"object": a, "presentation": []}));
            r.fixed.push("\"cmf\" was a list".into());
        }
        Some(Value::Object(_)) => {}
        _ => {
            obj.insert("cmf".into(), json!({"object": [], "presentation": []}));
        }
    }
    if let Some(cmf) = obj.get_mut("cmf").and_then(|c| c.as_object_mut()) {
        ensure_array(cmf, "object", &mut r);
        ensure_array(cmf, "presentation", &mut r);
        let mut objects: Vec<Value> = Vec::new();
        for (i, mut o) in cmf.get("object").and_then(|v| v.as_array()).cloned().unwrap_or_default().into_iter().enumerate() {
            let Some(m) = o.as_object_mut() else { continue };
            match m.get("hex").and_then(|h| h.as_str()).and_then(norm_hex) {
                Some(h) => {
                    m.insert("hex".into(), json!(h));
                }
                None => {
                    r.flagged.push(format!("Dropped product colour {} ({}): no valid hex value.", i + 1, m.get("role").and_then(|x| x.as_str()).unwrap_or("?")));
                    continue;
                }
            }
            if m.get("role").and_then(|x| x.as_str()).map(|s| s.trim().is_empty()).unwrap_or(true) {
                m.insert("role".into(), json!("detail"));
            }
            if let Some(ev) = m.get_mut("evidence") {
                string_list(ev);
            }
            if let Some(b) = m.get("basis").and_then(|x| x.as_str()).map(norm_rule_basis) {
                m.insert("basis".into(), json!(b));
            }
            objects.push(o);
        }
        cmf.insert("object".into(), Value::Array(objects));
        let mut pres: Vec<Value> = Vec::new();
        for mut p in cmf.get("presentation").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
            let Some(m) = p.as_object_mut() else { continue };
            let Some(h) = m.get("hex").and_then(|h| h.as_str()).and_then(norm_hex) else {
                r.fixed.push("dropped a presentation colour without a valid hex value".into());
                continue;
            };
            m.insert("hex".into(), json!(h));
            let role = m.get("role").and_then(|x| x.as_str()).unwrap_or("").trim().to_lowercase();
            let role = if PRESENTATION_ROLES.contains(&role.as_str()) {
                role
            } else if role.contains("background") || role.contains("backdrop") {
                "background".into()
            } else if role.contains("shadow") {
                "shadow".into()
            } else if role.contains("light") || role.contains("highlight") {
                "light".into()
            } else {
                "other".into()
            };
            m.insert("role".into(), json!(role));
            pres.push(p);
        }
        cmf.insert("presentation".into(), Value::Array(pres));
    }

    // Prompts
    match obj.get_mut("prompts") {
        Some(Value::Object(p)) => {
            for k in ["positive", "negative"] {
                match p.get(k) {
                    Some(Value::String(_)) => {}
                    Some(Value::Array(a)) => {
                        let joined = a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", ");
                        p.insert(k.into(), json!(joined));
                        r.fixed.push(format!("\"prompts.{k}\" was a list"));
                    }
                    _ => {
                        p.insert(k.into(), json!(""));
                        r.flagged.push(format!("The model gave no {k} prompt."));
                    }
                }
            }
        }
        _ => {
            obj.insert("prompts".into(), json!({"positive": "", "negative": ""}));
            r.flagged.push("The model gave no image-generation prompts.".into());
        }
    }

    // Do / don't, references
    for key in ["do", "dont"] {
        if let Some(Value::Array(a)) = obj.get_mut(key) {
            for d in a.iter_mut() {
                text_object(d, "text");
                if let Some(m) = d.as_object_mut() {
                    if m.get("scope").and_then(|x| x.as_str()).map(|s| s.trim().is_empty()).unwrap_or(true) {
                        m.insert("scope".into(), json!("global"));
                    }
                }
            }
        }
    }
    for key in ["anti_references", "key_references"] {
        if let Some(Value::Array(a)) = obj.get_mut(key) {
            for d in a.iter_mut() {
                text_object(d, "item");
                if let Some(m) = d.as_object_mut() {
                    if !m.contains_key("item") {
                        if let Some(id) = m.remove("item_id") {
                            m.insert("item".into(), id);
                        }
                    }
                }
            }
        }
    }

    // Principles
    if let Some(Value::Array(ps)) = obj.get_mut("principles") {
        for (i, p) in ps.iter_mut().enumerate() {
            text_object(p, "text");
            let Some(m) = p.as_object_mut() else { continue };
            if m.get("id").and_then(|x| x.as_str()).map(|s| s.trim().is_empty()).unwrap_or(true) {
                m.insert("id".into(), json!(format!("P{}", i + 1)));
                r.fixed.push("a principle had no ID".into());
            }
            let prio = m.get("priority").and_then(get_i64).filter(|p| *p >= 1).unwrap_or(i as i64 + 1);
            m.insert("priority".into(), json!(prio));
            let basis = norm_rule_basis(m.get("basis").and_then(|x| x.as_str()).unwrap_or(""));
            m.insert("basis".into(), json!(basis));
            m.entry("evidence").or_insert(json!([]));
            if let Some(ev) = m.get_mut("evidence") {
                string_list(ev);
            }
        }
    }

    // Definitions
    if let Some(Value::Array(ds)) = obj.get_mut("definitions") {
        ds.retain(|d| {
            d.get("term").and_then(|x| x.as_str()).map(|s| !s.trim().is_empty()).unwrap_or(false)
                && d.get("meaning").and_then(|x| x.as_str()).map(|s| !s.trim().is_empty()).unwrap_or(false)
        });
    }

    // Families
    if let Some(Value::Array(fs)) = obj.get_mut("families") {
        for (fi, f) in fs.iter_mut().enumerate() {
            let Some(m) = f.as_object_mut() else { continue };
            let name = m.get("name").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
            let id = m.get("id").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
            if id.is_empty() {
                let new_id = if name.is_empty() { format!("family-{}", fi + 1) } else { crate::util::slugify(&name) };
                m.insert("id".into(), json!(new_id));
                r.fixed.push("a family had no ID".into());
            }
            let fid = m.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
            if name.is_empty() {
                m.insert("name".into(), json!(fid.clone()));
            }
            if m.get("applies_to").and_then(|x| x.as_str()).map(|s| s.trim().is_empty()).unwrap_or(true) {
                let fallback = match m.get("applies_to") {
                    Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", "),
                    _ => String::new(),
                };
                if fallback.is_empty() {
                    m.insert("applies_to".into(), json!("Not stated"));
                    r.flagged.push(format!("Family {fid} does not say which objects it applies to."));
                } else {
                    m.insert("applies_to".into(), json!(fallback));
                }
            }
            for key in ["items", "measures", "rules"] {
                ensure_array(m, key, &mut r);
            }
            if let Some(items) = m.get_mut("items") {
                string_list(items);
            }
            // Rules
            if let Some(Value::Array(rs)) = m.get_mut("rules") {
                for (ri, rule) in rs.iter_mut().enumerate() {
                    text_object(rule, "text");
                    let Some(rm) = rule.as_object_mut() else { continue };
                    if rm.get("id").and_then(|x| x.as_str()).map(|s| s.trim().is_empty()).unwrap_or(true) {
                        let prefix: String = fid.split('-').filter_map(|w| w.chars().next()).collect::<String>().to_uppercase();
                        rm.insert("id".into(), json!(format!("{}{}", if prefix.is_empty() { "R".into() } else { prefix }, ri + 1)));
                        r.fixed.push("a rule had no ID".into());
                    }
                    let basis = norm_rule_basis(rm.get("basis").and_then(|x| x.as_str()).unwrap_or(""));
                    rm.insert("basis".into(), json!(basis));
                    rm.entry("evidence").or_insert(json!([]));
                    if let Some(ev) = rm.get_mut("evidence") {
                        string_list(ev);
                    }
                }
            }
            // Measures
            let measures = m.get("measures").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let mut kept: Vec<Value> = Vec::new();
            for mut ms in measures {
                let Some(mm) = ms.as_object_mut() else { continue };
                let mname = mm.get("name").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
                if mname.is_empty() {
                    r.fixed.push(format!("dropped a measure without a name in {fid}"));
                    continue;
                }
                // Basis
                let basis = mm
                    .get("basis")
                    .and_then(|x| x.as_str())
                    .and_then(norm_measure_basis)
                    .unwrap_or("preferred");
                mm.insert("basis".into(), json!(basis));
                // Unit
                match mm.get("unit").and_then(|x| x.as_str()).and_then(norm_unit) {
                    Some(u) => {
                        mm.insert("unit".into(), json!(u));
                    }
                    None => {
                        r.flagged.push(format!(
                            "Dropped measure \"{mname}\" in {fid}: unknown unit \"{}\".",
                            mm.get("unit").and_then(|x| x.as_str()).unwrap_or("")
                        ));
                        continue;
                    }
                }
                // Value
                let value = mm.get("value").cloned().unwrap_or(Value::Null);
                let fixed_value: Option<Value> = match &value {
                    Value::Null => None,
                    Value::Number(_) => Some(value.clone()),
                    Value::String(s) => parse_range_str(s)
                        .map(|(a, b)| json!([a.min(b), a.max(b)]))
                        .or_else(|| parse_num(&value).map(|x| json!(x))),
                    Value::Array(a) if a.len() == 2 => match (parse_num(&a[0]), parse_num(&a[1])) {
                        (Some(x), Some(y)) => Some(json!([x.min(y), x.max(y)])),
                        _ => None,
                    },
                    Value::Array(a) if a.len() == 1 => parse_num(&a[0]).map(|x| json!(x)),
                    Value::Object(o) => match (o.get("min").and_then(parse_num), o.get("max").and_then(parse_num)) {
                        (Some(x), Some(y)) => Some(json!([x.min(y), x.max(y)])),
                        _ => None,
                    },
                    _ => None,
                };
                if basis == "process-dependent" {
                    if fixed_value.is_some() {
                        r.fixed.push(format!("removed the value of process-dependent measure \"{mname}\""));
                    }
                    mm.insert("value".into(), Value::Null);
                } else {
                    match fixed_value {
                        Some(v) => {
                            mm.insert("value".into(), v);
                        }
                        None => {
                            r.flagged.push(format!("Dropped measure \"{mname}\" in {fid}: it has no usable value."));
                            continue;
                        }
                    }
                }
                // Of
                if mm.get("of").and_then(|x| x.as_str()).map(|s| s.trim().is_empty()).unwrap_or(true) {
                    mm.insert("of".into(), json!(mname.clone()));
                    r.flagged.push(format!("Measure \"{mname}\" in {fid} does not say exactly what it measures."));
                }
                // Confidence
                let conf = mm.get("confidence").and_then(parse_num);
                let conf = match conf {
                    Some(c) if c > 1.0 && c <= 100.0 => c / 100.0,
                    Some(c) if (0.0..=1.0).contains(&c) => c,
                    _ => {
                        r.fixed.push(format!("set a default confidence on \"{mname}\""));
                        0.5
                    }
                };
                mm.insert("confidence".into(), json!(conf));
                mm.entry("evidence").or_insert(json!([]));
                if let Some(ev) = mm.get_mut("evidence") {
                    string_list(ev);
                }
                kept.push(ms);
            }
            m.insert("measures".into(), Value::Array(kept));
        }
    }

    // Exceptions
    if let Some(Value::Array(xs)) = obj.get_mut("exceptions") {
        for (i, x) in xs.iter_mut().enumerate() {
            text_object(x, "text");
            let Some(m) = x.as_object_mut() else { continue };
            if m.get("id").and_then(|v| v.as_str()).map(|s| s.trim().is_empty()).unwrap_or(true) {
                m.insert("id".into(), json!(format!("X{}", i + 1)));
            }
            let prio = m.get("priority").and_then(get_i64).filter(|p| *p >= 1).unwrap_or(1);
            m.insert("priority".into(), json!(prio));
            if m.get("scope").and_then(|v| v.as_str()).map(|s| s.trim().is_empty()).unwrap_or(true) {
                m.insert("scope".into(), json!("global"));
            }
            if m.get("when").and_then(|v| v.as_str()).map(|s| s.trim().is_empty()).unwrap_or(true) {
                m.insert("when".into(), json!("Not stated"));
                r.flagged.push(format!("Exception {} does not say when it applies.", m.get("id").and_then(|v| v.as_str()).unwrap_or("?")));
            }
            if m.get("overrides").and_then(|v| v.as_str()).is_none() {
                m.insert("overrides".into(), json!(""));
            }
        }
    }

    // Duplicate IDs: rename later ones.
    let mut seen: HashSet<String> = HashSet::new();
    let mut fix_id = |m: &mut serde_json::Map<String, Value>, r: &mut Repair| {
        if let Some(id) = m.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()) {
            if !seen.insert(id.clone()) {
                let mut n = 2;
                while seen.contains(&format!("{id}-{n}")) {
                    n += 1;
                }
                let new_id = format!("{id}-{n}");
                seen.insert(new_id.clone());
                m.insert("id".into(), json!(new_id));
                r.fixed.push(format!("renamed duplicate ID {id}"));
            }
        }
    };
    if let Some(Value::Array(ps)) = obj.get_mut("principles") {
        for p in ps.iter_mut().filter_map(|p| p.as_object_mut()) {
            fix_id(p, &mut r);
        }
    }
    if let Some(Value::Array(fs)) = obj.get_mut("families") {
        for f in fs.iter_mut().filter_map(|f| f.as_object_mut()) {
            fix_id(f, &mut r);
            if let Some(Value::Array(rs)) = f.get_mut("rules") {
                for rule in rs.iter_mut().filter_map(|x| x.as_object_mut()) {
                    fix_id(rule, &mut r);
                }
            }
        }
    }
    if let Some(Value::Array(os)) = obj.get_mut("cmf").and_then(|c| c.get_mut("object")) {
        for o in os.iter_mut().filter_map(|x| x.as_object_mut()) {
            fix_id(o, &mut r);
        }
    }
    if let Some(Value::Array(xs)) = obj.get_mut("exceptions") {
        for x in xs.iter_mut().filter_map(|x| x.as_object_mut()) {
            fix_id(x, &mut r);
        }
    }
    r
}

/// Result of a check: hard errors make the spec unusable; warnings are saved
/// with the version and shown to the designer.
#[derive(Default, Debug)]
pub struct Check {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// Validates a schema 2 spec. `known_items` are the 8-character item IDs
/// that the synthesis pass received.
fn dup(set: &mut HashSet<String>, id: &str, errs: &mut Vec<String>) {
    if !set.insert(id.to_string()) {
        errs.push(format!("The ID \"{id}\" is used more than one time."));
    }
}

pub fn validate(t: &Value, known_items: &[String]) -> Check {
    let mut c = Check::default();
    let known: HashSet<&str> = known_items.iter().map(|s| s.as_str()).collect();
    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    if !t.is_object() {
        c.errors.push("The spec must be a JSON object.".into());
        return c;
    }
    if s(t, "summary").trim().is_empty() {
        errors.push("\"summary\" must be a non-empty string.".into());
    }

    let mut all_ids: HashSet<String> = HashSet::new();
    let mut narrative: Vec<(String, String)> = vec![("summary".into(), s(t, "summary").to_string())];
    let mut evidence_refs: Vec<(String, String)> = Vec::new();

    // Principles
    match t.get("principles").and_then(|v| v.as_array()) {
        None => errors.push("\"principles\" must be an array.".into()),
        Some(ps) => {
            for (i, p) in ps.iter().enumerate() {
                let id = s(p, "id");
                if id.is_empty() || s(p, "text").trim().is_empty() {
                    errors.push(format!("principles[{i}] needs \"id\" and \"text\"."));
                    continue;
                }
                dup(&mut all_ids, id, &mut errors);
                if p.get("priority").and_then(|v| v.as_i64()).map(|v| v < 1).unwrap_or(true) {
                    errors.push(format!("Principle {id}: \"priority\" must be an integer of 1 or more."));
                }
                if !RULE_BASES.contains(&s(p, "basis")) {
                    errors.push(format!("Principle {id}: \"basis\" must be observed, designer, inferred or source."));
                }
                narrative.push((format!("principle {id}"), s(p, "text").to_string()));
                for e in ids_of(p, "evidence") {
                    evidence_refs.push((format!("principle {id}"), e));
                }
            }
        }
    }

    // Definitions
    match t.get("definitions").and_then(|v| v.as_array()) {
        None => errors.push("\"definitions\" must be an array.".into()),
        Some(ds) => {
            for (i, d) in ds.iter().enumerate() {
                if s(d, "term").trim().is_empty() || s(d, "meaning").trim().is_empty() {
                    errors.push(format!("definitions[{i}] needs \"term\" and \"meaning\"."));
                }
            }
        }
    }

    // Families
    let mut family_ids: HashSet<String> = HashSet::new();
    match t.get("families").and_then(|v| v.as_array()) {
        None => errors.push("\"families\" must be an array.".into()),
        Some(fs) if fs.is_empty() => errors.push("\"families\" needs at least one family.".into()),
        Some(fs) => {
            for (i, f) in fs.iter().enumerate() {
                let fid = s(f, "id").to_string();
                if fid.is_empty() || s(f, "name").is_empty() || s(f, "applies_to").is_empty() {
                    errors.push(format!("families[{i}] needs \"id\", \"name\" and \"applies_to\"."));
                    continue;
                }
                dup(&mut all_ids, &fid, &mut errors);
                family_ids.insert(fid.clone());
                narrative.push((format!("family {fid} intent"), s(f, "intent").to_string()));
                for e in ids_of(f, "items") {
                    evidence_refs.push((format!("family {fid}"), e));
                }
                match f.get("measures").and_then(|v| v.as_array()) {
                    None => errors.push(format!("Family {fid}: \"measures\" must be an array.")),
                    Some(ms) => {
                        for (k, m) in ms.iter().enumerate() {
                            let label = format!("Family {fid}, measure {}", if s(m, "name").is_empty() { k.to_string() } else { format!("\"{}\"", s(m, "name")) });
                            if s(m, "name").trim().is_empty() {
                                errors.push(format!("{label}: needs a \"name\"."));
                            }
                            if !MEASURE_UNITS.contains(&s(m, "unit")) {
                                errors.push(format!("{label}: \"unit\" must be one of {}.", MEASURE_UNITS.join(", ")));
                            }
                            if s(m, "of").trim().is_empty() {
                                errors.push(format!("{label}: \"of\" must say exactly what is measured."));
                            }
                            let basis = s(m, "basis");
                            if !MEASURE_BASES.contains(&basis) {
                                errors.push(format!("{label}: \"basis\" must be observed, preferred or process-dependent."));
                            }
                            let value = m.get("value").unwrap_or(&Value::Null);
                            let value_ok = match value {
                                Value::Null => true,
                                Value::Number(_) => true,
                                Value::Array(a) => {
                                    a.len() == 2
                                        && a[0].as_f64().is_some()
                                        && a[1].as_f64().is_some()
                                        && a[0].as_f64() <= a[1].as_f64()
                                }
                                _ => false,
                            };
                            if !value_ok {
                                errors.push(format!("{label}: \"value\" must be a number, [min, max] with min ≤ max, or null."));
                            }
                            if basis == "process-dependent" && !value.is_null() {
                                errors.push(format!("{label}: a process-dependent measure must have \"value\": null. The process profile supplies it."));
                            }
                            if basis != "process-dependent" && value.is_null() {
                                errors.push(format!("{label}: only a process-dependent measure can have a null value."));
                            }
                            match m.get("confidence").and_then(|v| v.as_f64()) {
                                Some(x) if (0.0..=1.0).contains(&x) => {}
                                _ => errors.push(format!("{label}: \"confidence\" must be a number from 0 to 1.")),
                            }
                            let ev = ids_of(m, "evidence");
                            if basis == "observed" && ev.is_empty() {
                                warnings.push(format!("{label} is marked observed but has no evidence IDs."));
                            }
                            for e in ev {
                                evidence_refs.push((label.clone(), e));
                            }
                        }
                    }
                }
                match f.get("rules").and_then(|v| v.as_array()) {
                    None => errors.push(format!("Family {fid}: \"rules\" must be an array.")),
                    Some(rs) => {
                        for (k, r) in rs.iter().enumerate() {
                            let rid = s(r, "id");
                            if rid.is_empty() || s(r, "text").trim().is_empty() {
                                errors.push(format!("Family {fid}, rules[{k}] needs \"id\" and \"text\"."));
                                continue;
                            }
                            dup(&mut all_ids, rid, &mut errors);
                            if !RULE_BASES.contains(&s(r, "basis")) {
                                errors.push(format!("Rule {rid}: \"basis\" must be observed, designer, inferred or source."));
                            }
                            narrative.push((format!("rule {rid}"), s(r, "text").to_string()));
                            for e in ids_of(r, "evidence") {
                                evidence_refs.push((format!("rule {rid}"), e));
                            }
                        }
                    }
                }
            }
        }
    }

    // CMF
    match t.get("cmf") {
        Some(cmf) if cmf.is_object() => {
            match cmf.get("object").and_then(|v| v.as_array()) {
                None => errors.push("\"cmf.object\" must be an array.".into()),
                Some(os) => {
                    for (i, o) in os.iter().enumerate() {
                        let label = format!("cmf.object[{i}]");
                        if s(o, "role").is_empty() {
                            errors.push(format!("{label} needs a \"role\"."));
                        }
                        if !hex_ok(s(o, "hex")) {
                            errors.push(format!("{label}: \"hex\" must look like #RRGGBB."));
                        }
                        let id = s(o, "id");
                        if !id.is_empty() {
                            dup(&mut all_ids, id, &mut errors);
                        }
                        let scope = s(o, "scope");
                        if !scope.is_empty() && scope != "global" && !family_ids.contains(scope) {
                            warnings.push(format!("{label}: scope \"{scope}\" is not a family ID."));
                        }
                        for e in ids_of(o, "evidence") {
                            evidence_refs.push((label.clone(), e));
                        }
                    }
                }
            }
            match cmf.get("presentation").and_then(|v| v.as_array()) {
                None => errors.push("\"cmf.presentation\" must be an array.".into()),
                Some(ps) => {
                    for (i, p) in ps.iter().enumerate() {
                        if !PRESENTATION_ROLES.contains(&s(p, "role")) {
                            errors.push(format!("cmf.presentation[{i}]: \"role\" must be one of {}.", PRESENTATION_ROLES.join(", ")));
                        }
                        if !hex_ok(s(p, "hex")) {
                            errors.push(format!("cmf.presentation[{i}]: \"hex\" must look like #RRGGBB."));
                        }
                    }
                }
            }
        }
        _ => errors.push("\"cmf\" must be an object with \"object\" and \"presentation\" arrays.".into()),
    }

    // Exceptions (after all IDs are known)
    match t.get("exceptions").and_then(|v| v.as_array()) {
        None => errors.push("\"exceptions\" must be an array.".into()),
        Some(xs) => {
            for (i, x) in xs.iter().enumerate() {
                let xid = s(x, "id");
                if xid.is_empty() || s(x, "text").trim().is_empty() || s(x, "when").trim().is_empty() {
                    errors.push(format!("exceptions[{i}] needs \"id\", \"when\" and \"text\"."));
                    continue;
                }
                if x.get("priority").and_then(|v| v.as_i64()).map(|v| v < 1).unwrap_or(true) {
                    errors.push(format!("Exception {xid}: \"priority\" must be an integer of 1 or more."));
                }
                let target = s(x, "overrides");
                if target.is_empty() {
                    errors.push(format!("Exception {xid}: \"overrides\" must name the ID that it overrides."));
                } else if !all_ids.contains(target) {
                    warnings.push(format!("Exception {xid} overrides \"{target}\", but no principle, rule or colour has that ID."));
                }
                let scope = s(x, "scope");
                if scope != "global" && !family_ids.contains(scope) {
                    warnings.push(format!("Exception {xid}: scope \"{scope}\" is not \"global\" or a family ID."));
                }
                narrative.push((format!("exception {xid}"), s(x, "text").to_string()));
                narrative.push((format!("exception {xid} condition"), s(x, "when").to_string()));
            }
            for x in xs {
                let xid = s(x, "id");
                if !xid.is_empty() {
                    dup(&mut all_ids, xid, &mut errors);
                }
            }
        }
    }

    // Prompts
    match t.get("prompts") {
        Some(p) if p.get("positive").map(|v| v.is_string()).unwrap_or(false) && p.get("negative").map(|v| v.is_string()).unwrap_or(false) => {
            if let Some(fam) = p.get("families").and_then(|v| v.as_object()) {
                for k in fam.keys() {
                    if !family_ids.contains(k) {
                        warnings.push(format!("prompts.families has \"{k}\", which is not a family ID."));
                    }
                }
            }
        }
        _ => errors.push("\"prompts\" must have \"positive\" and \"negative\" strings.".into()),
    }

    // Do / don't
    for key in ["do", "dont"] {
        match t.get(key).and_then(|v| v.as_array()) {
            None => errors.push(format!("\"{key}\" must be an array.")),
            Some(list) => {
                for (i, d) in list.iter().enumerate() {
                    if s(d, "text").trim().is_empty() {
                        errors.push(format!("{key}[{i}] needs \"text\"."));
                    }
                    narrative.push((format!("{key}[{i}]"), s(d, "text").to_string()));
                    let scope = s(d, "scope");
                    if !scope.is_empty() && scope != "global" && !family_ids.contains(scope) {
                        warnings.push(format!("{key}[{i}]: scope \"{scope}\" is not \"global\" or a family ID."));
                    }
                }
            }
        }
    }

    for key in ["anti_references", "key_references"] {
        for r in arr(t, key) {
            let item = s(r, "item");
            if !item.is_empty() {
                evidence_refs.push((key.to_string(), item.to_string()));
            }
        }
    }

    // Soft checks
    for (where_, text) in &narrative {
        if let Some(snip) = find_numeric_value(text) {
            warnings.push(format!("{where_} has a number in its text (\"{snip}\"). Numbers belong in measures."));
        }
    }
    if !known.is_empty() {
        let mut bad: Vec<String> = Vec::new();
        for (where_, id) in &evidence_refs {
            let short: String = id.chars().take(8).collect();
            if !known.contains(short.as_str()) {
                bad.push(format!("{where_} cites \"{id}\""));
            }
        }
        if !bad.is_empty() {
            warnings.push(format!(
                "Unknown evidence IDs (not in the references): {}.",
                bad.into_iter().take(8).collect::<Vec<_>>().join("; ")
            ));
        }
    }

    c.errors = errors;
    c.warnings = warnings;
    c
}

/// Adds the fields that the app owns.
pub fn finalize(t: &mut Value, language_id: &str, version: i64) {
    if let Some(o) = t.as_object_mut() {
        o.insert("schema".into(), json!(SCHEMA_VERSION));
        o.insert("language".into(), json!(language_id));
        o.insert("version".into(), json!(version));
    }
}

pub fn schema_of(t: &Value) -> i64 {
    t.get("schema").and_then(|v| v.as_i64()).unwrap_or(1)
}

// ----------------------------------------------------------------------
// Markdown
// ----------------------------------------------------------------------

/// Which part of the spec to render.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    All,
    Cad,
    Cmf,
    Prompts,
}

impl Scope {
    pub fn parse(s: &str) -> Self {
        match s {
            "cad" => Scope::Cad,
            "cmf" => Scope::Cmf,
            "prompts" => Scope::Prompts,
            _ => Scope::All,
        }
    }
}

fn fmt_num(x: f64) -> String {
    let s = format!("{x:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s.is_empty() || s == "-" {
        "0".into()
    } else {
        s
    }
}

pub fn fmt_value(m: &Value) -> String {
    let unit = s(m, "unit");
    let v = match m.get("value") {
        Some(Value::Number(n)) => fmt_num(n.as_f64().unwrap_or(0.0)),
        Some(Value::Array(a)) if a.len() == 2 => format!(
            "{}–{}",
            fmt_num(a[0].as_f64().unwrap_or(0.0)),
            fmt_num(a[1].as_f64().unwrap_or(0.0))
        ),
        _ => return "from the process profile".into(),
    };
    match unit {
        "mm" => format!("{v} mm"),
        "deg" => format!("{v}°"),
        "percent" => format!("{v} %"),
        "ratio" => format!("{v} ×"),
        _ => v,
    }
}

fn ev(v: &Value) -> String {
    let ids = ids_of(v, "evidence");
    if ids.is_empty() {
        "—".into()
    } else {
        ids.iter().map(|i| format!("`{}`", i.chars().take(8).collect::<String>())).collect::<Vec<_>>().join(" ")
    }
}

fn cell(x: &str) -> String {
    x.replace('|', "\\|").replace('\n', " ")
}

fn in_scope(item_scope: &str, family: Option<&str>) -> bool {
    match family {
        None => true,
        Some(f) => item_scope.is_empty() || item_scope == "global" || item_scope == f,
    }
}

fn sec_summary(t: &Value, out: &mut String) {
    out.push_str("## Summary\n\n");
    out.push_str(s(t, "summary").trim());
    out.push_str("\n\n");
}

fn short_json(v: &Value) -> String {
    let s = match v {
        Value::Null => "nothing".to_string(),
        Value::String(x) => format!("\"{x}\""),
        other => other.to_string(),
    };
    if s.chars().count() > 140 {
        format!("{}…", s.chars().take(140).collect::<String>())
    } else {
        s
    }
}

/// For a language that extends a base: the base and what this language changes.
fn sec_inheritance(t: &Value, out: &mut String) {
    let Some(e) = t.get("extends").filter(|e| e.is_object()) else { return };
    out.push_str(&format!(
        "## Inheritance\n\nThis language extends **{}** (`{}`), built on {} v{}. Everything in this document comes from that version, except the changes below. The changes are part of this document already.\n\n",
        s(e, "name"),
        s(e, "language"),
        s(e, "name"),
        e.get("version").and_then(|v| v.as_i64()).unwrap_or(0)
    ));
    let overrides = arr(e, "overrides");
    if overrides.is_empty() {
        out.push_str("No changes: the document is the same as the base.\n\n");
        return;
    }
    for o in overrides {
        let (op, path) = (s(o, "op"), s(o, "path"));
        let (before, after) = (o.get("before").unwrap_or(&Value::Null), o.get("after").unwrap_or(&Value::Null));
        out.push_str(&match (op, before, after) {
            ("add", _, _) => format!("- added `{path}`: {}\n", short_json(after)),
            ("remove", _, _) => format!("- removed `{path}` (the base has {})\n", short_json(before)),
            // Two long texts: start where they differ, so the change is in view.
            (_, Value::String(a), Value::String(b)) if a.chars().count() > 130 || b.chars().count() > 130 => {
                let same = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
                let from = same.saturating_sub(40);
                let cut = |t: &str| {
                    let n = t.chars().count();
                    format!("\"{}{}{}\"", if from > 0 { "…" } else { "" }, t.chars().skip(from).take(130).collect::<String>(), if n > from + 130 { "…" } else { "" })
                };
                format!("- changed `{path}`: {} → {}\n", cut(a), cut(b))
            }
            _ => format!("- changed `{path}`: {} → {}\n", short_json(before), short_json(after)),
        });
    }
    out.push('\n');
}

fn sec_principles(t: &Value, out: &mut String) {
    let mut ps: Vec<&Value> = arr(t, "principles").iter().collect();
    if ps.is_empty() {
        return;
    }
    ps.sort_by_key(|p| p.get("priority").and_then(|v| v.as_i64()).unwrap_or(99));
    out.push_str("## Principles\n\nThese apply to every family. A lower priority number wins a conflict.\n\n");
    for p in ps {
        out.push_str(&format!(
            "- **{}** (priority {}) {} — *{}* · evidence {}\n",
            s(p, "id"),
            p.get("priority").and_then(|v| v.as_i64()).unwrap_or(0),
            s(p, "text").trim(),
            s(p, "basis"),
            ev(p)
        ));
    }
    out.push('\n');
}

fn sec_definitions(t: &Value, out: &mut String) {
    let ds = arr(t, "definitions");
    if ds.is_empty() {
        return;
    }
    out.push_str("## Definitions\n\n");
    for d in ds {
        out.push_str(&format!("- **{}**: {}\n", s(d, "term"), s(d, "meaning").trim()));
    }
    out.push('\n');
}

fn render_family(f: &Value, out: &mut String, with_cad: bool) {
    out.push_str(&format!("### {} (`{}`)\n\n", s(f, "name"), s(f, "id")));
    out.push_str(&format!("**Applies to:** {}\n\n", s(f, "applies_to").trim()));
    if !s(f, "intent").trim().is_empty() {
        out.push_str(s(f, "intent").trim());
        out.push_str("\n\n");
    }
    let items = ids_of(f, "items");
    if !items.is_empty() {
        out.push_str(&format!(
            "References: {}\n\n",
            items.iter().map(|i| format!("`{}`", i.chars().take(8).collect::<String>())).collect::<Vec<_>>().join(" ")
        ));
    }
    if !with_cad {
        return;
    }
    let ms = arr(f, "measures");
    if !ms.is_empty() {
        out.push_str("| Measure | Value | Of | Basis | Confidence | Evidence |\n| --- | --- | --- | --- | --- | --- |\n");
        for m in ms {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} |\n",
                cell(s(m, "name")),
                fmt_value(m),
                cell(s(m, "of")),
                s(m, "basis"),
                m.get("confidence").and_then(|v| v.as_f64()).map(|c| format!("{:.0} %", c * 100.0)).unwrap_or_else(|| "—".into()),
                ev(m)
            ));
        }
        out.push('\n');
    }
    let rs = arr(f, "rules");
    if !rs.is_empty() {
        out.push_str("Rules:\n\n");
        for r in rs {
            out.push_str(&format!("- **{}** {} — *{}* · evidence {}\n", s(r, "id"), s(r, "text").trim(), s(r, "basis"), ev(r)));
        }
        out.push('\n');
    }
}

fn sec_families(t: &Value, out: &mut String, family: Option<&str>, with_cad: bool) {
    let fs: Vec<&Value> = arr(t, "families")
        .iter()
        .filter(|f| family.map(|id| s(f, "id") == id).unwrap_or(true))
        .collect();
    if fs.is_empty() {
        return;
    }
    out.push_str(if family.is_some() { "## Object family\n\n" } else { "## Object families\n\nUse the family whose \"Applies to\" matches the object. Do not apply one family's measures to another family.\n\n" });
    for f in fs {
        render_family(f, out, with_cad);
    }
}

fn sec_exceptions(t: &Value, out: &mut String, family: Option<&str>) {
    let mut xs: Vec<&Value> = arr(t, "exceptions").iter().filter(|x| in_scope(s(x, "scope"), family)).collect();
    if xs.is_empty() {
        return;
    }
    xs.sort_by_key(|x| x.get("priority").and_then(|v| v.as_i64()).unwrap_or(99));
    out.push_str("## Exceptions\n\nAn exception replaces the rule that it overrides, only inside its scope and condition.\n\n");
    out.push_str("| ID | Overrides | Scope | When | Instead | Priority |\n| --- | --- | --- | --- | --- | --- |\n");
    for x in xs {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            s(x, "id"),
            s(x, "overrides"),
            s(x, "scope"),
            cell(s(x, "when")),
            cell(s(x, "text")),
            x.get("priority").and_then(|v| v.as_i64()).unwrap_or(0)
        ));
    }
    out.push('\n');
}

fn sec_process_dependent(t: &Value, out: &mut String, family: Option<&str>) {
    let mut rows = Vec::new();
    for f in arr(t, "families") {
        if family.map(|id| s(f, "id") != id).unwrap_or(false) {
            continue;
        }
        for m in arr(f, "measures") {
            if s(m, "basis") == "process-dependent" {
                rows.push(format!("- {} · {}: {}", s(f, "name"), s(m, "name"), s(m, "of")));
            }
        }
    }
    if rows.is_empty() {
        return;
    }
    out.push_str("## Process-dependent values\n\nThe language does not set these values. Take them from the process profile of the project.\n\n");
    out.push_str(&rows.join("\n"));
    out.push_str("\n\n");
}

fn sec_cmf(t: &Value, out: &mut String, family: Option<&str>) {
    let objs: Vec<&Value> = t
        .get("cmf")
        .and_then(|c| c.get("object"))
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter(|o| in_scope(s(o, "scope"), family)).collect())
        .unwrap_or_default();
    if objs.is_empty() {
        return;
    }
    out.push_str("## Materials and CMF\n\nProduct colours, materials and finishes.\n\n");
    out.push_str("| ID | Role | Colour | RAL | Pantone (approx.) | Material | Finish | Scope | Basis | Evidence |\n| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n");
    for o in objs {
        let opt = |k: &str| o.get(k).and_then(|v| v.as_str()).filter(|x| !x.is_empty()).unwrap_or("—").to_string();
        out.push_str(&format!(
            "| {} | {} | `{}` | {} | {} | {} | {} | {} | {} | {} |\n",
            opt("id"),
            cell(s(o, "role")),
            s(o, "hex"),
            opt("ral"),
            opt("pantone_approx"),
            cell(&opt("material")),
            cell(&opt("finish")),
            if s(o, "scope").is_empty() { "global" } else { s(o, "scope") },
            opt("basis"),
            ev(o)
        ));
    }
    out.push('\n');
}

fn sec_presentation(t: &Value, out: &mut String) {
    let ps = t
        .get("cmf")
        .and_then(|c| c.get("presentation"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if ps.is_empty() {
        return;
    }
    out.push_str("## Presentation colours\n\nFor renders and concept images only. These are never product colours.\n\n");
    for p in &ps {
        out.push_str(&format!("- **{}** `{}` {}\n", s(p, "role"), s(p, "hex"), s(p, "note").trim()));
    }
    out.push('\n');
}

fn sec_prompts(t: &Value, out: &mut String, family: Option<&str>) {
    let Some(p) = t.get("prompts") else { return };
    out.push_str("## Image-generation tokens\n\n");
    out.push_str(&format!("**Positive:** {}\n\n**Negative:** {}\n\n", s(p, "positive").trim(), s(p, "negative").trim()));
    if let Some(fam) = p.get("families").and_then(|v| v.as_object()) {
        for (k, v) in fam {
            if family.map(|f| f == k).unwrap_or(true) {
                out.push_str(&format!("**{k}:** {}\n\n", v.as_str().unwrap_or("").trim()));
            }
        }
    }
}

fn sec_do_dont(t: &Value, out: &mut String, family: Option<&str>) {
    let pick = |k: &str| -> Vec<String> {
        arr(t, k)
            .iter()
            .filter(|d| in_scope(s(d, "scope"), family))
            .map(|d| {
                let sc = s(d, "scope");
                if sc.is_empty() || sc == "global" {
                    format!("- {}", s(d, "text").trim())
                } else {
                    format!("- {} *({sc})*", s(d, "text").trim())
                }
            })
            .collect()
    };
    let (d, n) = (pick("do"), pick("dont"));
    if d.is_empty() && n.is_empty() {
        return;
    }
    out.push_str("## Do and do not\n\n");
    if !d.is_empty() {
        out.push_str("Do:\n\n");
        out.push_str(&d.join("\n"));
        out.push_str("\n\n");
    }
    if !n.is_empty() {
        out.push_str("Do not:\n\n");
        out.push_str(&n.join("\n"));
        out.push_str("\n\n");
    }
}

fn sec_refs(t: &Value, out: &mut String, key: &str, title: &str, field: &str) {
    let rs = arr(t, key);
    if rs.is_empty() {
        return;
    }
    out.push_str(&format!("## {title}\n\n"));
    for r in rs {
        out.push_str(&format!("- `{}` {}\n", s(r, "item").chars().take(8).collect::<String>(), s(r, field).trim()));
    }
    out.push('\n');
}

fn sec_basis_key(out: &mut String) {
    out.push_str("## How to read the values\n\n");
    out.push_str("- *observed*: seen in the references. The evidence IDs name the items.\n");
    out.push_str("- *preferred*: a starting value that matches the references. Adjust it to the scale and the material.\n");
    out.push_str("- *process-dependent*: the process profile of the project sets it.\n");
    out.push_str("- *designer*: from the designer's notes, tags or description. *inferred*: a conclusion from several references. *source*: taught by a linked tutorial or document (the evidence ID names it).\n");
    out.push_str("- Confidence is the model's estimate (0–100 %) that the value is right for the references.\n\n");
}

/// The full language document.
pub fn render_markdown(name: &str, t: &Value, warnings: &[String]) -> String {
    let mut out = format!("# {name}\n\n");
    out.push_str(&format!(
        "Version {} · schema {} · {} famil{}\n\n",
        t.get("version").and_then(|v| v.as_i64()).unwrap_or(0),
        SCHEMA_VERSION,
        arr(t, "families").len(),
        if arr(t, "families").len() == 1 { "y" } else { "ies" }
    ));
    if !warnings.is_empty() {
        out.push_str("> **Check before use.** The automatic review found:\n");
        for w in warnings {
            out.push_str(&format!("> - {w}\n"));
        }
        out.push('\n');
    }
    sec_summary(t, &mut out);
    sec_inheritance(t, &mut out);
    sec_principles(t, &mut out);
    sec_definitions(t, &mut out);
    sec_families(t, &mut out, None, true);
    sec_exceptions(t, &mut out, None);
    sec_process_dependent(t, &mut out, None);
    sec_cmf(t, &mut out, None);
    sec_presentation(t, &mut out);
    sec_prompts(t, &mut out, None);
    sec_do_dont(t, &mut out, None);
    sec_refs(t, &mut out, "anti_references", "Anti-references", "lesson");
    sec_refs(t, &mut out, "key_references", "Key references", "why");
    sec_basis_key(&mut out);
    out
}

/// One family with the global rules that apply to it.
pub fn render_family_brief(name: &str, t: &Value, family: &str) -> Option<String> {
    if !arr(t, "families").iter().any(|f| s(f, "id") == family) {
        return None;
    }
    let mut out = format!("# {name} · family `{family}`\n\n");
    sec_summary(t, &mut out);
    sec_inheritance(t, &mut out);
    sec_principles(t, &mut out);
    sec_definitions(t, &mut out);
    sec_families(t, &mut out, Some(family), true);
    sec_exceptions(t, &mut out, Some(family));
    sec_process_dependent(t, &mut out, Some(family));
    sec_cmf(t, &mut out, Some(family));
    sec_prompts(t, &mut out, Some(family));
    sec_do_dont(t, &mut out, Some(family));
    sec_basis_key(&mut out);
    Some(out)
}

/// The part of the spec that a project pin scope allows.
pub fn render_scope(name: &str, t: &Value, scope: Scope) -> String {
    match scope {
        Scope::All => render_markdown(name, t, &[]),
        Scope::Cad => {
            let mut out = String::new();
            sec_principles(t, &mut out);
            sec_definitions(t, &mut out);
            sec_families(t, &mut out, None, true);
            sec_exceptions(t, &mut out, None);
            sec_process_dependent(t, &mut out, None);
            sec_basis_key(&mut out);
            out
        }
        Scope::Cmf => {
            let mut out = String::new();
            sec_cmf(t, &mut out, None);
            out
        }
        Scope::Prompts => {
            let mut out = String::new();
            sec_prompts(t, &mut out, None);
            sec_presentation(t, &mut out);
            out
        }
    }
}

/// Family IDs and names, for agents.
pub fn family_list(t: &Value) -> Vec<(String, String, String)> {
    arr(t, "families")
        .iter()
        .map(|f| (s(f, "id").to_string(), s(f, "name").to_string(), s(f, "applies_to").to_string()))
        .collect()
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn sample() -> Value {
        json!({
            "summary": "Continuous masses with flow-following texture and quiet materials.",
            "principles": [
                {"id": "P1", "text": "Keep the mass continuous; no visible frame.", "priority": 1, "basis": "observed", "evidence": ["aaaaaaaa"]},
                {"id": "P2", "text": "One matte colour for the whole skin.", "priority": 2, "basis": "designer", "evidence": []}
            ],
            "definitions": [{"term": "pitch", "meaning": "Centre-to-centre distance between adjacent blades."}],
            "families": [
                {
                    "id": "shell", "name": "Laminated shell", "applies_to": "benches, seats", "intent": "A single soft shell.",
                    "items": ["aaaaaaaa"],
                    "measures": [
                        {"name": "edge fillet", "value": [0.3, 0.5], "unit": "ratio", "of": "shell thickness at the edge", "basis": "observed", "confidence": 0.6, "evidence": ["aaaaaaaa"]},
                        {"name": "minimum wall", "value": null, "unit": "mm", "of": "shell wall thickness", "basis": "process-dependent", "confidence": 1.0, "evidence": []}
                    ],
                    "rules": [{"id": "S1", "text": "Seams follow the flow lines.", "basis": "observed", "evidence": ["aaaaaaaa"]}]
                },
                {
                    "id": "blade-field", "name": "Blade field", "applies_to": "louvres, screens", "intent": "Constant blades.",
                    "items": ["bbbbbbbb"],
                    "measures": [{"name": "blade pitch", "value": [8, 10], "unit": "mm", "of": "pitch", "basis": "preferred", "confidence": 0.4, "evidence": ["bbbbbbbb"]}],
                    "rules": [{"id": "B1", "text": "Blades keep a constant pitch.", "basis": "observed", "evidence": ["bbbbbbbb"]}]
                }
            ],
            "exceptions": [{"id": "X1", "overrides": "P2", "scope": "blade-field", "when": "The blades face the sun.", "text": "A second, warmer colour on the sun side.", "priority": 1}],
            "cmf": {
                "object": [{"id": "C1", "role": "body", "hex": "#E8E4DC", "ral": "RAL 9001", "pantone_approx": null, "material": "oak veneer", "finish": "matte", "scope": "global", "basis": "observed", "evidence": ["aaaaaaaa"]}],
                "presentation": [{"role": "background", "hex": "#F4F2EE", "note": "Studio backdrop."}]
            },
            "prompts": {"positive": "continuous laminated shell", "negative": "visible frame, chrome", "families": {"blade-field": "constant-pitch blades"}},
            "do": [{"text": "Follow the flow.", "scope": "global"}],
            "dont": [{"text": "Visible fasteners.", "scope": "shell"}],
            "anti_references": [{"item": "cccccccc", "lesson": "Gloss reads as cheap."}],
            "key_references": [{"item": "aaaaaaaa", "why": "Best shell."}]
        })
    }

    fn known() -> Vec<String> {
        vec!["aaaaaaaa".into(), "bbbbbbbb".into(), "cccccccc".into()]
    }

    #[test]
    fn valid_sample() {
        let c = validate(&sample(), &known());
        assert!(c.errors.is_empty(), "{:?}", c.errors);
        assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    }

    #[test]
    fn hard_errors() {
        let mut t = sample();
        t["families"][0]["measures"][1]["value"] = json!(2.0); // process-dependent with a value
        t["families"][0]["measures"][0]["of"] = json!("");
        t["families"][1]["measures"][0]["value"] = json!([10, 8]);
        t["cmf"]["presentation"][0]["role"] = json!("body");
        let c = validate(&t, &known());
        assert!(c.errors.iter().any(|e| e.contains("process-dependent")));
        assert!(c.errors.iter().any(|e| e.contains("\"of\"")));
        assert!(c.errors.iter().any(|e| e.contains("min ≤ max")));
        assert!(c.errors.iter().any(|e| e.contains("presentation")));
    }

    #[test]
    fn soft_warnings() {
        let mut t = sample();
        t["principles"][0]["text"] = json!("Keep walls at 2 mm and fillets at 0.4× thickness.");
        t["families"][0]["evidence"] = json!(["zzzzzzzz"]);
        t["families"][0]["items"] = json!(["zzzzzzzz"]);
        t["exceptions"][0]["overrides"] = json!("P9");
        let c = validate(&t, &known());
        assert!(c.errors.is_empty(), "{:?}", c.errors);
        assert!(c.warnings.iter().any(|w| w.contains("Numbers belong in measures")));
        assert!(c.warnings.iter().any(|w| w.contains("zzzzzzzz")));
        assert!(c.warnings.iter().any(|w| w.contains("P9")));
    }

    #[test]
    fn numeric_finder() {
        assert!(find_numeric_value("walls of 2 mm").is_some());
        assert!(find_numeric_value("a 0.4× ratio").is_some());
        assert!(find_numeric_value("about 45° from vertical").is_some());
        assert!(find_numeric_value("ratio 1:1.6").is_some());
        assert!(find_numeric_value("8–10 mm gaps").is_some());
        assert!(find_numeric_value("rule P1 and family 2 of the set").is_none());
        assert!(find_numeric_value("v2 schema, 3 blades").is_none());
    }

    #[test]
    fn render() {
        let mut t = sample();
        finalize(&mut t, "organic", 3);
        let md = render_markdown("Organic", &t, &["one warning".into()]);
        assert!(md.contains("## Object families"));
        assert!(md.contains("| edge fillet | 0.3–0.5 × | shell thickness at the edge | observed | 60 % | `aaaaaaaa` |"));
        assert!(md.contains("from the process profile"));
        assert!(md.contains("## Process-dependent values"));
        assert!(md.contains("Check before use"));
        let fam = render_family_brief("Organic", &t, "blade-field").unwrap();
        assert!(fam.contains("Blade field"));
        assert!(!fam.contains("Laminated shell"));
        assert!(fam.contains("X1")); // exception scoped to blade-field
        assert!(!fam.contains("Visible fasteners")); // dont scoped to shell
        assert!(render_family_brief("Organic", &t, "nope").is_none());
        let cmf = render_scope("Organic", &t, Scope::Cmf);
        assert!(cmf.contains("#E8E4DC") && !cmf.contains("#F4F2EE"));
        let prompts = render_scope("Organic", &t, Scope::Prompts);
        assert!(prompts.contains("#F4F2EE"));
    }

    #[test]
    fn repair_fixes_small_problems() {
        let mut t = sample();
        t["families"][0]["measures"][0]["value"] = json!("0.5–0.3");
        t["families"][0]["measures"][0]["unit"] = json!("x");
        t["families"][0]["measures"][0]["confidence"] = json!(60);
        t["families"][0]["measures"][0]["basis"] = json!("measured");
        t["families"][0]["measures"][1]["value"] = json!(2.0);
        t["families"][1]["measures"][0]["value"] = json!(["8", "10 mm"]);
        t["families"][1]["measures"].as_array_mut().unwrap().push(json!({"name": "blade twist", "value": null, "unit": "deg", "of": "twist", "basis": "observed", "confidence": 0.3}));
        t["families"][1]["measures"].as_array_mut().unwrap().push(json!({"name": "spacing", "value": 3, "unit": "inches", "of": "x", "basis": "observed", "confidence": 0.3}));
        t["principles"][1]["priority"] = json!("2");
        t["principles"][1]["id"] = json!("P1");
        t["cmf"]["object"][0]["hex"] = json!("e8e4dc");
        t["cmf"]["presentation"][0]["role"] = json!("studio backdrop");
        t["do"] = json!(["Follow the flow."]);
        t["prompts"]["negative"] = json!(["chrome", "gloss"]);
        t["exceptions"][0]["when"] = json!("");
        let rep = repair(&mut t);
        let c = validate(&t, &[ "aaaaaaaa".into(), "bbbbbbbb".into(), "cccccccc".into()]);
        assert!(c.errors.is_empty(), "{:?}", c.errors);
        let m0 = &t["families"][0]["measures"][0];
        assert_eq!(m0["value"], json!([0.3, 0.5]));
        assert_eq!(m0["unit"], "ratio");
        assert_eq!(m0["confidence"], json!(0.6));
        assert_eq!(m0["basis"], "observed");
        assert!(t["families"][0]["measures"][1]["value"].is_null());
        assert_eq!(t["families"][1]["measures"][0]["value"], json!([8.0, 10.0]));
        assert_eq!(t["families"][1]["measures"].as_array().unwrap().len(), 1);
        assert_eq!(t["principles"][1]["id"], "P1-2");
        assert_eq!(t["cmf"]["object"][0]["hex"], "#E8E4DC");
        assert_eq!(t["cmf"]["presentation"][0]["role"], "background");
        assert_eq!(t["do"][0]["text"], "Follow the flow.");
        assert_eq!(t["prompts"]["negative"], "chrome, gloss");
        assert!(rep.flagged.iter().any(|f| f.contains("blade twist")));
        assert!(rep.flagged.iter().any(|f| f.contains("inches")));
        assert!(rep.flagged.iter().any(|f| f.contains("X1")));
        assert!(!rep.fixed.is_empty());
    }

    #[test]
    fn repair_schema1_cmf_and_missing_parts() {
        let mut t = json!({"summary": "s", "families": [{"name": "Shell", "applies_to": "benches"}], "cmf": [{"role": "body", "hex": "#ABC"}], "prompts": {"positive": "a"}});
        let rep = repair(&mut t);
        let c = validate(&t, &[]);
        assert!(c.errors.is_empty(), "{:?}", c.errors);
        assert_eq!(t["families"][0]["id"], "shell");
        assert_eq!(t["cmf"]["object"][0]["hex"], "#AABBCC");
        assert!(rep.flagged.iter().any(|f| f.contains("negative")));
    }
}
