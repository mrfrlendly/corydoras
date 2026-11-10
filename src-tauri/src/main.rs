#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use base64::Engine;
use corydoras_core::{
    activity::{Activity, Current, LogEntry},
    agent,
    ai::{self, AiConfig, ProviderProfile, Role},
    config::{self, AppConfig},
    library::{JOB_ANALYZE, JOB_SOURCE, JOB_SYNTH},
    models::*,
    provider::{self, CheckReport, Provider},
    secrets, snapshot,
    storage::{self, FolderInfo, MoveReport},
    video, worker, Library,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tauri::{Emitter, Listener, Manager, RunEvent, State, WindowEvent};

mod bridge;
mod capture;
mod features;
#[cfg(test)]
mod ipc_tests;

// ----------------------------------------------------------------------
// State
// ----------------------------------------------------------------------

#[derive(Clone, Serialize, Default)]
struct WorkerState {
    /// "idle", "working" or "paused".
    state: String,
    message: String,
    last_event: String,
}

#[derive(Default)]
struct AppState {
    lib: RwLock<Option<Arc<Library>>>,
    worker: Mutex<WorkerState>,
    activity: Arc<Activity>,
    /// A library move runs: the worker starts no job.
    moving: AtomicBool,
    /// The worker runs a step now.
    busy: AtomicBool,
    /// The local bridge for the browser extension.
    bridge: bridge::BridgeState,
}

type CmdResult<T> = Result<T, String>;

fn e2s<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn lib(state: &State<'_, AppState>) -> CmdResult<Arc<Library>> {
    state
        .lib
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or_else(|| "No library folder is set.".to_string())
}

// ----------------------------------------------------------------------
// App info and library
// ----------------------------------------------------------------------

#[derive(Serialize)]
struct AppInfo {
    library_path: Option<String>,
    default_library: String,
    config_path: String,
    mcp_command: Option<String>,
    version: String,
    /// Other libraries that the app opened before and that still exist.
    recent_libraries: Vec<String>,
    /// A warning when the library folder syncs with a cloud service.
    library_warning: Option<String>,
}

/// The installed MCP server binary, if it exists.
fn mcp_command() -> Option<String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs_home() {
        candidates.push(home.join(".local/bin/corydoras-mcp"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("corydoras-mcp"));
        }
    }
    candidates
        .into_iter()
        .find(|p| p.exists())
        .map(|p| p.to_string_lossy().into_owned())
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn app_info_of(state: &State<'_, AppState>) -> AppInfo {
    let path = state
        .lib
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|l| l.root().to_string_lossy().into_owned());
    let cfg = AppConfig::load();
    let recent_libraries = cfg
        .recent_libraries
        .iter()
        .filter(|p| storage::is_library(p) && Some(p.to_string_lossy().into_owned()) != path)
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let library_warning = path.as_ref().and_then(|p| storage::cloud_warning(std::path::Path::new(p)));
    AppInfo {
        recent_libraries,
        library_warning,
        library_path: path,
        default_library: config::default_library_path().to_string_lossy().into_owned(),
        config_path: config::config_path().to_string_lossy().into_owned(),
        mcp_command: mcp_command(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// Keeps today's copy of the database (the safety net of Settings › Library).
/// A failure is not an error for the user: the library still works.
fn daily_copy(lib: &Library) {
    if let Err(e) = snapshot::daily_db_copy(lib) {
        eprintln!("Cannot keep the daily copy of the database: {e}");
    }
}

#[tauri::command]
fn app_info(state: State<'_, AppState>) -> AppInfo {
    app_info_of(&state)
}

#[tauri::command]
fn set_library(app: tauri::AppHandle, state: State<'_, AppState>, path: String) -> CmdResult<AppInfo> {
    let path = PathBuf::from(path.trim());
    if path.as_os_str().is_empty() {
        return Err("Choose a folder.".into());
    }
    if state.moving.load(Ordering::SeqCst) {
        return Err("A library move runs. Wait until it ends.".into());
    }
    let info = storage::folder_info(&path);
    if info.exists && !info.is_library && !info.empty {
        return Err("That folder has other files in it. Choose a library folder, an empty folder, or a new folder.".into());
    }
    let lib = Library::open(&path).map_err(e2s)?;
    daily_copy(&lib);
    let mut cfg = AppConfig::load();
    cfg.set_library(path);
    cfg.save().map_err(e2s)?;
    *state.lib.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(lib));
    capture::refresh_tray(&app);
    Ok(app_info_of(&state))
}

#[tauri::command]
fn folder_info(path: String) -> FolderInfo {
    storage::folder_info(&PathBuf::from(path.trim()))
}

/// Shows a folder in Finder.
#[tauri::command]
fn reveal_path(path: String) -> CmdResult<()> {
    let p = PathBuf::from(path.trim());
    if !p.exists() {
        return Err(format!("{} does not exist.", p.display()));
    }
    std::process::Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" })
        .arg(&p)
        .spawn()
        .map_err(e2s)?;
    Ok(())
}

#[derive(Serialize, Clone)]
struct MoveProgress {
    done: u64,
    total: u64,
}

#[derive(Serialize)]
struct MoveResult {
    report: MoveReport,
    info: AppInfo,
    old_path: String,
    /// Set when the old folder could not go to the Trash.
    trash_error: Option<String>,
    trashed: bool,
}

/// Moves the library to a new folder: stops the worker, copies and checks the
/// library, switches to the copy, and then (optional) moves the old folder to the Trash.
#[tauri::command]
async fn move_library(app: tauri::AppHandle, state: State<'_, AppState>, path: String, trash_old: bool) -> CmdResult<MoveResult> {
    let lib = lib(&state)?;
    let dst = PathBuf::from(path.trim());
    let old = lib.root().to_path_buf();
    storage::check_move_target(&old, &dst).map_err(e2s)?;
    if state.moving.swap(true, Ordering::SeqCst) {
        return Err("A library move runs already.".into());
    }
    // Stop the running job; it goes back in the queue of the moved library.
    let resume = state.activity.current().map(|c| (c.kind, c.item_id, c.language_id));
    if resume.is_some() {
        state.activity.cancel(None);
        state.activity.log("info", None, "Stopped the running job for the library move. It runs again after the move.");
    }
    for _ in 0..600 {
        if !state.busy.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if state.busy.load(Ordering::SeqCst) {
        state.moving.store(false, Ordering::SeqCst);
        return Err("The running job did not stop. Try the move again in a moment.".into());
    }
    let (lib2, dst2, app2) = (lib.clone(), dst.clone(), app.clone());
    let copied = tauri::async_runtime::spawn_blocking(move || {
        let mut last = std::time::Instant::now() - Duration::from_secs(1);
        storage::copy_library(&lib2, &dst2, &mut |done, total| {
            if last.elapsed() > Duration::from_millis(150) || done == total {
                last = std::time::Instant::now();
                let _ = app2.emit("library-move-progress", MoveProgress { done, total });
            }
        })
    })
    .await
    .map_err(e2s)
    .and_then(|r| r.map_err(e2s));
    let report = match copied {
        Ok(r) => r,
        Err(e) => {
            state.moving.store(false, Ordering::SeqCst);
            return Err(format!("The move failed and the library stays at {}. {e}", old.display()));
        }
    };
    let switched = (|| -> anyhow::Result<()> {
        let new_lib = Library::open(&dst)?;
        if let Some((kind, item, language)) = &resume {
            match (kind.as_str(), item) {
                (JOB_ANALYZE, Some(id)) => new_lib.reanalyze_item(id)?,
                (JOB_SOURCE, Some(id)) => new_lib.reanalyze_source(id)?,
                (JOB_SYNTH, _) => new_lib.enqueue(JOB_SYNTH, language, 0)?,
                _ => {}
            }
        }
        let mut cfg = AppConfig::load();
        cfg.set_library(dst.clone());
        // The old folder is not a library to come back to after a move.
        cfg.recent_libraries.retain(|p| p != &old);
        cfg.save()?;
        *state.lib.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(new_lib));
        Ok(())
    })();
    state.moving.store(false, Ordering::SeqCst);
    if let Err(e) = switched {
        return Err(format!("The copy at {} is complete, but the app could not switch to it: {e}. The app still uses {}.", dst.display(), old.display()));
    }
    drop(lib);
    state.activity.log("ok", None, format!("Moved the library to {}.", dst.display()));
    let (mut trashed, mut trash_error) = (false, None);
    if trash_old {
        // Give open file handles of the old library a moment to close.
        tokio::time::sleep(Duration::from_millis(300)).await;
        match storage::trash_folder(&old) {
            Ok(()) => trashed = true,
            Err(e) => trash_error = Some(e.to_string()),
        }
    }
    let _ = app.emit("library-changed", "");
    Ok(MoveResult {
        report,
        info: app_info_of(&state),
        old_path: old.to_string_lossy().into_owned(),
        trash_error,
        trashed,
    })
}

// ----------------------------------------------------------------------
// Languages and items
// ----------------------------------------------------------------------

#[tauri::command]
fn list_languages(state: State<'_, AppState>) -> CmdResult<Vec<Language>> {
    lib(&state)?.list_languages().map_err(e2s)
}

#[tauri::command]
fn create_language(app: tauri::AppHandle, state: State<'_, AppState>, name: String, description: String) -> CmdResult<Language> {
    let l = lib(&state)?.create_language(&name, &description).map_err(e2s)?;
    capture::refresh_tray(&app);
    Ok(l)
}

#[tauri::command]
fn update_language(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
    name: String,
    description: String,
) -> CmdResult<Language> {
    let l = lib(&state)?.update_language(&id, &name, &description).map_err(e2s)?;
    capture::refresh_tray(&app);
    Ok(l)
}

#[tauri::command]
fn list_items(state: State<'_, AppState>, language_id: String) -> CmdResult<Vec<Item>> {
    lib(&state)?.list_items(&language_id).map_err(e2s)
}

#[tauri::command]
fn set_canvas(state: State<'_, AppState>, language_id: String, places: Vec<CanvasPlacement>) -> CmdResult<usize> {
    lib(&state)?.set_canvas(&language_id, &places).map_err(e2s)
}

#[tauri::command]
fn clear_canvas(state: State<'_, AppState>, language_id: String) -> CmdResult<()> {
    lib(&state)?.clear_canvas(&language_id).map_err(e2s)
}

#[tauri::command]
fn item_detail(state: State<'_, AppState>, item_id: String) -> CmdResult<ItemDetail> {
    lib(&state)?.item_detail(&item_id).map_err(e2s)
}

#[derive(Serialize)]
struct ImportReport {
    added: Vec<Item>,
    errors: Vec<String>,
    /// One line for each video: its name, the number of frames and its length.
    videos: Vec<VideoNote>,
}

#[derive(Serialize)]
struct VideoNote {
    name: String,
    frames: usize,
    duration: f64,
}

/// Images and videos from disk. Videos give their key frames. Runs off the main
/// thread, because a video takes seconds.
#[tauri::command]
async fn import_files(state: State<'_, AppState>, language_id: String, paths: Vec<String>) -> CmdResult<ImportReport> {
    let lib = lib(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut report = ImportReport {
            added: vec![],
            errors: vec![],
            videos: vec![],
        };
        for p in paths {
            let path = PathBuf::from(&p);
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if video::is_video_name(&name) {
                match lib.import_video(&language_id, &path, &name, Some(&p)) {
                    Ok(v) => {
                        report.videos.push(VideoNote { name, frames: v.items.len(), duration: v.duration });
                        report.added.extend(v.items);
                    }
                    Err(e) => report.errors.push(e.to_string()),
                }
                continue;
            }
            // A 3D model: the app draws four views of it.
            if corydoras_core::mesh::is_model_name(&name) {
                match lib.import_model(&language_id, &path, &name, Some(&p)) {
                    Ok(item) => report.added.push(item),
                    Err(e) => report.errors.push(format!("{name}: {e}")),
                }
                continue;
            }
            match lib.import_path(&language_id, &path) {
                Ok(item) => report.added.push(item),
                Err(e) => report.errors.push(e.to_string()),
            }
        }
        report
    })
    .await
    .map_err(e2s)
}

/// Largest video that a drop can send (bytes).
const MAX_VIDEO_BYTES: usize = 2 * 1024 * 1024 * 1024;

/// A video dropped from Finder. The body is the raw file; the headers carry
/// x-language-id and x-file-name (URI-encoded). The file goes to a temporary
/// file for AVFoundation and is removed afterwards.
#[tauri::command]
async fn import_video_bytes(state: State<'_, AppState>, request: tauri::ipc::Request<'_>) -> CmdResult<VideoImport> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("The video data is missing.".into());
    };
    if bytes.len() > MAX_VIDEO_BYTES {
        return Err("The video is larger than 2 GB.".into());
    }
    let header = |k: &str| -> CmdResult<String> {
        let v = request
            .headers()
            .get(k)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| format!("The header {k} is missing."))?;
        urlencoding::decode(v).map(|s| s.into_owned()).map_err(e2s)
    };
    let language_id = header("x-language-id")?;
    let name = header("x-file-name")?;
    let lib = lib(&state)?;
    let ext = std::path::Path::new(&name)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "mov".into());
    let tmp_dir = lib.root().join(".tmp");
    std::fs::create_dir_all(&tmp_dir).map_err(e2s)?;
    let tmp = tmp_dir.join(format!("{}.{ext}", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, bytes).map_err(e2s)?;
    let result = tauri::async_runtime::spawn_blocking({
        let tmp = tmp.clone();
        move || lib.import_video(&language_id, &tmp, &name, None)
    })
    .await
    .map_err(e2s);
    let _ = std::fs::remove_file(&tmp);
    result?.map_err(e2s)
}

