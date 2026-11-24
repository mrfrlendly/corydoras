use anyhow::Result;
use corydoras_core::{
    inventory::{InventoryDraft, SupplierListing, MEASURE_UNITS},
    inventory_categories::{CategoryInfo, COLUMN_IDS},
    inventory_specs::{Spec, SpecRole, SpecSource},
    snapshot, Library,
};
use std::path::Path;

fn example() -> InventoryDraft {
    InventoryDraft {
        name: "Socket head screws".into(),
        category: "Fasteners".into(),
        colour: "Zinc grey".into(),
        brand: "Fixture Supply".into(),
        part_number: "A2-816".into(),
        dimensions: "M8 × 16 mm".into(),
        quantity: None,
        pack_quantity: Some(25),
        price_cents: None,
        notes: "Bin B7".into(),
        specs: vec![],
        supplier: Some(SupplierListing {
            title: "A2-816 socket head screws".into(),
            url: "https://supplier.example.com/parts/a2-816".into(),
            details: "Stainless steel".into(),
        }),
        ..Default::default()
    }
}

#[test]
fn inventory_crud_persists_and_keeps_pack_count_independent() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    let draft = example();
    let item = lib.save_inventory(None, &draft)?;
    assert_eq!(lib.list_inventory()?, vec![item.clone()]);
    assert_eq!(serde_json::to_value(&item)?["name"], draft.name);
    assert_eq!(item.draft.quantity, None);
    let revised = InventoryDraft {
        quantity: Some(0),
        ..draft
    };
    let edited = lib.save_inventory(Some(&item.id), &revised)?;
    assert_eq!(edited.id, item.id);
    drop(lib);
    let reopened = Library::open(root.path())?;
    assert_eq!(reopened.list_inventory()?, vec![edited]);
    reopened.delete_inventory(&item.id)?;
    assert!(reopened.save_inventory(Some(&item.id), &revised).is_err());
    assert!(reopened.delete_inventory(&item.id).is_err());
    assert!(reopened.list_inventory()?.is_empty());
    Ok(())
}

#[test]
fn inventory_validates_and_normalizes_before_it_writes() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    for draft in [
        InventoryDraft {
            name: "  ".into(),
            ..example()
        },
        InventoryDraft {
            brand: "x".repeat(513),
            ..example()
        },
        InventoryDraft {
            notes: "invalid\0note".into(),
            ..example()
        },
        InventoryDraft {
            quantity: Some(-1),
            ..example()
        },
        InventoryDraft {
            pack_quantity: Some(-1),
            ..example()
        },
        InventoryDraft {
            quantity: Some(9_007_199_254_740_992),
            ..example()
        },
    ] {
        assert!(lib.save_inventory(None, &draft).is_err());
    }
    for url in [
        "file:///some/path",
        "http://supplier.com/part",
        "javascript:alert(1)",
        "https://shop:secret@supplier.com/p",
        "https://localhost/p",
        "https://foo.localhost/p",
        "https://machine.local/p",
        "https://machine.local./p",
        "https://intranet/p",
        "https://127.0.0.1/p",
        "https://[::1]/p",
        "https://0x7f000001/p",
        "https://good.com\\@bad.com/p",
    ] {
        let listing = SupplierListing {
            url: url.into(),
            ..example().supplier.unwrap()
        };
        let draft = InventoryDraft {
            supplier: Some(listing),
            ..example()
        };
        assert!(lib.save_inventory(None, &draft).is_err(), "Accepted {url}");
    }
    assert!(lib.list_inventory()?.is_empty());
    let clean = InventoryDraft {
        name: "  Steel rule  ".into(),
        ..Default::default()
    };
    let unsupported = SupplierListing {
        title: "  ".into(),
        ..example().supplier.unwrap()
    };
    assert!(lib
        .save_inventory(
            None,
            &InventoryDraft {
                supplier: Some(unsupported),
                ..clean.clone()
            }
        )
        .is_err());
    let long_url = format!("https://supplier.com/{}", "é".repeat(750));
    assert!(SupplierListing::validate_url(&long_url).is_err());
    let saved = lib.save_inventory(None, &clean)?;
    assert_eq!(saved.draft.name, "Steel rule");
    assert_eq!(saved.draft.category, "Other");
    assert_eq!(saved.draft.supplier, None);
    Ok(())
}

#[test]
fn earlier_libraries_gain_an_empty_inventory_and_snapshots_preserve_new_rows() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    let language = lib.create_language("Before inventory", "")?;
    drop(lib);
    rusqlite::Connection::open(root.path().join("library.sqlite"))?
        .execute("DROP TABLE workshop_inventory", [])?;
    let upgraded = Library::open(root.path())?;
    assert_eq!(
        upgraded.get_language(&language.id)?.name,
        "Before inventory"
    );
    assert!(upgraded.list_inventory()?.is_empty());
    let item = upgraded.save_inventory(None, &example())?;
    let other = tempfile::tempdir()?;
    assert!(Library::open(other.path())?.list_inventory()?.is_empty());
    let archive = snapshot::create(&upgraded, other.path(), &mut |_, _| {})?;
    let copy = other.path().join("restored");
    snapshot::restore(Path::new(&archive.path), &copy)?;
    assert_eq!(Library::open(&copy)?.list_inventory()?, vec![item]);
    Ok(())
}

