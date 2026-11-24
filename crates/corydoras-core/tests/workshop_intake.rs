//! Intake of the workshop inventory: price source, category keywords, keyword rules, review and apply.
use anyhow::Result;
use corydoras_core::{
    inventory::{InventoryDraft, SupplierListing, MAX_PRICE_CENTS},
    inventory_intake::{FieldInput, SuggestionChange},
    Library,
};
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use std::io::Cursor;

fn png() -> Vec<u8> {
    let img = DynamicImage::ImageRgb8(RgbImage::from_pixel(64, 32, Rgb([9, 50, 80])));
    let mut w = Cursor::new(Vec::new());
    img.write_to(&mut w, ImageFormat::Png).unwrap();
    w.into_inner()
}

fn draft(name: &str, category: &str) -> InventoryDraft {
    InventoryDraft {
        name: name.into(),
        category: category.into(),
        ..Default::default()
    }
}

fn estimated(price: Option<i64>, source: &str) -> InventoryDraft {
    InventoryDraft {
        price_cents: price,
        price_source: source.into(),
        ..draft("Spool", "Filament")
    }
}

/// A library with the categories of a small workshop.
fn shop() -> Result<(tempfile::TempDir, Library)> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    for name in ["Filament", "Adhesives", "Nuts", "Tools"] {
        lib.create_category(name)?;
    }
    Ok((root, lib))
}

fn category_of(lib: &Library, id: &str) -> Result<String> {
    Ok(lib
        .list_inventory()?
        .into_iter()
        .find(|i| i.id == id)
        .unwrap()
        .draft
        .category)
}

#[test]
fn the_price_source_is_validated_and_kept_only_with_a_price() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    let ok = lib.save_inventory(None, &estimated(Some(2499), " listing:www.canadabolts.ca "))?;
    assert_eq!(ok.draft.price_source, "listing:www.canadabolts.ca");
    assert_eq!(
        lib.list_inventory()?[0].draft.price_source,
        "listing:www.canadabolts.ca"
    );
    assert_eq!(
        serde_json::to_value(&ok)?["price_source"],
        "listing:www.canadabolts.ca"
    );
    // A price the designer typed has no source, and an estimate without a price is not one.
    assert_eq!(
        lib.save_inventory(None, &estimated(Some(100), ""))?
            .draft
            .price_source,
        ""
    );
    assert_eq!(
        lib.save_inventory(None, &estimated(None, "listing:www.canadabolts.ca"))?
            .draft
            .price_source,
        ""
    );
    let long = format!("listing:{}", "a".repeat(120 - "listing:".len() + 1));
    let at_limit = format!("listing:{}", "a".repeat(120 - "listing:".len()));
    assert!(lib
        .save_inventory(None, &estimated(Some(1), &at_limit))
        .is_ok());
    for bad in [
        "manual",
        "Listing:x",
        "listing:x\ny",
        "listing:\u{7}",
        long.as_str(),
    ] {
        assert!(
            lib.save_inventory(None, &estimated(Some(1), bad)).is_err(),
            "{bad:?}"
        );
    }
    // A bad source is refused even without a price.
    assert!(lib
        .save_inventory(None, &estimated(None, "manual"))
        .is_err());
    // An old client sends no price_source at all.
    let old: InventoryDraft = serde_json::from_str(r#"{"name":"Old","price_cents":5}"#)?;
    assert_eq!(old.price_source, "");
    assert_eq!(lib.save_inventory(None, &old)?.draft.price_source, "");
    // The estimate survives a reopen, and an edit that sends "" clears it.
    let id = ok.id.clone();
    drop(lib);
    let lib = Library::open(root.path())?;
    let kept = lib
        .list_inventory()?
        .into_iter()
        .find(|i| i.id == id)
        .unwrap();
    assert_eq!(kept.draft.price_source, "listing:www.canadabolts.ca");
    let cleared = lib.save_inventory(Some(&id), &estimated(Some(2000), ""))?;
    assert_eq!(cleared.draft.price_source, "");
    Ok(())
}