#[tauri::command]
fn import_bytes(
    state: State<'_, AppState>,
    language_id: String,
    name: String,
    data: String,
    source: Option<String>,
) -> CmdResult<Item> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.trim())
        .map_err(|e| format!("Bad file data: {e}"))?;
    lib(&state)?
        .import_bytes(&language_id, &name, &bytes, source.as_deref())
        .map_err(e2s)
}

#[tauri::command]
async fn import_url(
    state: State<'_, AppState>,
    language_id: String,
    url: String,
    source: Option<String>,
) -> CmdResult<Item> {
    let lib = lib(&state)?;
    let bytes = provider::download(&url).await.map_err(e2s)?;
    let name = url
        .split(['?', '#'])
        .next()
        .and_then(|u| u.rsplit('/').next())
        .filter(|s| !s.is_empty())
        .unwrap_or("web-image")
        .to_string();
    let src = source.filter(|s| !s.trim().is_empty()).unwrap_or(url);
    lib.import_bytes(&language_id, &name, &bytes, Some(&src)).map_err(e2s)
}

#[tauri::command]
fn list_sources(state: State<'_, AppState>, language_id: String) -> CmdResult<Vec<Source>> {
    lib(&state)?.list_sources(&language_id).map_err(e2s)
}

#[tauri::command]
fn source_detail(state: State<'_, AppState>, source_id: String) -> CmdResult<SourceDetail> {
    lib(&state)?.source_detail(&source_id).map_err(e2s)
}