#[test]
fn disable_hides_stock_and_pipeline_changes_preserve_the_preference() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    assert!(!lib.inventory_enabled()?);
    lib.set_inventory_enabled(true)?;
    let item = lib.save_inventory(None, &example())?;
    lib.save_settings(&lib.settings()?)?;
    assert!(lib.inventory_enabled()?);
    lib.set_inventory_enabled(false)?;
    drop(lib);
    let reopened = Library::open(root.path())?;
    assert!(!reopened.inventory_enabled()?);
    reopened.set_inventory_enabled(true)?;
    assert_eq!(reopened.list_inventory()?, vec![item]);
    Ok(())
}

fn spec(role: SpecRole, value: &str, unit: &str, source: SpecSource) -> Spec {
    Spec {
        role,
        name: String::new(),
        value: value.into(),
        unit: unit.into(),
        tolerance: String::new(),
        source,
        evidence: String::new(),
    }
}

#[test]
fn specs_persist_and_generate_the_dimensions_text() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    let draft = InventoryDraft {
        dimensions: "ignored".into(),
        specs: vec![
            spec(SpecRole::Thread, "M3", "", SpecSource::Label),
            spec(SpecRole::Length, "5.7", "mm", SpecSource::Label),
        ],
        ..example()
    };
    let item = lib.save_inventory(None, &draft)?;
    assert_eq!(item.draft.dimensions, "M3 thread; length 5.7 mm");
    assert_eq!(lib.list_inventory()?, vec![item.clone()]);
    assert_eq!(item.draft.specs, draft.specs);
    Ok(())
}

#[test]
fn an_item_without_specs_keeps_its_typed_dimensions() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    let item = lib.save_inventory(
        None,
        &InventoryDraft {
            dimensions: "Length 20 mm".into(),
            ..example()
        },
    )?;
    assert_eq!(item.draft.dimensions, "Length 20 mm");
    assert!(item.draft.specs.is_empty());
    Ok(())
}

#[test]
fn saving_refreshes_standard_rows() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    let draft = InventoryDraft {
        specs: vec![
            spec(SpecRole::Thread, "M3", "", SpecSource::Label),
            spec(SpecRole::Pitch, "0.5", "mm", SpecSource::Standard),
        ],
        ..example()
    };
    let item = lib.save_inventory(None, &draft)?;
    let mut edited = item.draft.clone();
    edited.specs[0] = spec(SpecRole::Thread, "M4", "", SpecSource::Entered);
    let saved = lib.save_inventory(Some(&item.id), &edited)?;
    let pitch = saved
        .draft
        .specs
        .iter()
        .find(|s| s.role == SpecRole::Pitch)
        .unwrap();
    assert_eq!(
        (pitch.value.as_str(), pitch.source),
        ("0.7", SpecSource::Standard)
    );
    assert_eq!(saved.draft.dimensions, "M4 thread; pitch 0.7 mm (standard)");
    Ok(())
}

#[test]
fn a_library_without_the_specs_column_migrates() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    let item = lib.save_inventory(None, &example())?;
    drop(lib);
    rusqlite::Connection::open(root.path().join("library.sqlite"))?
        .execute("ALTER TABLE workshop_inventory DROP COLUMN specs", [])?;
    let reopened = Library::open(root.path())?;
    assert_eq!(reopened.list_inventory()?, vec![item]);
    Ok(())
}

/// A 1x1 PNG, enough for the batch save to store a photo.
fn tiny_png() -> Vec<u8> {
    let mut bytes = Vec::new();
    image::DynamicImage::new_rgb8(1, 1)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    bytes
}

fn priced(name: &str, category: &str, price_cents: Option<i64>) -> InventoryDraft {
    InventoryDraft {
        name: name.into(),
        category: category.into(),
        price_cents,
        ..Default::default()
    }
}

fn columns(ids: &[&str]) -> Option<Vec<String>> {
    Some(ids.iter().map(|id| id.to_string()).collect())
}

