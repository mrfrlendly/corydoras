//! Validates photo storage across edits, shared labels, and database copies.
use anyhow::Result;
use corydoras_core::{
    inventory::InventoryDraft, inventory_media::InventoryPhotoChange as Photo, Library,
};
use image::{DynamicImage, GenericImageView, ImageFormat, Rgb, RgbImage};
use std::io::Cursor;

fn png(colour: [u8; 3]) -> Vec<u8> {
    let img = DynamicImage::ImageRgb8(RgbImage::from_pixel(480, 240, Rgb(colour)));
    let mut w = Cursor::new(Vec::new());
    img.write_to(&mut w, ImageFormat::Png).unwrap();
    w.into_inner()
}

fn draft(name: &str) -> InventoryDraft {
    InventoryDraft {
        name: name.into(),
        colour: "  Clear Blue  ".into(),
        ..Default::default()
    }
}

#[test]
fn photos_are_shared_preserved_and_removed_after_the_last_reference() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let lib = Library::open(dir.path())?;
    let bytes = png([9, 50, 80]);
    let one = lib.save_inventory_with_photo(None, &draft("One"), Photo::Replace(&bytes))?;
    let key = one.photo_key.as_deref().unwrap();
    assert_eq!(one.draft.colour, "Clear Blue");
    let first_jpg = lib.inventory_photo(key, false)?;
    assert_eq!(image::guess_format(&first_jpg)?, ImageFormat::Jpeg);
    assert_eq!(
        image::load_from_memory(&first_jpg)?.dimensions(),
        (480, 240)
    );
    let thumb = lib.inventory_photo(key, true)?;
    assert!(thumb.len() < first_jpg.len());
    assert!(image::load_from_memory(&thumb)?.width() <= 192);
    let two = lib.save_inventory_with_photo(None, &draft("Two"), Photo::Replace(&bytes))?;
    assert_eq!(two.photo_key, one.photo_key);
    let changed = lib.save_inventory(Some(&two.id), &draft("Two edited"))?;
    assert_eq!(one.photo_key, changed.photo_key);
    let new = lib.save_inventory_with_photo(
        Some(&one.id),
        &draft("One"),
        Photo::Replace(&png([120, 20, 40])),
    )?;
    let new_key = new.photo_key.as_deref().unwrap();
    assert_ne!(new_key, key);
    let copied = tempfile::tempdir()?;
    lib.copy_database_to(&copied.path().join("library.sqlite"))?;
    let restored = Library::open(copied.path())?;
    assert_eq!(restored.inventory_photo(key, false)?, first_jpg);
    assert_eq!(restored.list_inventory()?, lib.list_inventory()?);
    lib.delete_inventory(&two.id)?;
    assert!(lib.inventory_photo(key, false).is_err());
    let clear = lib.save_inventory_with_photo(Some(&one.id), &draft("One"), Photo::Remove)?;
    assert!(clear.photo_key.is_none() && lib.inventory_photo(new_key, true).is_err());
    Ok(())
}

#[test]
fn rejection_does_not_change_stock_or_create_an_orphan() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let lib = Library::open(dir.path())?;
    let one =
        lib.save_inventory_with_photo(None, &draft("One"), Photo::Replace(&png([100, 100, 100])))?;
    assert!(lib
        .save_inventory_with_photo(Some(&one.id), &draft("Changed"), Photo::Replace(b"broken"))
        .is_err());
    assert_eq!(lib.list_inventory()?, vec![one]);
    let bytes = png([1, 2, 3]);
    assert!(lib
        .save_inventory_with_photo(Some("missing"), &draft("Other"), Photo::Replace(&bytes))
        .is_err());
    let db = rusqlite::Connection::open(dir.path().join("library.sqlite"))?;
    assert_eq!(
        db.query_row("SELECT count(*) FROM workshop_photos", [], |row| row
            .get::<_, u32>(0))?,
        1
    );
    let bad = InventoryDraft {
        colour: "two\nlines".into(),
        ..draft("New")
    };
    assert!(lib.save_inventory(None, &bad).is_err());
    assert!(lib.inventory_photo("../photo.jpg", true).is_err());
    Ok(())
}

#[test]
fn established_legacy_rows_gain_nullable_media_and_unknown_colour() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let db = rusqlite::Connection::open(dir.path().join("library.sqlite"))?;
    db.execute_batch("CREATE TABLE workshop_inventory(id TEXT PRIMARY KEY, name TEXT, category TEXT,
        brand TEXT, part_number TEXT, dimensions TEXT, quantity INTEGER, pack_quantity INTEGER, notes TEXT,
        supplier_url TEXT, supplier_title TEXT, supplier_details TEXT);
        INSERT INTO workshop_inventory VALUES ('old','Washer','Hardware','','','',0,25,'',NULL,'','');")?;
    drop(db);
    let lib = Library::open(dir.path())?;
    let rows = lib.list_inventory()?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].photo_key, None);
    assert_eq!(rows[0].draft.colour, "");
    assert_eq!(rows[0].draft.quantity, Some(0));
    assert_eq!(rows[0].draft.pack_quantity, Some(25));
    Ok(())
}