#[tauri::command]
async fn add_link(state: State<'_, AppState>, language_id: String, url: String) -> CmdResult<Source> {
    let lib = lib(&state)?;
    lib.get_language(&language_id).map_err(e2s)?;
    let url = url.trim().to_string();
    let ex = corydoras_core::sources::fetch_link(&url).await.map_err(e2s)?;
    tauri::async_runtime::spawn_blocking(move || lib.add_link_source(&language_id, &url, &ex))
        .await
        .map_err(e2s)?
        .map_err(e2s)
}

#[tauri::command]
async fn import_source_files(state: State<'_, AppState>, language_id: String, paths: Vec<String>) -> CmdResult<SourceImportReport> {
    let lib = lib(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut report = SourceImportReport { added: vec![], errors: vec![] };
        for p in paths {
            let path = PathBuf::from(&p);
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| lib.import_document(&language_id, &name, &b).map_err(e2s)) {
                Ok(s) => report.added.push(s),
                Err(e) => report.errors.push(format!("{name}: {e}")),
            }
        }
        report
    })
    .await
    .map_err(e2s)
}

#[tauri::command]
async fn import_source_bytes(state: State<'_, AppState>, language_id: String, name: String, data: String) -> CmdResult<Source> {
    let lib = lib(&state)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.trim())
        .map_err(|e| format!("Bad file data: {e}"))?;
    tauri::async_runtime::spawn_blocking(move || lib.import_document(&language_id, &name, &bytes))
        .await
        .map_err(e2s)?
        .map_err(e2s)
}

