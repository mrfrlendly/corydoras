use super::*;
use crate::ai::ProviderProfile;
use crate::testserver::{serve, Resp};
use image::{DynamicImage, ImageFormat};
use serde_json::{json, Value};

const MAX_BYTES: usize = 128 * 1024;

fn row(text: &str, name: &str, product: &str, specs: Value) -> Value {
    json!({"row_text": text, "name": name, "category": "fasteners", "product_type": product, "specs": specs})
}

fn spec(role: &str, value: &str, unit: &str, basis: &str, evidence: &str) -> Value {
    json!({"role": role, "value": value, "unit": unit, "basis": basis, "evidence": evidence})
}

fn sheet(shared: &str, items: Vec<Value>) -> String {
    json!({"shared_text": shared, "items": items}).to_string()
}

fn two_rows() -> Vec<Value> {
    vec![
        row(
            "M3 x 8 mm 50 pcs",
            "M3 screw",
            "screw",
            json!([
                spec("thread", "M3", "", "stated", "M3"),
                spec("length", "8", "mm", "stated", "8 mm")
            ]),
        ),
        row(
            "M4 x 10 mm 40 pcs",
            "M4 screw",
            "screw",
            json!([
                spec("thread", "M4", "", "stated", "M4"),
                spec("length", "10", "mm", "stated", "10 mm")
            ]),
        ),
    ]
}