#[test]
fn a_listing_price_is_checked_but_never_stored() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    let listing = |price, currency: &str| InventoryDraft {
        supplier: Some(SupplierListing {
            title: "Spool".into(),
            url: "https://supplier.example.com/p".into(),
            details: String::new(),
            price_cents: price,
            currency: currency.into(),
        }),
        ..draft("Spool", "Filament")
    };
    let saved = lib.save_inventory(None, &listing(Some(2999), "CAD"))?;
    let supplier = saved.draft.supplier.clone().unwrap();
    assert_eq!(
        (supplier.price_cents, supplier.currency.as_str()),
        (None, "")
    );
    assert_eq!(lib.list_inventory()?, vec![saved.clone()]);
    assert_eq!(saved.draft.price_cents, None);
    for (price, currency) in [
        (Some(-1), "CAD"),
        (Some(MAX_PRICE_CENTS + 1), "CAD"),
        (Some(1), "cad"),
        (Some(1), "CA"),
        (Some(1), "CADD"),
        (None, "C4D"),
    ] {
        assert!(
            lib.save_inventory(None, &listing(price, currency)).is_err(),
            "{price:?} {currency}"
        );
    }
    assert!(lib
        .save_inventory(None, &listing(Some(MAX_PRICE_CENTS), ""))
        .is_ok());
    assert!(lib.save_inventory(None, &listing(None, "GBP")).is_ok());
    // A listing from an older frontend has neither field.
    let old: SupplierListing =
        serde_json::from_str(r#"{"title":"T","url":"https://a.example.com/","details":""}"#)?;
    assert_eq!((old.price_cents, old.currency.as_str()), (None, ""));
    Ok(())
}

#[test]
fn a_library_without_the_new_columns_migrates() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    let item = lib.save_inventory(None, &estimated(Some(500), "listing:a.example.com"))?;
    lib.set_category_keywords("Filament", vec!["spool".into()])?;
    drop(lib);
    let raw = rusqlite::Connection::open(root.path().join("library.sqlite"))?;
    raw.execute(
        "ALTER TABLE workshop_inventory DROP COLUMN price_source",
        [],
    )?;
    raw.execute("ALTER TABLE workshop_categories DROP COLUMN keywords", [])?;
    drop(raw);
    let lib = Library::open(root.path())?;
    let listed = lib.list_inventory()?;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].draft.price_cents, Some(500));
    assert_eq!(listed[0].draft.price_source, "");
    assert!(lib.list_categories()?[0].keywords.is_empty());
    lib.set_category_keywords("Filament", vec!["spool".into()])?;
    let edited = lib.save_inventory(
        Some(&item.id),
        &estimated(Some(500), "listing:a.example.com"),
    )?;
    assert_eq!(edited.draft.price_source, "listing:a.example.com");
    assert_eq!(lib.list_categories()?[0].keywords, ["spool"]);
    // Opening again is harmless.
    drop(lib);
    assert_eq!(Library::open(root.path())?.list_inventory()?.len(), 1);
    Ok(())
}

#[test]
fn category_keywords_are_validated_trimmed_and_listed() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    // The row is made when absent, and an empty list means none.
    lib.set_category_keywords("Hinges", vec!["  piano hinge ".into(), "Scharnier".into()])?;
    let listed = lib.list_categories()?;
    assert_eq!(
        (listed[0].name.as_str(), listed[0].keywords.clone()),
        (
            "Hinges",
            vec!["piano hinge".to_owned(), "Scharnier".to_owned()]
        )
    );
    assert_eq!(
        serde_json::to_value(&listed[0])?["keywords"],
        serde_json::json!(["piano hinge", "Scharnier"])
    );
    // The spelling of the stored category wins.
    lib.set_category_keywords("hinges", vec!["Hinge".into()])?;
    assert_eq!(lib.list_categories()?.len(), 1);
    assert_eq!(lib.list_categories()?[0].keywords, ["Hinge"]);
    lib.set_category_keywords("Hinges", vec![])?;
    assert!(lib.list_categories()?[0].keywords.is_empty());
    // Setting columns does not touch the keywords, and the other way round.
    lib.set_category_keywords("Hinges", vec!["x1".into()])?;
    lib.set_category_columns("Hinges", Some(vec!["brand".into()]))?;
    lib.set_category_keywords("Hinges", vec!["x2".into()])?;
    let one = &lib.list_categories()?[0];
    assert_eq!(
        (one.keywords.clone(), one.columns.clone()),
        (vec!["x2".to_owned()], Some(vec!["brand".to_owned()]))
    );
    // The validation matrix.
    let many: Vec<String> = (0..21).map(|i| format!("word{i}")).collect();
    let twenty: Vec<String> = (0..20).map(|i| format!("word{i}")).collect();
    assert!(lib.set_category_keywords("Hinges", many).is_err());
    assert!(lib.set_category_keywords("Hinges", twenty).is_ok());
    assert!(lib
        .set_category_keywords("Hinges", vec!["a".repeat(41)])
        .is_err());
    assert!(lib
        .set_category_keywords("Hinges", vec!["a".repeat(40)])
        .is_ok());
    for bad in [
        vec!["".to_owned()],
        vec!["   ".into()],
        vec!["a\nb".into()],
        vec!["a".into(), "A".into()],
        vec!["Äb".into(), "äb".into()],
    ] {
        assert!(
            lib.set_category_keywords("Hinges", bad.clone()).is_err(),
            "{bad:?}"
        );
    }
    assert!(lib.set_category_keywords("", vec!["a".into()]).is_err());
    // A failed call changes nothing.
    assert_eq!(lib.list_categories()?[0].keywords, ["a".repeat(40)]);
    // Deleting the category deletes its keywords.
    lib.delete_category("Hinges")?;
    assert!(lib.list_categories()?.is_empty());
    Ok(())
}

