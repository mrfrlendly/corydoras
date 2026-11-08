//! Screen capture: the menu bar icon, its menu and the global keyboard shortcut.
//!
//! A capture runs macOS `screencapture -i` (drag a region, Space for a window,
//! Escape to cancel) and adds the picture to the capture language as an item.

use crate::{lib, AppState, CmdResult};
use corydoras_core::{
    config::{AppConfig, CaptureConfig},
    models::Language,
    util, Library,
};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

const TRAY_ID: &str = "corydoras";

/// A capture runs now; a second press of the shortcut does nothing.
static BUSY: AtomicBool = AtomicBool::new(false);
/// Why the saved shortcut could not be registered, if it could not.
static SHORTCUT_ERROR: Mutex<Option<String>> = Mutex::new(None);

#[derive(Serialize, Clone)]
pub struct CaptureView {
    /// The language that captures go to now (the saved one, or the fallback).
    pub language_id: Option<String>,
    pub language_name: Option<String>,
    /// The saved shortcut ("Control+Alt+Super+KeyC"), or None.
    pub shortcut: Option<String>,
    /// Why the shortcut does not work, for example another app has it.
    pub shortcut_error: Option<String>,
}

#[derive(Serialize, Clone)]
struct CaptureDone {
    language_id: String,
    language_name: String,
    item_id: String,
}

/// The capture language: the saved one, else the language that changed last.
fn target_language(lib: &Library, cfg: &CaptureConfig) -> Option<Language> {
    let langs = lib.list_languages().ok()?;
    if let Some(id) = &cfg.language_id {
        if let Some(l) = langs.iter().find(|l| &l.id == id) {
            return Some(l.clone());
        }
    }
    langs.into_iter().max_by(|a, b| a.updated_at.cmp(&b.updated_at))
}

fn current_lib(app: &AppHandle) -> Option<Arc<Library>> {
    lib(&app.state::<AppState>()).ok()
}

pub fn view(app: &AppHandle) -> CaptureView {
    let cfg = AppConfig::load().capture;
    let target = current_lib(app).and_then(|l| target_language(&l, &cfg));
    CaptureView {
        language_id: target.as_ref().map(|l| l.id.clone()),
        language_name: target.map(|l| l.name),
        shortcut: cfg.shortcut,
        shortcut_error: SHORTCUT_ERROR.lock().unwrap_or_else(|e| e.into_inner()).clone(),
    }
}

/// Starts a capture on a worker thread. The result arrives as the events
/// "capture-done" or "capture-error"; a cancelled capture sends nothing.
pub fn start(app: &AppHandle) {
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let result = run(&app);
        BUSY.store(false, Ordering::SeqCst);
        match result {
            Ok(Some(done)) => {
                let _ = app.emit("library-changed", done.language_id.clone());
                let _ = app.emit("capture-done", done);
            }
            Ok(None) => {}
            Err(e) => {
                let _ = app.emit("capture-error", e);
            }
        }
    });
}

fn run(app: &AppHandle) -> Result<Option<CaptureDone>, String> {
    let lib = current_lib(app).ok_or("Choose a library folder in Corydoras first.")?;
    let lang = target_language(&lib, &AppConfig::load().capture)
        .ok_or("Make a language in Corydoras first; captures go into a language.")?;
    let dir = lib.root().join(".tmp");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!("capture-{}.png", uuid::Uuid::new_v4()));
    let status = std::process::Command::new("/usr/sbin/screencapture")
        .arg("-i")
        .arg(&tmp)
        .status()
        .map_err(|e| format!("Cannot start the screen capture: {e}"))?;
    if !tmp.exists() {
        // Escape, or a click without a selection.
        return if status.success() { Ok(None) } else { Err("The screen capture failed.".into()) };
    }
    let bytes = std::fs::read(&tmp).map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&tmp);
    let item = lib
        .import_bytes(&lang.id, &util::capture_name(), &bytes?, Some("screen capture"))
        .map_err(|e| e.to_string())?;
    Ok(Some(CaptureDone { language_id: lang.id, language_name: lang.name, item_id: item.id }))
}

// ----------------------------------------------------------------------
// Shortcut
// ----------------------------------------------------------------------

/// Registers the saved shortcut. Errors (a bad string, or another app has the
/// shortcut) are kept for the Settings panel.
pub fn register_saved(app: &AppHandle) {
    let shortcut = AppConfig::load().capture.shortcut;
    let err = match shortcut {
        Some(s) => register(app, &s).err(),
        None => {
            let _ = app.global_shortcut().unregister_all();
            None
        }
    };
    *SHORTCUT_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = err;
}

