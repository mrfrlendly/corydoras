//! Slicer profile import: makes a hard rule set from a Bambu Studio or
//! OrcaSlicer profile, and shows where its values differ from the built-in
//! "FDM · PETG · Bambu P1S" set.
//!
//! Both slicers keep profiles as JSON files. A profile holds only the values
//! that differ from its parent ("inherits"), so the reader follows the chain
//! of parents. A project file (.3mf) holds every value in one place.
//!
//! A slicer profile says how the printer lays lines down. The limits for a
//! designer follow from that: two outer lines make the thinnest wall, three
//! layers the smallest embossed detail, and so on. Each rule says in its
//! details how it was worked out.

use crate::agent::names_match;
use crate::library::Library;
use crate::models::{Rule, RuleSet, RuleSetInput};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::io::Read;
use std::path::{Path, PathBuf};

/// A profile that a slicer on this computer has.
#[derive(Serialize, Clone, Debug)]
pub struct ProfileRef {
    /// "Bambu Studio" or "OrcaSlicer"
    pub app: String,
    /// "process", "machine" or "filament"
    pub kind: String,
    pub name: String,
    pub path: String,
    /// True for a profile that you made; false for one that came with the slicer.
    pub user: bool,
}

/// What to import. Only the process profile (or a project file) is necessary.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Input {
    /// Path of a process profile (.json).
    #[serde(default)]
    pub process: Option<String>,
    /// Path of a printer profile (.json): gives the nozzle and the build volume.
    #[serde(default)]
    pub machine: Option<String>,
    /// Path of a filament profile (.json): gives the material.
    #[serde(default)]
    pub filament: Option<String>,
    /// Path of a project file (.3mf) of Bambu Studio or OrcaSlicer, instead of the three profiles.
    #[serde(default)]
    pub project: Option<String>,
}

/// One row of the comparison with the built-in rule set.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct CompareRow {
    pub rule: String,
    pub unit: Option<String>,
    pub imported: Option<f64>,
    pub builtin: Option<f64>,
    /// "same", "differs", "new" (only in the profile) or "builtin-only"
    pub status: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Derived {
    /// A name to suggest for the rule set.
    pub name: String,
    pub description: String,
    pub rules: Vec<Rule>,
    pub compare: Vec<CompareRow>,
    /// What the reader could not find or had to assume.
    pub notes: Vec<String>,
}

const APPS: &[(&str, &str)] = &[("Bambu Studio", "BambuStudio"), ("OrcaSlicer", "OrcaSlicer")];
const KINDS: &[&str] = &["process", "machine", "filament"];

fn app_dirs() -> Vec<(String, PathBuf)> {
    let Some(base) = dirs::config_dir() else { return Vec::new() };
    APPS.iter().map(|(label, dir)| (label.to_string(), base.join(dir))).filter(|(_, p)| p.is_dir()).collect()
}

fn json_files(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().map(|e| e == "json").unwrap_or(false))
        .collect();
    out.sort();
    out
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    out.sort();
    out
}

/// The profiles of the slicers that are installed: your own first, then the
/// ones that came with the slicer. Parent profiles that hold only shared
/// values (their names start with "fdm_") are left out.
pub fn installed_profiles() -> Vec<ProfileRef> {
    profiles_in(&app_dirs())
}

fn profiles_in(apps: &[(String, PathBuf)]) -> Vec<ProfileRef> {
    let mut out = Vec::new();
    for user in [true, false] {
        for (app, root) in apps {
            for group in subdirs(&root.join(if user { "user" } else { "system" })) {
                for kind in KINDS {
                    for path in json_files(&group.join(kind)) {
                        let name = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        if name.starts_with("fdm_") || name.contains(" template ") {
                            continue;
                        }
                        out.push(ProfileRef { app: app.clone(), kind: kind.to_string(), name, path: path.to_string_lossy().into_owned(), user });
                    }
                }
            }
        }
    }
    out
}