#[test]
fn price_round_trips_through_save_list_and_batch() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    let item = lib.save_inventory(None, &priced("Spool", "Filament", Some(2499)))?;
    assert_eq!(item.draft.price_cents, Some(2499));
    assert_eq!(lib.list_inventory()?, vec![item.clone()]);
    let free = lib.save_inventory(Some(&item.id), &priced("Spool", "Filament", Some(0)))?;
    assert_eq!(free.draft.price_cents, Some(0));
    let cleared = lib.save_inventory(Some(&item.id), &priced("Spool", "Filament", None))?;
    assert_eq!(cleared.draft.price_cents, None);
    let batch = lib.save_inventory_batch(
        &[
            priced("Washer", "Fasteners", Some(1_000_000_000_000)),
            priced("Nut", "Fasteners", None),
        ],
        &tiny_png(),
    )?;
    assert_eq!(batch[0].draft.price_cents, Some(1_000_000_000_000));
    assert_eq!(batch[1].draft.price_cents, None);
    let listed = lib.list_inventory()?;
    for saved in &batch {
        assert!(listed.contains(saved));
    }
    assert_eq!(
        serde_json::to_value(&listed[0])?["price_cents"],
        serde_json::Value::Null
    );
    let json = serde_json::json!({"name": "From JSON", "price_cents": 1250});
    assert_eq!(
        serde_json::from_value::<InventoryDraft>(json)?.price_cents,
        Some(1250)
    );
    assert_eq!(
        serde_json::from_value::<InventoryDraft>(serde_json::json!({"name": "Old"}))?.price_cents,
        None
    );
    Ok(())
}

#[test]
fn price_is_validated_before_anything_is_written() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    for price in [-1, 1_000_000_000_001, i64::MAX, i64::MIN] {
        let draft = priced("Bad price", "Other", Some(price));
        assert!(
            lib.save_inventory(None, &draft).is_err(),
            "Accepted {price}"
        );
        let error = lib
            .save_inventory_batch(&[draft], &tiny_png())
            .unwrap_err()
            .to_string();
        assert!(error.starts_with("Item 1: Enter a price"), "{error}");
    }
    assert!(lib.list_inventory()?.is_empty());
    Ok(())
}

#[test]
fn a_library_without_the_price_column_migrates() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    let item = lib.save_inventory(None, &priced("Old row", "Other", None))?;
    drop(lib);
    let raw = rusqlite::Connection::open(root.path().join("library.sqlite"))?;
    raw.execute("ALTER TABLE workshop_inventory DROP COLUMN price_cents", [])?;
    raw.execute("DROP TABLE workshop_categories", [])?;
    drop(raw);
    let reopened = Library::open(root.path())?;
    assert_eq!(reopened.list_inventory()?, vec![item.clone()]);
    let priced_item =
        reopened.save_inventory(Some(&item.id), &priced("Old row", "Other", Some(500)))?;
    assert_eq!(reopened.list_inventory()?, vec![priced_item]);
    reopened.create_category("Fresh")?;
    drop(reopened);
    let check = rusqlite::Connection::open(root.path().join("library.sqlite"))?;
    // The column keeps its check, so a bad price cannot be stored by any writer.
    assert!(check
        .execute("UPDATE workshop_inventory SET price_cents = -5", [])
        .is_err());
    Ok(())
}

fn names(categories: &[CategoryInfo]) -> Vec<(&str, i64)> {
    categories
        .iter()
        .map(|c| (c.name.as_str(), c.count))
        .collect()
}

#[test]
fn categories_list_counts_and_keep_empty_categories() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    assert!(lib.list_categories()?.is_empty());
    lib.save_inventory(None, &priced("A", "bolts", None))?;
    lib.save_inventory(None, &priced("B", "Bolts", None))?;
    lib.save_inventory(None, &priced("C", "Bolts", None))?;
    lib.save_inventory(None, &priced("D", "Electronics", None))?;
    lib.create_category("Adhesives")?;
    let listed = lib.list_categories()?;
    assert_eq!(
        names(&listed),
        vec![("Adhesives", 0), ("Bolts", 3), ("Electronics", 1)]
    );
    assert!(listed.iter().all(|c| c.columns.is_none()));
    lib.set_category_columns("electronics", columns(&["brand", "quantity"]))?;
    let listed = lib.list_categories()?;
    let electronics = listed.iter().find(|c| c.name == "Electronics").unwrap();
    assert_eq!(electronics.columns, columns(&["brand", "quantity"]));
    assert_eq!(listed.len(), 3);
    // Setting the columns of an unknown name makes the category.
    lib.set_category_columns("Resin", columns(&[]))?;
    let listed = lib.list_categories()?;
    let resin = listed.iter().find(|c| c.name == "Resin").unwrap();
    assert_eq!((resin.count, resin.columns.clone()), (0, columns(&[])));
    lib.set_category_columns("RESIN", None)?;
    assert_eq!(
        lib.list_categories()?
            .iter()
            .find(|c| c.name == "Resin")
            .unwrap()
            .columns,
        None
    );
    Ok(())
}

#[test]
fn creating_a_category_checks_names_and_duplicates() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    lib.save_inventory(None, &priced("Spool", "Filament", None))?;
    lib.create_category("  Tools  ")?;
    for name in ["tools", "TOOLS", "Tools", "filament", "Filament"] {
        assert!(lib.create_category(name).is_err(), "Accepted {name}");
    }
    for name in ["", "   ", "Bad\nname", "Bad\0name", &"x".repeat(121)] {
        assert!(lib.create_category(name).is_err(), "Accepted {name:?}");
    }
    assert!(lib.create_category(&"x".repeat(120)).is_ok());
    assert_eq!(lib.list_categories()?.len(), 3);
    Ok(())
}

