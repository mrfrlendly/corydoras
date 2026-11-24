use super::*;
use crate::inventory::{InventoryDraft, SupplierListing};
use serde_json::json;
use tempfile::{tempdir, TempDir};

const IMPORT_ID: &str = "f3ea7412f80f497595eb090b105f3fe6";

fn fixture() -> Result<(TempDir, Library, String)> {
    let dir = tempdir()?;
    let lib = Library::open(dir.path())?;
    lib.set_inventory_enabled(true)?;
    let screw = InventoryDraft {
        name: "Machine screw M3x8".into(), category: "Fasteners".into(),
        brand: "Acme".into(), part_number: "BL-041".into(), colour: "Black".into(),
        dimensions: "Thread M3; length 8mm".into(), quantity: Some(5), pack_quantity: Some(100),
        notes: "Material: Brass (label)\nLocation: Drawer 8\nParent kit: Ring terminal M5 (archived)\nAmazon (similar product): Green M12 screw https://example.com/needle".into(),
        specs: vec![],
        supplier: Some(SupplierListing {title:"Box of screws".into(),url:"https://example.com/needle".into(),details:"Supplier says green".into(),..Default::default()}),
        price_cents: None,
        ..InventoryDraft::default()
    };
    let id = lib.save_inventory(None, &screw)?.id;
    lib.c().execute(
        "UPDATE workshop_inventory SET id=?1 WHERE id=?2",
        [IMPORT_ID, &id],
    )?;
    let other = lib.save_inventory(
        None,
        &InventoryDraft {
            name: "Machine screw M30×80".into(),
            category: "Fasteners".into(),
            quantity: Some(0),
            ..InventoryDraft::default()
        },
    )?;
    Ok((dir, lib, other.id))
}

#[test]
fn searches_identifying_fields_and_ignores_external_evidence() -> Result<()> {
    let (_dir, lib, _) = fixture()?;
    for query in [
        "M3",
        "black Acme m3×8",
        "M3 x 8 mm",
        "BL-041 screw",
        "brass fastener",
        "Drawer 8",
    ] {
        let hits = search(&lib, query, 10, 0)?;
        assert_eq!(hits["total"], 1, "{query}");
        assert_eq!(hits["items"][0]["id"], IMPORT_ID);
        assert_eq!(hits["items"][0]["has_photo"], false);
        assert!(hits["items"][0].get("notes").is_none());
    }
    for query in ["M5", "M12", "green", "needle", "ring terminal"] {
        assert_eq!(search(&lib, query, 10, 0)?["status"], "no_match", "{query}");
    }
    Ok(())
}

#[test]
fn browse_paginates_in_stable_order_and_search_prefers_item_name() -> Result<()> {
    let (_dir, lib, _) = fixture()?;
    let first = search(&lib, "", 1, 0)?;
    let last = search(&lib, "", 1, 1)?;
    assert_eq!(first["total"], 2);
    assert_eq!(first["next_offset"], 1);
    assert!(last["next_offset"].is_null());
    assert_ne!(first["items"][0]["id"], last["items"][0]["id"]);
    let past = search(&lib, "screw", 1, 5)?;
    assert_eq!(past["status"], "ok");
    assert_eq!(past["items"], json!([]));
    let name_match = lib.save_inventory(
        None,
        &InventoryDraft {
            name: "Black".into(),
            ..InventoryDraft::default()
        },
    )?;
    assert_eq!(
        search(&lib, "black", 1, 0)?["items"][0]["id"],
        name_match.id
    );
    Ok(())
}

#[test]
fn confirms_only_current_stock_and_preserves_record_evidence() -> Result<()> {
    let (_dir, lib, zero_id) = fixture()?;
    let detail = item(&lib, IMPORT_ID, Some(5))?;
    assert_eq!(detail["status"], "ok");
    let record = &detail["item"];
    assert_eq!(record["stock_state"], "in_stock");
    assert_eq!(record["quantity"], 5);
    assert_eq!(record["pack_quantity"], 100);
    assert_eq!(record["supplier"]["title"], "Box of screws");
    assert!(record["notes"]
        .as_str()
        .unwrap()
        .contains("Location: Drawer 8"));
    assert_eq!(
        detail["quantity_check"],
        json!({"required_quantity":5,"status":"sufficient"})
    );
    assert_eq!(
        item(&lib, IMPORT_ID, Some(6))?["quantity_check"]["status"],
        "insufficient"
    );
    assert_eq!(
        item(&lib, &zero_id, None)?["item"]["stock_state"],
        "out_of_stock"
    );
    assert!(item(&lib, &zero_id, None)?["quantity_check"].is_null());
    lib.c().execute(
        "UPDATE workshop_inventory SET quantity=NULL WHERE id=?1",
        [IMPORT_ID],
    )?;
    let detail = item(&lib, IMPORT_ID, Some(1))?;
    assert_eq!(detail["item"]["stock_state"], "unknown");
    assert!(detail["item"]["quantity"].is_null());
    assert_eq!(detail["quantity_check"]["status"], "unknown");
    assert_eq!(item(&lib, &"0".repeat(32), None)?["status"], "not_found");
    Ok(())
}