fn read_json(path: &Path) -> Result<Map<String, Value>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("Cannot read {}", path.display()))?;
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(m)) => Ok(m),
        _ => bail!("{} is not a slicer profile (it is not a JSON object).", path.display()),
    }
}

/// Finds the parent profile with this name: beside the child first, then in
/// the folders of the installed slicers.
fn find_parent(name: &str, child: &Path, kind: &str, apps: &[(String, PathBuf)]) -> Option<PathBuf> {
    let file = format!("{name}.json");
    if let Some(dir) = child.parent() {
        for d in [dir.to_path_buf(), dir.join("base")] {
            if d.join(&file).is_file() {
                return Some(d.join(&file));
            }
        }
    }
    for (_, root) in apps {
        for top in ["system", "user"] {
            for group in subdirs(&root.join(top)) {
                for d in [group.join(kind), group.join(kind).join("base")] {
                    if d.join(&file).is_file() {
                        return Some(d.join(&file));
                    }
                }
            }
        }
    }
    None
}

/// A profile with the values of all its parents. The second value lists parents that are missing.
fn resolve(path: &Path, apps: &[(String, PathBuf)]) -> Result<(Map<String, Value>, Vec<String>)> {
    let mut chain = vec![read_json(path)?];
    let mut missing = Vec::new();
    let mut at = path.to_path_buf();
    let kind = chain[0].get("type").and_then(|v| v.as_str()).unwrap_or("process").to_string();
    for _ in 0..16 {
        let parent = chain.last().and_then(|m| m.get("inherits")).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(|s| s.to_string());
        let Some(parent) = parent else { break };
        match find_parent(&parent, &at, &kind, apps) {
            Some(p) => {
                chain.push(read_json(&p)?);
                at = p;
            }
            None => {
                missing.push(parent);
                break;
            }
        }
    }
    let mut out = Map::new();
    for m in chain.into_iter().rev() {
        for (k, v) in m {
            out.insert(k, v);
        }
    }
    Ok((out, missing))
}

/// The settings of a project file: Metadata/project_settings.config.
fn project_settings(path: &Path) -> Result<Map<String, Value>> {
    let file = std::fs::File::open(path).with_context(|| format!("Cannot read {}", path.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| anyhow!("{} is not a 3MF project file.", path.display()))?;
    let mut entry = zip
        .by_name("Metadata/project_settings.config")
        .map_err(|_| anyhow!("This 3MF has no slicer settings. Save the project in Bambu Studio or OrcaSlicer, not as a plain 3MF export."))?;
    let mut text = String::new();
    entry.read_to_string(&mut text)?;
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(m)) => Ok(m),
        _ => bail!("The slicer settings in this 3MF are not readable."),
    }
}

