//! A language document reaches agents through the MCP server.

mod common;

use common::Client;
use corydoras_core::Library;
use serde_json::json;

#[test]
fn agents_get_the_midjourney_prompt() {
    let root = tempfile::tempdir().unwrap();
    let lib = Library::open(root.path()).unwrap();
    let language = lib.create_language("Rounded", "").unwrap();
    let spec = json!({
        "families": [{"id": "shell", "name": "Shell", "applies_to": "enclosures"}],
        "prompts": {"positive": "", "negative": "", "midjourney": {
            "language": "product concept render, one matte shell",
            "no": "chrome",
            "families": {"shell": "product concept render, soft laminated shell"}
        }}
    });
    lib.save_spec_version(&language.id, &spec, "test", &[], true, "", &[]).unwrap();
    drop(lib);

    let (mut agent, instructions) = Client::connect(root.path());
    assert!(
        instructions.contains("For concept images in Midjourney, use the \"Midjourney prompt\" section of the language document: replace [your object] with the object and keep the rest of the line."),
        "{instructions}"
    );
    let listing = agent.request("tools/list", json!({}));
    let get_language = listing["tools"].as_array().unwrap().iter().find(|t| t["name"] == "get_language").unwrap();
    assert!(get_language["description"].as_str().unwrap().contains("\"Midjourney prompt\" section"));

    let whole = agent.call("get_language", json!({"language": "Rounded"}));
    let text = whole[0]["text"].as_str().unwrap();
    assert!(text.contains("## Midjourney prompt"), "{text}");
    assert!(text.contains("[your object], product concept render, one matte shell --no chrome --raw"));

    let family = agent.call("get_language", json!({"language": "Rounded", "family": "shell"}));
    let text = family[0]["text"].as_str().unwrap();
    assert!(text.contains("[your object], product concept render, soft laminated shell --no chrome --raw"), "{text}");
    assert!(!text.contains("one matte shell"));
}
