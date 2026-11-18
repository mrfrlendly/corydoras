use anyhow::Result;
use corydoras_core::{
    inventory::{InventoryDraft, SupplierListing},
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
        notes: "Bin B7".into(),
        specs: vec![],
        supplier: Some(SupplierListing {
            title: "A2-816 socket head screws".into(),
            url: "https://supplier.example.com/parts/a2-816".into(),
            details: "Stainless steel".into(),
        }),
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
    let pitch = saved.draft.specs.iter().find(|s| s.role == SpecRole::Pitch).unwrap();
    assert_eq!((pitch.value.as_str(), pitch.source), ("0.7", SpecSource::Standard));
    assert_eq!(saved.draft.dimensions, "M4 thread; pitch 0.7 mm");
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
