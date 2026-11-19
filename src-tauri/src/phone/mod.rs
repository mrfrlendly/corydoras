//! The phone link: a page and three routes on the Wi-Fi, so a phone can photograph labels and save
//! inventory items through this app.
//!
//! - It listens on every network of this Mac (0.0.0.0), on its own port, with its own token.
//! - The page shell and its scripts are public; reading, spending provider tokens and saving need the token.
//! - It is off until you turn it on in Settings.

mod reads;
mod server;

pub(crate) use server::Backend;

use crate::{inventory, AppState, CmdResult};
use corydoras_core::{
    ai::{AiConfig, Role},
    config::AppConfig,
};
use reads::{ReadCache, READ_CAPACITY, READ_TTL};
use serde::Serialize;
use serde_json::json;
use server::{BoxFut, Reply};
use std::collections::{BTreeSet, HashMap};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// A connection may wait this long for a label read, which can take over a minute.
const CONNECTION_LIMIT: Duration = Duration::from_secs(330);

#[derive(Default)]
pub struct PhoneState {
    /// The port that the listener runs on, and the way to stop it.
    running: Mutex<Option<(u16, tokio::sync::watch::Sender<bool>)>>,
    /// Why the listener could not start.
    error: Mutex<Option<String>>,
}

#[derive(Serialize, Clone)]
pub struct PhoneView {
    pub enabled: bool,
    pub port: u16,
    pub running: bool,
    pub error: Option<String>,
    /// The page address with the token in its fragment, for the QR code.
    pub url: Option<String>,
    pub local_url: Option<String>,
    pub qr_svg: Option<String>,
}

fn phone_url(ip: Ipv4Addr, port: u16, token: &str) -> String {
    format!("http://{ip}:{port}/#t={token}")
}

/// The same page by this Mac's .local name, which survives a new Wi-Fi address. Not every phone resolves it.
fn local_url(host: &str, port: u16, token: &str) -> String {
    format!("http://{host}.local:{port}/#t={token}")
}

fn check_port(port: u16, bridge_port: u16) -> Result<(), String> {
    if port < 1024 {
        return Err("Choose a port from 1024 to 65535.".into());
    }
    if port == bridge_port {
        return Err(format!("The browser extension uses port {bridge_port}. Choose another port."));
    }
    Ok(())
}

/// The address of this Mac on the network that holds the default route. A UDP connect sends no packet.
fn lan_ipv4() -> Option<Ipv4Addr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("1.1.1.1:9").ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

fn local_host_name() -> Option<String> {
    let out = std::process::Command::new("scutil").args(["--get", "LocalHostName"]).output().ok()?;
    let name = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !name.is_empty()).then_some(name)
}

fn qr_svg(text: &str) -> Option<String> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    Some(code.render::<qrcode::render::svg::Color>().min_dimensions(240, 240).quiet_zone(true).build())
}

fn view<R: Runtime>(app: &AppHandle<R>) -> PhoneView {
    let cfg = AppConfig::load().phone;
    let st = app.state::<AppState>();
    let running = st.phone.running.lock().unwrap_or_else(|e| e.into_inner()).is_some();
    let error = st.phone.error.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let url = running.then(lan_ipv4).flatten().map(|ip| phone_url(ip, cfg.port, &cfg.token));
    let local_url = running.then(local_host_name).flatten().map(|host| local_url(&host, cfg.port, &cfg.token));
    let qr_svg = url.as_deref().and_then(qr_svg);
    PhoneView { enabled: cfg.enabled, port: cfg.port, running, error, url, local_url, qr_svg }
}

fn stop<R: Runtime>(app: &AppHandle<R>) {
    let running = app.state::<AppState>().phone.running.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some((_, tx)) = running {
        let _ = tx.send(true);
    }
}

/// Starts the listener when the settings turn it on. Call it at launch and after each change.
pub fn start<R: Runtime>(app: &AppHandle<R>) {
    stop(app);
    let st = app.state::<AppState>();
    *st.phone.error.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let cfg = AppConfig::load().phone;
    if !cfg.enabled || cfg.token.is_empty() {
        return;
    }
    let listener = std::net::TcpListener::bind(("0.0.0.0", cfg.port)).and_then(|l| l.set_nonblocking(true).map(|_| l));
    let listener = match listener {
        Ok(l) => l,
        Err(e) => {
            *st.phone.error.lock().unwrap_or_else(|x| x.into_inner()) =
                Some(format!("Port {} is not free ({e}). Choose another port.", cfg.port));
            return;
        }
    };
    let (tx, rx) = tokio::sync::watch::channel(false);
    *st.phone.running.lock().unwrap_or_else(|e| e.into_inner()) = Some((cfg.port, tx));
    let (backend, token) = (AppBackend::new(app.clone()), Arc::new(cfg.token));
    tauri::async_runtime::spawn(async move {
        let Ok(listener) = tokio::net::TcpListener::from_std(listener) else { return };
        server::run(listener, backend, token, rx, server::MAX_CONNECTIONS, CONNECTION_LIMIT).await;
    });
}

#[tauri::command]
pub fn get_phone<R: Runtime>(app: AppHandle<R>) -> PhoneView {
    view(&app)
}