#[test]
fn applies_opt_in_and_handles_older_schema_with_read_only_sql() -> Result<()> {
    let (_dir, lib, _) = fixture()?;
    lib.c().execute_batch(
        "DROP TRIGGER workshop_photo_deleted; DROP TRIGGER workshop_photo_replaced;
        DROP INDEX idx_workshop_photo_key; ALTER TABLE workshop_inventory DROP COLUMN colour;
        ALTER TABLE workshop_inventory DROP COLUMN photo_key;",
    )?;
    lib.c().execute_batch("PRAGMA query_only=ON")?;
    assert_eq!(search(&lib, "m3", 1, 0)?["items"][0]["colour"], "");
    assert_eq!(item(&lib, IMPORT_ID, None)?["item"]["has_photo"], false);
    lib.c().execute_batch("PRAGMA query_only=OFF; DELETE FROM settings; DROP TABLE workshop_inventory; PRAGMA query_only=ON")?;
    assert_eq!(search(&lib, "", 1, 0)?, json!({"status":"disabled"}));
    assert_eq!(
        item(&lib, IMPORT_ID, Some(1))?,
        json!({"status":"disabled"})
    );
    Ok(())
}

#[test]
fn rejects_invalid_ids_counts_queries_and_pagination() -> Result<()> {
    let (_dir, lib, _) = fixture()?;
    for id in [
        "",
        "1234",
        "../stock",
        "urn:uuid:f3ea7412-f80f-4975-95eb-090b105f3fe6",
    ] {
        assert!(item(&lib, id, None).is_err());
    }
    assert!(item(&lib, IMPORT_ID, Some(0)).is_err());
    assert!(item(&lib, IMPORT_ID, Some(1 << 53)).is_err());
    assert!(search(&lib, "M3", 0, 0).is_err());
    assert!(search(&lib, "M3", 51, 0).is_err());
    assert!(search(&lib, "M3", 1, usize::MAX).is_err());
    assert!(search(&lib, "M3\0", 1, 0).is_err());
    assert!(search(&lib, "***", 1, 0).is_err());
    assert!(search(&lib, &"a".repeat(257), 1, 0).is_err());
    Ok(())
}