/// The first value of a setting as text. The slicers write numbers as text,
/// and settings for each extruder as a list.
fn text(m: &Map<String, Value>, key: &str) -> Option<String> {
    match m.get(key)? {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::Array(a) => a.first().and_then(|v| match v {
            Value::String(s) => Some(s.trim().to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }),
        _ => None,
    }
    .filter(|s| !s.is_empty())
}

fn number(m: &Map<String, Value>, key: &str) -> Option<f64> {
    text(m, key).and_then(|s| s.trim_end_matches('%').trim().parse::<f64>().ok()).filter(|v| v.is_finite())
}

/// A width that can be millimetres or a percentage of the nozzle. 0 means "automatic".
fn width(m: &Map<String, Value>, key: &str, nozzle: f64) -> Option<f64> {
    let t = text(m, key)?;
    let v = t.trim_end_matches('%').trim().parse::<f64>().ok()?;
    let w = if t.ends_with('%') { nozzle * v / 100.0 } else { v };
    (w > 0.0).then_some(w)
}

fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn num(v: f64) -> String {
    let t = format!("{v:.2}");
    t.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Works the rules out from the merged settings of a process, a printer and a filament.
fn derive_from(settings: &Map<String, Value>, has_process: bool, source: &str) -> Derived {
    let mut rules: Vec<Rule> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let limit = |name: &str, value: f64, unit: &str, note: String| Rule { name: name.into(), value: Some(r2(value)), unit: Some(unit.into()), note };
    let nozzle = number(settings, "nozzle_diameter");
    let noz = nozzle.unwrap_or(0.4);
    if nozzle.is_none() {
        notes.push("No printer profile: the nozzle is taken as 0.4 mm.".into());
    }
    let layer = number(settings, "layer_height");
    let outer = width(settings, "outer_wall_line_width", noz).or_else(|| width(settings, "line_width", noz));
    let inner = width(settings, "inner_wall_line_width", noz).or_else(|| width(settings, "line_width", noz)).or(outer);
    let loops = number(settings, "wall_loops").map(|v| v.round().max(1.0));
    let arachne = text(settings, "wall_generator").map(|g| g.eq_ignore_ascii_case("arachne")).unwrap_or(false);
    let thin = text(settings, "detect_thin_wall").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false);

    if let Some(w) = outer {
        let extra = if arachne {
            let bead = number(settings, "min_bead_width").map(|p| noz * p / 100.0);
            format!(" The Arachne wall generator can go thinner{}, with a weaker wall.", bead.map(|b| format!(", down to about {} mm", num(b))).unwrap_or_default())
        } else {
            String::new()
        };
        rules.push(limit("Minimum wall", 2.0 * w, "mm", format!("Two outer lines of {} mm. A thinner wall prints as one line or not at all.{extra}", num(w))));
        if let (Some(n), Some(i)) = (loops, inner) {
            rules.push(limit(
                "Load-bearing wall",
                2.0 * (w + (n - 1.0) * i),
                "mm",
                format!("{} wall loop{} on each side: a wall of this thickness is solid lines, without infill.", n as i64, if n == 1.0 { "" } else { "s" }),
            ));
        }
        let single = arachne || thin;
        rules.push(limit(
            "Minimum standing feature",
            if single { w } else { 2.0 * w },
            "mm",
            if single { format!("One line of {} mm: the profile prints thin walls as single lines.", num(w)) } else { format!("Two lines of {} mm. Thin-wall detection is off in the profile.", num(w)) },
        ));
    } else if has_process {
        notes.push("The profile has no line width, so no wall limits.".into());
    }
    if let Some(h) = layer {
        rules.push(limit("Embossed or engraved detail", 3.0 * h, "mm", format!("Minimum height or depth: three layers of {} mm.", num(h))));
        rules.push(limit("Layer height", h, "mm", "Vertical detail finer than this does not print.".into()));
    } else if has_process {
        notes.push("The profile has no layer height.".into());
    }
    if let Some(a) = number(settings, "support_threshold_angle").filter(|a| *a > 0.0 && *a < 90.0) {
        rules.push(limit(
            "Maximum unsupported overhang",
            90.0 - a,
            "deg",
            format!("Measured from the vertical. The profile makes support for faces flatter than {}° from the horizontal.", num(a)),
        ));
    }
    if let Some(b) = number(settings, "max_bridge_length").filter(|b| *b > 0.0) {
        rules.push(limit("Maximum bridge", b, "mm", "Longest span without support, from the profile.".into()));
    }
    if let Some(e) = number(settings, "elefant_foot_compensation").filter(|e| *e > 0.0) {
        rules.push(limit("Elephant foot compensation", e, "mm", "The slicer pulls the first layer in by this much. Do not add a second correction in the model.".into()));
    }
    // Build volume: "0x0", "256x0", "256x256", "0x256" and a height.
    let area: Vec<(f64, f64)> = settings
        .get("printable_area")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|p| p.as_str())
        .filter_map(|p| p.split_once('x').and_then(|(x, y)| Some((x.trim().parse::<f64>().ok()?, y.trim().parse::<f64>().ok()?))))
        .collect();
    let height = number(settings, "printable_height");
    if area.len() >= 3 {
        let x = area.iter().map(|p| p.0).fold(f64::MIN, f64::max) - area.iter().map(|p| p.0).fold(f64::MAX, f64::min);
        let y = area.iter().map(|p| p.1).fold(f64::MIN, f64::max) - area.iter().map(|p| p.1).fold(f64::MAX, f64::min);
        let z = height.unwrap_or(f64::MAX);
        let smallest = x.min(y).min(z);
        rules.push(limit(
            "Maximum part size",
            smallest.floor(),
            "mm",
            format!("The build volume is {} × {}{} mm. Split larger parts and join them with pins or dovetails.", num(x), num(y), height.map(|h| format!(" × {}", num(h))).unwrap_or_default()),
        ));
    }
    if let Some(n) = nozzle {
        rules.push(limit("Nozzle diameter", n, "mm", "From the printer profile. Features narrower than the nozzle do not print.".into()));
    }
    let material = text(settings, "filament_type");
    if let Some(mat) = &material {
        let temp = number(settings, "nozzle_temperature").map(|t| format!(", nozzle {} °C", num(t))).unwrap_or_default();
        let bed = ["hot_plate_temp", "textured_plate_temp", "cool_plate_temp", "eng_plate_temp"].iter().find_map(|k| number(settings, k).filter(|t| *t > 0.0)).map(|t| format!(", bed {} °C", num(t))).unwrap_or_default();
        rules.push(Rule { name: "Material".into(), value: None, unit: None, note: format!("{mat}{}{temp}{bed}.", text(settings, "filament_settings_id").map(|n| format!(" ({n})")).unwrap_or_default()) });
    }
    if let Some(h) = number(settings, "xy_hole_compensation").filter(|h| h.abs() > 1e-9) {
        rules.push(Rule { name: "Hole compensation".into(), value: None, unit: None, note: format!("The profile already changes the size of holes by {} mm. Model holes at their nominal size.", num(h)) });
    }

    let printer = text(settings, "printer_settings_id").or_else(|| text(settings, "printer_model"));
    let process = text(settings, "print_settings_id").or_else(|| if has_process { text(settings, "name") } else { None });
    let parts: Vec<String> = [printer.clone(), material.clone(), process.clone()].into_iter().flatten().collect();
    let name = if parts.is_empty() { "Slicer profile".to_string() } else { parts.join(" · ") };
    let description = format!(
        "Made from {source} on {}. The limits follow from the slicer settings (line width, layer height, support angle, build volume); each rule says how. Check them against test prints, then edit them.",
        chrono::Local::now().format("%Y-%m-%d")
    );
    let compare = compare(&rules);
    Derived { name, description, rules, compare, notes }
}

