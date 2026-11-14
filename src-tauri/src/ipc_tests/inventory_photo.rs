//! One photo travels from Tauri through the configured provider into a reviewed SQLite record.
use super::{send, InvokeBody, Library, MockRuntime, WebviewWindow, PNG};
use base64::Engine;
use corydoras_core::{
    ai::{AiConfig, ProviderProfile},
    config::AppConfig,
};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::time::Duration;

fn serve_label(listener: TcpListener) {
    let (mut stream, _) = listener.accept().unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut reader = BufReader::new(&stream);
    let mut lines = reader.by_ref().lines().map(Result::unwrap);
    assert_eq!(lines.next().unwrap(), "POST /v1/chat/completions HTTP/1.1");
    let headers: Vec<_> = lines.take_while(|s| !s.is_empty()).collect();
    let length: usize = headers
        .iter()
        .filter_map(|h| h.split_once(':'))
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .unwrap()
        .1
        .trim()
        .parse()
        .unwrap();
    assert!(length < 64 * 1024);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let request: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(request["model"], "test-label-vision");
    let image = request["messages"][1]["content"][1]["image_url"]["url"]
        .as_str()
        .unwrap();
    assert!(image.starts_with("data:image/jpeg;base64,"));
    let label = json!({
        "label_text":"Fixture Tools F-08 Hex key 8 mm 2 pcs",
        "name":"Hex key", "category":"Tools", "brand":"Fixture Tools", "part_number":"F-08",
        "dimensions":["8 mm"], "pack_quantity":2, "pack_evidence":"2 pcs", "notes":[]
    });
    let reply = json!({
        "choices":[{"message":{"content":label.to_string()},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":41,"completion_tokens":17}
    })
    .to_string();
    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", reply.len(), reply).unwrap();
}

pub(super) fn reviewed_record(w: &WebviewWindow<MockRuntime>, library: &Library) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a local port for the test provider");
    let profile = ProviderProfile {
        base_url: format!("http://{}/v1", listener.local_addr().unwrap()),
        model: "test-label-vision".into(),
        vision: Some(true),
        ..ProviderProfile::from_preset("lmstudio")
    };
    assert!(!profile.needs_key);
    let previous = AppConfig::load();
    let cfg = AiConfig {
        analysis_profile: profile.id.clone(),
        synthesis_profile: profile.id.clone(),
        profiles: vec![profile],
    };
    AppConfig {
        ai: Some(cfg),
        ..previous.clone()
    }
    .save()
    .unwrap();
    let before = library.usage().unwrap();
    let server = std::thread::spawn(move || serve_label(listener));
    let png = base64::engine::general_purpose::STANDARD
        .decode(PNG)
        .unwrap();
    let result = send(
        w,
        "read_inventory_label",
        InvokeBody::Raw(png),
        Default::default(),
    );
    previous.save().unwrap();
    let draft = result.unwrap();
    server.join().unwrap();

    let cmd = |name, args| send(w, name, InvokeBody::Json(args), Default::default()).unwrap();
    assert_eq!(cmd("list_inventory", json!({})), json!([]));
    assert_eq!(
        draft,
        json!({"name":"Hex key","category":"Tools","brand":"Fixture Tools","part_number":"F-08",
        "dimensions":"8 mm","quantity":null,"pack_quantity":2,"notes":"","supplier":null})
    );
    let after = library.usage().unwrap();
    assert_eq!(
        (
            after.input_tokens - before.input_tokens,
            after.output_tokens - before.output_tokens,
            after.requests - before.requests
        ),
        (41, 17, 1)
    );

    let mut reviewed = draft;
    reviewed["quantity"] = json!(1);
    let saved = cmd("save_inventory", json!({"id":null,"draft":reviewed}));
    let reopened = Library::open(library.root()).unwrap();
    let stored = serde_json::to_value(reopened.list_inventory().unwrap()).unwrap();
    assert_eq!(stored, json!([saved]));
    assert_eq!(stored[0]["quantity"], 1);
    assert_eq!(stored[0]["pack_quantity"], 2);
}
