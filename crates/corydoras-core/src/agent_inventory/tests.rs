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
        supplier: Some(SupplierListing {title:"Box of screws".into(),url:"https://example.com/needle".into(),details:"Supplier says green".into()})
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
