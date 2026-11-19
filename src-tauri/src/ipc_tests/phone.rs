//! The phone link reads and saves through the same guards as the Mac app, and pays for a photo once.
use super::{call, inventory_photo::serve_label, Library, MockRuntime, WebviewWindow, PNG};
use crate::phone::{AppBackend, Backend};
use base64::Engine;
use corydoras_core::{
    ai::{AiConfig, ProviderProfile},
    config::AppConfig,
};
use serde_json::json;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use tauri::{Listener, Manager};

pub(super) fn phone_link(app: &tauri::App<MockRuntime>, w: &WebviewWindow<MockRuntime>, library: &Library) {
    let view = call(w, "get_phone", json!({})).unwrap();
    assert_eq!((view["enabled"].as_bool(), view["running"].as_bool()), (Some(false), Some(false)));
    assert_eq!(
        call(w, "set_phone", json!({"enabled": false, "port": 80, "newTokenNow": false})).unwrap_err(),
        "Choose a port from 1024 to 65535."
    );

    // A stand-in provider that answers one request only.
    let listener = TcpListener::bind("127.0.0.1:0").expect("a local port for the test provider");
    let profile = ProviderProfile {
        base_url: format!("http://{}/v1", listener.local_addr().unwrap()),
        model: "test-label-vision".into(),
        vision: Some(true),
        ..ProviderProfile::from_preset("lmstudio")
    };
    let previous = AppConfig::load();
    let ai = AiConfig { analysis_profile: profile.id.clone(), synthesis_profile: profile.id.clone(), profiles: vec![profile] };
    AppConfig { ai: Some(ai), ..previous.clone() }.save().unwrap();
    let server = std::thread::spawn(move || serve_label(listener));

    let backend = AppBackend::new(app.handle().clone());
    let png = base64::engine::general_purpose::STANDARD.decode(PNG).unwrap();
    let before = library.usage().unwrap().requests;
    let first = tauri::async_runtime::block_on(backend.read(png.clone()));
    server.join().unwrap();
    // The stand-in is gone: a second provider call would fail.
    let second = tauri::async_runtime::block_on(backend.read(png.clone()));
    previous.save().unwrap();
    let (first, second) = (first.unwrap(), second.unwrap());
    assert_eq!(first, second);
    assert_eq!(first["drafts"].as_array().map(Vec::len), Some(1));
    assert_eq!(first["drafts"][0]["name"], "Hex key");
    assert_eq!(library.usage().unwrap().requests, before + 1);

    let status = backend.status().unwrap();
    assert_eq!(status["inventory_enabled"], true);
    assert_eq!(status["provider_ready"], true);

    let mut draft = first["drafts"][0].clone();
    draft["quantity"] = json!(3);
    let metadata = serde_json::to_vec(&json!({ "drafts": [draft] })).unwrap();
    let frame = [(metadata.len() as u32).to_be_bytes().as_slice(), &metadata, &png].concat();
    // The Mac hears about the save: the inventory list reloads and a toast names the count.
    let heard = Arc::new(Mutex::new(Vec::<String>::new()));
    for event in ["phone-added", "library-changed"] {
        let heard = heard.clone();
        app.listen_any(event, move |e| heard.lock().unwrap().push(format!("{event} {}", e.payload())));
    }
    let batch = Some("a1b2c3d4e5f60718293a4b5c6d7e8f90".to_string());
    let saved = tauri::async_runtime::block_on(backend.save(frame.clone(), batch.clone())).unwrap();
    let heard = heard.lock().unwrap().clone();
    assert!(heard.iter().any(|h| h == "library-changed \"\""), "{heard:?}");
    assert!(heard.iter().any(|h| h.starts_with("phone-added ") && h.contains("\"count\":1")), "{heard:?}");
    assert_eq!(saved["items"].as_array().map(Vec::len), Some(1));
    let stored = library.list_inventory().unwrap();
    assert!(stored.iter().any(|item| item.draft.name == "Hex key" && item.draft.quantity == Some(3)));
    // The phone lost the answer and saves again: the same batch saves nothing new.
    let again = tauri::async_runtime::block_on(backend.save(frame.clone(), batch)).unwrap();
    assert_eq!(again, saved);
    assert_eq!(library.list_inventory().unwrap().len(), stored.len());
    assert!(backend.status().unwrap()["categories"].as_array().unwrap().contains(&json!("Tools")));

    // During a library move the phone gets the same answer as the Mac, and nothing is read or saved.
    let state = app.state::<crate::AppState>();
    state.moving.store(true, std::sync::atomic::Ordering::SeqCst);
    let moving = (400, "Wait for the library move to finish.".to_string());
    let count = library.list_inventory().unwrap().len();
    assert_eq!(tauri::async_runtime::block_on(backend.read(png.clone())).unwrap_err(), moving);
    assert_eq!(tauri::async_runtime::block_on(backend.save(frame.clone(), None)).unwrap_err(), moving);
    state.moving.store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(library.list_inventory().unwrap().len(), count);

    // With the module off, the phone gets the same answer as the Mac.
    library.set_inventory_enabled(false).unwrap();
    let off = "Workshop inventory is off for this library. Open Settings to turn it on.";
    assert_eq!(tauri::async_runtime::block_on(backend.save(frame, None)).unwrap_err(), (400, off.to_string()));
    let status = backend.status().unwrap();
    assert_eq!((status["inventory_enabled"].clone(), status["categories"].clone()), (json!(false), json!([])));
    library.set_inventory_enabled(true).unwrap();
}

/// The extension bridge moved onto the shared request reader: it still answers as before.
pub(super) fn bridge_still_answers(app: &tauri::App<MockRuntime>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let exchange = |request: &'static [u8]| {
        let handle = app.handle().clone();
        tauri::async_runtime::block_on(async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                crate::bridge::serve(stream, handle, Arc::new("tok".to_string())).await;
            });
            let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
            client.write_all(request).await.unwrap();
            let mut answer = Vec::new();
            tokio::time::timeout(std::time::Duration::from_secs(3), client.read_to_end(&mut answer)).await.unwrap().unwrap();
            server.await.unwrap();
            String::from_utf8_lossy(&answer).into_owned()
        })
    };
    let preflight = exchange(b"OPTIONS /v1/items HTTP/1.1\r\nOrigin: chrome-extension://abc\r\n\r\n");
    assert!(preflight.starts_with("HTTP/1.1 204"), "{preflight}");
    assert!(preflight.contains("Access-Control-Allow-Origin: chrome-extension://abc"), "{preflight}");
    let refused = exchange(b"GET /v1/languages HTTP/1.1\r\nAuthorization: Bearer nope\r\n\r\n");
    assert!(refused.starts_with("HTTP/1.1 401"), "{refused}");
    let page_origin = exchange(b"OPTIONS /v1/items HTTP/1.1\r\nOrigin: http://evil.example\r\n\r\n");
    assert!(!page_origin.contains("Access-Control-"), "{page_origin}");
    let answered = exchange(b"GET /v1/languages HTTP/1.1\r\nAuthorization: Bearer tok\r\n\r\n");
    assert!(answered.starts_with("HTTP/1.1 200"), "{answered}");
    assert!(answered.contains("\"app\":\"Corydoras\""), "{answered}");
}
