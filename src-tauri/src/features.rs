//! Commands of Phase 3.2 to 3.4: 3D models and palettes, the geometry check,
//! the print log, slicer profiles, version diff and rollback, coverage,
//! inheritance, exports, cost estimates, snapshots, usage insight and board
//! import.

use crate::{ai_config, e2s, lib, AppState, CmdResult};
use corydoras_core::{
    agent,
    ai::Role,
    cost::{self, Estimate, OtherModel},
    coverage::{self, Coverage},
    diff::{self, VersionDiff},
    export::{self, Exported},
    geometry,
    importers::{self, Board, ImportSummary},
    inherit::Inheritance,
    insight::{self, Usage},
    models::*,
    outputs::Output,
    palette::Swatch,
    printlog::{PrintResult, PrintResultInput, RuleFeedback},
    slicer::{self, Derived, ProfileRef},
    snapshot::{self, DbCopy, SnapshotReport},
};
use serde::Serialize;
use std::path::PathBuf;
use tauri::{Emitter, State};

// ----------------------------------------------------------------------
// 3D models, palettes and board search
// ----------------------------------------------------------------------

/// Largest model that a drop can send (bytes).
const MAX_MODEL_DROP: usize = 400 * 1024 * 1024;

/// A model file dropped from Finder. The body is the raw file; the headers
/// carry x-language-id and x-file-name (URI-encoded). The file goes to a
/// temporary file, because the STEP reader needs a path.
#[tauri::command]
pub async fn import_model_bytes(state: State<'_, AppState>, request: tauri::ipc::Request<'_>) -> CmdResult<Item> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("The model data is missing.".into());
    };
    if bytes.len() > MAX_MODEL_DROP {
        return Err("The model file is larger than 400 MB.".into());
    }
    let header = |k: &str| -> CmdResult<String> {
        let v = request.headers().get(k).and_then(|v| v.to_str().ok()).ok_or_else(|| format!("The header {k} is missing."))?;
        urlencoding::decode(v).map(|s| s.into_owned()).map_err(e2s)
    };
    let language_id = header("x-language-id")?;
    let name = header("x-file-name")?;
    let lib = lib(&state)?;
    let ext = std::path::Path::new(&name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let tmp_dir = lib.root().join(".tmp");
    std::fs::create_dir_all(&tmp_dir).map_err(e2s)?;
    let tmp = tmp_dir.join(format!("{}.{ext}", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, bytes).map_err(e2s)?;
    let result = tauri::async_runtime::spawn_blocking({
        let tmp = tmp.clone();
        move || lib.import_model(&language_id, &tmp, &name, None)
    })
    .await
    .map_err(e2s);
    let _ = std::fs::remove_file(&tmp);
    result?.map_err(e2s)
}

/// Finds the colours of images that older versions added. Returns the count,
/// so the board knows whether to load the items again.
#[tauri::command]
pub async fn backfill_palettes(state: State<'_, AppState>, language_id: String) -> CmdResult<usize> {
    let lib = lib(&state)?;
    tauri::async_runtime::spawn_blocking(move || lib.backfill_palettes(&language_id, 400)).await.map_err(e2s)?.map_err(e2s)
}

#[tauri::command]
pub fn language_palette(state: State<'_, AppState>, language_id: String) -> CmdResult<Vec<Swatch>> {
    lib(&state)?.language_palette(&language_id).map_err(e2s)
}

#[derive(Serialize)]
pub struct BoardHit {
    item_id: String,
    /// How many of the query words the item matches.
    matched: usize,
    /// The text that matched ("tag: fillet").
    snippets: Vec<String>,
}

/// The board search: the same search that agents use, on one language, with
/// the "not this" items included. Best match first.
#[tauri::command]
pub fn search_board(state: State<'_, AppState>, language_id: String, query: String) -> CmdResult<Vec<BoardHit>> {
    let lib = lib(&state)?;
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let hits = agent::search_items_all(&lib, &query, Some(&language_id), true).map_err(e2s)?;
    Ok(hits.into_iter().map(|h| BoardHit { item_id: h.item.id, matched: h.matched, snippets: h.snippets }).collect())
}

// ----------------------------------------------------------------------
// Geometry check and print log
// ----------------------------------------------------------------------

/// Checks a model file for an output and saves the report with it. The rule
/// sets are those of the output's project (when the project exists) plus the
/// ones that apply everywhere.
#[tauri::command]
pub async fn check_output_geometry(state: State<'_, AppState>, output_id: String, path: String, up: Option<String>) -> CmdResult<Output> {
    let lib = lib(&state)?;
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<Output> {
        let out = lib.get_output(&output_id)?;
        let latest = lib.read_latest(&out.language_id)?.filter(|c| c.info.schema >= 2);
        let family = latest.as_ref().and_then(|c| {
            let key = out.family.trim().to_lowercase();
            corydoras_core::spec::family_list(&c.tokens).into_iter().find(|(id, name, _)| !key.is_empty() && (id.to_lowercase() == key || name.to_lowercase() == key)).map(|f| f.0)
        });
        let args = geometry::Args {
            language: latest.is_some().then(|| out.language_id.clone()),
            family,
            project: Some(out.project.clone()).filter(|p| !p.trim().is_empty() && lib.find_project(p).is_ok()),
            up: up.filter(|u| !u.trim().is_empty()),
            clearance: None,
            measurements: Vec::new(),
            output: Some(out.id.clone()),
        };
        geometry::check_file(&lib, &PathBuf::from(path.trim()), &args)?;
        lib.get_output(&output_id)
    })
    .await
    .map_err(e2s)?
    .map_err(e2s)
}