/// Where the rules differ from the built-in set.
fn compare(rules: &[Rule]) -> Vec<CompareRow> {
    let builtin = Library::starter_rule_set().rules;
    let mut out = Vec::new();
    for r in rules.iter().filter(|r| r.value.is_some()) {
        let b = builtin.iter().find(|b| b.value.is_some() && names_match(&b.name, &r.name) && b.unit == r.unit);
        out.push(CompareRow {
            rule: r.name.clone(),
            unit: r.unit.clone(),
            imported: r.value,
            builtin: b.and_then(|b| b.value),
            status: match b.and_then(|b| b.value) {
                Some(v) if (v - r.value.unwrap_or(0.0)).abs() < 1e-6 => "same",
                Some(_) => "differs",
                None => "new",
            }
            .into(),
        });
    }
    for b in builtin.iter().filter(|b| b.value.is_some()) {
        if !rules.iter().any(|r| r.value.is_some() && names_match(&b.name, &r.name) && b.unit == r.unit) {
            out.push(CompareRow { rule: b.name.clone(), unit: b.unit.clone(), imported: None, builtin: b.value, status: "builtin-only".into() });
        }
    }
    out
}

/// Reads the profiles and works the rules out. Nothing is saved.
pub fn derive(input: &Input) -> Result<Derived> {
    derive_with(input, &app_dirs())
}