#[test]
fn renaming_a_category_updates_its_items() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    let a = lib.save_inventory(None, &priced("A", "Screws", None))?;
    lib.save_inventory(None, &priced("B", "screws", None))?;
    let other = lib.save_inventory(None, &priced("C", "Other", None))?;
    lib.set_category_columns("Screws", columns(&["dimensions", "quantity"]))?;
    assert_eq!(lib.rename_category("Screws", "Fasteners")?, 2);
    let items = lib.list_inventory()?;
    assert_eq!(
        items
            .iter()
            .filter(|i| i.draft.category == "Fasteners")
            .count(),
        2
    );
    assert!(items.contains(&other));
    assert_eq!(items.iter().find(|i| i.id == a.id).unwrap().draft.name, "A");
    let listed = lib.list_categories()?;
    assert_eq!(names(&listed), vec![("Fasteners", 2), ("Other", 1)]);
    assert_eq!(listed[0].columns, columns(&["dimensions", "quantity"]));
    assert!(lib.rename_category("Missing", "Anything").is_err());
    assert!(lib.rename_category("Fasteners", "  ").is_err());
    // A category made without items moves along with its columns.
    lib.create_category("Empty")?;
    lib.set_category_columns("Empty", columns(&["notes"]))?;
    assert_eq!(lib.rename_category("Empty", "Still empty")?, 0);
    let listed = lib.list_categories()?;
    let moved = listed.iter().find(|c| c.name == "Still empty").unwrap();
    assert_eq!(moved.columns, columns(&["notes"]));
    assert!(listed.iter().all(|c| c.name != "Empty"));
    Ok(())
}

#[test]
fn merging_keeps_the_columns_of_the_target() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    lib.save_inventory(None, &priced("A", "bolts", None))?;
    lib.save_inventory(None, &priced("B", "Bolts", None))?;
    lib.save_inventory(None, &priced("C", "Hex bolts", None))?;
    lib.set_category_columns("Bolts", columns(&["brand"]))?;
    lib.set_category_columns("Hex bolts", columns(&["dimensions"]))?;
    // The target spelling wins, in the items and in the list.
    assert_eq!(lib.rename_category("Hex bolts", "BOLTS")?, 1);
    let listed = lib.list_categories()?;
    assert_eq!(names(&listed), vec![("Bolts", 3)]);
    assert_eq!(listed[0].columns, columns(&["brand"]));
    assert!(lib
        .list_inventory()?
        .iter()
        .all(|i| i.draft.category == "Bolts"));

    // Without a row on the target, the columns of the source move over.
    lib.save_inventory(None, &priced("D", "Studs", None))?;
    lib.save_inventory(None, &priced("E", "Pins", None))?;
    lib.set_category_columns("Pins", columns(&["part_number", "quantity"]))?;
    assert_eq!(lib.rename_category("Pins", "Studs")?, 1);
    let listed = lib.list_categories()?;
    let studs = listed.iter().find(|c| c.name == "Studs").unwrap();
    assert_eq!(
        (studs.count, studs.columns.clone()),
        (2, columns(&["part_number", "quantity"]))
    );
    assert!(listed.iter().all(|c| c.name != "Pins"));

    // A target on Auto stays on Auto: its row wins even when the source had columns.
    lib.save_inventory(None, &priced("F", "Rods", None))?;
    lib.save_inventory(None, &priced("G", "Tubes", None))?;
    lib.set_category_columns("Rods", None)?;
    lib.set_category_columns("Tubes", columns(&["colour"]))?;
    assert_eq!(lib.rename_category("Tubes", "Rods")?, 1);
    let listed = lib.list_categories()?;
    let rods = listed.iter().find(|c| c.name == "Rods").unwrap();
    assert_eq!((rods.count, rods.columns.clone()), (2, None));
    Ok(())
}

#[test]
fn renaming_to_the_same_name_in_other_letters_only_changes_the_letters() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    lib.save_inventory(None, &priced("A", "bolts", None))?;
    lib.save_inventory(None, &priced("B", "BOLTS", None))?;
    lib.set_category_columns("bolts", columns(&["brand"]))?;
    assert_eq!(lib.rename_category("bolts", "Bolts")?, 2);
    let listed = lib.list_categories()?;
    assert_eq!(names(&listed), vec![("Bolts", 2)]);
    assert_eq!(listed[0].columns, columns(&["brand"]));
    assert!(lib
        .list_inventory()?
        .iter()
        .all(|i| i.draft.category == "Bolts"));
    // The same spelling again changes nothing and is not an error.
    assert_eq!(lib.rename_category("Bolts", "Bolts")?, 0);
    assert_eq!(lib.list_categories()?, listed);
    Ok(())
}

