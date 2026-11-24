use super::*;

fn info(name: &str, keywords: &[&str]) -> CategoryInfo {
    CategoryInfo {
        name: name.into(),
        count: 0,
        columns: None,
        keywords: keywords.iter().map(|k| k.to_string()).collect(),
    }
}

fn rules(names: &[&str]) -> Vec<CategoryRule> {
    category_rules(&names.iter().map(|n| info(n, &[])).collect::<Vec<_>>())
}

fn shop() -> Vec<CategoryRule> {
    rules(&[
        "Adhesives",
        "Filament",
        "Bolts",
        "Nuts",
        "Washers",
        "Magnets",
        "Tape",
        "Threaded Inserts",
        "Tools",
        "Cord & Rope",
        "Electronics",
        "Materials",
        "Hardware",
        "Other",
    ])
}

fn input(name: &str, category: &str) -> FieldInput {
    FieldInput {
        name: name.into(),
        category: category.into(),
        ..Default::default()
    }
}

fn category(name: &str, category: &str) -> Option<String> {
    suggest_category(&input(name, category), &shop()).map(|s| s.name)
}

fn colour(text: &str) -> Option<String> {
    suggest_colour(&input(text, "")).map(|c| c.colour)
}

#[test]
fn the_spec_examples_get_their_category_and_colour() {
    let found = suggest_category(
        &input("Overture PETG Filament Purple", "Materials"),
        &shop(),
    )
    .unwrap();
    assert_eq!(found.name, "Filament");
    assert_eq!(found.reason, "name says \"filament\"");
    assert_eq!(
        colour("Overture PETG Filament Purple").as_deref(),
        Some("Purple")
    );
    let found = suggest_category(&input("Gorilla Glue Sticks", "Other"), &shop()).unwrap();
    assert_eq!(
        (found.name.as_str(), found.reason.as_str()),
        ("Adhesives", "name says \"glue\"")
    );
}

#[test]
fn a_category_the_library_lacks_is_never_suggested() {
    let without = rules(&["Bolts", "Materials"]);
    assert!(suggest_category(
        &input("Overture PETG Filament Purple", "Materials"),
        &without
    )
    .is_none());
    // The category's own name still works for a category the designer made.
    let own = rules(&["Widgets", "Other"]);
    assert_eq!(
        suggest_category(&input("Blue widget", "Other"), &own)
            .unwrap()
            .name,
        "Widgets"
    );
    assert!(suggest_category(&input("Widgets, 5 pack", ""), &own).is_some());
    // Built-in words are keyed by name ignoring case.
    let lower = rules(&["filament", "Other"]);
    assert_eq!(
        suggest_category(&input("PETG 1 kg", "Other"), &lower)
            .unwrap()
            .name,
        "filament"
    );
}

#[test]
fn only_a_generic_category_is_replaced_and_other_is_never_the_target() {
    assert_eq!(category("Nyloc nut M4", "").as_deref(), Some("Nuts"));
    for generic in [
        "",
        "  ",
        "Other",
        "other",
        "Materials",
        "Hardware",
        "Consumables",
        "Fasteners",
    ] {
        assert_eq!(
            category("Nyloc nut M4", generic).as_deref(),
            Some("Nuts"),
            "{generic:?}"
        );
    }
    // A category of the library that is not generic stays, even when a rule matches another.
    assert_eq!(category("Nyloc nut M4", "Tools"), None);
    assert_eq!(category("Nyloc nut M4", "tools"), None);
    // The item is already where the rule would put it.
    assert_eq!(category("Nyloc nut M4", "Nuts"), None);
    // Other holds what no rule could place: nothing matches it, not even its own name.
    assert_eq!(category("Other things", "Materials"), None);
}

