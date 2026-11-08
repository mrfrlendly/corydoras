//! Text that agents read: used by the MCP server and by the previews in the app.

use crate::analysis;
use crate::library::Library;
use crate::models::*;
use crate::spec;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};

pub fn languages_json(lib: &Library) -> Result<String> {
    let langs = lib.list_languages()?;
    let mut rows: Vec<Value> = Vec::new();
    for l in &langs {
        let families = match lib.read_latest(&l.id)? {
            Some(c) if c.info.schema >= 2 => spec::family_list(&c.tokens)
                .into_iter()
                .map(|(id, name, applies)| json!({"id": id, "name": name, "applies_to": applies}))
                .collect::<Vec<_>>(),
            _ => vec![],
        };
        rows.push(json!({
            "id": l.id,
            "name": l.name,
            "description": l.description,
            "latest_version": l.latest_version,
            "item_count": l.item_count,
            "families": families,
        }));
    }
    Ok(serde_json::to_string_pretty(&json!({ "languages": rows }))?)
}

pub fn projects_json(lib: &Library) -> Result<String> {
    let projects = lib.list_projects()?;
    let rows: Vec<Value> = projects
        .iter()
        .map(|p| {
            json!({
                "name": p.name,
                "id": p.id,
                "rule_set_ids": p.rule_set_ids,
                "pins": p.pins.iter().map(|pin| json!({
                    "language_id": pin.language_id,
                    "language_name": pin.language_name,
                    "pinned_version": pin.version,
                    "latest_version": pin.latest_version,
                    "role": pin.role,
                    "scope": pin.scope,
                })).collect::<Vec<_>>()
            })
        })
        .collect();
    Ok(serde_json::to_string_pretty(&json!({ "projects": rows }))?)
}

pub fn rule_sets_json(lib: &Library) -> Result<String> {
    Ok(serde_json::to_string_pretty(&json!({ "rule_sets": lib.list_rule_sets()? }))?)
}

fn rule_value(r: &Rule) -> String {
    match (r.value, r.unit.as_deref()) {
        (Some(v), Some(u)) => spec::fmt_value(&json!({"value": v, "unit": u})),
        (Some(v), None) => spec::fmt_value(&json!({"value": v, "unit": "count"})),
        _ => "—".into(),
    }
}

/// A rule set as markdown.
pub fn rule_set_text(r: &RuleSet) -> String {
    let mut s = format!(
        "## Hard rules: {}{}\n\n",
        r.name,
        if r.always { " (applies to every project)" } else { "" }
    );
    if !r.description.trim().is_empty() {
        s.push_str(r.description.trim());
        s.push_str("\n\n");
    }
    if r.rules.is_empty() {
        s.push_str("This rule set has no rules.\n");
        return s;
    }
    s.push_str("| Rule | Limit | Details |\n| --- | --- | --- |\n");
    for rule in &r.rules {
        s.push_str(&format!(
            "| {} | {} | {} |\n",
            rule.name.replace('|', "\\|"),
            rule_value(rule),
            rule.note.replace('|', "\\|").replace('\n', " ")
        ));
    }
    s
}

const RULES_INTRO: &str = "Hard rules win over every design-language principle, measure, exception and prompt. If a design value breaks a hard rule, change the design value and keep the rule. Limits marked as minimum or maximum are limits, not targets.";

/// The hard rules that apply, as one markdown block (empty when none apply).
pub fn rules_block(sets: &[RuleSet]) -> String {
    if sets.is_empty() {
        return String::new();
    }
    let mut s = format!("# Hard rules\n\n{RULES_INTRO}\n\n");
    for r in sets {
        s.push_str(&rule_set_text(r));
        s.push('\n');
    }
    s
}

fn words(s: &str) -> Vec<String> {
    const STOP: &[&str] = &["the", "of", "a", "an", "at", "on", "in", "for", "to", "and", "or", "per", "each", "side", "thickness"];
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| match w {
            "min" | "minimal" => "minimum".to_string(),
            "max" | "maximal" => "maximum".to_string(),
            "walls" => "wall".to_string(),
            "holes" => "hole".to_string(),
            "clearances" | "gap" | "gaps" => "clearance".to_string(),
            other => other.to_string(),
        })
        .filter(|w| !STOP.contains(&w.as_str()))
        .collect()
}