#[test]
fn merge_and_rename_carry_the_keywords() -> Result<()> {
    let root = tempfile::tempdir()?;
    let lib = Library::open(root.path())?;
    let keywords = |lib: &Library, name: &str| -> Vec<String> {
        lib.list_categories()
            .unwrap()
            .into_iter()
            .find(|c| c.name == name)
            .unwrap()
            .keywords
    };
    // A plain rename keeps them.
    lib.create_category("Glues")?;
    lib.set_category_keywords("Glues", vec!["sticky".into()])?;
    lib.rename_category("Glues", "Adhesives")?;
    assert_eq!(keywords(&lib, "Adhesives"), ["sticky"]);
    // A merge into a target that has keywords keeps the target's.
    lib.set_category_keywords("Cement", vec!["mortar".into()])?;
    lib.rename_category("Cement", "adhesives")?;
    assert_eq!(keywords(&lib, "Adhesives"), ["sticky"]);
    assert!(lib.list_categories()?.iter().all(|c| c.name != "Cement"));
    // A merge into a target without keywords takes the source's, and keeps the target's columns.
    lib.create_category("Bolts")?;
    lib.set_category_columns("Bolts", Some(vec!["brand".into()]))?;
    lib.set_category_keywords("Screws", vec!["wood".into(), "Machine".into()])?;
    lib.set_category_columns("Screws", Some(vec!["notes".into()]))?;
    lib.rename_category("Screws", "Bolts")?;
    let bolts = lib
        .list_categories()?
        .into_iter()
        .find(|c| c.name == "Bolts")
        .unwrap();
    assert_eq!(bolts.keywords, ["wood", "Machine"]);
    assert_eq!(bolts.columns, Some(vec!["brand".to_owned()]));
    // A merge into a target that has no row at all takes the source's row.
    lib.save_inventory(None, &draft("Pin", "Pins"))?;
    lib.set_category_keywords("Dowels", vec!["wooden".into()])?;
    lib.rename_category("Dowels", "Pins")?;
    assert_eq!(keywords(&lib, "Pins"), ["wooden"]);
    // A merge of a source without keywords leaves the target's.
    lib.create_category("Empty")?;
    lib.rename_category("Empty", "Pins")?;
    assert_eq!(keywords(&lib, "Pins"), ["wooden"]);
    Ok(())
}

#[test]
fn suggestions_come_from_this_librarys_categories_and_keywords() -> Result<()> {
    let (_root, lib) = shop()?;
    let input = |name: &str, category: &str| FieldInput {
        name: name.into(),
        category: category.into(),
        ..Default::default()
    };
    let found = lib.suggest_fields(&input("Overture PETG Filament Purple", "Materials"))?;
    assert_eq!(
        found.category.as_ref().map(|c| c.name.as_str()),
        Some("Filament")
    );
    assert_eq!(found.colour.as_deref(), Some("Purple"));
    // A category the library does not have is not suggested.
    assert!(lib
        .suggest_fields(&input("Neodymium magnet", "Other"))?
        .category
        .is_none());
    lib.create_category("Magnets")?;
    assert_eq!(
        lib.suggest_fields(&input("Neodymium magnet", "Other"))?
            .category
            .unwrap()
            .name,
        "Magnets"
    );
    // A designer keyword makes a new rule, and it is read straight away.
    assert!(lib
        .suggest_fields(&input("Flux pen", "Other"))?
        .category
        .is_none());
    lib.set_category_keywords("Tools", vec!["flux".into()])?;
    assert_eq!(
        lib.suggest_fields(&input("Flux pen", ""))?
            .category
            .unwrap()
            .name,
        "Tools"
    );
    // A colour the item already has stays, and so does a category that is not generic.
    let kept = lib.suggest_fields(&FieldInput {
        colour: "Teal".into(),
        ..input("Black glue", "Tools")
    })?;
    assert!(kept.category.is_none() && kept.colour.is_none());
    Ok(())
}

