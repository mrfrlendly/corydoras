use super::*;

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

fn find(specs: &[Spec], role: SpecRole) -> Option<&Spec> {
    specs.iter().find(|s| s.role == role)
}

fn standard(specs: &[Spec], role: SpecRole) -> Option<String> {
    specs
        .iter()
        .find(|s| s.role == role && s.source == SpecSource::Standard)
        .map(|s| format!("{} {}", s.value, s.unit))
}

#[test]
fn summary_lists_specs_in_order() {
    let specs = [
        spec(SpecRole::Thread, "M3", "", SpecSource::Label),
        spec(SpecRole::Pitch, "0.5", "mm", SpecSource::Standard),
        spec(SpecRole::Length, "5.7", "mm", SpecSource::Label),
    ];
    assert_eq!(
        summary(&specs),
        "M3 thread; pitch 0.5 mm (standard); length 5.7 mm"
    );
    assert_eq!(summary(&[]), "");
}

#[test]
fn a_screw_gets_pitch_clearance_and_tap_drill_from_the_table() {
    let m3 = with_standard(
        vec![spec(SpecRole::Thread, "M3", "", SpecSource::Label)],
        ProductType::Screw,
    );
    assert_eq!(standard(&m3, SpecRole::Pitch).as_deref(), Some("0.5 mm"));
    assert_eq!(
        standard(&m3, SpecRole::ClearanceHole).as_deref(),
        Some("3.4 mm")
    );
    assert_eq!(standard(&m3, SpecRole::TapDrill).as_deref(), Some("2.5 mm"));
    let m4 = with_standard(
        vec![spec(SpecRole::Thread, "M4", "", SpecSource::Notation)],
        ProductType::Bolt,
    );
    assert_eq!(standard(&m4, SpecRole::Pitch).as_deref(), Some("0.7 mm"));
    assert_eq!(
        standard(&m4, SpecRole::ClearanceHole).as_deref(),
        Some("4.5 mm")
    );
    assert_eq!(standard(&m4, SpecRole::TapDrill).as_deref(), Some("3.3 mm"));
    assert!(find(&m3, SpecRole::Pitch).unwrap().evidence.contains("M3"));
}

#[test]
fn an_insert_gets_pitch_only() {
    let insert = with_standard(
        vec![spec(SpecRole::Thread, "M3", "", SpecSource::Label)],
        ProductType::Insert,
    );
    assert_eq!(insert.len(), 2);
    assert_eq!(
        standard(&insert, SpecRole::Pitch).as_deref(),
        Some("0.5 mm")
    );
    assert!(find(&insert, SpecRole::HoleDiameter).is_none());
    let spool = vec![spec(SpecRole::Material, "PETG", "", SpecSource::Label)];
    assert_eq!(with_standard(spool.clone(), ProductType::Filament), spool);
}

#[test]
fn a_printed_pitch_beats_the_table() {
    let fine = with_standard(
        vec![
            spec(SpecRole::Thread, "M8", "", SpecSource::Label),
            spec(SpecRole::Pitch, "1", "mm", SpecSource::Label),
        ],
        ProductType::Screw,
    );
    assert!(standard(&fine, SpecRole::Pitch).is_none());
    assert_eq!(find(&fine, SpecRole::Pitch).unwrap().value, "1");
    assert_eq!(
        standard(&fine, SpecRole::ClearanceHole).as_deref(),
        Some("9 mm")
    );
    assert_eq!(standard(&fine, SpecRole::TapDrill).as_deref(), Some("7 mm"));
}