/// The rule that best matches a measure name (shared words over the shorter name).
pub fn match_rule<'a>(sets: &'a [RuleSet], measure_name: &str) -> Option<(&'a RuleSet, &'a Rule)> {
    let m = words(measure_name);
    if m.is_empty() {
        return None;
    }
    let mut best: Option<(f64, &RuleSet, &Rule)> = None;
    for set in sets {
        for rule in set.rules.iter().filter(|r| r.value.is_some()) {
            let r = words(&rule.name);
            if r.is_empty() {
                continue;
            }
            let shared = m.iter().filter(|w| r.contains(w)).count();
            let score = shared as f64 / m.len().min(r.len()) as f64;
            if shared > 0 && score >= 0.99 && best.as_ref().map(|b| score > b.0).unwrap_or(true) {
                best = Some((score, set, rule));
            }
        }
    }
    best.map(|(_, s, r)| (s, r))
}

/// A table of the language's process-dependent measures and the hard-rule
/// values that fill them.
pub fn resolve_process_values(sets: &[RuleSet], content: &VersionContent, language_name: &str) -> String {
    if content.info.schema < 2 {
        return String::new();
    }
    let mut rows: Vec<String> = Vec::new();
    for f in content.tokens.get("families").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
        let fam = f.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        for m in f.get("measures").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
            if m.get("basis").and_then(|v| v.as_str()) != Some("process-dependent") {
                continue;
            }
            let name = m.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let value = match match_rule(sets, name) {
                Some((set, rule)) => format!("{} (\"{}\" in {})", rule_value(rule), rule.name, set.name),
                None => "No hard rule sets it. Ask the designer.".into(),
            };
            rows.push(format!("| {fam} | {name} | {value} |"));
        }
    }
    if rows.is_empty() {
        return String::new();
    }
    format!(
        "## Process-dependent values for {language_name}\n\n| Family | Measure | Value |\n| --- | --- | --- |\n{}\n\n",
        rows.join("\n")
    )
}

/// The language document: markdown plus tokens.json. With `family`, only that
/// family and the global rules that apply to it (schema 2 only).
pub fn language_text(lib: &Library, key: &str, version: Option<i64>, family: Option<&str>) -> Result<String> {
    let lang = lib.find_language(key)?;
    let content = match version {
        Some(v) => Some(lib.read_version(&lang.id, v)?),
        None => lib.read_latest(&lang.id)?,
    };
    let Some(c) = content else {
        let items = lib.list_items(&lang.id)?;
        let done = items.iter().filter(|i| i.analysis_status == "done").count();
        return Ok(format!(
            "Language \"{}\" ({}) has no version yet. It has {} items; {} are analyzed. \
             The first version appears after the analysis finishes.",
            lang.name,
            lang.id,
            items.len(),
            done
        ));
    };
    let header = format!(
        "LANGUAGE {} ({}), VERSION {} of {}{}\n\n",
        lang.name,
        lang.id,
        c.info.version,
        lang.latest_version.unwrap_or(c.info.version),
        if c.info.edited { " (edited by the designer)" } else { "" },
    );
    if let Some(fam) = family.map(|f| f.trim()).filter(|f| !f.is_empty()) {
        if c.info.schema < 2 {
            return Err(anyhow!(
                "Version {} of \"{}\" uses the older format, which has no families. Call get_language without \"family\".",
                c.info.version,
                lang.name
            ));
        }
        let always = lib.effective_rule_sets(None)?;
        let rules = rules_block(&always);
        return spec::render_family_brief(&lang.name, &c.tokens, fam)
            .map(|md| format!("{header}{rules}{md}"))
            .ok_or_else(|| {
                let ids: Vec<String> = spec::family_list(&c.tokens).into_iter().map(|f| f.0).collect();
                anyhow!("No family \"{fam}\" in {} v{}. Families: {}.", lang.name, c.info.version, ids.join(", "))
            });
    }
    let always = lib.effective_rule_sets(None)?;
    let rules = rules_block(&always);
    let resolved = resolve_process_values(&always, &c, &lang.name);
    Ok(format!(
        "{header}{rules}{resolved}{}\n\n## tokens.json\n\n```json\n{}\n```\n",
        c.markdown.trim(),
        serde_json::to_string_pretty(&c.tokens)?
    ))
}

/// The markdown section whose "## " heading contains one of the keywords (schema 1).
pub fn extract_section(md: &str, keywords: &[&str]) -> Option<String> {
    let mut out: Vec<&str> = Vec::new();
    let mut inside = false;
    for line in md.lines() {
        if line.starts_with("## ") {
            if inside {
                break;
            }
            let h = line.to_lowercase();
            inside = keywords.iter().any(|k| h.contains(k));
        }
        if inside {
            out.push(line);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out.join("\n").trim().to_string())
    }
}