#[test]
fn words_match_whole_words_only() {
    for (text, expected) in [
        ("Peanut butter", None),
        ("Plate", None),
        ("Plated steel", None),
        ("Pulled cable", None),
        ("Absolute", None),
        ("Tapestry", None),
        ("Washerless", None),
        ("Hex nut", Some("Nuts")),
        ("Hex nuts", Some("Nuts")),
        ("HEX NUT", Some("Nuts")),
        ("PLA+ spool", Some("Filament")),
        ("pla spool", Some("Filament")),
        ("PLA+", Some("Filament")),
        ("Heat-set M3", Some("Threaded Inserts")),
        ("heat set M3", Some("Threaded Inserts")),
        ("M3 heat‑set", Some("Threaded Inserts")),
        ("Raspberry Pi 5", Some("Electronics")),
        ("LED strip", Some("Electronics")),
        ("5mm LED", Some("Electronics")),
        ("Wood screws", Some("Bolts")),
        ("Gorilla's glue", Some("Adhesives")),
        ("Superglue", Some("Adhesives")),
        ("Super  Glue gel", Some("Adhesives")),
    ] {
        assert_eq!(category(text, "Other").as_deref(), expected, "{text}");
    }
}

#[test]
fn name_beats_brand_beats_description_and_the_longest_word_wins() {
    let rules = shop();
    let at = |name: &str, brand: &str, description: &str| {
        suggest_category(
            &FieldInput {
                name: name.into(),
                brand: brand.into(),
                description: description.into(),
                ..Default::default()
            },
            &rules,
        )
        .map(|s| (s.name, s.reason))
    };
    assert_eq!(
        at("Tape", "", "strong glue"),
        Some(("Tape".into(), "name says \"tape\"".into()))
    );
    assert_eq!(
        at("Mystery", "Magnet Co", "strong glue"),
        Some(("Magnets".into(), "brand says \"magnet\"".into()))
    );
    assert_eq!(
        at("Mystery", "Acme", "strong glue"),
        Some(("Adhesives".into(), "description says \"glue\"".into()))
    );
    assert_eq!(at("Mystery", "Acme", "nothing"), None);
    // "super glue" is longer than "tape".
    assert_eq!(at("Super glue tape", "", "").unwrap().0, "Adhesives");
    // Two words of one length: the category with more matching words, then the alphabetical one.
    let custom = category_rules(&[
        info("Beta", &["gizmo", "thing"]),
        info("Alpha", &["gizmo"]),
        info("Gamma", &["gizmo", "thing", "stuff"]),
        info("Delta", &["gizmo", "thing", "stuff"]),
    ]);
    let pick = |text: &str| suggest_category(&input(text, ""), &custom).map(|s| s.name);
    assert_eq!(pick("gizmo"), Some("Alpha".into()));
    assert_eq!(pick("gizmo thing"), Some("Beta".into()));
    assert_eq!(pick("gizmo thing stuff"), Some("Delta".into()));
}

#[test]
fn designer_keywords_and_own_names_work() {
    let custom = category_rules(&[
        info("Hinges", &["piano hinge", "Scharnier", "köpfe"]),
        info("Threaded Inserts", &[]),
        info("Hardware", &["bracket"]),
    ]);
    let pick = |text: &str, current: &str| {
        suggest_category(&input(text, current), &custom).map(|s| (s.name, s.reason))
    };
    assert_eq!(
        pick("Brass PIANO hinge 1 m", "Other"),
        Some(("Hinges".into(), "name says \"piano hinge\"".into()))
    );
    assert_eq!(pick("SCHARNIER klein", "Other").unwrap().0, "Hinges");
    assert_eq!(pick("KÖPFE 10er", "Other").unwrap().0, "Hinges");
    assert_eq!(pick("Hinge", "Other").unwrap().0, "Hinges");
    assert_eq!(pick("Hinges", "Other").unwrap().0, "Hinges");
    assert_eq!(
        pick("M3 threaded insert", "Other").unwrap().0,
        "Threaded Inserts"
    );
    assert_eq!(
        pick("Threaded Inserts", "Other").unwrap().0,
        "Threaded Inserts"
    );
    // A generic category can have keywords too, and "hardware" in a name is not one of them.
    assert_eq!(pick("L bracket", "Other").unwrap().0, "Hardware");
    assert_eq!(pick("Hardware kit", "Other"), None);
    // Unicode letters never split a word.
    assert_eq!(pick("Köpfeschrank", "Other"), None);
}