fn specs_of(draft: &InventoryDraft) -> Vec<(String, String, String)> {
    draft
        .specs
        .iter()
        .map(|s| {
            (
                serde_json::to_value(s.role)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned(),
                s.value.clone(),
                serde_json::to_value(s.source)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned(),
            )
        })
        .collect()
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::new_rgb8(width, height)
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

#[test]
fn each_row_is_checked_against_its_own_text() {
    let mut rows = two_rows();
    rows[0]["specs"]
        .as_array_mut()
        .unwrap()
        .push(spec("length", "10", "mm", "stated", "10 mm"));
    let drafts = parse_labels(&sheet("", rows)).unwrap();
    assert_eq!(drafts.len(), 2);
    let m3 = specs_of(&drafts[0]);
    assert!(m3.contains(&("length".into(), "8".into(), "label".into())));
    assert!(!m3.iter().any(|s| s.0 == "length" && s.1 == "10"));
    assert!(specs_of(&drafts[1]).contains(&("length".into(), "10".into(), "label".into())));
}

#[test]
fn shared_text_supports_only_brand_material_colour_and_finish() {
    let item = json!({"row_text": "Hex screws", "name": "Hex screws", "product_type": "screw",
        "brand": "Acme", "colour": "Black", "pack_quantity": 50, "pack_evidence": "50 pcs",
        "specs": [spec("length", "1.75", "mm", "stated", "1.75 mm"),
            spec("thread", "M3", "", "stated", "M3"),
            spec("material", "PETG", "", "stated", "PETG")]});
    let shared = "Acme Black PETG 1.75 mm M3 50 pcs";
    let draft = &parse_labels(&sheet(shared, vec![item])).unwrap()[0];
    assert_eq!(
        (draft.brand.as_str(), draft.colour.as_str()),
        ("Acme", "Black")
    );
    assert_eq!(draft.pack_quantity, None);
    assert_eq!(
        specs_of(draft),
        vec![("material".to_owned(), "PETG".to_owned(), "label".to_owned())]
    );
}

#[test]
fn value_must_appear_inside_its_evidence() {
    let items = vec![row(
        "M3 x 12 mm. Length 5,7 mm",
        "Insert",
        "insert",
        json!([
            spec("length", "8", "mm", "stated", "M3 x 12"),
            spec("length", "5.7", "mm", "stated", "5,7 mm"),
            spec("length", "5", "mm", "stated", "5,7 mm")
        ]),
    )];
    let draft = &parse_labels(&sheet("", items)).unwrap()[0];
    assert_eq!(
        specs_of(draft),
        vec![("length".to_owned(), "5.7".to_owned(), "label".to_owned())]
    );
}

#[test]
fn notation_specs_are_marked_notation() {
    let items = vec![row(
        "M2x2x3.2",
        "Knurled nut",
        "nut",
        json!([
            spec("thread", "M2", "", "notation", "M2x2x3.2"),
            spec("length", "2", "mm", "notation", "M2x2x3.2"),
            spec("outer_diameter", "3.2", "mm", "notation", "M2x2x3.2"),
            spec("tolerance_guess", "9", "mm", "notation", "M2x2x3.2"),
            spec("length", "2", "mm", "guess", "M2x2x3.2")
        ]),
    )];
    let draft = &parse_labels(&sheet("", items)).unwrap()[0];
    let got = specs_of(draft);
    for role in ["thread", "length", "outer_diameter"] {
        assert!(
            got.iter().any(|s| s.0 == role && s.2 == "notation"),
            "{role}"
        );
    }
    assert!(got.iter().any(|s| s.0 == "pitch" && s.2 == "standard"));
    assert_eq!(got.iter().filter(|s| s.2 == "label").count(), 0);
    assert_eq!(
        draft.dimensions,
        "M2 thread; length 2 mm; outer diameter 3.2 mm; pitch 0.4 mm (standard)"
    );
}

#[test]
fn a_screw_row_gets_standard_rows_and_an_insert_row_gets_pitch_only() {
    let items = vec![
        row(
            "M3 x 8",
            "Screw",
            "screw",
            json!([spec("thread", "M3", "", "stated", "M3")]),
        ),
        row(
            "M3 x 5",
            "Insert",
            "insert",
            json!([spec("thread", "M3", "", "stated", "M3")]),
        ),
    ];
    let drafts = parse_labels(&sheet("", items)).unwrap();
    let standard = |d: &InventoryDraft| {
        specs_of(d)
            .into_iter()
            .filter(|s| s.2 == "standard")
            .map(|s| s.0)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        standard(&drafts[0]),
        ["pitch", "clearance_hole", "tap_drill"]
    );
    assert_eq!(standard(&drafts[1]), ["pitch"]);
}

#[test]
fn identical_rows_stay_separate_drafts() {
    let spool = || {
        row(
            "PETG 1.75 mm 1 kg Translucent Orange",
            "PETG filament",
            "filament",
            json!([
                spec("material", "PETG", "", "stated", "PETG"),
                spec("diameter", "1.75", "mm", "stated", "1.75 mm"),
                spec("net_weight", "1", "kg", "stated", "1 kg")
            ]),
        )
    };
    assert_eq!(
        parse_labels(&sheet("", vec![spool(), spool()]))
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn a_filament_spool_keeps_printed_colour_only() {
    let mut spool = row(
        "PETG 1.75 mm 1 kg Translucent Orange",
        "PETG filament",
        "filament",
        json!([
            spec("material", "PETG", "", "stated", "PETG"),
            spec("net_weight", "1", "kg", "stated", "1 kg")
        ]),
    );
    spool["colour"] = json!("Translucent Orange");
    let draft = &parse_labels(&sheet("", vec![spool.clone()])).unwrap()[0];
    assert_eq!(draft.colour, "Translucent Orange");
    assert_eq!(draft.dimensions, "material PETG; net weight 1 kg");
    spool["colour"] = json!("Clear");
    assert!(parse_labels(&sheet("", vec![spool])).unwrap()[0]
        .colour
        .is_empty());
}

#[test]
fn quantity_is_never_set_and_pack_counts_need_row_evidence() {
    let mut rows = two_rows();
    rows[0]["pack_quantity"] = json!(50);
    rows[0]["pack_evidence"] = json!("50 pcs");
    rows[1]["pack_quantity"] = json!(50);
    rows[1]["pack_evidence"] = json!("50 pcs");
    let drafts = parse_labels(&sheet("", rows)).unwrap();
    assert_eq!(
        (drafts[0].pack_quantity, drafts[1].pack_quantity),
        (Some(50), None)
    );
    assert!(drafts.iter().all(|d| d.quantity.is_none()));
}

#[test]
fn item_count_and_reply_size_are_limited() {
    let many = |n: usize| {
        sheet(
            "",
            (0..n)
                .map(|i| row(&format!("M3 x {i} mm"), "Screw", "screw", json!([])))
                .collect(),
        )
    };
    assert_eq!(parse_labels(&many(MAX_ITEMS)).unwrap().len(), MAX_ITEMS);
    assert!(parse_labels(&many(MAX_ITEMS + 1))
        .unwrap_err()
        .to_string()
        .contains("40"));
    let mut huge = row("M3 x 8", "Screw", "screw", json!([]));
    huge["notes"] = json!(["x".repeat(MAX_BYTES)]);
    assert!(parse_labels(&sheet("", vec![huge])).is_err());
    for answer in [
        sheet("", vec![]),
        sheet("", vec![row("", "Ghost", "screw", json!([]))]),
        sheet("", vec![json!({"row_text": "unrelated text"})]),
        "{}".to_owned(),
        r#"{"items":[{"row_text":"bolts","quantity":90}]}"#.to_owned(),
        r#"{"items":[{"row_text":"bolts","specs":"6 mm"}]}"#.to_owned(),
        "Here is your JSON: {}".to_owned(),
    ] {
        assert!(parse_labels(&answer).is_err(), "{answer}");
    }
}

#[test]
fn a_partial_model_number_size_or_ambiguous_count_is_not_evidence() {
    let mut item = row(
        "Acme SH-123. Screws 124 mm. Box of 1,500 pieces.",
        "Screws",
        "screw",
        json!([spec("length", "24", "mm", "stated", "24 mm")]),
    );
    item["part_number"] = json!("SH-12");
    item["pack_evidence"] = json!("124 mm");
    item["pack_quantity"] = json!(124);
    let draft = &parse_labels(&sheet("", vec![item.clone()])).unwrap()[0];
    assert!(draft.part_number.is_empty() && draft.specs.is_empty());
    assert!(draft.pack_quantity.is_none());
    item["pack_evidence"] = json!("1,500 pieces");
    item["pack_quantity"] = json!(1500);
    assert_eq!(
        parse_labels(&sheet("", vec![item])).unwrap()[0].pack_quantity,
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
}

#[test]
fn a_partly_legible_row_keeps_its_fields_for_review() {
    let mut item = json!({"row_text": "Acme 6 mm", "brand": "Acme"});
    item["specs"] = json!([spec("length", "6", "mm", "stated", "6 mm")]);
    let draft = &parse_labels(&sheet("", vec![item])).unwrap()[0];
    assert!(draft.name.is_empty());
    assert_eq!(
        (draft.brand.as_str(), draft.dimensions.as_str()),
        ("Acme", "length 6 mm")
    );
    assert!(draft.normalized().is_err());
}

#[test]
fn photo_conversion_checks_format_decoded_size_and_long_side() {
    assert!(prepare_image(b"unreadable").is_err());
    assert!(prepare_image(&png(10_001, 1)).is_err());
    let image = prepare_image(&png(80, 60)).unwrap();
    assert!(image.starts_with("data:image/jpeg;base64,"));
    let decoded = analysis::decode_oriented(&analysis::base64_decode(&image).unwrap()).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (80, 60));
    let large = prepare_image(&png(4000, 3000)).unwrap();
    let decoded = analysis::decode_oriented(&analysis::base64_decode(&large).unwrap()).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (3072, 2304));
}

fn chat_reply(content: &str, finish: &str) -> Resp {
    Resp::json(
        200,
        json!({"choices":[{"message":{"content":content},"finish_reason":finish}],
        "usage":{"prompt_tokens":36,"completion_tokens":7}}),
    )
}

#[tokio::test]
async fn the_request_asks_for_a_large_reply_records_usage_and_returns_every_row() {
    let reply = sheet("", two_rows());
    let server = serve(move |_| chat_reply(&reply, "stop")).await;
    let provider = Provider::new(&server.base, "label-test", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path()).unwrap();
    let drafts = request_labels(&lib, &provider, prepare_image(&png(32, 32)).unwrap())
        .await
        .unwrap();
    assert_eq!(drafts.len(), 2);
    let req = server.seen_at("/chat/completions");
    assert_eq!(req.len(), 1);
    let body = req[0].json();
    let asked = body["max_tokens"]
        .as_u64()
        .or(body["max_completion_tokens"].as_u64())
        .unwrap();
    assert_eq!(asked, 10_000);
    assert!(body["messages"][1]["content"][1]["image_url"]["url"]
        .as_str()
        .unwrap()
        .starts_with("data:image/jpeg;base64,"));
    let usage = lib.usage().unwrap();
    assert_eq!(
        (usage.input_tokens, usage.output_tokens, usage.requests),
        (36, 7, 1)
    );
}

#[tokio::test]
async fn a_cut_off_reply_is_an_error_and_an_invalid_one_still_records_usage() {
    let cut = sheet("", two_rows());
    let server = serve(move |_| chat_reply(&cut, "length")).await;
    let provider = Provider::new(&server.base, "label-test", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path()).unwrap();
    let image = prepare_image(&png(32, 32)).unwrap();
    let error = request_labels(&lib, &provider, image.clone())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("incomplete"), "{error}");
    let bad = serve(|_| chat_reply("{}", "stop")).await;
    let provider = Provider::new(&bad.base, "label-test", "").unwrap();
    assert!(request_labels(&lib, &provider, image).await.is_err());
    assert_eq!(lib.usage().unwrap().requests, 2);
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
    assert!(read_labels(&lib, &cfg, &photo)
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
    assert!(read_labels(&lib, &cfg, &photo)
        .await
        .unwrap_err()
        .to_string()
        .contains("budget"));
}

fn one_row(text: &str, specs: Value) -> InventoryDraft {
    parse_labels(&sheet("", vec![row(text, "Part", "other", specs)]))
        .unwrap()
        .remove(0)
}

#[test]
fn the_fraction_of_a_decimal_is_not_a_printed_number() {
    for (text, role, value, unit, evidence) in [
        ("1.75 mm", "diameter", "75", "mm", "75 mm"),
        ("Length 2.5 mm", "length", "5", "mm", "5 mm"),
        ("Length 5,7 mm", "length", "7", "mm", "7 mm"),
        ("M2x2x3.8 mm", "length", "8", "mm", "8 mm"),
    ] {
        let draft = one_row(text, json!([spec(role, value, unit, "stated", evidence)]));
        assert!(draft.specs.is_empty(), "{text}: {:?}", specs_of(&draft));
    }
}

#[test]
fn the_unit_must_be_printed_with_the_value() {
    let kept = |text: &str, role: &str, value: &str, unit: &str, evidence: &str| {
        one_row(text, json!([spec(role, value, unit, "stated", evidence)]))
            .specs
            .len()
            == 1
    };
    assert!(!kept("Net weight 1 kg", "net_weight", "1", "g", "1 kg"));
    assert!(!kept("1KG / 2.2LBS", "net_weight", "2.2", "kg", "2.2LBS"));
    assert!(!kept("Length 1/2 in", "length", "2", "mm", "1/2 in"));
    assert!(!kept("Net weight 1 kg", "net_weight", "1", "", "1 kg"));
    assert!(kept("1KG / 2.2LBS", "net_weight", "1", "kg", "1KG"));
    assert!(kept("Length 8 MM", "length", "8", "MM", "8 MM"));
}

#[test]
fn harmless_formatting_does_not_lose_a_printed_spec() {
    let draft = one_row(
        "Nozzle 220 °C. Length 8 mm",
        json!([
        spec("nozzle_temp", "220", "C", "stated", "220 °C"),
        {"role": "length", "name": "Length", "value": "8", "unit": "mm", "basis": "stated", "evidence": "8 mm"}]),
    );
    assert_eq!(draft.specs.len(), 2);
    assert_eq!(draft.specs[0].unit, "°C");
    assert!(draft.specs[1].name.is_empty());
}

#[test]
fn a_tolerance_must_be_printed() {
    let with = |tolerance: &str| {
        one_row(
            "1.75 ±0.03 mm",
            json!([{"role": "diameter", "value": "1.75", "unit": "mm", "tolerance": tolerance,
            "basis": "stated", "evidence": "1.75 ±0.03 mm"}]),
        )
        .specs
        .remove(0)
        .tolerance
    };
    assert_eq!(with("0.03"), "0.03");
    assert_eq!(with("0.02"), "");
}

#[test]
fn a_printed_pitch_in_the_marking_beats_the_table() {
    let bolt = |text: &str, thread: &str| {
        let items = vec![row(
            text,
            "Bolt",
            "bolt",
            json!([
                spec("thread", "M8", "", "notation", thread),
                spec("length", "30", "mm", "stated", "30 mm")
            ]),
        )];
        specs_of(&parse_labels(&sheet("", items)).unwrap()[0])
    };
    let fine = bolt("M8x1 x 30 mm", "M8x1");
    assert!(fine.contains(&("pitch".into(), "1".into(), "notation".into())));
    assert!(!fine.iter().any(|s| s.0 == "pitch" && s.2 == "standard"));
    assert!(fine.contains(&("tap_drill".into(), "7".into(), "standard".into())));
    let coarse = bolt("M8x30 mm 30 mm", "M8x30");
    assert!(coarse.contains(&("pitch".into(), "1.25".into(), "standard".into())));
    assert!(coarse.contains(&("tap_drill".into(), "6.8".into(), "standard".into())));
    let hyphen = one_row(
        "M4-.70 x 10 mm",
        json!([spec("thread", "M4", "", "notation", "M4-.70")]),
    );
    assert!(specs_of(&hyphen).contains(&("pitch".into(), "0.7".into(), "notation".into())));
}

#[test]
fn a_row_that_cannot_be_saved_is_reported_by_number() {
    let mut spool = row("Translucent\nOrange spool", "Spool", "filament", json!([]));
    spool["colour"] = json!("Translucent\nOrange");
    assert_eq!(
        parse_labels(&sheet("", vec![spool])).unwrap()[0].colour,
        "Translucent Orange"
    );
    let long = row("Row two text", &"x".repeat(600), "screw", json!([]));
    let error = parse_labels(&sheet(
        "",
        vec![row("Row one", "One", "screw", json!([])), long],
    ))
    .unwrap_err()
    .to_string();
    assert!(error.contains("Item 2"), "{error}");
    assert!(!SCHEMA.contains("at most 40"));
}

#[test]
fn notes_need_evidence_in_the_items_own_row() {
    let mut item = row("M3 x 5 mm", "Insert", "insert", json!([]));
    item["notes"] = json!(["M4 x 10 mm", "M3 x 5 mm"]);
    let draft = &parse_labels(&sheet("Kit M4 x 10 mm", vec![item])).unwrap()[0];
    assert_eq!(draft.notes, "M3 x 5 mm");
}