#[derive(Serialize)]
struct SourceImportReport {
    added: Vec<Source>,
    errors: Vec<String>,
}

#[tauri::command]
fn update_source(state: State<'_, AppState>, source_id: String, title: String, note: String, weight: i64) -> CmdResult<Source> {
    lib(&state)?.update_source(&source_id, &title, &note, weight).map_err(e2s)
}

#[tauri::command]
fn remove_source(state: State<'_, AppState>, source_id: String) -> CmdResult<()> {
    lib(&state)?.remove_source(&source_id).map_err(e2s)
}

#[tauri::command]
fn reanalyze_source(state: State<'_, AppState>, source_id: String) -> CmdResult<()> {
    lib(&state)?.reanalyze_source(&source_id).map_err(e2s)
}

/// Opens a link in the browser, or a document in its default app.
#[tauri::command]
fn open_source(state: State<'_, AppState>, source_id: String) -> CmdResult<()> {
    let lib = lib(&state)?;
    let s = lib.get_source(&source_id).map_err(e2s)?;
    let target = match (&s.url, &s.file_rel) {
        (Some(u), _) => u.clone(),
        (None, Some(f)) => lib.abs(f).map_err(e2s)?.to_string_lossy().into_owned(),
        _ => return Err("This source has nothing to open.".into()),
    };
    std::process::Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" })
        .arg(&target)
        .spawn()
        .map_err(e2s)?;
    Ok(())
}

#[tauri::command]
fn update_item(
    state: State<'_, AppState>,
    item_id: String,
    tags: Vec<String>,
    note: String,
    weight: i64,
    anti: bool,
) -> CmdResult<Item> {
    lib(&state)?
        .update_item(&item_id, &tags, &note, weight, anti)
        .map_err(e2s)
}

#[tauri::command]
fn set_regions(state: State<'_, AppState>, item_id: String, regions: Vec<RegionInput>) -> CmdResult<Vec<Region>> {
    lib(&state)?.set_regions(&item_id, &regions).map_err(e2s)
}

#[tauri::command]
fn remove_item(state: State<'_, AppState>, item_id: String) -> CmdResult<()> {
    lib(&state)?.remove_item(&item_id).map_err(e2s)
}

#[tauri::command]
fn reanalyze_item(state: State<'_, AppState>, item_id: String) -> CmdResult<()> {
    lib(&state)?.reanalyze_item(&item_id).map_err(e2s)
}

#[tauri::command]
fn reanalyze_all(state: State<'_, AppState>, language_id: String) -> CmdResult<usize> {
    lib(&state)?.reanalyze_all(&language_id).map_err(e2s)
}

#[tauri::command]
fn retry_failed(state: State<'_, AppState>, language_id: String) -> CmdResult<usize> {
    lib(&state)?.retry_failed_items(&language_id).map_err(e2s)
}

// ----------------------------------------------------------------------
// Versions
// ----------------------------------------------------------------------

#[tauri::command]
fn list_versions(state: State<'_, AppState>, language_id: String) -> CmdResult<Vec<VersionInfo>> {
    lib(&state)?.list_versions(&language_id).map_err(e2s)
}

#[tauri::command]
fn get_version(state: State<'_, AppState>, language_id: String, version: i64) -> CmdResult<VersionContent> {
    lib(&state)?.read_version(&language_id, version).map_err(e2s)
}

#[tauri::command]
fn save_edited_version(
    state: State<'_, AppState>,
    language_id: String,
    markdown: String,
    tokens: String,
) -> CmdResult<i64> {
    lib(&state)?
        .save_edited_version(&language_id, &markdown, &tokens)
        .map_err(e2s)
}

