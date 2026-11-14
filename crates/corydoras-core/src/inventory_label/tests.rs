use super::*;
use crate::ai::ProviderProfile;
use crate::testserver::{serve, Resp};
use image::{DynamicImage, ImageFormat};
use serde_json::{json, Value};

fn label() -> Value {
    json!({
        "label_text": "Acme SH-12 Socket screws M3 x 12 mm 50 pcs Stainless steel",
        "name": "Socket screws", "category": "fasteners", "brand": "Acme",
        "part_number": "SH-12", "dimensions": ["M3 x 12 mm"],
        "pack_quantity": 50, "pack_evidence": "50 pcs", "notes": ["Stainless steel"]
    })
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::new_rgb8(width, height)
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

#[test]
fn verified_fields_preserve_units_and_pack_count() {
    let draft = parse_label(&label().to_string()).unwrap();
    assert_eq!(
        (draft.category.as_str(), draft.part_number.as_str()),
        ("Fasteners", "SH-12")
    );
    assert_eq!(
        (draft.brand.as_str(), draft.dimensions.as_str()),
        ("Acme", "M3 x 12 mm")
    );
    assert_eq!(draft.notes, "Stainless steel");
    assert_eq!((draft.pack_quantity, draft.quantity), (Some(50), None));
    assert!(draft.supplier.is_none());
}

#[test]
fn missing_print_and_unrelated_values_remain_unknown() {
    let answer = json!({ "label_text":"Socket screws 150 pcs", "name":"Socket screws",
        "brand":"Acme", "part_number":"SH-12", "category":"a new category",
        "dimensions":["4 mm"], "pack_evidence":"150 pcs", "pack_quantity":50 });
    let draft = parse_label(&answer.to_string()).unwrap();
    assert_eq!(draft.category, "Other");
    assert!(draft.brand.is_empty() && draft.part_number.is_empty() && draft.dimensions.is_empty());
    assert_eq!((draft.pack_quantity, draft.quantity), (None, None));
}

#[test]
fn unreadable_invalid_and_unexpected_model_output_fail() {
    for answer in [
        "{}",
        r#"{"label_text":"unrelated text","category":"Materials"}"#,
        "Here is your JSON: {}",
        r#"{"name":"Bolts","quantity":90}"#,
        r#"{"label_text":"bolts","name":"Bolts","dimensions":"6 mm"}"#,
    ] {
        assert!(parse_label(answer).is_err(), "{answer}");
    }
}

#[test]
fn photo_conversion_checks_format_and_decoded_size() {
    assert!(prepare_image(b"unreadable").is_err());
    assert!(prepare_image(&png(10_001, 1)).is_err());
    let image = prepare_image(&png(80, 60)).unwrap();
    assert!(image.starts_with("data:image/jpeg;base64,"));
    let decoded = analysis::decode_oriented(&analysis::base64_decode(&image).unwrap()).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (80, 60));
}

#[tokio::test]
async fn provider_receives_an_image_and_records_usage_even_for_an_invalid_reply() {
    let server = serve(|_| {
        Resp::json(
            200,
            json!({"choices":[{"message":{"content":"{}"},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":36,"completion_tokens":7}}),
        )
    })
    .await;
    let provider = Provider::new(&server.base, "label-test", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path()).unwrap();
    assert!(
        request_label(&lib, &provider, prepare_image(&png(32, 32)).unwrap())
            .await
            .is_err()
    );
    let req = server.seen_at("/chat/completions");
    assert_eq!(req.len(), 1);
    assert!(
        req[0].json()["messages"][1]["content"][1]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/jpeg;base64,")
    );
    let usage = lib.usage().unwrap();
    assert_eq!(
        (usage.input_tokens, usage.output_tokens, usage.requests),
        (36, 7, 1)
    );
}

#[tokio::test]
async fn budget_and_incompatible_models_fail_before_any_request() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path()).unwrap();
    let profile = ProviderProfile {
        vision: Some(false),
        ..ProviderProfile::from_preset("custom")
    };
    let cfg = AiConfig {
        analysis_profile: profile.id.clone(),
        synthesis_profile: profile.id.clone(),
        profiles: vec![profile],
    };
    let photo = png(32, 32);
    assert!(read_label(&lib, &cfg, &photo)
        .await
        .unwrap_err()
        .to_string()
        .contains("images"));
    lib.save_settings(&crate::models::Settings {
        monthly_token_budget: 1,
        ..lib.settings().unwrap()
    })
    .unwrap();
    lib.add_usage(1, 0).unwrap();
    assert!(read_label(&lib, &cfg, &photo)
        .await
        .unwrap_err()
        .to_string()
        .contains("budget"));
}

#[test]
fn a_partial_model_number_size_or_ambiguous_count_is_not_evidence() {
    let mut response = label();
    response["label_text"] = json!("Acme SH-123. Screws 124 mm. Box of 1,500 pieces.");
    response["part_number"] = json!("SH-12");
    response["dimensions"] = json!(["24 mm"]);
    response["pack_evidence"] = json!("124 mm");
    response["pack_quantity"] = json!(124);
    let draft = parse_label(&response.to_string()).unwrap();
    assert!(draft.part_number.is_empty() && draft.dimensions.is_empty());
    assert!(draft.pack_quantity.is_none());
    response["pack_evidence"] = json!("1,500 pieces");
    response["pack_quantity"] = json!(1500);
    assert_eq!(
        parse_label(&response.to_string()).unwrap().pack_quantity,
        Some(1500)
    );
}

#[test]
fn pack_evidence_requires_a_whole_number_next_to_a_quantity_marker() {
    for phrase in [
        "25 pcs",
        "qty: 25",
        "pack of 25",
        "25-pack",
        "No. 12, 25pc",
        "25 count",
        "Box of 25",
        "25 ct.",
        "QTY / PK: 25",
    ] {
        assert!(printed_count(phrase, 25), "{phrase}");
    }
    for phrase in [
        "M4 × 25 mm",
        "AB-25",
        "124 mm. Pack of 50",
        "120 pcs",
        "1.25 mm, Qty. 6",
        "50 pcs, 25 mm",
        "25 x 50 pc",
    ] {
        assert!(!printed_count(phrase, 25), "{phrase}");
    }
    let mut response = label();
    response["label_text"] = json!("Socket screws 150 pcs");
    response["pack_evidence"] = json!("50 pcs");
    response["pack_quantity"] = json!(50);
    assert_eq!(
        parse_label(&response.to_string()).unwrap().pack_quantity,
        None
    );
}

#[test]
fn a_partly_legible_label_keeps_its_fields_for_review() {
    let response = json!({"label_text": "Acme 6 mm", "brand": "Acme", "dimensions": ["6 mm"]});
    let draft = parse_label(&response.to_string()).unwrap();
    assert!(draft.name.is_empty());
    assert_eq!(
        (draft.brand.as_str(), draft.dimensions.as_str()),
        ("Acme", "6 mm")
    );
    assert!(draft.normalized().is_err());
}