/// Turns the phone link on or off, changes its port, or makes a new token.
#[tauri::command]
pub fn set_phone<R: Runtime>(app: AppHandle<R>, enabled: bool, port: u16, new_token_now: bool) -> CmdResult<PhoneView> {
    let mut cfg = AppConfig::load();
    check_port(port, cfg.bridge.port)?;
    cfg.phone.enabled = enabled;
    cfg.phone.port = port;
    if cfg.phone.token.is_empty() || new_token_now {
        cfg.phone.token = crate::bridge::new_token();
    }
    cfg.save().map_err(|e| e.to_string())?;
    start(&app);
    Ok(view(&app))
}

/// The routes on the open library, through the same guards as the Mac app's commands.
pub(crate) struct AppBackend<R: Runtime> {
    app: AppHandle<R>,
    reads: Arc<ReadCache>,
    /// The answers of recent saves by batch id, so a save repeated after a lost answer adds nothing.
    saved: Arc<Mutex<HashMap<String, (Instant, serde_json::Value)>>>,
}

/// How long a save's answer is kept for a repeat. The phone keeps its cards until they are saved, so a
/// retry can come after a long lock; a day covers that, and old answers are dropped at each save.
const SAVED_TTL: Duration = Duration::from_secs(24 * 60 * 60);

impl<R: Runtime> AppBackend<R> {
    pub(crate) fn new(app: AppHandle<R>) -> Arc<Self> {
        Arc::new(Self { app, reads: ReadCache::new(READ_TTL, READ_CAPACITY), saved: Default::default() })
    }
}

fn bad(e: String) -> (u16, String) {
    (400, e)
}

fn library_name(library: &corydoras_core::Library) -> String {
    library.root().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

impl<R: Runtime> Backend for AppBackend<R> {
    fn status(&self) -> Reply {
        let library = crate::lib(&self.app.state::<AppState>()).map_err(|_| bad("Open a library in Corydoras first.".into()))?;
        let enabled = library.inventory_enabled().map_err(|e| bad(e.to_string()))?;
        let settings = library.settings().map_err(|e| bad(e.to_string()))?;
        let provider_ready = AiConfig::effective(AppConfig::load().ai.as_ref(), &settings).for_role(Role::Analysis).is_some();
        let categories: BTreeSet<String> = if enabled {
            library.list_inventory().map_err(|e| bad(e.to_string()))?.into_iter().map(|item| item.draft.category).collect()
        } else {
            BTreeSet::new()
        };
        Ok(json!({
            "app": "Corydoras",
            "version": env!("CARGO_PKG_VERSION"),
            "library": library_name(&library),
            "inventory_enabled": enabled,
            "provider_ready": provider_ready,
            "categories": categories,
        }))
    }

    fn read(&self, photo: Vec<u8>) -> BoxFut<Reply> {
        let (app, reads) = (self.app.clone(), self.reads.clone());
        Box::pin(async move {
            let library = inventory::enabled_library(&app.state::<AppState>()).map_err(bad)?;
            let key = photo.clone();
            let drafts = reads.get_or_read(&key, move || inventory::read_label(library, photo)).await.map_err(bad)?;
            Ok(json!({ "drafts": drafts }))
        })
    }

    fn save(&self, frame: Vec<u8>, batch: Option<String>) -> BoxFut<Reply> {
        let (app, saved) = (self.app.clone(), self.saved.clone());
        Box::pin(async move {
            if let Some(id) = &batch {
                let mut kept = saved.lock().unwrap_or_else(|e| e.into_inner());
                kept.retain(|_, (at, _)| at.elapsed() < SAVED_TTL);
                if let Some((_, reply)) = kept.get(id) {
                    return Ok(reply.clone());
                }
            }
            let library = inventory::enabled_library(&app.state::<AppState>()).map_err(bad)?;
            let name = library_name(&library);
            let items = inventory::save_batch_frame(library, frame).await.map_err(bad)?;
            let _ = app.emit("library-changed", "");
            let _ = app.emit("phone-added", json!({ "count": items.len(), "library": name }));
            let reply = json!({ "items": items });
            if let Some(id) = batch {
                saved.lock().unwrap_or_else(|e| e.into_inner()).insert(id, (Instant::now(), reply.clone()));
            }
            Ok(reply)
        })
    }

    fn asset(&self, path: &str) -> Option<(Vec<u8>, String)> {
        self.app.asset_resolver().get(path.to_string()).map(|a| (a.bytes, a.mime_type))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn phone_url_keeps_the_token_in_the_fragment() {
        assert_eq!(phone_url(Ipv4Addr::new(192, 168, 1, 20), 47822, "tok"), "http://192.168.1.20:47822/#t=tok");
    }

    #[test]
    fn local_url_adds_dot_local_and_the_token() {
        assert_eq!(local_url("Studio-Mac", 47822, "tok"), "http://Studio-Mac.local:47822/#t=tok");
    }

    #[test]
    fn ports_below_1024_and_the_bridge_port_are_refused() {
        assert_eq!(check_port(80, 47821).unwrap_err(), "Choose a port from 1024 to 65535.");
        assert_eq!(check_port(47821, 47821).unwrap_err(), "The browser extension uses port 47821. Choose another port.");
        assert!(check_port(47822, 47821).is_ok());
    }

    #[test]
    fn the_qr_code_is_an_svg() {
        let svg = qr_svg("http://192.168.1.20:47822/#t=tok").expect("an svg");
        assert!(svg.starts_with("<?xml") || svg.starts_with("<svg"), "{}", &svg[..40]);
        assert!(svg.contains("</svg>"));
    }
}