#[test]
fn only_an_empty_category_can_be_deleted() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    let item = lib.save_inventory(None, &priced("Spool", "Filament", None))?;
    lib.set_category_columns("Filament", columns(&["colour"]))?;
    lib.create_category("Spare")?;
    assert!(lib.delete_category("filament").is_err());
    assert_eq!(lib.list_inventory()?, vec![item.clone()]);
    assert_eq!(
        names(&lib.list_categories()?),
        vec![("Filament", 1), ("Spare", 0)]
    );
    lib.delete_category("spare")?;
    assert!(lib.delete_category("Spare").is_err());
    assert!(lib.delete_category("Never existed").is_err());
    lib.delete_inventory(&item.id)?;
    // The row of Filament stays after its last item goes, so the category can be deleted.
    assert_eq!(names(&lib.list_categories()?), vec![("Filament", 0)]);
    lib.delete_category("Filament")?;
    assert!(lib.list_categories()?.is_empty());
    Ok(())
}

#[test]
fn column_choices_are_validated() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    lib.create_category("Bolts")?;
    for bad in [
        vec!["nonsense"],
        vec!["brand", "brand"],
        vec!["name"],
        vec!["name", "brand"],
        vec!["Brand"],
        vec![""],
    ] {
        assert!(
            lib.set_category_columns("Bolts", columns(&bad)).is_err(),
            "Accepted {bad:?}"
        );
        assert!(
            lib.set_all_columns(columns(&bad)).is_err(),
            "Accepted {bad:?}"
        );
    }
    assert!(lib.set_category_columns("", columns(&["brand"])).is_err());
    assert_eq!(lib.list_categories()?[0].columns, None);
    // Every documented column is accepted together, in any order.
    let all: Vec<&str> = COLUMN_IDS.iter().rev().copied().collect();
    lib.set_category_columns("Bolts", columns(&all))?;
    assert_eq!(lib.list_categories()?[0].columns, columns(&all));
    assert!(!COLUMN_IDS.contains(&"name"));
    Ok(())
}

#[test]
fn the_all_view_columns_and_currency_persist() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    let defaults = lib.view_settings()?;
    assert_eq!(
        (defaults.currency.as_str(), defaults.all_columns),
        ("CAD", None)
    );
    lib.set_all_columns(columns(&["category", "price", "stock_value"]))?;
    lib.set_currency("USD")?;
    drop(lib);
    let reopened = Library::open(root.path())?;
    let saved = reopened.view_settings()?;
    assert_eq!(saved.currency, "USD");
    assert_eq!(
        saved.all_columns,
        columns(&["category", "price", "stock_value"])
    );
    reopened.set_all_columns(columns(&[]))?;
    assert_eq!(reopened.view_settings()?.all_columns, columns(&[]));
    reopened.set_all_columns(None)?;
    assert_eq!(reopened.view_settings()?.all_columns, None);
    // Other settings still save with these keys present.
    reopened.save_settings(&reopened.settings()?)?;
    assert_eq!(reopened.view_settings()?.currency, "USD");
    Ok(())
}

#[test]
fn currency_must_be_three_capital_letters() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    lib.set_currency("CAD")?;
    for bad in ["cad1", "us", "usd", "Usd", "", "USDX", "U D", "€€€", "ÀÀÀ"] {
        assert!(lib.set_currency(bad).is_err(), "Accepted {bad:?}");
    }
    assert_eq!(lib.view_settings()?.currency, "CAD");
    lib.set_currency("EUR")?;
    assert_eq!(lib.view_settings()?.currency, "EUR");
    Ok(())
}

#[test]
fn the_quantity_changes_alone() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    let item = lib.save_inventory(None, &priced("Spool", "Filament", Some(2000)))?;
    let more = lib.set_inventory_quantity(&item.id, 7)?;
    assert_eq!(more.draft.quantity, Some(7));
    assert_eq!(
        more.draft,
        InventoryDraft {
            quantity: Some(7),
            ..item.draft.clone()
        }
    );
    assert_eq!(lib.list_inventory()?, vec![more.clone()]);
    assert_eq!(
        lib.set_inventory_quantity(&item.id, 0)?.draft.quantity,
        Some(0)
    );
    assert_eq!(
        lib.set_inventory_quantity(&item.id, 9_007_199_254_740_991)?
            .draft
            .quantity,
        Some(9_007_199_254_740_991)
    );
    for bad in [-1, 9_007_199_254_740_992, i64::MIN] {
        assert!(lib.set_inventory_quantity(&item.id, bad).is_err());
    }
    assert_eq!(
        lib.list_inventory()?[0].draft.quantity,
        Some(9_007_199_254_740_991)
    );
    let error = lib.set_inventory_quantity("no-such-id", 1).unwrap_err();
    assert_eq!(error.to_string(), "This inventory item no longer exists.");
    Ok(())
}