#[test]
fn review_lists_only_what_a_rule_would_change_and_apply_checks_again() -> Result<()> {
    let (_root, lib) = shop()?;
    let spool = lib.save_inventory(None, &draft("Overture PETG Filament Purple", "Materials"))?;
    let glue = lib.save_inventory(None, &draft("Gorilla Glue Sticks", "Other"))?;
    let placed = lib.save_inventory(None, &draft("Nyloc nut", "Tools"))?;
    let plain = lib.save_inventory(None, &draft("Bin label", "Other"))?;
    let coloured = lib.save_inventory(
        None,
        &InventoryDraft {
            colour: "Teal".into(),
            ..draft("Red PETG", "Filament")
        },
    )?;
    let notes = lib.save_inventory(
        None,
        &InventoryDraft {
            notes: "Colour: Titan grey".into(),
            ..draft("Tool wrap", "Tools")
        },
    )?;
    let review = lib.review_suggestions()?;
    let ids: Vec<_> = review.iter().map(|r| r.id.as_str()).collect();
    for absent in [&placed.id, &plain.id, &coloured.id] {
        assert!(!ids.contains(&absent.as_str()), "{absent}");
    }
    let by = |id: &str| review.iter().find(|r| r.id == id).unwrap();
    let s = by(&spool.id);
    assert_eq!(
        (
            s.category_from.as_str(),
            s.category_to.as_deref(),
            s.category_reason.as_str()
        ),
        ("Materials", Some("Filament"), "name says \"filament\"")
    );
    assert_eq!(
        (s.colour_to.as_deref(), s.colour_reason.as_str()),
        (Some("Purple"), "name says \"Purple\"")
    );
    let g = by(&glue.id);
    assert_eq!(
        (g.category_to.as_deref(), g.colour_to.as_deref()),
        (Some("Adhesives"), None)
    );
    assert_eq!(g.colour_reason, "");
    let n = by(&notes.id);
    assert_eq!(
        (n.category_to.as_deref(), n.colour_to.as_deref()),
        (None, Some("Titan Grey"))
    );
    assert_eq!(n.category_reason, "");
    assert_eq!(review.len(), 3);
    // Reviewing changed nothing.
    assert_eq!(category_of(&lib, &spool.id)?, "Materials");

    // Apply: only what was ticked, one number for the items changed, unknown ids skipped.
    let changed = lib.apply_suggestions(&[
        SuggestionChange {
            id: spool.id.clone(),
            category: Some("Filament".into()),
            colour: Some("Purple".into()),
        },
        SuggestionChange {
            id: glue.id.clone(),
            category: Some("adhesives".into()),
            colour: None,
        },
        SuggestionChange {
            id: "missing".into(),
            category: Some("Filament".into()),
            colour: None,
        },
        SuggestionChange {
            id: notes.id.clone(),
            category: None,
            colour: Some("Titan Grey".into()),
        },
        SuggestionChange {
            id: plain.id.clone(),
            category: None,
            colour: None,
        },
    ])?;
    assert_eq!(changed, 3);
    let listed = lib.list_inventory()?;
    let item = |id: &str| listed.iter().find(|i| i.id == id).unwrap().draft.clone();
    assert_eq!(
        (item(&spool.id).category, item(&spool.id).colour),
        ("Filament".into(), "Purple".into())
    );
    assert_eq!(item(&glue.id).category, "Adhesives");
    assert_eq!(
        (item(&notes.id).category, item(&notes.id).colour),
        ("Tools".into(), "Titan Grey".into())
    );
    assert!(lib.review_suggestions()?.is_empty());
    assert_eq!(lib.apply_suggestions(&[])?, 0);
    Ok(())
}

