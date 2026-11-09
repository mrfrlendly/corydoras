//! corydoras-mcp: a Model Context Protocol server on stdio.
//!
//! It reads the Corydoras library (query-only), so agents can use the design
//! languages while the app is closed. The one write is propose_amendment: it adds
//! a proposal that the designer accepts or rejects in the app. Usage:
//!
//!     corydoras-mcp [--library /path/to/library]
//!
//! Without --library it uses CORYDORAS_LIBRARY, then the app configuration.

use anyhow::{anyhow, Result};
use corydoras_core::conformance::{self, Measurement};
use corydoras_core::{agent, analysis, config, outputs, Library};
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
the design: it compares them with the hard rules and the language's measures and lists every fail to fix.";

fn tools() -> Value {
    json!([
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
    library_path: Option<PathBuf>,
    /// The client name from initialize, recorded as the author of proposals.
    client: RefCell<String>,
}

impl Server {
    fn lib(&self) -> Result<Library> {
        let path = self.library_path.clone().ok_or_else(|| {
            anyhow!("No Corydoras library is set. Open the Corydoras app and choose a library folder, or start the server with --library <path>.")
        })?;
        if !path.join("library.sqlite").exists() {
            return Err(anyhow!("No library database at {}.", path.display()));
        }
        Library::open_query_only(&path)
    }

    fn call_tool(&self, name: &str, args: &Value) -> Result<Vec<Value>> {
        let text = |s: String| json!({"type": "text", "text": s});
        let arg_str = |k: &str| -> Result<String> {
            args.get(k)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| anyhow!("The argument \"{k}\" is necessary."))
        };
        let lib = self.lib()?;
        match name {
            "list_languages" => Ok(vec![text(agent::languages_json(&lib)?)]),
            "list_projects" => Ok(vec![text(agent::projects_json(&lib)?)]),
            "list_rule_sets" => Ok(vec![text(agent::rule_sets_json(&lib)?)]),
            "get_project_brief" => Ok(vec![text(agent::project_brief_text(&lib, &arg_str("project")?)?)]),
            "get_language" => {
                let version = args.get("version").and_then(|v| v.as_i64());
                let family = args.get("family").and_then(|v| v.as_str());
                Ok(vec![text(agent::language_text(&lib, &arg_str("language")?, version, family)?)])
            }
            "get_sources" => {
                let ids: Vec<String> = args
                    .get("ids")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default();
                Ok(vec![text(agent::sources_text(&lib, &arg_str("language")?, &ids)?)])
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
                Ok(out)
            }
            "search_items" => {
                let language = args.get("language").and_then(|v| v.as_str());
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
                let include_anti = args.get("include_anti").and_then(|v| v.as_bool()).unwrap_or(false);
                Ok(vec![text(agent::search_items_text(&lib, &arg_str("query")?, language, include_anti, limit)?)])
            }
            "propose_amendment" => {
                let patch = args.get("patch").cloned().ok_or_else(|| anyhow!("The argument \"patch\" is necessary."))?;
                let base = args.get("base_version").and_then(|v| v.as_i64());
                let rationale = args.get("rationale").and_then(|v| v.as_str()).unwrap_or("");
                let author = self.client.borrow().clone();
                let sub = lib.submit_proposal(&arg_str("language")?, base, &arg_str("summary")?, rationale, &patch, &author)?;
                Ok(vec![text(agent::proposal_text(&lib, &sub)?)])
            }
            "check_against_language" => {
                let ms = measurements_arg(args)?;
                let family = args.get("family").and_then(|v| v.as_str());
                let project = args.get("project").and_then(|v| v.as_str());
                let output = args.get("output").and_then(|v| v.as_str());
                let report = conformance::run(&lib, &arg_str("language")?, family, project, &ms, output)?;
                Ok(vec![text(conformance::report_text(&report))])
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
                let author = self.client.borrow().clone();
                let out = lib.submit_output(&arg_str("language")?, &name, &bytes, &get("family"), &get("project"), &author, &get("note"), &ms)?;
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
                    "instructions": INSTRUCTIONS
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
    config::resolve_library_path(cli)
}

fn main() {
    let server = Server {
        library_path: parse_args(),
        client: RefCell::new(String::new()),
    };
    eprintln!(
        "corydoras-mcp {} ready (library: {})",
        env!("CARGO_PKG_VERSION"),
        server
            .library_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "not set".into())
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