fn spool(unit: &str, full: Option<i64>, left: Option<i64>, quantity: Option<i64>) -> InventoryDraft {
    InventoryDraft {
        name: "PETG spool".into(),
        category: "Filament".into(),
        quantity,
        measure_unit: unit.into(),
        measure_full: full,
        measure_left: left,
        ..Default::default()
    }
}

#[test]
fn the_measured_pack_rules_are_checked_before_anything_is_written() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    let refused = [
        // unit outside the allowed list
        ("kg", Some(1), None, Some(1), "Choose g, mL or m"),
        ("G", Some(1000), None, Some(1), "Choose g, mL or m"),
        ("ml", Some(1000), None, Some(1), "Choose g, mL or m"),
        (" g", Some(1000), None, Some(1), "Choose g, mL or m"),
        // an amount without a unit
        ("", Some(1000), None, Some(1), "Choose a unit"),
        ("", None, Some(5), Some(1), "Choose a unit"),
        ("", Some(1000), Some(5), Some(1), "Choose a unit"),
        // an amount left without a full pack
        ("g", None, Some(5), Some(1), "full pack amount before the amount left"),
        // more left than a full pack
        ("g", Some(1000), Some(1001), Some(1), "cannot be more than the full pack"),
        // out of range
        ("g", Some(0), None, Some(1), "from 1 to"),
        ("g", Some(-3), None, Some(1), "from 1 to"),
        ("g", Some(1 << 53), None, Some(1), "from 1 to"),
        ("g", Some(1000), Some(-1), Some(1), "from 0 to"),
        // a draft that would have its amount cleared by a zero count is still checked
        ("g", Some(1000), Some(2000), Some(0), "cannot be more than the full pack"),
    ];
    for (unit, full, left, quantity, message) in refused {
        let draft = spool(unit, full, left, quantity);
        let error = lib.save_inventory(None, &draft).unwrap_err().to_string();
        assert!(error.contains(message), "{unit:?} {full:?} {left:?}: {error}");
        let batch = lib.save_inventory_batch(&[draft], &tiny_png()).unwrap_err().to_string();
        assert!(batch.starts_with("Item 1: "), "{batch}");
    }
    assert!(lib.list_inventory()?.is_empty());

    for (unit, full, left, quantity) in [
        ("", None, None, Some(3)),
        ("g", Some(1000), None, Some(1)),
        ("g", Some(1000), Some(0), Some(1)),
        ("g", Some(1000), Some(1000), Some(1)),
        ("mL", Some(500), Some(120), Some(4)),
        ("m", Some(1), Some(1), None),
        ("g", None, None, Some(1)),
    ] {
        let saved = lib.save_inventory(None, &spool(unit, full, left, quantity))?;
        let draft = &saved.draft;
        assert_eq!(
            (draft.measure_unit.as_str(), draft.measure_full, draft.measure_left),
            (unit, full, left)
        );
    }
    Ok(())
}

#[test]
fn a_zero_count_clears_the_open_amount_and_an_unknown_count_keeps_it() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    let none = lib.save_inventory(None, &spool("g", Some(1000), Some(300), Some(0)))?;
    assert_eq!(none.draft.measure_left, None);
    assert_eq!(none.draft.measure_full, Some(1000));
    assert_eq!(lib.list_inventory()?, vec![none]);
    let unknown = lib.save_inventory(None, &spool("g", Some(1000), Some(300), None))?;
    assert_eq!(unknown.draft.measure_left, Some(300));
    // The stepper clears it at zero, and raising the count again does not bring it back.
    let open = lib.save_inventory(None, &spool("g", Some(1000), Some(300), Some(1)))?;
    assert_eq!(lib.set_inventory_quantity(&open.id, 1)?.draft.measure_left, Some(300));
    let cleared = lib.set_inventory_quantity(&open.id, 0)?;
    assert_eq!((cleared.draft.quantity, cleared.draft.measure_left), (Some(0), None));
    assert_eq!(lib.set_inventory_quantity(&open.id, 3)?.draft.measure_left, None);
    assert_eq!(lib.set_inventory_quantity(&open.id, 3)?.draft.measure_full, Some(1000));
    // A count of zero on an item that never had an open pack changes nothing else.
    let counted = lib.save_inventory(None, &priced("Washer", "Fasteners", Some(5)))?;
    let zero = lib.set_inventory_quantity(&counted.id, 0)?;
    assert_eq!((zero.draft.measure_left, zero.draft.price_cents), (None, Some(5)));
    Ok(())
}