#[tauri::command]
fn list_proposals(
    state: State<'_, AppState>,
    language_id: String,
    include_decided: bool,
) -> CmdResult<Vec<corydoras_core::proposals::Proposal>> {
    lib(&state)?.list_proposals(&language_id, include_decided).map_err(e2s)
}

#[tauri::command]
fn accept_proposal(state: State<'_, AppState>, proposal_id: String) -> CmdResult<i64> {
    lib(&state)?.accept_proposal(&proposal_id).map_err(e2s)
}

#[tauri::command]
fn reject_proposal(state: State<'_, AppState>, proposal_id: String) -> CmdResult<()> {
    lib(&state)?.reject_proposal(&proposal_id).map_err(e2s)
}

#[tauri::command]
fn list_outputs(state: State<'_, AppState>, language_id: String) -> CmdResult<Vec<corydoras_core::outputs::Output>> {
    lib(&state)?.list_outputs(&language_id).map_err(e2s)
}

#[tauri::command]
fn set_output_verdict(
    state: State<'_, AppState>,
    output_id: String,
    verdict: String,
    note: String,
) -> CmdResult<corydoras_core::outputs::Output> {
    lib(&state)?.set_output_verdict(&output_id, &verdict, &note).map_err(e2s)
}

#[tauri::command]
fn remove_output(state: State<'_, AppState>, output_id: String) -> CmdResult<()> {
    lib(&state)?.remove_output(&output_id).map_err(e2s)
}

#[tauri::command]
fn synthesize_now(state: State<'_, AppState>, language_id: String) -> CmdResult<()> {
    lib(&state)?.synthesize_now(&language_id).map_err(e2s)
}

#[tauri::command]
fn language_text(state: State<'_, AppState>, language_id: String, version: Option<i64>) -> CmdResult<String> {
    let lib = lib(&state)?;
    agent::language_text(&lib, &language_id, version, None).map_err(e2s)
}

// ----------------------------------------------------------------------
// Projects
// ----------------------------------------------------------------------

#[tauri::command]
fn list_projects(state: State<'_, AppState>) -> CmdResult<Vec<Project>> {
    lib(&state)?.list_projects().map_err(e2s)
}

#[tauri::command]
fn create_project(state: State<'_, AppState>, name: String) -> CmdResult<Project> {
    lib(&state)?.create_project(&name).map_err(e2s)
}

#[tauri::command]
fn rename_project(state: State<'_, AppState>, id: String, name: String) -> CmdResult<Project> {
    lib(&state)?.rename_project(&id, &name).map_err(e2s)
}

#[tauri::command]
fn delete_project(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    lib(&state)?.delete_project(&id).map_err(e2s)
}

#[tauri::command]
fn set_pins(state: State<'_, AppState>, project_id: String, pins: Vec<PinInput>) -> CmdResult<Project> {
    lib(&state)?.set_pins(&project_id, &pins).map_err(e2s)
}

#[tauri::command]
fn set_project_rule_sets(state: State<'_, AppState>, project_id: String, rule_set_ids: Vec<String>) -> CmdResult<Project> {
    lib(&state)?
        .set_project_rule_sets(&project_id, &rule_set_ids)
        .map_err(e2s)
}

#[tauri::command]
fn list_rule_sets(state: State<'_, AppState>) -> CmdResult<Vec<RuleSet>> {
    lib(&state)?.list_rule_sets().map_err(e2s)
}

#[tauri::command]
fn create_rule_set(state: State<'_, AppState>, input: RuleSetInput) -> CmdResult<RuleSet> {
    lib(&state)?.create_rule_set(&input).map_err(e2s)
}

#[tauri::command]
fn update_rule_set(state: State<'_, AppState>, id: String, input: RuleSetInput) -> CmdResult<RuleSet> {
    lib(&state)?.update_rule_set(&id, &input).map_err(e2s)
}

#[tauri::command]
fn delete_rule_set(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    lib(&state)?.delete_rule_set(&id).map_err(e2s)
}

#[tauri::command]
fn rule_set_text(state: State<'_, AppState>, id: String) -> CmdResult<String> {
    let r = lib(&state)?.get_rule_set(&id).map_err(e2s)?;
    Ok(agent::rule_set_text(&r))
}

#[tauri::command]
fn project_brief(state: State<'_, AppState>, project_id: String) -> CmdResult<String> {
    let lib = lib(&state)?;
    agent::project_brief_text(&lib, &project_id).map_err(e2s)
}

// ----------------------------------------------------------------------
// Settings, key, status
// ----------------------------------------------------------------------

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> CmdResult<Settings> {
    lib(&state)?.settings().map_err(e2s)
}

#[tauri::command]
fn save_settings(state: State<'_, AppState>, settings: Settings) -> CmdResult<Settings> {
    lib(&state)?.save_settings(&settings).map_err(e2s)
}

/// The effective provider configuration (the 0.4 setting is migrated when none is saved).
fn ai_config(state: &State<'_, AppState>) -> AiConfig {
    let settings = lib(state).and_then(|l| l.settings().map_err(e2s)).unwrap_or_default();
    AiConfig::effective(AppConfig::load().ai.as_ref(), &settings)
}