#[test]
fn review_leaves_out_an_ambiguous_shade_word() -> Result<()> {
    let (_root, lib) = shop()?;
    let steel = lib.save_inventory(None, &draft("Stainless Steel Bolts", "Tools"))?;
    let rose = lib.save_inventory(None, &draft("Rose Gold wire", "Tools"))?;
    let ash = lib.save_inventory(None, &draft("Ash Wood board", "Tools"))?;
    let titan = lib.save_inventory(None, &draft("PETG Titan Grey", "Tools"))?;
    let review = lib.review_suggestions()?;
    assert!(review.iter().all(|r| r.id != steel.id));
    let colour_of = |id: &str| {
        review
            .iter()
            .find(|r| r.id == id)
            .and_then(|r| r.colour_to.clone())
    };
    assert_eq!(colour_of(&rose.id).as_deref(), Some("Rose Gold"));
    assert_eq!(colour_of(&ash.id), None);
    assert_eq!(colour_of(&titan.id).as_deref(), Some("Titan Grey"));
    Ok(())
}

#[test]
fn review_reads_names_brands_and_colour_lines_not_imported_noise() -> Result<()> {
    let (_root, lib) = shop()?;
    // A connector whose notes carry a shop breadcrumb that names two categories must not move.
    let wago = lib.save_inventory(
        None,
        &InventoryDraft {
            notes: "Amazon (similar product): Tools & Home Improvement > Lever Nuts".into(),
            dimensions: "Parent kit: Lever Nuts assortment".into(),
            ..draft("WAGO 221-413 splicing connector", "Other")
        },
    )?;
    // Aqua in notes is a scent, and listing text of other vendors is not this item's colour.
    let polish = lib.save_inventory(
        None,
        &InventoryDraft {
            notes: "Aqua scent, acetone free".into(),
            ..draft("Nail polish remover", "Consumables")
        },
    )?;
    let compartments = lib.save_inventory(
        None,
        &InventoryDraft {
            notes: "Amazon (similar product): Green M12 screw https://example.com/needle\nCatalogue: Black plastic box".into(),
            dimensions: "Red lid".into(),
            ..draft("Compartment organizer", "Other")
        },
    )?;
    // A line that states a colour is read, and so are the name and the brand.
    let stated = lib.save_inventory(
        None,
        &InventoryDraft {
            notes: "Bin B7\nColour: Black".into(),
            ..draft("Compartment lid", "Other")
        },
    )?;
    let branded = lib.save_inventory(
        None,
        &InventoryDraft {
            brand: "Hot Glue Co".into(),
            ..draft("Mystery stick", "Other")
        },
    )?;
    let review = lib.review_suggestions()?;
    for quiet in [&wago.id, &polish.id, &compartments.id] {
        assert!(review.iter().all(|r| &r.id != quiet), "{quiet}");
    }
    let row = |id: &str| review.iter().find(|r| r.id == id).unwrap();
    assert_eq!(row(&stated.id).colour_to.as_deref(), Some("Black"));
    assert_eq!(row(&stated.id).category_to, None);
    assert_eq!(row(&branded.id).category_to.as_deref(), Some("Adhesives"));
    // Apply re-runs the same reading, so a stale tick for the noisy item is refused.
    assert_eq!(
        lib.apply_suggestions(&[SuggestionChange {
            id: wago.id.clone(),
            category: Some("Nuts".into()),
            colour: None
        }])?,
        0
    );
    assert_eq!(category_of(&lib, &wago.id)?, "Other");
    Ok(())
}