#[test]
fn colours_are_stored_as_written_in_title_case() {
    for (text, expected) in [
        ("Titan Grey", "Titan Grey"),
        ("Titan grey", "Titan Grey"),
        ("TITAN GREY PLA", "Titan Grey"),
        ("Dark Brown", "Dark Brown"),
        ("Misty Blue", "Misty Blue"),
        ("dark misty blue", "Dark Misty Blue"),
        ("Pastel Pink", "Pastel Pink"),
        ("Gray", "Gray"),
        ("Light switch cover Red", "Red"),
        ("Overture PETG Filament Purple", "Purple"),
        ("Black", "Black"),
        ("Titan Gray", "Titan Gray"),
    ] {
        assert_eq!(colour(text).as_deref(), Some(expected), "{text}");
    }
    for text in [
        "Light switch",
        "Matte",
        "Hex bolt",
        "Titanium",
        "Blackout",
        "Reddish",
        "",
    ] {
        assert_eq!(colour(text), None, "{text}");
    }
}

#[test]
fn colours_joined_by_slash_ampersand_or_and_are_one_text() {
    for (text, expected) in [
        ("Black/Red", "Black / Red"),
        ("Black / Red", "Black / Red"),
        ("black & red", "Black / Red"),
        ("Black&Red", "Black / Red"),
        ("Black and Red", "Black / Red"),
        ("Red and Black tape", "Red / Black"),
        ("Dark Blue / Light Grey", "Dark Blue / Light Grey"),
        ("Red/Green/Blue/Black", "Red / Green / Blue"),
        ("Black and Decker drill", "Black"),
        ("Black/ widget", "Black"),
    ] {
        assert_eq!(colour(text).as_deref(), Some(expected), "{text}");
    }
}

#[test]
fn a_phrase_has_one_colour_and_clear_next_to_a_colour_is_a_finish() {
    // The list badge reads one colour per phrase, so two colour words are not one phrase.
    for (text, expected) in [
        ("Blue Greenish", "Blue"),
        ("Misty Grey", "Grey"),
        ("Black Red", "Black"),
        ("Titan Blue", "Blue"),
        ("Grey Titan", "Grey"),
        ("Translucent Red", "Red"),
        ("Clear Blue", "Blue"),
        ("Blue Clear", "Blue"),
        ("Transparent Light Blue", "Light Blue"),
        ("Clear", "Clear"),
        ("Translucent", "Clear"),
        ("Transparent PETG", "Clear"),
        ("Matte Clear", "Matte Clear"),
        ("Clear / Red", "Clear / Red"),
        ("Dark Dark Blue", "Dark Dark Blue"),
        ("Light Dark Pale Blue", "Dark Pale Blue"),
    ] {
        assert_eq!(colour(text).as_deref(), Some(expected), "{text}");
    }
}

#[test]
fn every_stored_colour_part_is_one_colour_of_at_most_three_words() {
    // What Rust stores, the list badge must be able to read: each part has 1 to 3 words, at most
    // one colour word besides a shade's own base, and no more than three parts.
    for text in [
        "Titan Grey",
        "Misty Grey",
        "Blue Green",
        "Translucent Red",
        "Clear Blue",
        "Dark Misty Blue",
        "Light Dark Pale Blue",
        "Red/Green/Blue/Black",
        "Midnight Blue",
        "Space Grey",
        "Sky Blue",
    ] {
        let stored = colour(text).unwrap();
        let parts: Vec<_> = stored.split(" / ").collect();
        assert!(parts.len() <= 3, "{stored}");
        for part in parts {
            let words: Vec<_> = part.split(' ').map(str::to_lowercase).collect();
            assert!((1..=3).contains(&words.len()), "{stored}");
            let lower = words.iter().map(String::as_str).collect::<Vec<_>>();
            assert!(parse_phrase(&lower).is_some(), "{stored}");
        }
    }
}