/// The part of a version that a pin scope allows.
pub fn scoped_text(name: &str, content: &VersionContent, scope: &str) -> Result<String> {
    if content.info.schema >= 2 {
        return Ok(spec::render_scope(name, &content.tokens, spec::Scope::parse(scope)));
    }
    let (keywords, token_key): (&[&str], &str) = match scope {
        "cad" => (&["cad", "geometry"], "geometry"),
        "cmf" => (&["cmf", "material"], "cmf"),
        "prompts" => (&["prompt"], "prompts"),
        _ => {
            return Ok(format!(
                "{}\n\n### tokens.json\n\n```json\n{}\n```",
                content.markdown.trim(),
                serde_json::to_string_pretty(&content.tokens)?
            ))
        }
    };
    let section = extract_section(&content.markdown, keywords)
        .unwrap_or_else(|| "(The document has no section for this scope.)".to_string());
    let tokens = content.tokens.get(token_key).cloned().unwrap_or(Value::Null);
    Ok(format!(
        "{section}\n\n### tokens.{token_key}\n\n```json\n{}\n```",
        serde_json::to_string_pretty(&tokens)?
    ))
}

/// The brief for a project: the hard rules, then each pinned language
/// version limited by its scope.
pub fn project_brief_text(lib: &Library, key: &str) -> Result<String> {
    let project = lib.find_project(key)?;
    let sets = lib.effective_rule_sets(Some(&project))?;
    let mut s = format!("# Project brief: {}\n\n", project.name);
    s.push_str("Order of authority, highest first:\n\n");
    s.push_str("1. Hard rules (rule sets below): they always win.\n");
    s.push_str("2. Exceptions of the languages, inside their scope and condition.\n");
    s.push_str("3. The primary language: its principles, then the family that matches the object.\n");
    s.push_str("4. Secondary languages, only inside their scope.\n\n");
    s.push_str("Use the family whose \"Applies to\" matches the object that you design. Measures marked process-dependent take their value from the hard rules. Values marked preferred are starting points: adjust them to the scale and the material. These are pinned versions; do not use newer versions unless the designer says so.\n\n");

    if sets.is_empty() {
        s.push_str("# Hard rules\n\nNo rule set applies to this project. Treat every process-dependent value as unknown, and ask the designer before you choose wall thicknesses, clearances or other manufacturing values.\n\n");
    } else {
        s.push_str(&rules_block(&sets));
    }

    if project.pins.is_empty() {
        s.push_str("# Languages\n\nThis project has no pinned languages. Pin a language in the Corydoras app.\n");
        return Ok(s);
    }
    s.push_str("# Pinned languages\n\n");
    for pin in &project.pins {
        let stale = match pin.latest_version {
            Some(l) if l > pin.version => format!(" (newer version v{l} exists, not pinned)"),
            _ => String::new(),
        };
        s.push_str(&format!(
            "- {}: {} ({}) v{}, scope {}{}\n",
            pin.role, pin.language_name, pin.language_id, pin.version, pin.scope, stale
        ));
    }
    for pin in &project.pins {
        let content = lib
            .read_version(&pin.language_id, pin.version)
            .map_err(|e| anyhow!("Cannot read {} v{}: {e}", pin.language_id, pin.version))?;
        s.push_str(&format!(
            "\n\n---\n\n# {} language: {} v{} (scope: {})\n\n",
            if pin.role == "primary" { "Primary" } else { "Secondary" },
            pin.language_name,
            pin.version,
            pin.scope
        ));
        if !content.info.warnings.is_empty() {
            s.push_str("> The automatic review flagged this version. Check these points before you rely on them:\n");
            for w in &content.info.warnings {
                s.push_str(&format!("> - {w}\n"));
            }
            s.push('\n');
        }
        if pin.scope == "all" || pin.scope == "cad" {
            s.push_str(&resolve_process_values(&sets, &content, &pin.language_name));
        }
        s.push_str(&scoped_text(&pin.language_name, &content, &pin.scope)?);
    }
    Ok(s)
}

/// The sources of a language with their notes, for agents.
pub fn sources_text(lib: &Library, key: &str, ids: &[String]) -> Result<String> {
    let lang = lib.find_language(key)?;
    let mut list = lib.list_sources(&lang.id)?;
    if !ids.is_empty() {
        list.retain(|x| ids.iter().any(|p| p.len() >= 4 && x.id.starts_with(p.trim().trim_matches('`'))));
    }
    if list.is_empty() {
        return Ok(format!("Language \"{}\" has no matching sources.", lang.name));
    }
    let mut s = format!("# Sources of {} ({})\n\nTutorials and documents that the designer added. The language document cites them by these IDs.\n", lang.name, lang.id);
    for x in &list {
        let short: String = x.id.chars().take(8).collect();
        s.push_str(&format!("\n## `{short}` {}\n\n", x.title));
        s.push_str(&format!(
            "- Kind: {} · weight {}/5{}\n",
            x.media,
            x.weight,
            x.url.as_ref().map(|u| format!(" · {u}")).unwrap_or_else(|| format!(" · file {}", x.original_name))
        ));
        if !x.note.is_empty() {
            s.push_str(&format!("- Designer note: {}\n", x.note));
        }
        match lib.get_source_analysis(&x.id)? {
            Some((a, _)) => {
                s.push_str("\n```json\n");
                s.push_str(&serde_json::to_string_pretty(&a)?);
                s.push_str("\n```\n");
            }
            None => s.push_str(&format!("- Not analyzed yet ({}).\n", x.analysis_status)),
        }
    }
    Ok(s)
}