#[test]
fn a_lower_count_clears_the_open_amount_and_other_counts_keep_it() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    let left_after = |from: Option<i64>, to: i64| -> Result<Option<i64>> {
        let item = lib.save_inventory(None, &spool("g", Some(1000), Some(100), from))?;
        Ok(lib.set_inventory_quantity(&item.id, to)?.draft.measure_left)
    };
    assert_eq!(left_after(Some(2), 1)?, None);
    assert_eq!(left_after(Some(1), 2)?, Some(100));
    assert_eq!(left_after(Some(2), 2)?, Some(100));
    assert_eq!(left_after(None, 1)?, Some(100));
    assert_eq!(left_after(Some(1), 0)?, None);
    // The cleared pack leaves full packs: 1 spool of 1000 g, not 100 g.
    let item = lib.save_inventory(None, &spool("g", Some(1000), Some(100), Some(2)))?;
    let stepped = lib.set_inventory_quantity(&item.id, 1)?;
    assert_eq!((stepped.draft.quantity, stepped.draft.measure_left, stepped.draft.measure_full), (Some(1), None, Some(1000)));
    // The editor's full save keeps whatever the designer typed, also with a lower count.
    let kept = lib.save_inventory(Some(&item.id), &spool("g", Some(1000), Some(100), Some(1)))?;
    assert_eq!(kept.draft.measure_left, Some(100));
    Ok(())
}

#[test]
fn the_measured_pack_round_trips_through_save_list_and_batch() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    let item = lib.save_inventory(None, &spool("g", Some(1000), Some(100), Some(1)))?;
    assert_eq!(lib.list_inventory()?, vec![item.clone()]);
    let json = serde_json::to_value(&item)?;
    assert_eq!(
        (&json["measure_unit"], &json["measure_full"], &json["measure_left"]),
        (&serde_json::json!("g"), &serde_json::json!(1000), &serde_json::json!(100))
    );
    let revised = lib.save_inventory(Some(&item.id), &spool("mL", Some(750), None, Some(2)))?;
    assert_eq!(lib.list_inventory()?, vec![revised.clone()]);
    assert_eq!(revised.draft.measure_left, None);
    let batch = lib.save_inventory_batch(
        &[
            spool("g", Some(1000), Some(250), Some(3)),
            priced("Nut", "Fasteners", None),
        ],
        &tiny_png(),
    )?;
    assert_eq!(batch[0].draft.measure_left, Some(250));
    assert_eq!(batch[1].draft.measure_unit, "");
    let listed = lib.list_inventory()?;
    for saved in &batch {
        assert!(listed.contains(saved));
    }
    // JSON from an older frontend has no measure fields.
    let old: InventoryDraft = serde_json::from_value(serde_json::json!({"name": "Old"}))?;
    assert_eq!((old.measure_unit.as_str(), old.measure_full, old.measure_left), ("", None, None));
    let new: InventoryDraft = serde_json::from_value(
        serde_json::json!({"name": "New", "measure_unit": "m", "measure_full": 50, "measure_left": 12}),
    )?;
    assert_eq!((new.measure_unit.as_str(), new.measure_full, new.measure_left), ("m", Some(50), Some(12)));
    Ok(())
}

#[test]
fn the_total_amount_follows_the_addendum_formula() {
    let total = |full, left, quantity| spool("g", full, left, quantity).total_amount();
    // One spool with 100 g left, three spools with 250 g left in the opened one.
    assert_eq!(total(Some(1000), Some(100), Some(1)), Some(100));
    assert_eq!(total(Some(1000), Some(250), Some(3)), Some(2250));
    assert_eq!(total(Some(1000), Some(0), Some(2)), Some(1000));
    assert_eq!(total(Some(1000), Some(1000), Some(2)), Some(2000));
    // No open pack recorded: whole packs.
    assert_eq!(total(Some(1000), None, Some(3)), Some(3000));
    assert_eq!(total(Some(1000), None, Some(0)), Some(0));
    // Unknown: no count, no full pack, no unit.
    assert_eq!(total(Some(1000), Some(100), None), None);
    assert_eq!(total(Some(1000), None, None), None);
    assert_eq!(total(None, None, Some(2)), None);
    assert_eq!(spool("", Some(1000), None, Some(2)).total_amount(), None);
    assert_eq!(spool("", None, None, Some(2)).total_amount(), None);
    // A result that JSON numbers cannot hold exactly is unknown, not wrong.
    assert_eq!(total(Some((1 << 53) - 1), None, Some(2)), None);
}