#[test]
fn colour_searches_the_name_then_the_brand_then_the_description_and_fills_only_empty() {
    let fields = |name: &str, brand: &str, description: &str, current: &str| FieldInput {
        name: name.into(),
        brand: brand.into(),
        description: description.into(),
        colour: current.into(),
        category: String::new(),
    };
    let found = suggest_colour(&fields(
        "PETG Filament",
        "Overture",
        "Colour: Titan grey, 1 kg",
        "",
    ))
    .unwrap();
    assert_eq!(
        (found.colour.as_str(), found.reason.as_str()),
        ("Titan Grey", "description says \"Titan grey\"")
    );
    let found = suggest_colour(&fields("PETG Red", "Blue Co", "green", "")).unwrap();
    assert_eq!(
        (found.colour.as_str(), found.reason.as_str()),
        ("Red", "name says \"Red\"")
    );
    let found = suggest_colour(&fields("PETG", "Black Diamond", "green", "")).unwrap();
    assert_eq!(
        (found.colour.as_str(), found.reason.as_str()),
        ("Black", "brand says \"Black\"")
    );
    let found = suggest_colour(&fields("Spool", "Acme", "Black/Red twin colour", "")).unwrap();
    assert_eq!(found.reason, "description says \"Black/Red\"");
    // A colour the designer entered is never touched, not even by a blank one.
    assert!(suggest_colour(&fields("PETG Red", "", "", "Blue")).is_none());
    assert!(suggest_colour(&fields("PETG Red", "", "", "  ")).is_some());
}

#[test]
fn colour_reasons_survive_multibyte_text() {
    let found = suggest_colour(&input("ÄÖÜ ñandú Müller — Titan Grey", "")).unwrap();
    assert_eq!(found.colour, "Titan Grey");
    assert_eq!(found.reason, "name says \"Titan Grey\"");
    let found = suggest_colour(&input("日本語 black/red", "")).unwrap();
    assert_eq!(found.reason, "name says \"black/red\"");
}

#[test]
fn an_ambiguous_shade_word_is_a_colour_only_next_to_its_base() {
    // extract_alone false in shared/colours.json: no colour without the shade's own base next to it.
    for text in [
        "Stainless Steel Bolts",
        "Sand Paper",
        "Ash Wood",
        "Space saver",
        "Stone",
        "Dark Steel",
        "Amber",
        "Mint",
        "Sunset",
        "Galaxy",
        "Slate",
        "Cobalt",
        "Dark Cobalt",
        "Graphite",
        "Ruby",
    ] {
        assert_eq!(colour(text), None, "{text}");
    }
    for (text, expected) in [
        ("Galaxy Black", "Black"),
        ("Stainless Steel Grey bolts", "Steel Grey"),
        ("Titan Grey", "Titan Grey"),
        ("Misty Blue", "Misty Blue"),
        ("Space Gray", "Space Gray"),
        ("Sand Beige", "Sand Beige"),
        ("Mint Green", "Mint Green"),
    ] {
        assert_eq!(colour(text).as_deref(), Some(expected), "{text}");
    }
    // extract_alone true: these stand alone.
    for (text, expected) in [
        ("Charcoal PLA", "Charcoal"),
        ("Gunmetal spray", "Gunmetal"),
        ("Midnight", "Midnight"),
        ("Azure", "Azure"),
        ("Sapphire", "Sapphire"),
        ("Emerald", "Emerald"),
        ("Jade", "Jade"),
        ("Ruby red", "Ruby Red"),
    ] {
        assert_eq!(colour(text).as_deref(), Some(expected), "{text}");
    }
}

#[test]
fn the_dictionary_is_the_shared_file() {
    let dictionary = dictionary();
    assert!(dictionary.base.contains_key("black") && dictionary.shades.contains_key("titan"));
    assert!(dictionary.aliases.contains_key("gray") && dictionary.modifiers.contains_key("dark"));
}

#[test]
fn compounds_are_one_colour_stored_as_written() {
    for (text, expected) in [
        ("Olive Green", "Olive Green"),
        ("olive green PLA", "Olive Green"),
        ("Navy Blue", "Navy Blue"),
        ("Rose Gold", "Rose Gold"),
        ("Blue Green", "Blue Green"),
        ("Hot Pink", "Hot Pink"),
        ("Baby blue", "Baby Blue"),
        ("Dark Olive Green", "Dark Olive Green"),
        ("Army Green / Black", "Army Green / Black"),
        ("Matte Navy Blue", "Matte Navy Blue"),
    ] {
        assert_eq!(colour(text).as_deref(), Some(expected), "{text}");
    }
    // A pair that is not a compound is still two colour words, and the first one is taken.
    assert_eq!(colour("Red Green").as_deref(), Some("Red"));
}