#[test]
fn recognises_metric_sizes_with_asymmetric_spacing() -> Result<()> {
    let (_dir, lib, _) = fixture()?;
    for label in ["M3 x5x4", "M3x 5x4", "M3 x 5 x 4", "M3*5*4"] {
        let fixture = lib.save_inventory(
            None,
            &InventoryDraft {
                name: label.into(),
                ..InventoryDraft::default()
            },
        )?;
        let hits = search(&lib, "M3x5x4", 20, 0)?;
        assert!(
            hits["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["id"] == fixture.id),
            "{label}"
        );
    }
    Ok(())
}

#[test]
fn spool_quantity_counts_whole_inventory_items() -> Result<()> {
    let (_dir, lib, _) = fixture()?;
    let spool = lib.save_inventory(
        None,
        &InventoryDraft {
            name: "PETG spool".into(),
            dimensions: "330 m".into(),
            notes: "Net weight 1000 g".into(),
            quantity: Some(2),
            ..InventoryDraft::default()
        },
    )?;
    let page = search(&lib, "PETG spool", 1, 0)?;
    assert_eq!(page["items"][0]["quantity_unit"], "item");
    let detail = item(&lib, &spool.id, Some(3))?;
    assert_eq!(detail["item"]["quantity_unit"], "item");
    assert_eq!(detail["item"]["quantity"], 2);
    assert_eq!(detail["quantity_check"]["status"], "insufficient");
    Ok(())
}

fn typed_spec(role: crate::inventory_specs::SpecRole, value: &str, unit: &str, source: crate::inventory_specs::SpecSource) -> crate::inventory_specs::Spec {
    crate::inventory_specs::Spec {
        role,
        name: String::new(),
        value: value.into(),
        unit: unit.into(),
        tolerance: String::new(),
        source,
        evidence: "printed".into(),
    }
}

#[test]
fn search_and_item_return_specs_with_sources() -> Result<()> {
    use crate::inventory_specs::{SpecRole::*, SpecSource::*};
    let (_dir, lib, _) = fixture()?;
    let saved = lib.save_inventory(
        None,
        &InventoryDraft {
            name: "Heat-set insert".into(),
            specs: vec![typed_spec(Thread, "M3", "", Label), typed_spec(HoleDiameter, "4.2", "mm", Entered)],
            ..InventoryDraft::default()
        },
    )?;
    let found = search(&lib, "heat-set insert", 5, 0)?;
    let specs = &found["items"][0]["specs"];
    assert_eq!((specs[0]["source"].as_str(), specs[1]["source"].as_str()), (Some("label"), Some("entered")));
    assert_eq!(specs[1]["role"], "hole_diameter");
    assert_eq!(found["items"][0]["dimensions"], "M3 thread; hole diameter 4.2 mm");
    let detail = item(&lib, &saved.id, None)?;
    assert_eq!(detail["item"]["specs"], *specs);
    Ok(())
}

#[test]
fn an_older_library_returns_empty_specs_without_a_write() -> Result<()> {
    let (_dir, lib, _) = fixture()?;
    lib.c().execute_batch("ALTER TABLE workshop_inventory DROP COLUMN specs; PRAGMA query_only=ON")?;
    assert_eq!(search(&lib, "m3", 1, 0)?["items"][0]["specs"], json!([]));
    assert_eq!(item(&lib, IMPORT_ID, None)?["item"]["specs"], json!([]));
    Ok(())
}

#[test]
fn unit_price_divides_the_pack_price_and_trims_zeros() {
    for (price, pack, expected) in [
        (Some(2220), Some(100), Some("0.222")),
        (Some(500), Some(1), Some("5.00")),
        (Some(500), None, Some("5.00")),
        (Some(500), Some(0), Some("5.00")),
        (Some(500), Some(-3), Some("5.00")),
        (Some(1999), Some(3), Some("6.6633")),
        (Some(1), Some(3), Some("0.0033")),
        (Some(0), Some(10), Some("0.00")),
        (Some(1_000_000_000_000), Some(1), Some("10000000000.00")),
        (None, Some(10), None),
    ] {
        assert_eq!(unit_price(price, pack).as_deref(), expected, "{price:?} / {pack:?}");
    }
}

#[test]
fn records_carry_price_currency_and_unit_price() -> Result<()> {
    let (_dir, lib, zero_id) = fixture()?;
    lib.c().execute("UPDATE workshop_inventory SET price_cents=2220 WHERE id=?1", [IMPORT_ID])?;
    let detail = item(&lib, IMPORT_ID, Some(1))?;
    assert_eq!(detail["item"]["price_cents"], 2220);
    assert_eq!(detail["item"]["currency"], "CAD");
    assert_eq!(detail["item"]["unit_price"], "0.222");
    assert_eq!(detail["quantity_check"]["status"], "sufficient");
    let hit = &search(&lib, "m3", 1, 0)?["items"][0];
    assert_eq!((&hit["price_cents"], &hit["currency"], &hit["unit_price"]), (&json!(2220), &json!("CAD"), &json!("0.222")));
    // A record without a price says so with nulls, and a price never counts as stock.
    let bare = item(&lib, &zero_id, Some(1))?;
    assert!(bare["item"]["price_cents"].is_null() && bare["item"]["unit_price"].is_null());
    assert_eq!(bare["item"]["currency"], "CAD");
    assert_eq!(bare["item"]["stock_state"], "out_of_stock");
    assert_eq!(bare["quantity_check"]["status"], "insufficient");
    lib.set_currency("USD")?;
    assert_eq!(item(&lib, IMPORT_ID, None)?["item"]["currency"], "USD");
    assert_eq!(search(&lib, "m3", 1, 0)?["items"][0]["currency"], "USD");
    lib.c().execute("UPDATE settings SET value='usd' WHERE key='workshop_currency'", [])?;
    assert_eq!(item(&lib, IMPORT_ID, None)?["item"]["currency"], "CAD");
    Ok(())
}

#[test]
fn an_older_library_returns_null_prices_without_a_write() -> Result<()> {
    let (_dir, lib, _) = fixture()?;
    lib.c().execute_batch("ALTER TABLE workshop_inventory DROP COLUMN price_cents; PRAGMA query_only=ON")?;
    let hit = &search(&lib, "m3", 1, 0)?["items"][0];
    assert!(hit["price_cents"].is_null() && hit["unit_price"].is_null());
    assert_eq!(hit["currency"], "CAD");
    assert!(item(&lib, IMPORT_ID, None)?["item"]["price_cents"].is_null());
    Ok(())
}

#[test]
fn an_older_library_has_no_estimates_and_the_flag_follows_the_price_source() -> Result<()> {
    let (_dir, lib, _) = fixture()?;
    assert_eq!(item(&lib, IMPORT_ID, None)?["item"]["price_is_estimate"], false);
    lib.c().execute("UPDATE workshop_inventory SET price_cents=500, price_source='listing:a.example.com' WHERE id=?1", [IMPORT_ID])?;
    assert_eq!(item(&lib, IMPORT_ID, None)?["item"]["price_is_estimate"], true);
    assert_eq!(search(&lib, "m3", 1, 0)?["items"][0]["price_is_estimate"], true);
    lib.c().execute_batch("ALTER TABLE workshop_inventory DROP COLUMN price_source; PRAGMA query_only=ON")?;
    let hit = &search(&lib, "m3", 1, 0)?["items"][0];
    assert_eq!(hit["price_is_estimate"], false);
    assert_eq!(hit["price_cents"], 500);
    assert_eq!(item(&lib, IMPORT_ID, None)?["item"]["price_is_estimate"], false);
    Ok(())
}

fn measured(lib: &Library, unit: &str, full: i64, left: Option<i64>, quantity: Option<i64>) -> Result<String> {
    Ok(lib
        .save_inventory(
            None,
            &InventoryDraft {
                name: "PETG spool".into(),
                category: "Filament".into(),
                quantity,
                price_cents: Some(2499),
                measure_unit: unit.into(),
                measure_full: Some(full),
                measure_left: left,
                ..InventoryDraft::default()
            },
        )?
        .id)
}

#[test]
fn records_carry_the_measured_pack_and_its_total_amount() -> Result<()> {
    let (_dir, lib, _) = fixture()?;
    let cases = [
        // (quantity, left, total)
        (Some(1), Some(100), json!(100)),
        (Some(3), Some(250), json!(2250)),
        (Some(3), None, json!(3000)),
        (Some(0), None, json!(0)),
        (None, Some(100), Value::Null),
    ];
    for (quantity, left, total) in cases {
        let id = measured(&lib, "g", 1000, left, quantity)?;
        let record = &item(&lib, &id, None)?["item"];
        assert_eq!(record["measure_unit"], "g");
        assert_eq!(record["measure_full"], 1000);
        assert_eq!(record["measure_left"], json!(left.filter(|_| quantity != Some(0))));
        assert_eq!(record["total_amount"], total, "{quantity:?} {left:?}");
        // The price is per gram once a full pack amount exists: 24.99 / 1000 g, to four places.
        assert_eq!(record["unit_price"], "0.025");
        let hit = search(&lib, "", 50, 0)?["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == json!(id))
            .cloned()
            .unwrap();
        assert_eq!(hit["total_amount"], total);
    }
    // An item without a measured pack says so with an empty unit and nulls, and keeps the piece price.
    let plain = item(&lib, IMPORT_ID, None)?;
    let record = &plain["item"];
    assert_eq!(record["measure_unit"], "");
    assert!(record["measure_full"].is_null() && record["measure_left"].is_null());
    assert!(record["total_amount"].is_null());
    Ok(())
}

#[test]
fn the_price_of_a_measured_pack_is_per_unit_and_ignores_the_pack_count() {
    let mut draft = InventoryDraft {
        pack_quantity: Some(4),
        price_cents: Some(2400),
        ..InventoryDraft::default()
    };
    assert_eq!(unit_price(draft.price_cents, price_divisor(&draft)).as_deref(), Some("6.00"));
    draft.measure_unit = "g".into();
    draft.measure_full = Some(1000);
    assert_eq!(unit_price(draft.price_cents, price_divisor(&draft)).as_deref(), Some("0.024"));
    assert_eq!(price_divisor(&draft), Some(1000));
    // A unit with no full pack amount falls back to the pack count.
    draft.measure_full = None;
    assert_eq!(price_divisor(&draft), Some(4));
}

#[test]
fn a_required_amount_is_compared_with_the_recorded_total() -> Result<()> {
    let (_dir, lib, zero_id) = fixture()?;
    let one = measured(&lib, "g", 1000, Some(100), Some(1))?;
    let three = measured(&lib, "g", 1000, Some(250), Some(3))?;
    let check = |id: &str, required, unit: &str| -> Result<Value> {
        Ok(item_in_unit(&lib, id, Some(required), unit)?["quantity_check"].clone())
    };
    // 1 spool with 100 g left holds 100 g.
    let pass = check(&one, 100, "g")?;
    assert_eq!(pass["status"], "sufficient");
    assert_eq!((&pass["total_amount"], &pass["required_unit"]), (&json!(100), &json!("g")));
    assert!(pass["note"].as_str().unwrap().contains("not verified by weight"));
    let short = check(&one, 101, "g")?;
    assert_eq!(short["status"], "insufficient");
    assert!(short["note"].as_str().unwrap().contains("shortage"));
    // 3 spools with 250 g left in the opened one hold 2250 g.
    assert_eq!(check(&three, 2250, "g")?["status"], "sufficient");
    assert_eq!(check(&three, 2251, "g")?["status"], "insufficient");
    // The spool count still answers in items, in the old shape.
    assert_eq!(
        item_in_unit(&lib, &three, Some(3), "item")?["quantity_check"],
        json!({"required_quantity":3,"status":"sufficient"})
    );
    assert_eq!(item(&lib, &three, Some(4))?["quantity_check"]["status"], "insufficient");
    // Unknown totals are unconfirmed: no count, no open amount recorded is fine (whole packs), a
    // missing count is not, and neither is another unit or an item without a measured pack.
    let unknown = measured(&lib, "g", 1000, Some(100), None)?;
    let none = check(&unknown, 1, "g")?;
    assert_eq!((&none["status"], &none["total_amount"]), (&json!("unknown"), &Value::Null));
    assert_eq!(check(&one, 1, "mL")?["status"], "unknown");
    assert!(check(&one, 1, "mL")?["note"].as_str().unwrap().contains("in g, not in mL"));
    assert_eq!(check(&zero_id, 1, "g")?["status"], "unknown");
    assert!(check(&zero_id, 1, "g")?["note"].as_str().unwrap().contains("as a count"));
    let empty = measured(&lib, "g", 1000, None, Some(0))?;
    assert_eq!(check(&empty, 1, "g")?["status"], "insufficient");
    // No requirement, no check. An unknown unit is refused.
    assert!(item_in_unit(&lib, &one, None, "g")?["quantity_check"].is_null());
    for unit in ["kg", "mm", "", "G", "kit_contents"] {
        assert!(item_in_unit(&lib, &one, Some(1), unit).is_err(), "{unit}");
    }
    Ok(())
}

#[test]
fn an_older_library_without_the_measure_columns_reads_without_a_write() -> Result<()> {
    let (_dir, lib, _) = fixture()?;
    lib.c().execute_batch(
        "ALTER TABLE workshop_inventory DROP COLUMN measure_unit;
         ALTER TABLE workshop_inventory DROP COLUMN measure_full;
         ALTER TABLE workshop_inventory DROP COLUMN measure_left;
         PRAGMA query_only=ON",
    )?;
    let hit = &search(&lib, "m3", 1, 0)?["items"][0];
    assert_eq!(hit["measure_unit"], "");
    assert!(hit["measure_full"].is_null() && hit["measure_left"].is_null() && hit["total_amount"].is_null());
    let detail = item_in_unit(&lib, IMPORT_ID, Some(1), "g")?;
    assert_eq!(detail["item"]["quantity"], 5);
    assert_eq!(detail["quantity_check"]["status"], "unknown");
    assert_eq!(detail["item"]["unit_price"], Value::Null);
    assert_eq!(item(&lib, IMPORT_ID, Some(5))?["quantity_check"]["status"], "sufficient");
    Ok(())
}