fn derive_with(input: &Input, apps: &[(String, PathBuf)]) -> Result<Derived> {
    let path = |p: &Option<String>| p.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(PathBuf::from);
    if let Some(project) = path(&input.project) {
        let settings = project_settings(&project)?;
        let name = project.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        return Ok(derive_from(&settings, true, &format!("the project file \"{name}\"")));
    }
    let process = path(&input.process).ok_or_else(|| anyhow!("Choose a process profile or a project file."))?;
    let mut merged = Map::new();
    let mut notes = Vec::new();
    let mut names = Vec::new();
    for (kind, p) in [("machine", path(&input.machine)), ("filament", path(&input.filament)), ("process", Some(process))] {
        let Some(p) = p else { continue };
        let (m, missing) = resolve(&p, apps)?;
        let found = m.get("type").and_then(|v| v.as_str()).unwrap_or(kind);
        if found != kind {
            bail!("\"{}\" is a {found} profile, not a {kind} profile.", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        }
        for parent in missing {
            notes.push(format!("The parent profile \"{parent}\" was not found, so some values of the {kind} profile can be missing. Install the slicer, or export a project file (.3mf)."));
        }
        let own = m.get("name").and_then(|v| v.as_str()).map(|s| s.to_string()).or_else(|| p.file_stem().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
        names.push(format!("\"{own}\""));
        for (k, v) in m {
            merged.insert(k, v);
        }
        // The name of each profile goes under the key that a project file uses.
        let key = match kind {
            "machine" => "printer_settings_id",
            "filament" => "filament_settings_id",
            _ => "print_settings_id",
        };
        merged.insert(key.into(), Value::String(own));
    }
    let mut d = derive_from(&merged, true, &format!("the slicer profile{} {}", if names.len() == 1 { "" } else { "s" }, names.join(", ")));
    d.notes.splice(0..0, notes);
    Ok(d)
}

impl Library {
    /// Makes a rule set from slicer profiles. With `fill_from_builtin`, the
    /// rules of the built-in set that a slicer profile cannot give (clearances,
    /// hole oversize, the instructions) are copied in, so the set is complete.
    pub fn import_slicer_profile(&self, input: &Input, name: &str, fill_from_builtin: bool) -> Result<RuleSet> {
        let d = derive(input)?;
        self.create_from_derived(&d, name, fill_from_builtin)
    }

    fn create_from_derived(&self, d: &Derived, name: &str, fill_from_builtin: bool) -> Result<RuleSet> {
        if d.rules.iter().all(|r| r.value.is_none()) {
            bail!("The profile gave no limits. {}", d.notes.join(" "));
        }
        let mut rules = d.rules.clone();
        if fill_from_builtin {
            for b in Library::starter_rule_set().rules {
                if !rules.iter().any(|r| names_match(&r.name, &b.name)) {
                    rules.push(Rule { note: format!("{} (From the built-in set; the slicer profile does not set it.)", b.note), ..b });
                }
            }
        }
        let name = name.trim();
        self.create_rule_set(&RuleSetInput { name: if name.is_empty() { d.name.clone() } else { name.to_string() }, description: d.description.clone(), always: false, rules })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write(path: &Path, v: Value) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    }

    /// A small copy of the folder that Bambu Studio keeps.
    fn tree(root: &Path) -> Vec<(String, PathBuf)> {
        let sys = root.join("BambuStudio/system/BBL");
        write(&sys.join("process/fdm_process_common.json"), json!({"type": "process", "name": "fdm_process_common", "layer_height": "0.2", "line_width": "0.42", "outer_wall_line_width": "0.42", "inner_wall_line_width": "0.45", "wall_loops": "2", "support_threshold_angle": "30", "wall_generator": "classic", "detect_thin_wall": "0", "max_bridge_length": "0"}));
        write(&sys.join("process/fdm_process_single_0.20.json"), json!({"type": "process", "name": "fdm_process_single_0.20", "inherits": "fdm_process_common", "elefant_foot_compensation": "0.15"}));
        write(&sys.join("process/0.20mm Standard @BBL X1C.json"), json!({"type": "process", "name": "0.20mm Standard @BBL X1C", "inherits": "fdm_process_single_0.20", "instantiation": "true"}));
        write(&sys.join("machine/Bambu Lab P1S 0.4 nozzle.json"), json!({"type": "machine", "name": "Bambu Lab P1S 0.4 nozzle", "nozzle_diameter": ["0.4"], "printable_area": ["0x0", "256x0", "256x256", "0x256"], "printable_height": "256"}));
        write(&sys.join("filament/Bambu PETG Basic @BBL X1C.json"), json!({"type": "filament", "name": "Bambu PETG Basic @BBL X1C", "filament_type": ["PETG"], "nozzle_temperature": ["255"], "hot_plate_temp": ["70"]}));
        let user = root.join("BambuStudio/user/123");
        write(&user.join("process/My slow profile.json"), json!({"type": "process", "name": "My slow profile", "inherits": "0.20mm Standard @BBL X1C", "layer_height": "0.16", "wall_loops": "3", "support_threshold_angle": "40"}));
        std::fs::write(user.join("process/My slow profile.info"), "x").unwrap();
        vec![("Bambu Studio".to_string(), root.join("BambuStudio"))]
    }

    fn value(d: &Derived, name: &str) -> f64 {
        d.rules.iter().find(|r| r.name == name).and_then(|r| r.value).unwrap_or_else(|| panic!("no rule {name}"))
    }

    #[test]
    fn lists_and_resolves_profiles() {
        let dir = tempfile::tempdir().unwrap();
        let apps = tree(dir.path());
        let list = profiles_in(&apps);
        // Your own profile first; the "fdm_" parents are not listed.
        assert_eq!((list[0].name.as_str(), list[0].user, list[0].kind.as_str()), ("My slow profile", true, "process"));
        assert_eq!(list.len(), 4);
        assert!(list.iter().all(|p| !p.name.starts_with("fdm_")));

        let p = |name: &str| list.iter().find(|x| x.name == name).map(|x| x.path.clone());
        let input = Input { process: p("My slow profile"), machine: p("Bambu Lab P1S 0.4 nozzle"), filament: p("Bambu PETG Basic @BBL X1C"), project: None };
        let d = derive_with(&input, &apps).unwrap();
        assert_eq!(d.name, "Bambu Lab P1S 0.4 nozzle · PETG · My slow profile");
        assert!(d.notes.is_empty(), "{:?}", d.notes);
        // Own values win; the rest comes from three levels of parents.
        assert_eq!(value(&d, "Layer height"), 0.16);
        assert_eq!(value(&d, "Embossed or engraved detail"), 0.48);
        assert_eq!(value(&d, "Minimum wall"), 0.84);
        assert_eq!(value(&d, "Load-bearing wall"), 2.64); // 2 x (0.42 + 2 x 0.45)
        assert_eq!(value(&d, "Minimum standing feature"), 0.84);
        assert_eq!(value(&d, "Maximum unsupported overhang"), 50.0);
        assert_eq!(value(&d, "Maximum part size"), 256.0);
        assert_eq!(value(&d, "Elephant foot compensation"), 0.15);
        assert!(d.rules.iter().all(|r| r.name != "Maximum bridge"), "a bridge length of 0 is not a limit");
        assert!(d.rules.iter().any(|r| r.name == "Material" && r.value.is_none() && r.note.contains("PETG") && r.note.contains("255")));

        // The comparison with the built-in set.
        let row = |rule: &str| d.compare.iter().find(|c| c.rule == rule).unwrap().clone();
        assert_eq!((row("Minimum wall").builtin, row("Minimum wall").status), (Some(1.2), "differs".to_string()));
        assert_eq!(row("Maximum unsupported overhang").status, "differs");
        assert_eq!(row("Nozzle diameter").status, "new");
        assert_eq!((row("Sliding clearance").imported, row("Sliding clearance").status), (None, "builtin-only".to_string()));

        // Only the process profile: the nozzle is assumed, and the reader says so.
        let d = derive_with(&Input { process: p("My slow profile"), ..Default::default() }, &apps).unwrap();
        assert!(d.notes.iter().any(|n| n.contains("0.4 mm")));
        assert!(d.rules.iter().all(|r| r.name != "Maximum part size"));
        // A profile of the wrong kind, and a parent that is missing.
        assert!(derive_with(&Input { process: p("Bambu Lab P1S 0.4 nozzle"), ..Default::default() }, &apps).is_err());
        let orphan = dir.path().join("orphan.json");
        write(&orphan, json!({"type": "process", "name": "Orphan", "inherits": "No such parent", "layer_height": "0.3"}));
        let d = derive_with(&Input { process: Some(orphan.to_string_lossy().into_owned()), ..Default::default() }, &apps).unwrap();
        assert!(d.notes.iter().any(|n| n.contains("No such parent")));
        assert!(derive_with(&Input::default(), &apps).is_err());
    }

    #[test]
    fn project_file_and_rule_set() {
        let dir = tempfile::tempdir().unwrap();
        let settings = json!({"layer_height": "0.2", "outer_wall_line_width": "105%", "inner_wall_line_width": "0.45", "wall_loops": "2", "wall_generator": "arachne", "min_bead_width": "85", "nozzle_diameter": ["0.4"], "support_threshold_angle": "45", "max_bridge_length": "10", "filament_type": ["PLA"], "printer_settings_id": "Bambu Lab P1S 0.4 nozzle", "print_settings_id": "0.20mm Standard", "printable_area": ["0x0", "256x0", "256x256", "0x256"], "printable_height": "250", "xy_hole_compensation": "0.1"});
        let project = dir.path().join("part.3mf");
        std::fs::write(&project, crate::mesh::tests::zip_of(&[("Metadata/project_settings.config", &settings.to_string()), ("3D/3dmodel.model", "<model/>")])).unwrap();
        let d = derive(&Input { project: Some(project.to_string_lossy().into_owned()), ..Default::default() }).unwrap();
        assert_eq!(d.name, "Bambu Lab P1S 0.4 nozzle · PLA · 0.20mm Standard");
        assert_eq!(value(&d, "Minimum wall"), 0.84); // 105 % of 0.4 mm, two lines
        assert_eq!(value(&d, "Minimum standing feature"), 0.42); // Arachne: one line
        assert_eq!(value(&d, "Maximum bridge"), 10.0);
        assert_eq!(value(&d, "Maximum part size"), 250.0);
        assert!(d.rules.iter().any(|r| r.name == "Hole compensation" && r.value.is_none()));
        assert_eq!(d.compare.iter().find(|c| c.rule == "Maximum bridge").unwrap().status, "same");
        // A plain 3MF without settings gets a clear message.
        let plain = dir.path().join("plain.3mf");
        std::fs::write(&plain, crate::mesh::tests::zip_of(&[("3D/3dmodel.model", "<model/>")])).unwrap();
        assert!(derive(&Input { project: Some(plain.to_string_lossy().into_owned()), ..Default::default() }).unwrap_err().to_string().contains("no slicer settings"));

        // The rule set: the derived rules, completed from the built-in set.
        let lib = Library::open(&dir.path().join("lib")).unwrap();
        let set = lib.create_from_derived(&d, "", true).unwrap();
        assert_eq!(set.name, d.name);
        assert!(!set.always);
        let rule = |n: &str| set.rules.iter().find(|r| r.name == n).unwrap().clone();
        assert_eq!(rule("Minimum wall").value, Some(0.84));
        assert_eq!(rule("Sliding clearance").value, Some(0.3));
        assert!(rule("Sliding clearance").note.contains("built-in set"));
        assert!(set.rules.iter().any(|r| r.name == "Load along the layers"));
        assert_eq!(set.rules.iter().filter(|r| r.name == "Minimum wall").count(), 1);
        let bare = lib.create_from_derived(&d, "Bare", false).unwrap();
        assert!(bare.rules.iter().all(|r| r.name != "Sliding clearance"));
    }
}