#[test]
fn skipped_words_are_never_taken_from_a_name() {
    for text in ["Aqua", "Aqua scent nail polish remover", "Light Aqua"] {
        assert_eq!(colour(text), None, "{text}");
    }
    assert_eq!(colour("Aqua Blue").as_deref(), Some("Blue"));
    // They stay words of the dictionary, so a typed colour still gets its badge.
    assert!(dictionary().aliases.contains_key("aqua"));
}

#[test]
fn colour_lines_keep_only_the_lines_that_state_a_colour() {
    assert_eq!(colour_lines("Colour: Black\nBin B7"), "Colour: Black");
    assert_eq!(
        colour_lines("  color - Red ; Amazon (similar product): Green M12"),
        "color - Red"
    );
    assert_eq!(
        colour_lines("Colours: Blue\nCOLOUR: Pink"),
        "Colours: Blue\nCOLOUR: Pink"
    );
    assert_eq!(
        colour_lines("Amazon (similar product): Green M12 screw\nParent kit: Black box\nTools & Home Improvement > Lever Nuts"),
        ""
    );
}

#[test]
fn a_matched_word_followed_by_another_product_word_is_ignored() {
    for (text, expected) in [
        ("Tape measure 5 m", None),
        ("Hot glue gun", None),
        ("Bolt cutters", None),
        ("Nut driver 8 mm", None),
        ("Filament dryer", None),
        ("Filament holder", None),
        ("Filament dispenser", None),
        ("Magnet puller", None),
        ("Wire stripper", None),
        ("Glue applicator", None),
        ("Tape remover", None),
        // "screw" is stopped by "tool"; the word "tool" itself is the Tools category.
        ("Screw tool", Some("Tools")),
        ("PLA plate", None),
        ("PLA plates", None),
        // The word elsewhere, or a second plain match, still counts.
        ("Tape", Some("Tape")),
        ("Measuring tape", Some("Tape")),
        ("Tape measure and tape", Some("Tape")),
        ("PLA filament", Some("Filament")),
        ("PLA", Some("Filament")),
        ("Gun glue", Some("Adhesives")),
        ("Bolt cutters and bolt", Some("Bolts")),
    ] {
        assert_eq!(category(text, "Other").as_deref(), expected, "{text}");
    }
    // The same for a category's own name and a designer's keyword, and for two-word keywords.
    let custom = category_rules(&[info("Widgets", &["super gizmo"]), info("Other", &[])]);
    let pick = |text: &str| suggest_category(&input(text, ""), &custom).map(|s| s.name);
    assert_eq!(pick("Widget holder"), None);
    assert_eq!(pick("Super gizmo gun"), None);
    assert_eq!(pick("Super gizmo"), Some("Widgets".into()));
    assert_eq!(pick("Widget"), Some("Widgets".into()));
}

#[test]
fn shades_of_one_base_colour_are_one_phrase() {
    for (text, expected) in [
        ("Prairie Rose", "Prairie Rose"),
        ("Prairie Rose Filament", "Prairie Rose"),
        ("prairie rose pink", "Prairie Rose Pink"),
        ("Dark Prairie Rose", "Dark Prairie Rose"),
        ("Titan Space Grey", "Titan Space Grey"),
        ("Titan Space", "Titan Space"),
        ("Misty Sky", "Misty Sky"),
    ] {
        assert_eq!(colour(text).as_deref(), Some(expected), "{text}");
    }
    // Shades of different bases are not one phrase, and a lone ambiguous shade is no colour.
    assert_eq!(colour("Titan Rose"), None);
    assert_eq!(colour("Stainless Steel"), None);
    assert_eq!(colour("Steel"), None);
    assert_eq!(colour("Rose"), None);
    assert_eq!(colour("Prairie Mint").as_deref(), None);
    assert_eq!(colour("Midnight Rose").as_deref(), Some("Midnight"));
    assert_eq!(colour("Titan Rose Gold").as_deref(), Some("Rose Gold"));
}