#[tauri::command]
pub fn list_print_results(state: State<'_, AppState>) -> CmdResult<Vec<PrintResult>> {
    lib(&state)?.list_print_results().map_err(e2s)
}

#[tauri::command]
pub fn add_print_result(state: State<'_, AppState>, input: PrintResultInput) -> CmdResult<PrintResult> {
    lib(&state)?.add_print_result(&input, "").map_err(e2s)
}

#[tauri::command]
pub fn remove_print_result(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    lib(&state)?.remove_print_result(&id).map_err(e2s)
}

#[tauri::command]
pub fn rule_feedback(state: State<'_, AppState>, rule_set_id: String) -> CmdResult<Vec<RuleFeedback>> {
    lib(&state)?.rule_feedback(&rule_set_id).map_err(e2s)
}

// ----------------------------------------------------------------------
// Slicer profiles
// ----------------------------------------------------------------------

#[tauri::command]
pub async fn slicer_profiles() -> CmdResult<Vec<ProfileRef>> {
    tauri::async_runtime::spawn_blocking(slicer::installed_profiles).await.map_err(e2s)
}

#[tauri::command]
pub async fn slicer_preview(input: slicer::Input) -> CmdResult<Derived> {
    tauri::async_runtime::spawn_blocking(move || slicer::derive(&input)).await.map_err(e2s)?.map_err(e2s)
}

#[tauri::command]
pub fn slicer_import(state: State<'_, AppState>, input: slicer::Input, name: String, fill_from_builtin: bool) -> CmdResult<RuleSet> {
    lib(&state)?.import_slicer_profile(&input, &name, fill_from_builtin).map_err(e2s)
}

// ----------------------------------------------------------------------
// Versions, coverage, inheritance
// ----------------------------------------------------------------------

#[tauri::command]
pub fn version_diff(state: State<'_, AppState>, language_id: String, from: i64, to: i64) -> CmdResult<VersionDiff> {
    let lib = lib(&state)?;
    diff::versions(&lib, &language_id, from, to).map_err(e2s)
}

#[tauri::command]
pub fn restore_version(state: State<'_, AppState>, language_id: String, version: i64) -> CmdResult<i64> {
    lib(&state)?.restore_version(&language_id, version).map_err(e2s)
}

#[tauri::command]
pub fn coverage(state: State<'_, AppState>, language_id: String) -> CmdResult<Coverage> {
    let lib = lib(&state)?;
    coverage::report(&lib, &language_id).map_err(e2s)
}

#[tauri::command]
pub fn set_base<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: State<'_, AppState>, language_id: String, base: Option<String>, version: Option<i64>) -> CmdResult<Language> {
    let l = lib(&state)?.set_base(&language_id, base.as_deref(), version).map_err(e2s)?;
    let _ = app.emit("library-changed", language_id);
    Ok(l)
}

#[tauri::command]
pub fn inheritance(state: State<'_, AppState>, language_id: String) -> CmdResult<Option<Inheritance>> {
    lib(&state)?.inheritance(&language_id).map_err(e2s)
}

#[tauri::command]
pub fn remove_override(state: State<'_, AppState>, language_id: String, index: usize) -> CmdResult<Option<i64>> {
    lib(&state)?.remove_override(&language_id, index).map_err(e2s)
}

// ----------------------------------------------------------------------
// Exports
// ----------------------------------------------------------------------

#[tauri::command]
pub fn export_language(state: State<'_, AppState>, language_id: String, version: Option<i64>, format: String, project: Option<String>) -> CmdResult<Exported> {
    let lib = lib(&state)?;
    export::export_language(&lib, &language_id, version, &format, project.as_deref()).map_err(e2s)
}