#[derive(Serialize)]
struct AiView {
    config: AiConfig,
    presets: Vec<ai::Preset>,
    /// Profile ID -> a key is saved (or none is necessary).
    keys: HashMap<String, bool>,
}

fn ai_view(cfg: AiConfig) -> AiView {
    let keys = cfg.profiles.iter().map(|p| (p.id.clone(), p.has_key())).collect();
    AiView { config: cfg, presets: ai::presets(), keys }
}

#[tauri::command]
fn get_ai(state: State<'_, AppState>) -> AiView {
    ai_view(ai_config(&state))
}

#[tauri::command]
fn save_ai(state: State<'_, AppState>, config: AiConfig) -> CmdResult<AiView> {
    let before = ai_config(&state);
    let cfg = config.normalized().map_err(e2s)?;
    let mut app_cfg = AppConfig::load();
    app_cfg.ai = Some(cfg.clone());
    app_cfg.save().map_err(e2s)?;
    // Remove the keys of removed providers.
    for old in &before.profiles {
        if !cfg.profiles.iter().any(|p| p.key_account == old.key_account) {
            let _ = secrets::clear_key(&old.key_account);
        }
    }
    Ok(ai_view(cfg))
}

#[tauri::command]
fn new_profile(kind: String) -> ProviderProfile {
    ProviderProfile::from_preset(&kind)
}

fn find_profile(state: &State<'_, AppState>, id: &str) -> CmdResult<ProviderProfile> {
    ai_config(state)
        .profile(id)
        .cloned()
        .ok_or_else(|| "Save the provider first.".to_string())
}

#[tauri::command]
fn set_provider_key(state: State<'_, AppState>, profile_id: String, key: String) -> CmdResult<()> {
    let p = find_profile(&state, &profile_id)?;
    secrets::set_key(&p.key_account, &key).map_err(e2s)
}

#[tauri::command]
fn clear_provider_key(state: State<'_, AppState>, profile_id: String) -> CmdResult<()> {
    let p = find_profile(&state, &profile_id)?;
    secrets::clear_key(&p.key_account).map_err(e2s)
}

/// Tests a provider (the form values, with the saved key) and records whether the model reads images.
#[tauri::command]
async fn check_provider(state: State<'_, AppState>, profile: ProviderProfile) -> CmdResult<CheckReport> {
    let profile = profile.normalized().map_err(e2s)?;
    let key = profile.key().map_err(e2s)?;
    let p = Provider::from_profile(&profile, &key).map_err(e2s)?;
    let report = p.check().await.map_err(e2s)?;
    // Save the vision result on the saved profile with the same model.
    let mut cfg = ai_config(&state);
    if let Some(saved) = cfg.profiles.iter_mut().find(|x| x.id == profile.id && x.model == profile.model) {
        saved.vision = Some(report.vision);
        let mut app_cfg = AppConfig::load();
        if app_cfg.ai.is_some() {
            app_cfg.ai = Some(cfg);
            let _ = app_cfg.save();
        }
    }
    Ok(report)
}

#[tauri::command]
async fn list_models(profile: ProviderProfile) -> CmdResult<Vec<String>> {
    let mut profile = profile.normalized().map_err(e2s)?;
    let key = profile.key().unwrap_or_default();
    if profile.model.is_empty() {
        profile.model = "list".into();
    }
    Provider::from_profile(&profile, &key).map_err(e2s)?.list_models().await.map_err(e2s)
}

// 0.4 commands, kept for older windows of the UI: they act on the analysis provider.
#[tauri::command]
fn has_api_key(state: State<'_, AppState>) -> bool {
    ai_config(&state).for_role(Role::Analysis).map(|p| p.has_key()).unwrap_or(false)
}

#[derive(Serialize)]
struct StatusReport {
    worker: WorkerState,
    languages: Vec<LanguageStatus>,
    usage: Usage,
    budget: i64,
    has_key: bool,
    /// The job that runs now, with its phase and live token counts.
    current: Option<Current>,
    paused: bool,
    /// Manual mode: jobs wait until you run the queue.
    manual: bool,
    /// Manual mode: the queue runs now, until it is empty.
    running: bool,
    /// Jobs in the queue, the running one included.
    queued: i64,
    now_ms: i64,
}

#[tauri::command]
fn status(state: State<'_, AppState>) -> CmdResult<StatusReport> {
    let lib = lib(&state)?;
    let worker = state.worker.lock().unwrap_or_else(|e| e.into_inner()).clone();
    Ok(StatusReport {
        worker,
        languages: lib.language_statuses().map_err(e2s)?,
        usage: lib.usage().map_err(e2s)?,
        budget: lib.settings().map_err(e2s)?.monthly_token_budget,
        has_key: {
            let cfg = ai_config(&state);
            [Role::Analysis, Role::Synthesis]
                .iter()
                .all(|r| cfg.for_role(*r).map(|p| p.has_key() && !p.model.is_empty()).unwrap_or(false))
        },
        current: state.activity.current(),
        paused: lib.queue_paused().map_err(e2s)?,
        manual: lib.queue_manual().map_err(e2s)?,
        running: state.activity.run_requested(),
        queued: lib.job_count().map_err(e2s)?,
        now_ms: corydoras_core::util::now_ms(),
    })
}

