//! corydoras-mcp: a Model Context Protocol server on stdio.
//!
//! It reads the Corydoras library (query-only), so agents can use the design
//! languages while the app is closed. The one write is propose_amendment: it adds
//! a proposal that the designer accepts or rejects in the app. Usage:
//!
//!     corydoras-mcp [--library /path/to/library]
//!
//! Without --library it uses CORYDORAS_LIBRARY, then the app configuration.

mod workshop;

use anyhow::{anyhow, Result};
use corydoras_core::conformance::{self, Measurement};
use corydoras_core::insight::Event;
use corydoras_core::printlog::PrintResultInput;
use corydoras_core::{agent, analysis, config, coverage, diff, export, geometry, outputs, printlog, Library};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::io::{BufRead, Write};
use std::path::PathBuf;

const SERVER_NAME: &str = "corydoras";
const DEFAULT_PROTOCOL: &str = "2025-06-18";

const INSTRUCTIONS: &str = "Corydoras holds the designer's visual design languages and their hard rule sets. \
Each language is a versioned spec made from a mood board of reference images. A rule set holds hard \
rules, for example 3D-printing limits; hard rules win over every language rule. When you work on a \
project, call get_project_brief with the project name first: it returns the hard rules, then the \
pinned language versions, with an order of authority. Without a project, call list_rule_sets and \
list_languages, pick the family whose applies_to matches your object, and call get_language with that \
family. Each numeric language value has a basis: observed (seen in the references), preferred (a \
starting value to adjust to scale and material) or process-dependent (take it from the hard rules). \
Never apply one family's measures to another family. Call get_reference_images with item_ids to see \
the evidence behind a rule. Rules with basis \"source\" come from tutorials or documents that the designer added; \
call get_sources with those IDs to read the technique behind them. Call search_items to find reference \
images by what they show. When the references or the hard rules show that a rule or measure is wrong or \
missing, call propose_amendment: the designer reviews the change in the app, and nothing changes until \
they accept it. When you have made a design from a language, call submit_output with a render of it so the designer \
can accept or reject it (that teaches the language), and call check_against_language with the numbers you measured on \
the design: it compares them with the hard rules and the language's measures and lists every fail to fix. \
When you have the model as a file (STL, 3MF, OBJ or STEP), call check_geometry: it measures the thinnest wall, the \
steepest overhang, the longest bridge, the gap between bodies and the part size, and judges them against the hard rules. \
The hard rules come with a print-feedback table: results of real prints. When the designer tells you how a print came \
out, call log_print_result, so the next design does not repeat a failure. Call get_coverage before you rely on a family: \
it lists what the language cannot answer yet. Call compare_versions to see what changed between two versions, for example \
after the designer updated a language. Call export_tokens for the measures and colours as CSS variables, design tokens or \
Grasshopper parameters. A language can extend a base language; its document is complete and says what it changes.";