#[test]
fn refresh_follows_the_thread() {
    let m3 = with_standard(
        vec![spec(SpecRole::Thread, "M3", "", SpecSource::Label)],
        ProductType::Screw,
    );
    let mut edited = m3.clone();
    edited[0] = spec(SpecRole::Thread, "M4", "", SpecSource::Entered);
    let refreshed = refresh_standard(edited);
    assert_eq!(
        standard(&refreshed, SpecRole::Pitch).as_deref(),
        Some("0.7 mm")
    );
    assert_eq!(
        standard(&refreshed, SpecRole::ClearanceHole).as_deref(),
        Some("4.5 mm")
    );
    assert_eq!(
        standard(&refreshed, SpecRole::TapDrill).as_deref(),
        Some("3.3 mm")
    );
    assert_eq!(refreshed.len(), 4);

    let without_thread: Vec<Spec> = m3
        .into_iter()
        .filter(|s| s.role != SpecRole::Thread)
        .collect();
    assert!(refresh_standard(without_thread).is_empty());

    let pitch_only = with_standard(
        vec![spec(SpecRole::Thread, "M3", "", SpecSource::Label)],
        ProductType::Insert,
    );
    assert_eq!(refresh_standard(pitch_only.clone()), pitch_only);
}

#[test]
fn normalized_rejects_imperial_units_and_overlong_values() {
    let good = spec(SpecRole::Length, "8", "mm", SpecSource::Label);
    assert_eq!(normalized(&[good.clone()]).unwrap(), vec![good.clone()]);
    let bad = |edit: &dyn Fn(&mut Spec)| {
        let mut s = good.clone();
        edit(&mut s);
        normalized(&[s]).is_err()
    };
    assert!(bad(&|s| s.unit = "in".into()));
    assert!(bad(&|s| s.value = "9".repeat(121)));
    assert!(bad(&|s| s.value = "8\u{7}".into()));
    assert!(bad(&|s| s.evidence = "x\u{0}".into()));
    assert!(bad(&|s| s.name = "Head height".into()));
    assert!(bad(&|s| s.value = " ".into()));
    assert!(normalized(&vec![good.clone(); 41]).is_err());
    let mut other = good;
    other.role = SpecRole::Other;
    other.name = "Head height".into();
    assert!(normalized(&[other.clone()]).is_ok());
    other.name.clear();
    assert!(normalized(&[other]).is_err());
}

#[test]
fn refresh_never_adds_rows_to_an_item_without_standard_ones() {
    let typed = vec![
        spec(SpecRole::Thread, "M3", "", SpecSource::Entered),
        spec(SpecRole::Length, "5.7", "mm", SpecSource::Entered),
    ];
    assert_eq!(refresh_standard(typed.clone()), typed);
}

#[test]
fn tap_drill_follows_a_printed_or_entered_fine_pitch() {
    let m8 = |pitch: &str, source| {
        vec![
            spec(SpecRole::Thread, "M8", "", SpecSource::Label),
            spec(SpecRole::Pitch, pitch, "mm", source),
        ]
    };
    assert_eq!(
        standard(
            &with_standard(m8("1", SpecSource::Label), ProductType::Screw),
            SpecRole::TapDrill
        )
        .as_deref(),
        Some("7 mm")
    );
    assert_eq!(
        standard(
            &with_standard(m8("1.25", SpecSource::Label), ProductType::Screw),
            SpecRole::TapDrill
        )
        .as_deref(),
        Some("6.8 mm")
    );
    let mut rows = with_standard(
        vec![spec(SpecRole::Thread, "M8", "", SpecSource::Label)],
        ProductType::Screw,
    );
    rows.push(spec(SpecRole::Pitch, "1", "mm", SpecSource::Entered));
    rows.retain(|s| !(s.role == SpecRole::Pitch && s.source == SpecSource::Standard));
    assert_eq!(
        standard(&refresh_standard(rows), SpecRole::TapDrill).as_deref(),
        Some("7 mm")
    );
}

#[test]
fn a_pitch_printed_in_a_marking_is_read_only_when_it_fits_the_size() {
    assert_eq!(printed_pitch("M8x1").as_deref(), Some("1"));
    assert_eq!(printed_pitch("M8 x 1.25").as_deref(), Some("1.25"));
    assert_eq!(printed_pitch("M12 x 1,5").as_deref(), Some("1.5"));
    assert_eq!(printed_pitch("M4-.70").as_deref(), Some("0.7"));
    assert_eq!(printed_pitch("M8x30"), None);
    assert_eq!(printed_pitch("M3x8"), None);
    assert_eq!(printed_pitch("M3"), None);
}