#[test]
fn apply_skips_what_changed_since_the_review() -> Result<()> {
    let (_root, lib) = shop()?;
    let a = lib.save_inventory(None, &draft("PETG filament Purple", "Other"))?;
    let b = lib.save_inventory(None, &draft("PETG filament Purple", "Other"))?;
    let c = lib.save_inventory(None, &draft("PETG filament Purple", "Other"))?;
    let reviewed = lib.review_suggestions()?;
    assert_eq!(reviewed.len(), 3);
    let tick = |id: &str, category: Option<&str>, colour: Option<&str>| SuggestionChange {
        id: id.into(),
        category: category.map(Into::into),
        colour: colour.map(Into::into),
    };
    // a: the designer moved it to Tools and typed a colour. b: its name changed, so the rule says
    // something else now. c: untouched.
    lib.save_inventory(
        Some(&a.id),
        &InventoryDraft {
            colour: "Teal".into(),
            ..draft("PETG filament Purple", "Tools")
        },
    )?;
    lib.save_inventory(Some(&b.id), &draft("Superglue", "Other"))?;
    let changed = lib.apply_suggestions(&[
        tick(&a.id, Some("Filament"), Some("Purple")),
        tick(&b.id, Some("Filament"), Some("Purple")),
        tick(&c.id, Some("Filament"), Some("Purple")),
    ])?;
    // b still takes the colour (it is empty) but not the category; c takes both; a takes nothing.
    assert_eq!(changed, 2);
    let listed = lib.list_inventory()?;
    let item = |id: &str| listed.iter().find(|i| i.id == id).unwrap().draft.clone();
    assert_eq!(
        (item(&a.id).category, item(&a.id).colour),
        ("Tools".into(), "Teal".into())
    );
    assert_eq!(
        (item(&b.id).category, item(&b.id).colour),
        ("Other".into(), "Purple".into())
    );
    assert_eq!(
        (item(&c.id).category, item(&c.id).colour),
        ("Filament".into(), "Purple".into())
    );
    // A target the rule never gives is refused, whatever the client sends.
    let d = lib.save_inventory(None, &draft("PETG filament", "Other"))?;
    assert_eq!(
        lib.apply_suggestions(&[tick(&d.id, Some("Tools"), None)])?,
        0
    );
    assert_eq!(
        lib.apply_suggestions(&[tick(&d.id, Some("Brand new"), None)])?,
        0
    );
    assert_eq!(category_of(&lib, &d.id)?, "Other");
    // A colour that breaks the rules of the field is an error, and the transaction is rolled back.
    let e = lib.save_inventory(None, &draft("PETG filament", "Other"))?;
    assert!(lib
        .apply_suggestions(&[tick(&e.id, Some("Filament"), Some("Red\nBlue"))])
        .is_err());
    assert_eq!(category_of(&lib, &e.id)?, "Other");
    // The same id twice counts as one item.
    assert_eq!(
        lib.apply_suggestions(&[
            tick(&e.id, Some("Filament"), None),
            tick(&e.id, None, Some("Red"))
        ])?,
        1
    );
    Ok(())
}

#[test]
fn a_new_item_with_no_category_gets_one_but_an_edit_never_does() -> Result<()> {
    let (_root, lib) = shop()?;
    let item = lib.save_inventory(None, &draft("Overture PETG Filament Purple", ""))?;
    assert_eq!(
        (item.draft.category.as_str(), item.draft.colour.as_str()),
        ("Filament", "")
    );
    assert_eq!(lib.list_inventory()?[0].draft.category, "Filament");
    // Nothing matches: the usual Other.
    assert_eq!(
        lib.save_inventory(None, &draft("Bin label", "  "))?
            .draft
            .category,
        "Other"
    );
    // A typed "Other" is on purpose, and a typed colour stays. A colour is never filled on save.
    let typed = lib.save_inventory(None, &draft("Gorilla Glue Sticks", "Other"))?;
    assert_eq!(typed.draft.category, "Other");
    let own = lib.save_inventory(
        None,
        &InventoryDraft {
            colour: "Teal".into(),
            ..draft("Gorilla Glue Sticks Black", "")
        },
    )?;
    assert_eq!(
        (own.draft.category.as_str(), own.draft.colour.as_str()),
        ("Adhesives", "Teal")
    );
    // An edit of an existing item never changes category or colour, empty or not.
    let edited = lib.save_inventory(Some(&typed.id), &draft("Gorilla Glue Sticks Black", ""))?;
    assert_eq!(
        (edited.draft.category.as_str(), edited.draft.colour.as_str()),
        ("Other", "")
    );
    // The batch of one photo does the same for the items without a category.
    let items = lib.save_inventory_batch(
        &[
            draft("Nyloc nut M4 Red", ""),
            draft("Nyloc nut M4", "Tools"),
            draft("Mystery", ""),
        ],
        &png(),
    )?;
    let got: Vec<_> = items
        .iter()
        .map(|i| (i.draft.category.as_str(), i.draft.colour.as_str()))
        .collect();
    assert_eq!(got, [("Nuts", ""), ("Tools", ""), ("Other", "")]);
    Ok(())
}