fn tools() -> Value {
    json!([
        workshop::search_tool(),
        workshop::item_tool(),
        {
            "name": "list_languages",
            "description": "List the design languages: ID, name, description, latest version and item count.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        },
        {
            "name": "list_projects",
            "description": "List the projects and the language versions that each project pins (primary and secondary, with scope).",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        },
        {
            "name": "get_project_brief",
            "description": "Get the design brief for a project: the hard rules that apply (they win over everything), then the pinned version of the primary language, plus each secondary language limited to its scope (cad, cmf, prompts or all). Call this first when you work on a named project.",
            "inputSchema": {
                "type": "object",
                "properties": {"project": {"type": "string", "description": "Project name or ID"}},
                "required": ["project"],
                "additionalProperties": false
            }
        },
        {
            "name": "list_rule_sets",
            "description": "List the hard rule sets, for example 3D-printing limits (minimum wall, overhang, clearances, orientation). Hard rules win over every language rule. Sets with always=true apply everywhere; other sets apply to the projects that select them.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        },
        {
            "name": "get_language",
            "description": "Get a design language: the rendered document and its tokens.json. Without a version, it returns the latest version. With \"family\", it returns only that object family plus the principles, definitions, exceptions, CMF and prompts that apply to it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "language": {"type": "string", "description": "Language ID or name"},
                    "version": {"type": "integer", "minimum": 1, "description": "Version number (optional)"},
                    "family": {"type": "string", "description": "Family ID, for example blade-field (optional; list_languages shows the families)"}
                },
                "required": ["language"],
                "additionalProperties": false
            }
        },
        {
            "name": "get_sources",
            "description": "Get the tutorials, articles and documents that the designer added to a language, with the notes extracted from each. Use it to learn the technique behind a rule; rules with basis \"source\" cite these IDs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "language": {"type": "string", "description": "Language ID or name"},
                    "ids": {"type": "array", "items": {"type": "string"}, "description": "Only these sources, by ID or 8-character evidence ID (optional)"}
                },
                "required": ["language"],
                "additionalProperties": false
            }
        },
        {
            "name": "get_reference_images",
            "description": "Get reference images of a language (highest weight first) with their tags, notes and analysis summary. Use it to see the visual intent behind the rules.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "language": {"type": "string", "description": "Language ID or name"},
                    "tag": {"type": "string", "description": "Only items with a tag that contains this text (optional)"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 12, "description": "Number of images (default 6)"},
                    "include_anti": {"type": "boolean", "description": "Include \"not this\" anti-references (default false)"},
                    "item_ids": {"type": "array", "items": {"type": "string"}, "maxItems": 12, "description": "Exact items to return, by ID or by the 8-character evidence ID that the spec cites (optional; overrides tag and limit)"}
                },
                "required": ["language"],
                "additionalProperties": false
            }
        },
        {
            "name": "search_items",
            "description": "Search the reference images by what they show: tags, the designer's notes, region notes and the analysis (object family, features, measurements, colours, summary). Returns the best matches with their 8-character evidence IDs and the matching text, no images; call get_reference_images with item_ids to see them.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Words to look for, for example \"fillet oak bench\""},
                    "language": {"type": "string", "description": "Language ID or name (optional; without it, all languages)"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 30, "description": "Number of results (default 10)"},
                    "include_anti": {"type": "boolean", "description": "Include \"not this\" anti-references (default false)"}
                },
                "required": ["query"],
                "additionalProperties": false
            }
        },
        {
            "name": "check_against_language",
            "description": "Check numbers that you measured on a design against the hard rules and the language's measures. Send each measurement with its name (use the names of the rules and measures: \"minimum wall\", \"blade pitch\"), value and unit. Hard rules are checked first and win: a number that breaks one is a FAIL and must change. Then the measures of the family are compared (observed range: FAIL outside it; preferred value: WARN). UNKNOWN means the name or unit matched nothing, so nothing was judged: it is not a pass. Without \"family\", only hard rules can judge. The check uses numbers only; it does not look at images. Pass \"output\" to save the report with a submitted output.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "language": {"type": "string", "description": "Language ID or name"},
                    "family": {"type": "string", "description": "Family ID or name of the object (list_languages shows the families)"},
                    "project": {"type": "string", "description": "Project name or ID, so that the rule sets of the project apply (optional)"},
                    "measurements": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 100,
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": {"type": "string"},
                                "value": {"type": "number"},
                                "unit": {"type": "string", "description": "mm, deg, percent, count or ratio"}
                            },
                            "required": ["name", "value", "unit"]
                        }
                    },
                    "output": {"type": "string", "description": "ID (or first 8 characters) of a submitted output to save the report with (optional)"}
                },
                "required": ["language", "measurements"],
                "additionalProperties": false
            }
        },
        {
            "name": "submit_output",
            "description": "Send the designer a render of a design that you made from a language. The designer accepts or rejects it in the app (Outputs tab): an accepted output supports the language, a rejected one becomes an anti-reference, so the next version avoids it. Nothing in the language changes until they decide. Send an image file by path (PNG, JPEG or WebP, up to 20 MB) or as base64. Add the measurements you took, so the designer sees them with the picture.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "language": {"type": "string", "description": "Language ID or name"},
                    "path": {"type": "string", "description": "Absolute path of the image file on this computer"},
                    "image_base64": {"type": "string", "description": "The image as base64, instead of path"},
                    "name": {"type": "string", "description": "Short name of the design, for example \"twisted rib shade v2\""},
                    "family": {"type": "string", "description": "Family of the object (optional)"},
                    "project": {"type": "string", "description": "Project the design belongs to (optional)"},
                    "note": {"type": "string", "description": "What you made and which choices you took (optional)"},
                    "measurements": {
                        "type": "array",
                        "maxItems": 100,
                        "items": {
                            "type": "object",
                            "properties": {"name": {"type": "string"}, "value": {"type": "number"}, "unit": {"type": "string"}},
                            "required": ["name", "value"]
                        }
                    }
                },
                "required": ["language"],
                "additionalProperties": false
            }
        },
        {
            "name": "check_geometry",
            "description": "Measure a model file and judge it against the hard rules. Reads STL, 3MF, OBJ and STEP (STEP needs FreeCAD on this computer). It measures: the thinnest wall (material between two near-parallel faces), the steepest overhang from the vertical (faces in the first two layers and bridged ceilings do not count), the longest bridge between two supports, the smallest open gap between separate bodies, and the largest side of a body. Spots under about 1 mm2 are ignored. Overhangs and bridges depend on the build direction: pass \"up\" when the part does not print with +Z up. A model with several bodies is an assembly: its overhangs are judged only when you pass \"up\". Numbers that another tool measured (for example ScanRuler's thickness or clearance) go in \"measurements\": a number with the name of a measured value replaces it. With \"language\" and \"family\", the language's measures are judged too. Pass \"output\" to save the report with a submitted output.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Absolute path of the model file on this computer"},
                    "project": {"type": "string", "description": "Project name or ID, so that its rule sets apply (optional)"},
                    "up": {"type": "string", "enum": ["+z", "-z", "+y", "-y", "+x", "-x"], "description": "Build direction: the axis that points up on the printer (default +z)"},
                    "clearance": {"type": "string", "description": "The rule that the gap between bodies is compared with (default \"sliding clearance\"; for a press fit, \"press-fit clearance\")"},
                    "language": {"type": "string", "description": "Language ID or name, to judge the language's measures too (optional)"},
                    "family": {"type": "string", "description": "Family ID or name of the object (with language)"},
                    "measurements": {
                        "type": "array",
                        "maxItems": 60,
                        "items": {
                            "type": "object",
                            "properties": {"name": {"type": "string"}, "value": {"type": "number"}, "unit": {"type": "string"}},
                            "required": ["name", "value", "unit"]
                        },
                        "description": "Numbers from another tool, to judge with the measured ones (optional)"
                    },
                    "output": {"type": "string", "description": "ID (or first 8 characters) of a submitted output to save the report with (optional)"}
                },
                "required": ["path"],
                "additionalProperties": false
            }
        },
        {
            "name": "log_print_result",
            "description": "Record what happened when a design was printed, as the designer told you. The entry goes into the print log: it appears with the hard rules in every later brief, and the conformance check warns when a new design is at or past a value that failed. Give the material, the rule (the name of a hard rule), or both. With \"value\", say the value that was printed, for example a wall of 1.0 mm that failed. Do not guess a result: record only what the designer reported. The designer can remove an entry in the app.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "result": {"type": "string", "enum": ["failed", "warped", "frosted", "fine", "other"], "description": "What happened. \"other\" needs a note."},
                    "material": {"type": "string", "description": "The material, for example \"PETG, clear\" (optional when a rule is given)"},
                    "rule": {"type": "string", "description": "The hard rule that the result is about, for example \"minimum wall\" (optional when a material is given)"},
                    "rule_set": {"type": "string", "description": "Rule set ID or name (optional; without it the entry applies everywhere)"},
                    "value": {"type": "number", "description": "The value that was printed (optional)"},
                    "unit": {"type": "string", "description": "mm, deg, percent, count or ratio (with value)"},
                    "note": {"type": "string", "description": "What the designer saw, in one or two sentences"},
                    "language": {"type": "string", "description": "Language of the design (optional)"},
                    "output": {"type": "string", "description": "ID (or first 8 characters) of the output that was printed (optional)"}
                },
                "required": ["result"],
                "additionalProperties": false
            }
        },
        {
            "name": "get_print_log",
            "description": "Get the print-feedback log: results of real prints (failed, warped, frosted, fine) against hard rules and materials. With \"project\", the entries that count for the rule sets of that project.",
            "inputSchema": {
                "type": "object",
                "properties": {"project": {"type": "string", "description": "Project name or ID (optional)"}},
                "additionalProperties": false
            }
        },
        {
            "name": "compare_versions",
            "description": "List what changed between two versions of a language, in words: measures (old value → new value), rules, principles, families, colours, prompts, do and do not, and references. Use it when a project pins an older version than the latest, or when the designer updated a language and you must redo a design: the changes say what to do differently.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "language": {"type": "string", "description": "Language ID or name"},
                    "from": {"type": "integer", "minimum": 1, "description": "The older version"},
                    "to": {"type": "integer", "minimum": 1, "description": "The newer version (default: the latest)"}
                },
                "required": ["language", "from"],
                "additionalProperties": false
            }
        },
        {
            "name": "get_coverage",
            "description": "Get the coverage report of a language: what it cannot answer yet. It lists families that rest on one reference, views that no reference shows, measures with weak evidence, process-dependent measures that no hard rule fills, and references that disagree. Where the language has a gap, do not invent a value: ask the designer.",
            "inputSchema": {
                "type": "object",
                "properties": {"language": {"type": "string", "description": "Language ID or name"}},
                "required": ["language"],
                "additionalProperties": false
            }
        },
        {
            "name": "export_tokens",
            "description": "Export the measures and colours of a language version as CSS variables (css), design tokens for Figma in the W3C form (figma), or a parameter list for Grasshopper sliders (grasshopper). A measure with a range gives a minimum and a maximum. A process-dependent measure takes its value from the hard rule with the same name.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "language": {"type": "string", "description": "Language ID or name"},
                    "format": {"type": "string", "enum": ["css", "figma", "grasshopper"]},
                    "version": {"type": "integer", "minimum": 1, "description": "Version number (default: the latest)"},
                    "project": {"type": "string", "description": "Project name or ID, so that its rule sets fill the process-dependent measures (optional)"}
                },
                "required": ["language", "format"],
                "additionalProperties": false
            }
        },
        {
            "name": "propose_amendment",
            "description": "Propose a change to the latest version of a language spec (tokens.json, schema 2). The change does not apply by itself: the designer reviews it in the app and accepts or rejects it; an accepted proposal becomes a new version. The patch is a JSON Patch (RFC 6902) with add, remove and replace. A path segment into an array can be an index, \"-\" (append, with add), or the id, name or term of an element, for example /families/blade-field/measures/blade pitch/value or /principles/P2. The amended spec must pass the same validation as a manual edit; measures keep their unit, \"of\", basis and evidence, and text fields hold no numbers with units. Read the language with get_language first and pass its version as base_version.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "language": {"type": "string", "description": "Language ID or name"},
                    "base_version": {"type": "integer", "minimum": 1, "description": "The version that you read; the proposal is refused if the language changed since (recommended)"},
                    "summary": {"type": "string", "description": "One sentence: what the amendment changes"},
                    "rationale": {"type": "string", "description": "Why: the evidence IDs, hard rules or observations behind it"},
                    "patch": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 50,
                        "items": {
                            "type": "object",
                            "properties": {
                                "op": {"type": "string", "enum": ["add", "remove", "replace"]},
                                "path": {"type": "string"},
                                "value": {}
                            },
                            "required": ["op", "path"]
                        }
                    }
                },
                "required": ["language", "summary", "patch"],
                "additionalProperties": false
            }
        }
    ])
}

