//! The app commands of Phase 3.2 to 3.4, called through the invoke layer with
//! the argument objects that the frontend sends.
//!
//! The argument names come from `src/api.ts` itself: `call` refuses a test
//! that sends other names than api.ts does. So a command that the frontend
//! calls with a wrong name, or with a value that the command cannot read,
//! fails here and not in front of the user.

use crate::{features, AppState};
use base64::Engine;
use corydoras_core::library::Library;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tauri::ipc::InvokeBody;
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY};
use tauri::WebviewWindow;

const API_TS: &str = include_str!("../../src/api.ts");

/// A 48 x 32 PNG of one colour.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAADAAAAAgCAIAAADbtmxLAAAAMUlEQVR42u3OQQ0AAAgEoAtmMCMayxbOBxsByHS9EiEhISEhISEhISEhISEhISGhSwuVQuh5yKgZWwAAAABJRU5ErkJggg==";

/// The argument names that api.ts sends with a command: `invoke<T>('cmd', { a, b })`.
fn api_args(cmd: &str) -> Vec<String> {
    let needle = format!("('{cmd}'");
    let at = API_TS.find(&needle).unwrap_or_else(|| panic!("api.ts does not call {cmd}"));
    let rest = API_TS[at + needle.len()..].trim_start();
    if !rest.starts_with(',') {
        return Vec::new();
    }
    let (open, close) = (rest.find('{').expect("argument object"), rest.find('}').expect("argument object"));
    let mut names: Vec<String> = rest[open + 1..close].split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    names.sort();
    names
}

