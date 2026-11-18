//! Structured specs of a workshop item, and the ISO metric table that fills in standard values.
//! A spec says where its value came from, so an agent can tell a printed number from a derived one.

use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecRole {
    Thread,
    Pitch,
    Length,
    OuterDiameter,
    /// The installation hole of an insert.
    HoleDiameter,
    ClearanceHole,
    TapDrill,
    Diameter,
    NetWeight,
    Material,
    NozzleTemp,
    BedTemp,
    Drying,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecSource {
    /// Value and role are both printed.
    Label,
    /// The digits are printed; their role comes from a size notation such as M2x2x3.2.
    Notation,
    /// From the built-in table. Not measured.
    Standard,
    /// Typed or edited by the designer.
    Entered,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductType {
    Screw,
    Bolt,
    Nut,
    Washer,
    Insert,
    Standoff,
    Filament,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Spec {
    pub role: SpecRole,
    /// Only for the `Other` role.
    #[serde(default)]
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub tolerance: String,
    pub source: SpecSource,
    #[serde(default)]
    pub evidence: String,
}

const MAX_SPECS: usize = 40;
const UNITS: &[&str] = &["", "mm", "g", "kg", "°C"];

/// ISO 261 coarse pitch, ISO 273 medium clearance hole and the usual tap drill, all in mm.
const METRIC: &[(&str, &str, &str, &str)] = &[
    ("1.6", "0.35", "1.8", "1.25"),
    ("2", "0.4", "2.4", "1.6"),
    ("2.5", "0.45", "2.9", "2.05"),
    ("3", "0.5", "3.4", "2.5"),
    ("4", "0.7", "4.5", "3.3"),
    ("5", "0.8", "5.5", "4.2"),
    ("6", "1", "6.6", "5"),
    ("8", "1.25", "9", "6.8"),
    ("10", "1.5", "11", "8.5"),
    ("12", "1.75", "13.5", "10.2"),
];

struct Row {
    thread: &'static str,
    pitch: &'static str,
    clearance: &'static str,
    tap: &'static str,
}

fn table_row(specs: &[Spec]) -> Option<Row> {
    let thread = specs
        .iter()
        .find(|s| s.role == SpecRole::Thread && s.source != SpecSource::Standard)?;
    let size = thread.value.trim().strip_prefix(['M', 'm'])?;
    METRIC.iter().find(|r| r.0 == size).map(|r| Row {
        thread: r.0,
        pitch: r.1,
        clearance: r.2,
        tap: r.3,
    })
}

fn standard_spec(role: SpecRole, value: &str, evidence: String) -> Spec {
    Spec {
        role,
        name: String::new(),
        value: value.into(),
        unit: "mm".into(),
        tolerance: String::new(),
        source: SpecSource::Standard,
        evidence,
    }
}

fn role_words(spec: &Spec) -> &str {
    match spec.role {
        SpecRole::Thread => "thread",
        SpecRole::Pitch => "pitch",
        SpecRole::Length => "length",
        SpecRole::OuterDiameter => "outer diameter",
        SpecRole::HoleDiameter => "hole diameter",
        SpecRole::ClearanceHole => "clearance hole",
        SpecRole::TapDrill => "tap drill",
        SpecRole::Diameter => "diameter",
        SpecRole::NetWeight => "net weight",
        SpecRole::Material => "material",
        SpecRole::NozzleTemp => "nozzle temperature",
        SpecRole::BedTemp => "bed temperature",
        SpecRole::Drying => "drying",
        SpecRole::Other => &spec.name,
    }
}

/// A readable line of the specs, in their order. It becomes the item's Dimensions text.
pub fn summary(specs: &[Spec]) -> String {
    specs
        .iter()
        .map(|s| {
            if s.role == SpecRole::Thread {
                return format!("{} thread", s.value);
            }
            let mut text = format!("{} {}", role_words(s), s.value);
            if !s.tolerance.is_empty() {
                text.push_str(&format!(" ±{}", s.tolerance));
            }
            if !s.unit.is_empty() {
                text.push_str(&format!(" {}", s.unit));
            }
            text
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn has(specs: &[Spec], role: SpecRole) -> bool {
    specs
        .iter()
        .any(|s| s.role == role && s.source != SpecSource::Standard)
}

/// Add table values to a drafted item. A printed or entered value always wins, and an insert only gets a pitch.
pub fn with_standard(mut specs: Vec<Spec>, product: ProductType) -> Vec<Spec> {
    let Some(row) = table_row(&specs) else {
        return specs;
    };
    let screw = matches!(product, ProductType::Screw | ProductType::Bolt);
    let thread = row.thread;
    if !specs.iter().any(|s| s.role == SpecRole::Pitch) {
        specs.push(standard_spec(
            SpecRole::Pitch,
            row.pitch,
            format!("ISO 261 coarse pitch, M{thread}"),
        ));
    }
    if screw && !specs.iter().any(|s| s.role == SpecRole::ClearanceHole) {
        specs.push(standard_spec(
            SpecRole::ClearanceHole,
            row.clearance,
            format!("ISO 273 medium clearance, M{thread}"),
        ));
    }
    if screw && !specs.iter().any(|s| s.role == SpecRole::TapDrill) {
        specs.push(standard_spec(
            SpecRole::TapDrill,
            row.tap,
            format!("Tap drill for M{thread}"),
        ));
    }
    specs
}

/// Rebuild the table rows after an edit. Only the kinds of standard row that were there come back,
/// because the product type is not stored.
pub fn refresh_standard(specs: Vec<Spec>) -> Vec<Spec> {
    let had = |role| {
        specs
            .iter()
            .any(|s| s.role == role && s.source == SpecSource::Standard)
    };
    let (clearance, tap) = (had(SpecRole::ClearanceHole), had(SpecRole::TapDrill));
    let mut kept: Vec<Spec> = specs
        .into_iter()
        .filter(|s| s.source != SpecSource::Standard)
        .collect();
    let Some(row) = table_row(&kept) else {
        return kept;
    };
    let thread = row.thread;
    if !has(&kept, SpecRole::Pitch) {
        kept.push(standard_spec(
            SpecRole::Pitch,
            row.pitch,
            format!("ISO 261 coarse pitch, M{thread}"),
        ));
    }
    if clearance && !has(&kept, SpecRole::ClearanceHole) {
        kept.push(standard_spec(
            SpecRole::ClearanceHole,
            row.clearance,
            format!("ISO 273 medium clearance, M{thread}"),
        ));
    }
    if tap && !has(&kept, SpecRole::TapDrill) {
        kept.push(standard_spec(
            SpecRole::TapDrill,
            row.tap,
            format!("Tap drill for M{thread}"),
        ));
    }
    kept
}

fn text(value: &str, what: &str, limit: usize) -> Result<String> {
    let value = value.trim();
    ensure!(
        value.chars().count() <= limit,
        "The spec {what} exceeds {limit} characters."
    );
    ensure!(
        !value.chars().any(char::is_control),
        "The spec {what} contains an invalid character."
    );
    Ok(value.to_owned())
}

/// Check and trim the specs of one item before they are stored.
pub fn normalized(specs: &[Spec]) -> Result<Vec<Spec>> {
    ensure!(
        specs.len() <= MAX_SPECS,
        "An item can have at most {MAX_SPECS} specs."
    );
    specs
        .iter()
        .map(|s| {
            let value = text(&s.value, "value", 120)?;
            ensure!(!value.is_empty(), "Enter a value for every spec.");
            ensure!(
                UNITS.contains(&s.unit.trim()),
                "Use a metric unit: mm, g, kg or °C."
            );
            let name = text(&s.name, "name", 60)?;
            ensure!(
                (s.role == SpecRole::Other) == !name.is_empty(),
                "Give a name to a spec with the role Other, and only to that role."
            );
            Ok(Spec {
                role: s.role,
                name,
                value,
                unit: s.unit.trim().to_owned(),
                tolerance: text(&s.tolerance, "tolerance", 60)?,
                source: s.source,
                evidence: text(&s.evidence, "evidence", 512)?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;