fn measurements_arg(args: &Value) -> Result<Vec<Measurement>> {
    let v = args.get("measurements").cloned().ok_or_else(|| anyhow!("The argument \"measurements\" is necessary."))?;
    serde_json::from_value(v).map_err(|e| anyhow!("The measurements are not valid: {e}. Each one is {{\"name\", \"value\" (number), \"unit\"}}."))
}

struct Server {
    library_override: Option<PathBuf>,
    /// The client name from initialize, recorded as the author of proposals.
    client: RefCell<String>,
}

impl Server {
    fn library_path(&self) -> Result<PathBuf> {
        let path = config::resolve_library_path(self.library_override.clone()).ok_or_else(|| {
            anyhow!("No Corydoras library is set. Open the Corydoras app and choose a library folder, or start the server with --library <path>.")
        })?;
        if !path.join("library.sqlite").exists() {
            return Err(anyhow!("No library database at {}.", path.display()));
        }
        Ok(path)
    }

    fn stock_lib(&self) -> Result<Library> {
        Library::open_read_only(&self.library_path()?)
    }

    fn lib(&self) -> Result<Library> {
        Library::open_query_only(&self.library_path()?)
    }

    fn call_tool(&self, name: &str, args: &Value) -> Result<Vec<Value>> {
        match name {
            "search_workshop_inventory" => return workshop::search(&self.stock_lib()?, args),
            "get_workshop_item" => return workshop::item(&self.stock_lib()?, args),
            _ => {}
        }
        let text = |s: String| json!({"type": "text", "text": s});
        let arg_str = |k: &str| -> Result<String> {
            args.get(k)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| anyhow!("The argument \"{k}\" is necessary."))
        };
        let lib = self.lib()?;
        let client = self.client.borrow().clone();
        // One record for each call that reads or checks a language, so the app can
        // show which parts agents use. A failed record must not fail the call.
        let record = |language_key: &str, version: Option<i64>, family: &str, project: &str, detail: Vec<String>| {
            if let Ok(lang) = lib.find_language(language_key) {
                let _ = lib.record_event(&Event { tool: name, language_id: &lang.id, version: version.or(lang.latest_version), family, project, client: &client, detail });
            }
        };
        let opt = |k: &str| args.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty());
        match name {
            "list_languages" => Ok(vec![text(agent::languages_json(&lib)?)]),
            "list_projects" => Ok(vec![text(agent::projects_json(&lib)?)]),
            "list_rule_sets" => Ok(vec![text(agent::rule_sets_json(&lib)?)]),
            "get_project_brief" => {
                let key = arg_str("project")?;
                let out = agent::project_brief_text(&lib, &key)?;
                if let Ok(project) = lib.find_project(&key) {
                    for pin in &project.pins {
                        record(&pin.language_id, Some(pin.version), "", &project.name, Vec::new());
                    }
                }
                Ok(vec![text(out)])
            }
            "get_language" => {
                let version = args.get("version").and_then(|v| v.as_i64());
                let family = args.get("family").and_then(|v| v.as_str());
                let key = arg_str("language")?;
                let out = agent::language_text(&lib, &key, version, family)?;
                record(&key, version, family.unwrap_or(""), "", Vec::new());
                Ok(vec![text(out)])
            }
            "get_sources" => {
                let ids: Vec<String> = args
                    .get("ids")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default();
                let key = arg_str("language")?;
                let out = agent::sources_text(&lib, &key, &ids)?;
                record(&key, None, "", "", ids);
                Ok(vec![text(out)])
            }
            "get_reference_images" => {
                let tag = args.get("tag").and_then(|v| v.as_str());
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(6) as usize;
                let include_anti = args.get("include_anti").and_then(|v| v.as_bool()).unwrap_or(false);
                let item_ids: Vec<String> = args
                    .get("item_ids")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default();
                let (lang, items) =
                    agent::pick_reference_items(&lib, &arg_str("language")?, tag, limit, include_anti, &item_ids)?;
                let mut out = vec![text(format!(
                    "{} reference image(s) from \"{}\" ({}).",
                    items.len(),
                    lang.name,
                    lang.id
                ))];
                for item in &items {
                    out.push(text(agent::reference_caption(&lib, item)));
                    match agent::reference_image_base64(&lib, item) {
                        Ok(b64) => out.push(json!({"type": "image", "data": b64, "mimeType": "image/jpeg"})),
                        Err(e) => out.push(text(format!("(Image not readable: {e})"))),
                    }
                }
                record(&lang.id, None, "", "", items.iter().map(|i| i.id.chars().take(8).collect()).collect());
                Ok(out)
            }
            "search_items" => {
                let language = args.get("language").and_then(|v| v.as_str());
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
                let include_anti = args.get("include_anti").and_then(|v| v.as_bool()).unwrap_or(false);
                let query = arg_str("query")?;
                let out = agent::search_items_text(&lib, &query, language, include_anti, limit)?;
                if let Some(l) = language {
                    record(l, None, "", "", vec![query]);
                }
                Ok(vec![text(out)])
            }
            "propose_amendment" => {
                let patch = args.get("patch").cloned().ok_or_else(|| anyhow!("The argument \"patch\" is necessary."))?;
                let base = args.get("base_version").and_then(|v| v.as_i64());
                let rationale = args.get("rationale").and_then(|v| v.as_str()).unwrap_or("");
                let key = arg_str("language")?;
                let sub = lib.submit_proposal(&key, base, &arg_str("summary")?, rationale, &patch, &client)?;
                record(&key, base, "", "", sub.proposal.changes.iter().map(|c| c.path.clone()).collect());
                Ok(vec![text(agent::proposal_text(&lib, &sub)?)])
            }
            "check_against_language" => {
                let ms = measurements_arg(args)?;
                let family = args.get("family").and_then(|v| v.as_str());
                let project = args.get("project").and_then(|v| v.as_str());
                let output = args.get("output").and_then(|v| v.as_str());
                let key = arg_str("language")?;
                let report = conformance::run(&lib, &key, family, project, &ms, output)?;
                record(&key, Some(report.version), family.unwrap_or(""), project.unwrap_or(""), ms.iter().map(|m| m.name.clone()).collect());
                Ok(vec![text(conformance::report_text(&report))])
            }
            "check_geometry" => {
                let path = std::path::PathBuf::from(arg_str("path")?.trim());
                let extra = if args.get("measurements").is_some() { measurements_arg(args)? } else { Vec::new() };
                let a = geometry::Args {
                    language: opt("language").map(|s| s.to_string()),
                    family: opt("family").map(|s| s.to_string()),
                    project: opt("project").map(|s| s.to_string()),
                    up: opt("up").map(|s| s.to_string()),
                    clearance: opt("clearance").map(|s| s.to_string()),
                    measurements: extra,
                    output: opt("output").map(|s| s.to_string()),
                };
                let report = geometry::check_file(&lib, &path, &a)?;
                if let Some(l) = &a.language {
                    record(l, None, a.family.as_deref().unwrap_or(""), a.project.as_deref().unwrap_or(""), report.measurements.iter().map(|m| m.name.clone()).collect());
                }
                Ok(vec![text(geometry::report_text(&report))])
            }
            "log_print_result" => {
                let input = PrintResultInput {
                    result: arg_str("result")?,
                    material: opt("material").unwrap_or("").to_string(),
                    rule: opt("rule").unwrap_or("").to_string(),
                    rule_set: opt("rule_set").map(|s| s.to_string()),
                    value: args.get("value").and_then(|v| v.as_f64()),
                    unit: opt("unit").map(|s| s.to_string()),
                    note: opt("note").unwrap_or("").to_string(),
                    language: opt("language").map(|s| s.to_string()),
                    output: opt("output").map(|s| s.to_string()),
                };
                let e = lib.add_print_result(&input, &client)?;
                Ok(vec![text(format!(
                    "Recorded in the print log: {}{}{}{}. It goes with the hard rules in every later brief, and the conformance check uses it. The designer can remove it in the Corydoras app.",
                    e.result,
                    if e.material.is_empty() { String::new() } else { format!(" · {}", e.material) },
                    if e.rule.is_empty() { String::new() } else { format!(" · rule \"{}\"", e.rule) },
                    e.rule_set_name.map(|n| format!(" · in {n}")).unwrap_or_else(|| " · applies everywhere".into())
                ))])
            }
            "get_print_log" => {
                let log = match opt("project") {
                    Some(p) => lib.print_results_for(&lib.effective_rule_sets(Some(&lib.find_project(p)?))?)?,
                    None => lib.list_print_results()?,
                };
                Ok(vec![text(printlog::log_text(&log))])
            }
            "compare_versions" => {
                let key = arg_str("language")?;
                let lang = lib.find_language(&key)?;
                let from = args.get("from").and_then(|v| v.as_i64()).ok_or_else(|| anyhow!("The argument \"from\" is necessary."))?;
                let to = match args.get("to").and_then(|v| v.as_i64()) {
                    Some(v) => v,
                    None => lang.latest_version.ok_or_else(|| anyhow!("\"{}\" has no version yet.", lang.name))?,
                };
                let d = diff::versions(&lib, &lang.id, from, to)?;
                record(&lang.id, Some(to), "", "", Vec::new());
                Ok(vec![text(diff::diff_text(&d))])
            }
            "get_coverage" => {
                let key = arg_str("language")?;
                let c = coverage::report(&lib, &key)?;
                record(&key, c.version, "", "", Vec::new());
                Ok(vec![text(coverage::coverage_text(&c))])
            }
            "export_tokens" => {
                let key = arg_str("language")?;
                let version = args.get("version").and_then(|v| v.as_i64());
                let x = export::export_language(&lib, &key, version, &arg_str("format")?, opt("project"))?;
                record(&key, version, "", opt("project").unwrap_or(""), vec![x.format.clone()]);
                Ok(vec![text(format!("{} (suggested file name: {})\n\n{}", x.format, x.file_name, x.content))])
            }
            "submit_output" => {
                let bytes = if let Some(p) = args.get("path").and_then(|v| v.as_str()).filter(|p| !p.trim().is_empty()) {
                    let path = std::path::Path::new(p.trim());
                    if !path.is_absolute() {
                        return Err(anyhow!("The path must be absolute."));
                    }
                    let meta = std::fs::metadata(path).map_err(|e| anyhow!("Cannot read {}: {e}", path.display()))?;
                    if meta.len() as usize > outputs::MAX_OUTPUT_BYTES {
                        return Err(anyhow!("The image is larger than 20 MB. Send a smaller render."));
                    }
                    std::fs::read(path).map_err(|e| anyhow!("Cannot read {}: {e}", path.display()))?
                } else if let Some(b) = args.get("image_base64").and_then(|v| v.as_str()) {
                    analysis::base64_decode(b)?
                } else {
                    return Err(anyhow!("Send the image as \"path\" or \"image_base64\"."));
                };
                let default_name = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .and_then(|p| std::path::Path::new(p).file_name())
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                let name = args.get("name").and_then(|v| v.as_str()).filter(|n| !n.trim().is_empty()).map(|n| n.to_string()).unwrap_or(default_name);
                let get = |k: &str| args.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
                let ms = if args.get("measurements").is_some() { measurements_arg(args)? } else { Vec::new() };
                let key = arg_str("language")?;
                let out = lib.submit_output(&key, &name, &bytes, &get("family"), &get("project"), &client, &get("note"), &ms)?;
                record(&key, out.language_version, &get("family"), &get("project"), Vec::new());
                Ok(vec![text(format!(
                    "Output {} is saved for the designer to review in the Corydoras app (Outputs tab). It supports or changes the language only after they accept or reject it.",
                    &out.id[..8]
                ))])
            }
            other => Err(anyhow!("Unknown tool \"{other}\".")),
        }
    }

    /// Returns None for notifications (no response).
    fn handle(&self, msg: &Value) -> Option<Value> {
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let id = id?; // Notifications have no id.
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let result: std::result::Result<Value, (i64, String)> = match method {
            "initialize" => {
                if let Some(name) = params.pointer("/clientInfo/name").and_then(|v| v.as_str()) {
                    *self.client.borrow_mut() = name.chars().take(60).collect();
                }
                let version = params
                    .get("protocolVersion")
                    .and_then(|v| v.as_str())
                    .unwrap_or(DEFAULT_PROTOCOL)
                    .to_string();
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION")},
                    "instructions": format!("{}{INSTRUCTIONS}", agent::WORKSHOP_CHECK)
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tools()})),
            "tools/call" => {
                let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                match self.call_tool(name, &args) {
                    Ok(content) => Ok(json!({"content": content, "isError": false})),
                    Err(e) => Ok(json!({"content": [{"type": "text", "text": e.to_string()}], "isError": true})),
                }
            }
            "resources/list" => Ok(json!({"resources": []})),
            "prompts/list" => Ok(json!({"prompts": []})),
            _ => Err((-32601, format!("Method not found: {method}"))),
        };
        Some(match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err((code, message)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
        })
    }
}

fn parse_args() -> Option<PathBuf> {
    let mut args = std::env::args().skip(1);
    let mut cli = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--library" => cli = args.next().map(PathBuf::from),
            "--version" => {
                println!("corydoras-mcp {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--help" | "-h" => {
                println!("corydoras-mcp [--library <path>]\nMCP server (stdio) for Corydoras design languages.");
                std::process::exit(0);
            }
            _ => {}
        }
    }
    cli
}

fn main() {
    let server = Server {
        library_override: parse_args(),
        client: RefCell::new(String::new()),
    };
    eprintln!(
        "corydoras-mcp {} ready (library: {})",
        env!("CARGO_PKG_VERSION"),
        server
            .library_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "not set".into())
    );
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(line) {
            Ok(Value::Array(batch)) => {
                let out: Vec<Value> = batch.iter().filter_map(|m| server.handle(m)).collect();
                if out.is_empty() {
                    None
                } else {
                    Some(Value::Array(out))
                }
            }
            Ok(msg) => server.handle(&msg),
            Err(e) => Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("Parse error: {e}")}})),
        };
        if let Some(r) = response {
            let _ = writeln!(stdout, "{}", serde_json::to_string(&r).unwrap_or_default());
            let _ = stdout.flush();
        }
    }
}