#[test]
fn the_open_amount_is_checked_against_the_item() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    let item = lib.save_inventory(None, &spool("g", Some(1000), None, Some(2)))?;
    let set = lib.set_open_amount(&item.id, Some(400))?;
    assert_eq!(set.draft.measure_left, Some(400));
    assert_eq!(set.draft.quantity, Some(2));
    assert_eq!(lib.list_inventory()?, vec![set.clone()]);
    // The edge values and clearing.
    assert_eq!(lib.set_open_amount(&item.id, Some(0))?.draft.measure_left, Some(0));
    assert_eq!(lib.set_open_amount(&item.id, Some(1000))?.draft.measure_left, Some(1000));
    assert_eq!(lib.set_open_amount(&item.id, None)?.draft.measure_left, None);
    // More than a full pack, or less than nothing, changes nothing.
    lib.set_open_amount(&item.id, Some(10))?;
    for bad in [1001, -1, i64::MAX, i64::MIN] {
        let error = lib.set_open_amount(&item.id, Some(bad)).unwrap_err().to_string();
        assert!(error.contains("from 0 to 1000 g"), "{bad}: {error}");
    }
    assert_eq!(lib.list_inventory()?[0].draft.measure_left, Some(10));
    // No count on hand means no open pack.
    let empty = lib.save_inventory(None, &spool("g", Some(1000), None, Some(0)))?;
    let error = lib.set_open_amount(&empty.id, Some(5)).unwrap_err().to_string();
    assert!(error.contains("none on hand"), "{error}");
    // An unknown count is allowed.
    let unknown = lib.save_inventory(None, &spool("g", Some(1000), None, None))?;
    assert_eq!(lib.set_open_amount(&unknown.id, Some(5))?.draft.measure_left, Some(5));
    // An item with no measured pack, or only a unit, cannot take an amount, but can be cleared.
    let counted = lib.save_inventory(None, &priced("Washer", "Fasteners", None))?;
    let error = lib.set_open_amount(&counted.id, Some(5)).unwrap_err().to_string();
    assert!(error.contains("full pack amount"), "{error}");
    let unit_only = lib.save_inventory(None, &spool("g", None, None, Some(1)))?;
    assert!(lib.set_open_amount(&unit_only.id, Some(5)).is_err());
    assert_eq!(lib.set_open_amount(&counted.id, None)?.draft.measure_left, None);
    // A missing item is an error.
    assert!(lib.set_open_amount("missing", Some(1)).is_err());
    assert!(lib.set_open_amount("missing", None).is_err());
    Ok(())
}

#[test]
fn a_library_without_the_measure_columns_migrates() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    let item = lib.save_inventory(None, &priced("Old spool", "Filament", Some(2000)))?;
    drop(lib);
    let raw = rusqlite::Connection::open(root.path().join("library.sqlite"))?;
    for column in ["measure_unit", "measure_full", "measure_left"] {
        raw.execute(&format!("ALTER TABLE workshop_inventory DROP COLUMN {column}"), [])?;
    }
    drop(raw);
    let reopened = Library::open(root.path())?;
    assert_eq!(reopened.list_inventory()?, vec![item.clone()]);
    assert_eq!(item.draft.measure_unit, "");
    let measured =
        reopened.save_inventory(Some(&item.id), &spool("g", Some(1000), Some(100), Some(1)))?;
    assert_eq!(reopened.list_inventory()?, vec![measured]);
    drop(reopened);
    // The columns keep their checks, so no writer can store a bad amount.
    let check = rusqlite::Connection::open(root.path().join("library.sqlite"))?;
    assert!(check.execute("UPDATE workshop_inventory SET measure_full = 0", []).is_err());
    assert!(check.execute("UPDATE workshop_inventory SET measure_left = -1", []).is_err());
    // An older version writes rows without the new columns, and they take their defaults.
    check.execute(
        "INSERT INTO workshop_inventory (id,name,category,brand,part_number,dimensions,notes,supplier_title,supplier_details)
         VALUES ('legacy','Legacy','Other','','','','','','')",
        [],
    )?;
    drop(check);
    let again = Library::open(root.path())?;
    let legacy = again.list_inventory()?.into_iter().find(|i| i.id == "legacy").unwrap();
    assert_eq!(
        (legacy.draft.measure_unit.as_str(), legacy.draft.measure_full, legacy.draft.measure_left),
        ("", None, None)
    );
    Ok(())
}

#[test]
fn the_column_ids_match_the_typescript_contract() {
    let source = include_str!("../../../src/inventoryApi.ts");
    let start = source.find("export const columnIds = [").expect("columnIds");
    let list = &source[start..start + source[start..].find("] as const").unwrap()];
    let ids: Vec<&str> = list
        .split('\'')
        .skip(1)
        .step_by(2)
        .collect();
    assert_eq!(ids, COLUMN_IDS);
    let after_pack = COLUMN_IDS.iter().position(|id| *id == "pack_quantity").unwrap() + 1;
    assert_eq!(COLUMN_IDS[after_pack], "open_amount");
}

#[test]
fn the_measure_units_match_the_typescript_contract() {
    let source = include_str!("../../../src/inventoryApi.ts");
    let start = source.find("export const measureUnits = [").expect("measureUnits");
    let list = &source[start..start + source[start..].find("] as const").unwrap()];
    let units: Vec<&str> = list.split('\'').skip(1).step_by(2).collect();
    // The TypeScript list starts with the empty string for "not measured"; the Rust constant holds the real units.
    assert_eq!(units.first(), Some(&""));
    assert_eq!(&units[1..], MEASURE_UNITS);
}
