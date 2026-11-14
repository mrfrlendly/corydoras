use super::*;
use crate::inventory::InventoryDraft;
use crate::inventory_media::InventoryPhotoChange;
use image::{DynamicImage, ImageFormat};
use serde_json::{json, Value};
use std::io::Cursor;
use tempfile::tempdir;

const A: &str = "11111111111111111111111111111111";
const B: &str = "22222222222222222222222222222222";
const C: &str = "33333333333333333333333333333333";
const D: &str = "44444444444444444444444444444444";
const P: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn jpeg(width: u32) -> Vec<u8> {
    let mut data = Cursor::new(Vec::new());
    DynamicImage::new_rgb8(width, 10)
        .write_to(&mut data, ImageFormat::Jpeg)
        .unwrap();
    data.into_inner()
}

fn entry(id: &str, archived: bool, colour: &str) -> Value {
    json!({"id":id,"archived":archived,"photos":[P],"attributes":{"colour":{"value":colour}}})
}

fn archive(dir: &Path, entries: &[Value]) {
    std::fs::create_dir_all(dir.join("photos")).unwrap();
    let photo = format!("photos/{P}.jpg");
    let mut files = serde_json::Map::new();
    for (name, bytes) in [
        (
            "catalogue.json",
            serde_json::to_vec(&json!({"items": entries})).unwrap(),
        ),
        (&photo, jpeg(13)),
    ] {
        std::fs::write(dir.join(name), &bytes).unwrap();
        files.insert(
            name.into(),
            json!({"sha256":sha256(&bytes),"bytes":bytes.len()}),
        );
    }
    std::fs::write(
        dir.join("manifest.json"),
        json!({"files":files}).to_string(),
    )
    .unwrap();
}

fn item(lib: &Library, id: &str, colour: &str, photo: Option<&[u8]>) {
    let draft = InventoryDraft {
        name: "Edited label".into(),
        notes: "Keep the user note".into(),
        colour: colour.into(),
        ..Default::default()
    };
    let saved = lib
        .save_inventory_with_photo(
            None,
            &draft,
            photo
                .map(InventoryPhotoChange::Replace)
                .unwrap_or(InventoryPhotoChange::Keep),
        )
        .unwrap();
    lib.c()
        .execute(
            "UPDATE workshop_inventory SET id=? WHERE id=?",
            [id, &saved.id],
        )
        .unwrap();
}

#[test]
fn backfill_matches_active_ids_and_preserves_manual_fields_and_later_removals() {
    let root = tempdir().unwrap();
    let lib = Library::open(&root.path().join("lib")).unwrap();
    let arc = root.path().join("archive");
    archive(
        &arc,
        &[
            entry(A, false, "Amber"),
            entry(B, false, "Grey"),
            entry(C, true, "Red"),
            entry(D, false, "Blue"),
        ],
    );
    item(&lib, A, "", None);
    item(&lib, B, "Manual blue", Some(&jpeg(16)));
    item(&lib, C, "", None);
    let report = import_workshop_media(&lib, &arc).unwrap();
    assert_eq!(
        (
            report.photos_added,
            report.colours_added,
            report.matched,
            report.archived,
            report.unmatched
        ),
        (1, 1, 2, 1, 1)
    );
    let rows = lib.list_inventory().unwrap();
    let a = rows.iter().find(|i| i.id == A).unwrap();
    let b = rows.iter().find(|i| i.id == B).unwrap();
    assert_eq!(
        (&*a.draft.colour, &*a.draft.notes),
        ("Amber", "Keep the user note")
    );
    assert_eq!(b.draft.colour, "Manual blue");
    assert_ne!(a.photo_key, b.photo_key);
    assert!(rows.iter().find(|i| i.id == C).unwrap().photo_key.is_none());
    let repeat = import_workshop_media(&lib, &arc).unwrap();
    assert_eq!((repeat.photos_added, repeat.colours_added), (0, 0));
    let draft = InventoryDraft {
        colour: String::new(),
        ..a.draft.clone()
    };
    lib.save_inventory_with_photo(Some(A), &draft, InventoryPhotoChange::Remove)
        .unwrap();
    let repeat = import_workshop_media(&lib, &arc).unwrap();
    assert_eq!((repeat.photos_added, repeat.colours_added), (0, 0));
    let a = lib
        .list_inventory()
        .unwrap()
        .into_iter()
        .find(|i| i.id == A)
        .unwrap();
    assert!(a.photo_key.is_none() && a.draft.colour.is_empty());
}

#[test]
fn shared_photos_reuse_one_blob_and_corrupt_or_unsafe_archives_fail_before_items_change() {
    let root = tempdir().unwrap();
    let lib = Library::open(&root.path().join("lib")).unwrap();
    let arc = root.path().join("archive");
    let entries = [entry(A, false, ""), entry(B, false, "Grey")];
    archive(&arc, &entries);
    item(&lib, A, "", None);
    item(&lib, B, "", None);
    std::fs::write(arc.join(format!("photos/{P}.jpg")), b"bad").unwrap();
    assert!(import_workshop_media(&lib, &arc).is_err());
    assert!(lib
        .list_inventory()
        .unwrap()
        .iter()
        .all(|i| i.photo_key.is_none() && i.draft.colour.is_empty()));
    let mut unsafe_row = entry(A, false, "Amber");
    unsafe_row["photos"] = json!(["../../private"]);
    archive(&arc, &[unsafe_row]);
    assert!(import_workshop_media(&lib, &arc).is_err());
    archive(&arc, &entries);
    let report = import_workshop_media(&lib, &arc).unwrap();
    assert_eq!((report.photos_added, report.colours_added), (2, 1));
    let rows = lib.list_inventory().unwrap();
    assert_eq!(rows[0].photo_key, rows[1].photo_key);
    assert_eq!(
        lib.c()
            .query_row("SELECT COUNT(*) FROM workshop_photos", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