fn send(w: &WebviewWindow<MockRuntime>, cmd: &str, body: InvokeBody, headers: tauri::http::HeaderMap) -> Result<Value, String> {
    tauri::test::get_ipc_response(
        w,
        tauri::webview::InvokeRequest {
            cmd: cmd.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body,
            headers,
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .map(|b| b.deserialize::<Value>().unwrap())
    .map_err(|e| e.as_str().map(str::to_string).unwrap_or_else(|| e.to_string()))
}

/// Calls a command the way api.ts does. `args` must have the names that api.ts sends.
fn call(w: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Result<Value, String> {
    let mut sent: Vec<String> = args.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
    sent.sort();
    assert_eq!(sent, api_args(cmd), "this test and api.ts send different arguments with {cmd}");
    send(w, cmd, InvokeBody::Json(args), Default::default())
}

fn ok(w: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Value {
    call(w, cmd, args).unwrap_or_else(|e| panic!("{cmd} failed: {e}"))
}

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("corydoras-ipc-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A spec that needs no references: every value is the designer's.
fn spec(fillet: [f64; 2]) -> Value {
    json!({
        "summary": "Soft shells with one quiet colour.",
        "principles": [{"id": "P1", "text": "Keep the mass continuous; no visible frame.", "priority": 1, "basis": "designer", "evidence": []}],
        "definitions": [],
        "families": [{
            "id": "shell", "name": "Soft shell", "applies_to": "enclosures", "intent": "A single soft shell.", "items": [],
            "measures": [
                {"name": "edge fillet", "value": fillet, "unit": "mm", "of": "radius of the outer edge", "basis": "preferred", "confidence": 0.5, "evidence": []},
                {"name": "minimum wall", "value": null, "unit": "mm", "of": "shell wall thickness", "basis": "process-dependent", "confidence": 1.0, "evidence": []}
            ],
            "rules": [{"id": "S1", "text": "Seams follow the flow lines.", "basis": "designer", "evidence": []}]
        }],
        "exceptions": [],
        "cmf": {
            "object": [{"id": "C1", "role": "body", "hex": "#E8E4DC", "ral": null, "pantone_approx": null, "material": "PETG", "finish": "matte", "scope": "global", "basis": "designer", "evidence": []}],
            "presentation": []
        },
        "prompts": {"positive": "soft continuous shell", "negative": "visible frame", "families": {}},
        "do": [], "dont": [], "anti_references": [], "key_references": []
    })
}

/// A cube of 20 mm as an ASCII STL.
fn cube_stl() -> String {
    let s = 20.0_f32;
    let v = |i: usize| [if i & 1 == 0 { 0.0 } else { s }, if i & 2 == 0 { 0.0 } else { s }, if i & 4 == 0 { 0.0 } else { s }];
    // Two triangles for each face, counter-clockwise seen from outside.
    let faces: [[usize; 4]; 6] = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
    let mut out = String::from("solid cube\n");
    for f in faces {
        for t in [[f[0], f[1], f[2]], [f[0], f[2], f[3]]] {
            out.push_str("facet normal 0 0 0\n outer loop\n");
            for i in t {
                let p = v(i);
                out.push_str(&format!("  vertex {} {} {}\n", p[0], p[1], p[2]));
            }
            out.push_str(" endloop\nendfacet\n");
        }
    }
    out.push_str("endsolid cube\n");
    out
}

fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_else(|| panic!("no text \"{key}\" in {v}")).to_string()
}

fn app_with(lib: Arc<Library>) -> tauri::App<MockRuntime> {
    mock_builder()
        .manage(AppState { lib: RwLock::new(Some(lib)), ..Default::default() })
        .invoke_handler(tauri::generate_handler![
            features::import_model_bytes,
            features::backfill_palettes,
            features::language_palette,
            features::search_board,
            features::check_output_geometry,
            features::list_print_results,
            features::add_print_result,
            features::remove_print_result,
            features::rule_feedback,
            features::slicer_profiles,
            features::slicer_preview,
            features::slicer_import,
            features::version_diff,
            features::restore_version,
            features::coverage,
            features::set_base,
            features::inheritance,
            features::remove_override,
            features::export_language,
            features::save_text_file,
            features::estimate,
            features::other_model,
            features::reanalyze_other_model,
            features::create_snapshot,
            features::restore_snapshot,
            features::db_copies,
            features::usage_insight,
        ])
        .build(mock_context(noop_assets()))
        .expect("the test app")
}

fn write(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

#[test]
fn commands_take_what_the_frontend_sends() {
    let dir = scratch();
    // The commands that read the settings must not read the settings of the person who runs the test.
    std::env::set_var("CORYDORAS_CONFIG_DIR", dir.join("config"));
    let lib = Arc::new(Library::open(&dir.join("library")).unwrap());
    let app = app_with(lib.clone());
    let w = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();

    let lang = lib.create_language("Soft shell", "").unwrap();
    assert_eq!(lib.save_manual_spec(&lang.id, &spec([2.0, 4.0]), "test").unwrap(), 1);
    assert_eq!(lib.save_manual_spec(&lang.id, &spec([3.0, 5.0]), "test").unwrap(), 2);

    // --- Versions: what changed, and the rollback.
    let diff = ok(&w, "version_diff", json!({"languageId": lang.id, "from": 1, "to": 2}));
    let changes = diff["changes"].as_array().unwrap();
    assert!(changes.iter().any(|c| text(c, "text").contains("edge fillet")), "{diff}");
    assert_eq!(ok(&w, "restore_version", json!({"languageId": lang.id, "version": 1})), json!(3));
    assert!(call(&w, "restore_version", json!({"languageId": lang.id, "version": 9})).is_err());

    // --- Coverage and usage need no model.
    let cov = ok(&w, "coverage", json!({"languageId": lang.id}));
    assert_eq!(cov["version"], json!(3));
    assert!(cov["gaps"].as_array().is_some_and(|g| !g.is_empty()), "{cov}");
    let usage = ok(&w, "usage_insight", json!({"languageId": lang.id, "days": 90}));
    assert_eq!(usage["calls"], json!(0));

    // --- Inheritance: a base, a change, and the change removed again.
    let child = lib.create_language("Soft shell PETG", "").unwrap();
    let l = ok(&w, "set_base", json!({"languageId": child.id, "base": lang.id, "version": null}));
    assert_eq!(l["base_id"], json!(lang.id));
    let inh = ok(&w, "inheritance", json!({"languageId": child.id}));
    assert_eq!((text(&inh, "base_name").as_str(), inh["overrides"].as_array().unwrap().len()), ("Soft shell", 0));
    let mut edited = lib.read_latest(&child.id).unwrap().unwrap().tokens;
    edited["summary"] = json!("Soft shells in clear PETG.");
    lib.save_manual_spec(&child.id, &edited, "test").unwrap();
    assert_eq!(ok(&w, "inheritance", json!({"languageId": child.id}))["overrides"].as_array().unwrap().len(), 1);
    assert!(ok(&w, "remove_override", json!({"languageId": child.id, "index": 0})).is_number());
    assert_eq!(ok(&w, "inheritance", json!({"languageId": child.id}))["overrides"].as_array().unwrap().len(), 0);
    assert!(ok(&w, "inheritance", json!({"languageId": lang.id})).is_null());
    let l = ok(&w, "set_base", json!({"languageId": child.id, "base": null, "version": null}));
    assert!(l["base_id"].is_null());

    // --- Export, and the file that the save dialog names.
    for (format, end, mark) in [("css", ".css", "--"), ("figma", ".tokens.json", "$value"), ("grasshopper", ".grasshopper.json", "parameters")] {
        let x = ok(&w, "export_language", json!({"languageId": lang.id, "version": null, "format": format, "project": null}));
        assert!(text(&x, "file_name").ends_with(end), "{x}");
        assert!(text(&x, "content").contains(mark), "{format}: {x}");
        let path = dir.join("exports").join(text(&x, "file_name"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        ok(&w, "save_text_file", json!({"path": path, "content": text(&x, "content")}));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text(&x, "content"));
    }
    assert!(call(&w, "export_language", json!({"languageId": lang.id, "version": 2, "format": "docx", "project": null})).is_err());
    assert!(call(&w, "save_text_file", json!({"path": "relative.css", "content": "x"})).is_err());

    // --- Cost: the estimate for each scope, and the analyses of another model.
    for scope in ["queue", "all", "document", "other-model"] {
        let e = ok(&w, "estimate", json!({"languageId": lang.id, "scope": scope}));
        assert!(e["total_tokens"].is_number(), "{scope}: {e}");
    }
    assert!(ok(&w, "other_model", json!({"languageId": lang.id})).is_object());
    assert_eq!(ok(&w, "reanalyze_other_model", json!({"languageId": lang.id})), json!(0));

    // --- Slicer profile: the preview and the rule set.
    let profile = dir.join("profiles/IPC test 0.20.json");
    write(
        &profile,
        &json!({"type": "process", "name": "IPC test 0.20", "layer_height": "0.2", "line_width": "0.42", "outer_wall_line_width": "0.42", "inner_wall_line_width": "0.45", "wall_loops": "2", "support_threshold_angle": "30", "wall_generator": "classic"}).to_string(),
    );
    assert!(ok(&w, "slicer_profiles", json!({})).is_array());
    let input = json!({"process": profile, "machine": null, "filament": null, "project": null});
    let derived = ok(&w, "slicer_preview", json!({"input": input}));
    let wall = derived["rules"].as_array().unwrap().iter().find(|r| r["name"] == "Minimum wall").expect("Minimum wall");
    assert_eq!(wall["value"], json!(0.84));
    assert!(derived["compare"].as_array().is_some_and(|c| !c.is_empty()));
    let set = ok(&w, "slicer_import", json!({"input": input, "name": "From the test", "fillFromBuiltin": true}));
    assert_eq!(text(&set, "name"), "From the test");
    assert!(set["rules"].as_array().unwrap().iter().any(|r| r["name"] == "Minimum wall" && r["value"] == json!(0.84)));
    assert!(call(&w, "slicer_preview", json!({"input": {"process": dir.join("none.json"), "machine": null, "filament": null, "project": null}})).is_err());

    // --- Print log on that rule set.
    let entry = ok(
        &w,
        "add_print_result",
        json!({"input": {"result": "failed", "material": "PETG", "rule": "Minimum wall", "rule_set": set["id"], "value": 0.9, "unit": "mm", "note": "Split at the lip.", "language": null, "output": null}}),
    );
    assert_eq!(text(&entry, "result"), "failed");
    assert!(ok(&w, "list_print_results", json!({})).as_array().unwrap().iter().any(|p| p["id"] == entry["id"]));
    let feedback = ok(&w, "rule_feedback", json!({"ruleSetId": set["id"]}));
    let hint = feedback.as_array().unwrap().iter().find(|f| f["rule"] == "Minimum wall").expect("feedback for Minimum wall");
    assert_eq!(hint["failed"], json!(1));
    assert!(hint["hints"].as_array().is_some_and(|h| !h.is_empty()), "a print failed above the limit: {hint}");
    ok(&w, "remove_print_result", json!({"id": entry["id"]}));
    assert!(ok(&w, "list_print_results", json!({})).as_array().unwrap().iter().all(|p| p["id"] != entry["id"]));

    // --- A model on the Board (the drop sends the raw file), and the board tools.
    let stl = cube_stl();
    let mut headers = tauri::http::HeaderMap::new();
    headers.insert("x-language-id", urlencoding::encode(&lang.id).parse().unwrap());
    headers.insert("x-file-name", urlencoding::encode("Test cube ö.stl").parse().unwrap());
    assert!(API_TS.contains("'x-language-id'") && API_TS.contains("'x-file-name'"), "api.ts sends other headers with a dropped model");
    let item = send(&w, "import_model_bytes", InvokeBody::Raw(stl.clone().into_bytes()), headers).expect("import_model_bytes");
    assert_eq!((text(&item, "kind").as_str(), text(&item, "original_name").as_str()), ("model", "Test cube ö.stl"));
    assert!(lib.root().join(text(&item, "model_rel")).is_file());
    // A render shows no colour, so only the image gives the board its palette.
    assert_eq!(item["palette"], json!([]));
    let png = base64::engine::general_purpose::STANDARD.decode(PNG).unwrap();
    lib.import_bytes(&lang.id, "swatch.png", &png, None).unwrap();
    assert!(ok(&w, "backfill_palettes", json!({"languageId": lang.id})).is_number());
    let board = ok(&w, "language_palette", json!({"languageId": lang.id}));
    assert_eq!(board.as_array().map(|p| p.len()), Some(1), "{board}");
    assert!(ok(&w, "search_board", json!({"languageId": lang.id, "query": "cube"})).is_array());

    // --- Geometry check on an output: a 20 mm cube passes the built-in limits.
    let out = lib.submit_output(&lang.id, "render.png", &png, "shell", "", "test", "", &[]).unwrap();
    let model = dir.join("models/cube.stl");
    write(&model, &stl);
    let checked = ok(&w, "check_output_geometry", json!({"outputId": out.id, "path": model, "up": "+z"}));
    let geo = &checked["geometry"];
    assert_eq!((geo["bodies"].as_i64(), geo["triangles"].as_i64()), (Some(1), Some(12)), "{geo}");
    assert!((geo["min_wall"]["value"].as_f64().unwrap() - 20.0).abs() < 0.05, "{geo}");
    assert!(call(&w, "check_output_geometry", json!({"outputId": out.id, "path": dir.join("models/none.stl"), "up": null})).is_err());

    // --- Snapshot: save, list the database copies, and open it as a library.
    let dest = dir.join("backup");
    std::fs::create_dir_all(&dest).unwrap();
    let report = ok(&w, "create_snapshot", json!({"dest": dest}));
    let zip = PathBuf::from(text(&report, "path"));
    assert!(zip.is_file() && report["files"].as_u64().unwrap() > 3, "{report}");
    assert!(ok(&w, "db_copies", json!({})).is_array());
    let restored = dir.join("restored");
    assert!(ok(&w, "restore_snapshot", json!({"zip": zip, "dest": restored})).as_u64().unwrap() > 3);
    let again = Library::open(&restored).unwrap();
    assert!(again.list_languages().unwrap().iter().any(|l| l.name == "Soft shell"));
    assert!(call(&w, "create_snapshot", json!({"dest": lib.root().join("inside")})).is_err(), "a snapshot inside the library is refused");

    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}