#[derive(Serialize)]
struct ActivityReport {
    current: Option<Current>,
    jobs: Vec<JobView>,
    log: Vec<LogEntry>,
    paused: bool,
    /// Manual mode, and whether the user started the queue.
    manual: bool,
    running: bool,
    now_ms: i64,
}

#[tauri::command]
fn activity(state: State<'_, AppState>) -> CmdResult<ActivityReport> {
    let lib = lib(&state)?;
    let snap = state.activity.snapshot();
    let running = snap.current.as_ref().map(|c| c.job_id);
    let jobs = lib
        .list_jobs()
        .map_err(e2s)?
        .into_iter()
        .map(|mut j| {
            j.running = Some(j.id) == running;
            j
        })
        .collect();
    Ok(ActivityReport {
        current: snap.current,
        jobs,
        log: snap.log,
        paused: lib.queue_paused().map_err(e2s)?,
        manual: lib.queue_manual().map_err(e2s)?,
        running: state.activity.run_requested(),
        now_ms: snap.now_ms,
    })
}

#[tauri::command]
fn cancel_job(app: tauri::AppHandle, state: State<'_, AppState>, job_id: i64) -> CmdResult<()> {
    let lib = lib(&state)?;
    let running = state.activity.cancel(Some(job_id));
    let lang = lib.cancel_job(job_id).map_err(e2s)?;
    if !running {
        if let Some(l) = &lang {
            state.activity.log("warn", Some(l), "Removed a job from the queue.");
        }
    }
    if let Some(l) = lang {
        let _ = app.emit("library-changed", l);
    }
    Ok(())
}

#[tauri::command]
fn cancel_all(app: tauri::AppHandle, state: State<'_, AppState>, language_id: Option<String>) -> CmdResult<usize> {
    let lib = lib(&state)?;
    match language_id.as_deref() {
        Some(l) => {
            state.activity.cancel_language(l);
        }
        None => {
            state.activity.cancel(None);
        }
    }
    let n = lib.cancel_jobs(language_id.as_deref()).map_err(e2s)?;
    state.activity.log(
        "warn",
        language_id.as_deref(),
        format!("Cancelled {n} job{} in the queue.", if n == 1 { "" } else { "s" }),
    );
    let _ = app.emit("library-changed", language_id.unwrap_or_default());
    Ok(n)
}

#[tauri::command]
fn set_manual(state: State<'_, AppState>, manual: bool) -> CmdResult<()> {
    let lib = lib(&state)?;
    lib.set_queue_manual(manual).map_err(e2s)?;
    state.activity.set_run_requested(false);
    state.activity.log(
        "info",
        None,
        if manual {
            "Manual mode: new jobs wait in the queue until you click Run queue. A running job finishes."
        } else {
            "Automatic mode: jobs run as soon as they are due."
        },
    );
    Ok(())
}

#[tauri::command]
fn run_queue(state: State<'_, AppState>, run: bool) -> CmdResult<()> {
    let lib = lib(&state)?;
    state.activity.set_run_requested(run);
    let n = lib.job_count().map_err(e2s)?;
    state.activity.log(
        "info",
        None,
        if run {
            format!("Started the queue by hand: {n} job{} to run.", if n == 1 { "" } else { "s" })
        } else {
            "Stopped the queue run. The running job finishes; the rest wait.".to_string()
        },
    );
    Ok(())
}

#[tauri::command]
fn set_paused(state: State<'_, AppState>, paused: bool) -> CmdResult<()> {
    let lib = lib(&state)?;
    lib.set_queue_paused(paused).map_err(e2s)?;
    state.activity.log(
        "info",
        None,
        if paused {
            "Paused the queue. The running job finishes; no new job starts."
        } else {
            "Resumed the queue."
        },
    );
    Ok(())
}

// ----------------------------------------------------------------------
// Background worker
// ----------------------------------------------------------------------

fn set_worker(app: &tauri::AppHandle, state: &str, message: &str, event: Option<&str>) {
    let st = app.state::<AppState>();
    let mut w = st.worker.lock().unwrap_or_else(|e| e.into_inner());
    w.state = state.to_string();
    w.message = message.to_string();
    if let Some(ev) = event {
        w.last_event = ev.to_string();
    }
}