/// Writes an export to the file that the save dialog returned.
#[tauri::command]
pub fn save_text_file(path: String, content: String) -> CmdResult<()> {
    let p = PathBuf::from(path.trim());
    if !p.is_absolute() {
        return Err("The path must be absolute.".into());
    }
    std::fs::write(&p, content).map_err(|e| format!("Cannot write {}: {e}", p.display()))
}

// ----------------------------------------------------------------------
// Cost
// ----------------------------------------------------------------------

/// The tokens (and the money, when the providers have prices) of a pass,
/// before it runs. `scope`: "queue", "all", "document" or "other-model".
#[tauri::command]
pub fn estimate(state: State<'_, AppState>, language_id: String, scope: String) -> CmdResult<Estimate> {
    let lib = lib(&state)?;
    cost::estimate(&lib, &ai_config(&state), &language_id, cost::Scope::parse(&scope)).map_err(e2s)
}

#[derive(Serialize)]
pub struct OtherModelView {
    /// The model that analyzes images now.
    model: String,
    other: OtherModel,
}

/// The analyses of a language that a model other than the current one wrote.
#[tauri::command]
pub fn other_model(state: State<'_, AppState>, language_id: String) -> CmdResult<OtherModelView> {
    let model = ai_config(&state).for_role(Role::Analysis).map(|p| p.model.clone()).unwrap_or_default();
    if model.is_empty() {
        return Ok(OtherModelView { model, other: OtherModel::default() });
    }
    let other = lib(&state)?.other_model_analyses(&language_id, &model).map_err(e2s)?;
    Ok(OtherModelView { model, other })
}

#[tauri::command]
pub fn reanalyze_other_model(state: State<'_, AppState>, language_id: String) -> CmdResult<usize> {
    let model = ai_config(&state).for_role(Role::Analysis).map(|p| p.model.clone()).unwrap_or_default();
    if model.is_empty() {
        return Err("Set a provider for images and sources in Settings first.".into());
    }
    lib(&state)?.reanalyze_other_model(&language_id, &model).map_err(e2s)
}

// ----------------------------------------------------------------------
// Snapshots
// ----------------------------------------------------------------------

#[derive(Serialize, Clone)]
struct Progress {
    done: u64,
    total: u64,
}

#[tauri::command]
pub async fn create_snapshot<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: State<'_, AppState>, dest: String) -> CmdResult<SnapshotReport> {
    let lib = lib(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
        snapshot::create(&lib, &PathBuf::from(dest.trim()), &mut |done, total| {
            if last.elapsed() > std::time::Duration::from_millis(150) || done == total {
                last = std::time::Instant::now();
                let _ = app.emit("snapshot-progress", Progress { done, total });
            }
        })
    })
    .await
    .map_err(e2s)?
    .map_err(e2s)
}

/// Opens a snapshot (zip) as a library in `dest`. The app switches to it with set_library.
#[tauri::command]
pub async fn restore_snapshot(zip: String, dest: String) -> CmdResult<u64> {
    tauri::async_runtime::spawn_blocking(move || snapshot::restore(&PathBuf::from(zip.trim()), &PathBuf::from(dest.trim()))).await.map_err(e2s)?.map_err(e2s)
}

#[tauri::command]
pub fn db_copies(state: State<'_, AppState>) -> CmdResult<Vec<DbCopy>> {
    Ok(snapshot::db_copies(lib(&state)?.root()))
}

// ----------------------------------------------------------------------
// Usage insight and board import
// ----------------------------------------------------------------------

#[tauri::command]
pub fn usage_insight(state: State<'_, AppState>, language_id: String, days: i64) -> CmdResult<Usage> {
    let lib = lib(&state)?;
    insight::usage(&lib, &language_id, days).map_err(e2s)
}

/// Reads the list of images of an Are.na channel or a Pinterest board. Nothing is added yet.
#[tauri::command]
pub async fn preview_board(url: String) -> CmdResult<Board> {
    importers::fetch_board(&url).await.map_err(e2s)
}

#[derive(Serialize, Clone)]
struct BoardProgress {
    done: usize,
    total: usize,
}

#[tauri::command]
pub async fn import_board(app: tauri::AppHandle, state: State<'_, AppState>, language_id: String, url: String) -> CmdResult<ImportSummary> {
    let lib = lib(&state)?;
    let board = importers::fetch_board(&url).await.map_err(e2s)?;
    let app2 = app.clone();
    let sum = importers::import_board(&lib, &language_id, &board, &move |done, total| {
        let _ = app2.emit("board-import-progress", BoardProgress { done, total });
    })
    .await
    .map_err(e2s)?;
    let _ = app.emit("library-changed", language_id);
    Ok(sum)
}