fn register(app: &AppHandle, s: &str) -> Result<(), String> {
    let sc: Shortcut = s.parse().map_err(|e| format!("\"{s}\" is not a shortcut: {e}"))?;
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    gs.register(sc)
        .map_err(|e| format!("macOS refused the shortcut; another app may use it ({e})."))
}

// ----------------------------------------------------------------------
// Menu bar
// ----------------------------------------------------------------------

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let cfg = AppConfig::load().capture;
    let lib = current_lib(app);
    let langs = lib.as_ref().and_then(|l| l.list_languages().ok()).unwrap_or_default();
    let target = lib.as_ref().and_then(|l| target_language(l, &cfg));
    let label = match &target {
        Some(l) => format!("Capture to {}", l.name),
        None => "Capture (make a language first)".to_string(),
    };
    // The menu shows the shortcut; a string that the menu cannot show is left out.
    let capture = MenuItem::with_id(app, "capture", &label, target.is_some(), cfg.shortcut.as_deref())
        .or_else(|_| MenuItem::with_id(app, "capture", &label, target.is_some(), None::<&str>))?;
    let menu = Menu::with_items(app, &[&capture])?;
    if !langs.is_empty() {
        let sub = Submenu::with_id(app, "capture-to", "Capture Into", true)?;
        for l in &langs {
            let checked = target.as_ref().map(|t| t.id == l.id).unwrap_or(false);
            sub.append(&CheckMenuItem::with_id(app, format!("lang:{}", l.id), &l.name, true, checked, None::<&str>)?)?;
        }
        menu.append(&sub)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "open", "Open Corydoras", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "settings", "Capture Settings…", true, None::<&str>)?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "quit", "Quit Corydoras", true, None::<&str>)?)?;
    Ok(menu)
}

/// Rebuilds the menu, for example after a new language or a new capture language.
pub fn refresh_tray(app: &AppHandle) {
    if let (Some(tray), Ok(menu)) = (app.tray_by_id(TRAY_ID), build_menu(app)) {
        let _ = tray.set_menu(Some(menu));
    }
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
    #[cfg(target_os = "macos")]
    let _ = app.show();
}

fn set_language(app: &AppHandle, id: &str) {
    let mut cfg = AppConfig::load();
    cfg.capture.language_id = Some(id.to_string());
    let _ = cfg.save();
    refresh_tray(app);
    let _ = app.emit("capture-changed", ());
}

pub fn create_tray(app: &AppHandle) -> tauri::Result<()> {
    let icon = Image::from_bytes(include_bytes!("../icons/tray.png"))?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .icon_as_template(true)
        .tooltip("Corydoras")
        .menu(&build_menu(app)?)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            match id {
                "capture" => start(app),
                "open" => show_main(app),
                "settings" => {
                    show_main(app);
                    let _ = app.emit("open-settings", ());
                }
                "quit" => app.exit(0),
                _ => {
                    if let Some(lang) = id.strip_prefix("lang:") {
                        set_language(app, lang);
                    }
                }
            }
        })
        .build(app)?;
    Ok(())
}

// ----------------------------------------------------------------------
// Commands
// ----------------------------------------------------------------------

#[tauri::command]
pub fn get_capture(app: AppHandle) -> CaptureView {
    view(&app)
}

/// Saves the capture language and the shortcut (None clears the shortcut).
/// A shortcut that does not parse or that macOS refuses is not saved.
#[tauri::command]
pub fn set_capture(app: AppHandle, language_id: Option<String>, shortcut: Option<String>) -> CmdResult<CaptureView> {
    let shortcut = shortcut.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let mut cfg = AppConfig::load();
    if shortcut != cfg.capture.shortcut {
        match &shortcut {
            Some(s) => {
                if let Err(e) = register(&app, s) {
                    // Put the old shortcut back.
                    register_saved(&app);
                    return Err(e);
                }
            }
            None => {
                let _ = app.global_shortcut().unregister_all();
            }
        }
        *SHORTCUT_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
    cfg.capture.shortcut = shortcut;
    cfg.capture.language_id = language_id;
    cfg.save().map_err(|e| e.to_string())?;
    refresh_tray(&app);
    Ok(view(&app))
}

#[tauri::command]
pub fn capture_now(app: AppHandle) {
    start(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_parse() {
        for s in [corydoras_core::config::DEFAULT_CAPTURE_SHORTCUT, "Super+Shift+Digit2", "F5", "Control+Alt+Space"] {
            assert!(s.parse::<Shortcut>().is_ok(), "{s}");
        }
        assert!("Control+Nope".parse::<Shortcut>().is_err());
    }
}