async fn worker_loop(app: tauri::AppHandle) {
    loop {
        let lib = {
            let st = app.state::<AppState>();
            let guard = st.lib.read().unwrap_or_else(|e| e.into_inner());
            guard.clone()
        };
        let Some(lib) = lib else {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        };
        let st = app.state::<AppState>();
        if st.moving.load(Ordering::SeqCst) {
            set_worker(&app, "paused", "The library moves.", None);
            tokio::time::sleep(Duration::from_millis(300)).await;
            continue;
        }
        let act = st.activity.clone();
        st.busy.store(true, Ordering::SeqCst);
        set_worker(&app, "working", "", None);
        let outcome = worker::step(&lib, &act).await;
        app.state::<AppState>().busy.store(false, Ordering::SeqCst);
        drop(lib);
        match outcome {
            worker::Outcome::Idle => {
                set_worker(&app, "idle", "", None);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            worker::Outcome::Did { language_id, message } => {
                set_worker(&app, "idle", "", Some(&message));
                let _ = app.emit("library-changed", language_id);
            }
            worker::Outcome::Paused(msg) => {
                set_worker(&app, "paused", &msg, None);
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

// ----------------------------------------------------------------------
// cory:// protocol: serves files from the library folder to the UI
// ----------------------------------------------------------------------

fn mime_for(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        Some("tif") | Some("tiff") => "image/tiff",
        Some("json") => "application/json",
        Some("md") => "text/plain; charset=utf-8",
        Some("pdf") => "application/pdf",
        // Never serve stored web pages as HTML: they must not run in the app.
        Some("txt") | Some("csv") | Some("html") | Some("htm") | Some("markdown") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn serve_file(app: &tauri::AppHandle, uri_path: &str) -> tauri::http::Response<Vec<u8>> {
    let not_found = || {
        tauri::http::Response::builder()
            .status(404)
            .header("Content-Type", "text/plain")
            .body(b"not found".to_vec())
            .unwrap()
    };
    let root = {
        let st = app.state::<AppState>();
        let guard = st.lib.read().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().map(|l| l.root().to_path_buf())
    };
    let Some(root) = root else { return not_found() };
    let rel = corydoras_core::util::percent_decode(uri_path);
    let Some(path) = corydoras_core::util::safe_join(&root, &rel) else {
        return not_found();
    };
    match std::fs::read(&path) {
        Ok(bytes) => tauri::http::Response::builder()
            .status(200)
            .header("Content-Type", mime_for(&path))
            .header("Cache-Control", "max-age=31536000, immutable")
            .header("Access-Control-Allow-Origin", "*")
            .body(bytes)
            .unwrap(),
        Err(_) => not_found(),
    }
}

// ----------------------------------------------------------------------
// Main
// ----------------------------------------------------------------------

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        capture::start(app);
                    }
                })
                .build(),
        )
        // The menu bar icon keeps the app running: closing the window hides it.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .manage(AppState::default())
        .register_uri_scheme_protocol("cory", |ctx, request| {
            let path = request.uri().path().to_string();
            serve_file(ctx.app_handle(), &path)
        })
        .setup(|app| {
            // CORYDORAS_LIBRARY opens another library for this run only (development);
            // the saved setting does not change.
            if let Some(path) = config::resolve_library_path(None) {
                match Library::open(&path) {
                    Ok(lib) => {
                        daily_copy(&lib);
                        let st = app.state::<AppState>();
                        *st.lib.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(lib));
                    }
                    Err(e) => eprintln!("Cannot open the library at {}: {e}", path.display()),
                }
            }
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(worker_loop(handle));
            if let Err(e) = capture::create_tray(app.handle()) {
                eprintln!("Cannot make the menu bar icon: {e}");
            }
            capture::register_saved(app.handle());
            bridge::start(app.handle());
            // New, renamed or removed languages change the capture menu.
            let h = app.handle().clone();
            app.listen_any("library-changed", move |_| capture::refresh_tray(&h));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            set_library,
            list_languages,
            create_language,
            update_language,
            list_items,
            item_detail,
            set_canvas,
            clear_canvas,
            import_files,
            import_video_bytes,
            import_bytes,
            import_url,
            update_item,
            list_sources,
            source_detail,
            add_link,
            import_source_files,
            import_source_bytes,
            update_source,
            remove_source,
            reanalyze_source,
            open_source,
            set_regions,
            remove_item,
            reanalyze_item,
            retry_failed,
            reanalyze_all,
            activity,
            cancel_job,
            cancel_all,
            set_paused,
            set_manual,
            run_queue,
            list_versions,
            get_version,
            save_edited_version,
            list_proposals,
            accept_proposal,
            reject_proposal,
            list_outputs,
            set_output_verdict,
            remove_output,
            synthesize_now,
            language_text,
            list_projects,
            create_project,
            rename_project,
            delete_project,
            set_pins,
            set_project_rule_sets,
            list_rule_sets,
            create_rule_set,
            update_rule_set,
            delete_rule_set,
            rule_set_text,
            project_brief,
            get_settings,
            save_settings,
            has_api_key,
            get_ai,
            save_ai,
            new_profile,
            set_provider_key,
            clear_provider_key,
            check_provider,
            list_models,
            folder_info,
            reveal_path,
            move_library,
            status,
            capture::get_capture,
            capture::set_capture,
            capture::capture_now,
            bridge::get_bridge,
            bridge::set_bridge,
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
            features::preview_board,
            features::import_board
        ])
        .build(tauri::generate_context!())
        .expect("Corydoras failed to start")
        .run(|app, event| {
            // A click on the Dock icon brings the hidden window back.
            if let RunEvent::Reopen { has_visible_windows: false, .. } = event {
                capture::show_main(app);
            }
        });
}