/// Items for get_reference_images. With `item_ids`, exactly those items
/// (8-character prefixes are enough); otherwise the highest weight first.
pub fn pick_reference_items(
    lib: &Library,
    key: &str,
    tag: Option<&str>,
    limit: usize,
    include_anti: bool,
    item_ids: &[String],
) -> Result<(Language, Vec<Item>)> {
    let lang = lib.find_language(key)?;
    if !item_ids.is_empty() {
        let mut items = lib.items_by_prefix(&lang.id, item_ids)?;
        items.truncate(12);
        if items.is_empty() {
            return Err(anyhow!("None of these item IDs are in \"{}\": {}.", lang.name, item_ids.join(", ")));
        }
        return Ok((lang, items));
    }
    let mut items = lib.list_items(&lang.id)?;
    if let Some(tag) = tag.map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty()) {
        items.retain(|i| i.tags.iter().any(|t| t.contains(&tag)));
    }
    if !include_anti {
        items.retain(|i| !i.anti);
    }
    items.sort_by(|a, b| b.weight.cmp(&a.weight).then(b.added_at.cmp(&a.added_at)));
    items.truncate(limit.clamp(1, 12));
    Ok((lang, items))
}

/// Context for a reference image.
pub fn reference_caption(lib: &Library, item: &Item) -> String {
    let analysis = lib.get_analysis(&item.id).ok().flatten().map(|(a, _)| a);
    let get = |k: &str| {
        analysis
            .as_ref()
            .and_then(|a| a.get(k))
            .and_then(|s| s.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default()
    };
    let summary = get("summary");
    let family = get("object_family");
    let short: String = item.id.chars().take(8).collect();
    format!(
        "Item {short}{} · weight {}/5{}{}{}{}",
        if item.anti { " · ANTI-REFERENCE (not this)" } else { "" },
        item.weight,
        if family.is_empty() { String::new() } else { format!(" · {family}") },
        if item.tags.is_empty() { String::new() } else { format!(" · tags: {}", item.tags.join(", ")) },
        if item.note.is_empty() { String::new() } else { format!(" · note: {}", item.note) },
        if summary.is_empty() { String::new() } else { format!("\n{summary}") },
    )
}

/// JPEG of an item for an agent (longest side 1024 px), base64.
pub fn reference_image_base64(lib: &Library, item: &Item) -> Result<String> {
    let path = lib.abs(&item.file_rel)?;
    let jpeg = analysis::jpeg_from_file(&path, 1024)?;
    Ok(analysis::base64_of(&jpeg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_matching() {
        let r = |n: &str, v: f64| Rule { name: n.into(), value: Some(v), unit: Some("mm".into()), note: String::new() };
        let set = RuleSet {
            id: "s".into(),
            name: "FDM".into(),
            description: String::new(),
            always: false,
            rules: vec![r("Minimum wall", 1.2), r("Load-bearing wall", 2.0), r("Sliding clearance", 0.3), Rule { name: "Load along the layers".into(), value: None, unit: None, note: String::new() }],
            created_at: String::new(),
            updated_at: String::new(),
            project_count: 0,
        };
        let sets = vec![set];
        assert_eq!(match_rule(&sets, "minimum wall").unwrap().1.value, Some(1.2));
        assert_eq!(match_rule(&sets, "min wall thickness").unwrap().1.value, Some(1.2));
        assert_eq!(match_rule(&sets, "clearance per side").unwrap().1.value, Some(0.3));
        assert!(match_rule(&sets, "draft angle").is_none());
        assert!(match_rule(&sets, "layers").is_none()); // text rules have no value
        let block = rules_block(&sets);
        assert!(block.contains("| Minimum wall | 1.2 mm |"));
        assert!(block.contains("| Load along the layers | — |"));
    }

    #[test]
    fn sections() {
        let md = "# L\n\n## Summary\nx\n\n## CAD geometry rules\n- fillet 0.1\n\n## Materials and CMF\n- matte";
        assert_eq!(extract_section(md, &["cad"]).unwrap(), "## CAD geometry rules\n- fillet 0.1");
        assert!(extract_section(md, &["cmf"]).unwrap().contains("matte"));
        assert!(extract_section(md, &["nothing"]).is_none());
    }
}
