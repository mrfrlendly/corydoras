//! Geometry check: measures a model (STL, 3MF, OBJ or STEP) and compares the
//! numbers with the hard rules: minimum wall, overhang, bridge, clearance and
//! part size.
//!
//! What it measures, for a build direction (default +Z):
//! - wall: the distance through the material, from sample points on every face
//!   straight into the part to the face where the ray comes out;
//! - overhang: the angle of each downward face, from the vertical. A flat
//!   ceiling between two supports is a bridge, not an overhang; its span is
//!   measured instead;
//! - clearance: the gap between separate bodies, face to face;
//! - size: the largest side of the bounding box of a body.
//!
//! Every value ignores spots smaller than about one square millimetre, so one
//! sliver triangle does not decide the result. Numbers that another tool
//! measured (for example ScanRuler's thickness and clearance) can be passed in;
//! they replace the values measured here.

use crate::agent::match_rule;
use crate::conformance::{self, Measurement, Report};
use crate::mesh::{self, add, dot, scale, sub, Bvh, Mesh, Up, V3};
use crate::models::RuleSet;
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// Sample points for the wall measure.
const WALL_SAMPLES: f64 = 120_000.0;
/// Sample points for the clearance measure.
const GAP_SAMPLES: f64 = 60_000.0;
/// A downward face flatter than this (degrees from the vertical) is a ceiling.
const FLAT_FROM: f64 = 80.0;
/// A flat ceiling that reaches at most this far past its support is ignored (mm).
const FLAT_TOLERANCE: f64 = 1.0;
/// Overhang limit when no rule sets one (degrees from the vertical).
const DEFAULT_OVERHANG: f64 = 45.0;
/// Distance of the probe rays from a ceiling (mm).
const FLAT_PROBE: f64 = 0.02;
/// Two bodies closer than this touch (mm).
const TOUCH: f64 = 0.02;
/// Layer height when no rule sets one (mm).
const DEFAULT_LAYER: f64 = 0.2;
/// Two faces are the two sides of a wall when they are parallel within about
/// 25 degrees (the cosine of that angle). A chamfer or a knife edge is not a wall.
const WALL_PARALLEL: f64 = 0.9;

/// A measured value and where on the model it is.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Spot {
    pub value: f64,
    pub at: [f64; 3],
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct GeometryReport {
    pub file: String,
    /// "stl", "3mf", "obj" or "step"
    pub format: String,
    /// The build direction that the check used.
    pub up: String,
    pub triangles: usize,
    /// Separate solid bodies.
    pub bodies: usize,
    /// Closed voids inside a body.
    pub cavities: usize,
    /// Edges that do not join exactly two triangles. 0 for a closed mesh.
    pub open_edges: usize,
    /// Size of the bounding box (mm), with the build direction last.
    pub size: [f64; 3],
    pub volume_cm3: Option<f64>,
    pub area_cm2: f64,
    /// The thinnest wall with a real area: the distance between two faces that
    /// are parallel within about 25 degrees.
    pub min_wall: Option<Spot>,
    /// The thinnest section of any shape, chamfers and tapered edges included.
    pub thinnest: Option<Spot>,
    pub median_wall: Option<f64>,
    /// The steepest overhang with a real area (degrees from the vertical).
    pub max_overhang: Option<Spot>,
    /// Area of the faces beyond the overhang limit (mm²), bridges excluded.
    pub overhang_area_mm2: f64,
    pub overhang_limit: f64,
    /// The longest flat span between two supports (mm).
    pub max_bridge: Option<Spot>,
    /// The longest reach of a flat ceiling past its support (mm).
    pub flat_overhang: Option<Spot>,
    /// The smallest gap between two bodies (mm).
    pub min_clearance: Option<Spot>,
    /// Two bodies overlap.
    pub overlap: bool,
    /// Area where two bodies rest on each other (mm²).
    #[serde(default)]
    pub contact_mm2: f64,
    /// The numbers that were judged.
    pub measurements: Vec<Measurement>,
    pub notes: Vec<String>,
    pub check: Option<Report>,
    pub seconds: f64,
}

/// Settings of one analysis.
#[derive(Clone, Debug)]
pub struct Settings {
    pub overhang_limit: f64,
    /// Layer height (mm). Faces in the first two layers above the build plate
    /// are not overhangs: the plate carries them.
    pub layer: f64,
    /// Judge overhangs and bridges. None: only for a single body, because the
    /// bodies of an assembly are not in their print orientation.
    pub overhangs: Option<bool>,
    /// The model comes from a closed CAD solid: edges that do not match are
    /// hairline cracks of the triangulation, not holes.
    pub solid_source: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { overhang_limit: DEFAULT_OVERHANG, layer: DEFAULT_LAYER, overhangs: None, solid_source: false }
    }
}

struct Sample {
    value: f64,
    area: f64,
    at: V3,
}

/// The value where the area of the samples below it (or above it, with
/// `from_top`) first reaches `threshold`. None when the samples are too few.
fn robust(samples: &mut [Sample], threshold: f64, from_top: bool) -> Option<Spot> {
    samples.sort_by(|a, b| a.value.partial_cmp(&b.value).unwrap_or(std::cmp::Ordering::Equal));
    if from_top {
        samples.reverse();
    }
    let mut sum = 0.0;
    for s in samples.iter() {
        sum += s.area;
        if sum >= threshold {
            return Some(Spot { value: s.value, at: s.at });
        }
    }
    None
}

fn halton(mut i: u32, base: u32) -> f64 {
    let (mut f, mut r) = (1.0, 0.0);
    while i > 0 {
        f /= base as f64;
        r += f * (i % base) as f64;
        i /= base;
    }
    r
}

/// `k` points spread over a triangle.
fn points_on(tri: &[V3; 3], k: u32, out: &mut Vec<V3>) {
    out.clear();
    if k <= 1 {
        out.push(scale(add(add(tri[0], tri[1]), tri[2]), 1.0 / 3.0));
        return;
    }
    for j in 1..=k {
        let (r1, r2) = (halton(j, 2).sqrt(), halton(j, 3));
        let (u, v, w) = (1.0 - r1, r1 * (1.0 - r2), r1 * r2);
        out.push(add(add(scale(tri[0], u), scale(tri[1], v)), scale(tri[2], w)));
    }
}

fn round3(v: V3) -> [f64; 3] {
    [(v[0] * 100.0).round() / 100.0, (v[1] * 100.0).round() / 100.0, (v[2] * 100.0).round() / 100.0]
}

struct Flat {
    faces: Vec<u32>,
    area: f64,
    bridge: Option<Spot>,
    reach: Option<Spot>,
}

/// Measures a mesh whose build direction is +Z.
pub fn analyze(mesh: &Mesh, settings: &Settings) -> GeometryReport {
    let n = mesh.tris.len();
    let mut rep = GeometryReport { triangles: n, overhang_limit: settings.overhang_limit, ..Default::default() };
    let (lo, hi) = mesh.bounds();
    rep.size = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
    let diag = mesh::length(sub(hi, lo)).max(1e-9);
    let eps = (diag * 1e-7).max(1e-6);

    let na: Vec<(V3, f64)> = (0..n).map(|i| mesh.normal_area(i)).collect();
    let total_area: f64 = na.iter().map(|x| x.1).sum();
    rep.area_cm2 = total_area / 100.0;
    let threshold = (total_area * 0.0005).max(1.0).min(total_area * 0.25);

    // Bodies and cavities.
    let (labels, parts) = mesh.components();
    let mut part_tris: Vec<Vec<usize>> = vec![Vec::new(); parts];
    for (i, l) in labels.iter().enumerate() {
        part_tris[*l as usize].push(i);
    }
    let volumes: Vec<f64> = part_tris.iter().map(|t| mesh.signed_volume(t.iter().copied())).collect();
    let is_body: Vec<bool> = volumes.iter().map(|v| *v > 0.0).collect();
    rep.bodies = is_body.iter().filter(|b| **b).count().max(if parts > 0 { 1 } else { 0 });
    rep.cavities = parts.saturating_sub(is_body.iter().filter(|b| **b).count());
    let edges = mesh.edge_faces();
    rep.open_edges = edges.values().filter(|f| f.len() != 2).count();
    if rep.open_edges == 0 || settings.solid_source {
        rep.volume_cm3 = Some(volumes.iter().sum::<f64>() / 1000.0);
        if rep.open_edges > 0 {
            rep.notes.push(format!(
                "The triangles of the converted solid have {} hairline crack{} between faces. The solid itself is closed; the values are measured with rays, which the cracks do not change.",
                rep.open_edges,
                if rep.open_edges == 1 { "" } else { "s" }
            ));
        }
    } else {
        rep.notes.push(format!(
            "The mesh is not closed: {} edge{} do not join exactly two triangles. Wall and volume values can be wrong near the gaps.",
            rep.open_edges,
            if rep.open_edges == 1 { "" } else { "s" }
        ));
    }
    if volumes.iter().sum::<f64>() < 0.0 {
        rep.notes.push("The triangles point inward (the volume is negative), so the wall and overhang values are not reliable. Flip the normals and check again.".into());
    }
    if rep.cavities > 0 {
        rep.notes.push(format!(
            "The model has {} closed void{} inside. Support material in a closed void cannot come out.",
            rep.cavities,
            if rep.cavities == 1 { "" } else { "s" }
        ));
    }

    let bvh = Bvh::build(mesh);
    let mut pts: Vec<V3> = Vec::new();

    // --- Walls: from each sample point straight into the part ---
    let cell = (total_area / WALL_SAMPLES).max(1e-9);
    let mut walls: Vec<Sample> = Vec::new();
    let mut sections: Vec<Sample> = Vec::new();
    let mut bad = 0usize;
    let mut tried = 0usize;
    for i in 0..n {
        let (normal, area) = na[i];
        if area <= 0.0 {
            continue;
        }
        let k = ((area / cell).ceil() as u32).clamp(1, 48);
        points_on(&mesh.tri(i), k, &mut pts);
        let dir = scale(normal, -1.0);
        for p in pts.iter() {
            tried += 1;
            let mut origin = add(*p, scale(dir, eps));
            let mut travelled = eps;
            let mut skip = i as u32;
            let mut found = None;
            // A hit on another body is not the end of this wall: go on through it.
            for _ in 0..6 {
                let Some(h) = bvh.ray(mesh, origin, dir, eps * 0.5, f64::MAX, skip) else { break };
                let other = labels[h.tri as usize];
                if is_body[labels[i] as usize] && other != labels[i] && is_body[other as usize] {
                    travelled += h.t;
                    origin = add(origin, scale(dir, h.t));
                    skip = h.tri;
                    continue;
                }
                found = Some((travelled + h.t, dot(dir, na[h.tri as usize].0)));
                break;
            }
            match found {
                Some((t, facing)) if facing > 0.2 => {
                    if facing >= WALL_PARALLEL {
                        walls.push(Sample { value: t, area: area / k as f64, at: *p });
                    }
                    sections.push(Sample { value: t, area: area / k as f64, at: *p });
                }
                Some((_, facing)) if facing > 0.0 => {}
                _ => bad += 1,
            }
        }
    }
    if tried > 0 && bad * 5 > tried {
        rep.notes.push("Many rays did not come out of the part, so the mesh is open or its normals are mixed. The wall value is not reliable.".into());
    }
    if !walls.is_empty() {
        let sampled: f64 = walls.iter().map(|s| s.area).sum();
        let thr = threshold.min(sampled * 0.25);
        rep.min_wall = robust(&mut walls, thr, false);
        let mut half = 0.0;
        for s in &walls {
            half += s.area;
            if half >= sampled * 0.5 {
                rep.median_wall = Some(s.value);
                break;
            }
        }
        if let Some(w) = rep.min_wall.as_mut() {
            w.at = round3(w.at);
        }
    }
    if !sections.is_empty() {
        let sampled: f64 = sections.iter().map(|s| s.area).sum();
        rep.thinnest = robust(&mut sections, threshold.min(sampled * 0.25), false).map(|s| Spot { value: s.value, at: round3(s.at) });
    }

    // --- Overhangs and bridges ---
    let body_count = is_body.iter().filter(|b| **b).count();
    let judge_overhangs = settings.overhangs.unwrap_or(body_count <= 1);
    if !judge_overhangs {
        rep.notes.push(format!(
            "The model has {body_count} bodies in their assembled places, so overhangs and bridges were not judged. Check each part alone in its print orientation, or pass the build direction to judge the bodies as they are placed."
        ));
    }
    // Each body stands on its own lowest point. Faces in the first two layers are carried by the plate.
    let mut part_low = vec![f64::MAX; parts];
    for (i, t) in mesh.tris.iter().enumerate() {
        for &v in t {
            let l = labels[i] as usize;
            part_low[l] = part_low[l].min(mesh.positions[v as usize][2] as f64);
        }
    }
    let on_plate: Vec<bool> = (0..n).map(|i| mesh.centroid(i)[2] <= part_low[labels[i] as usize] + 2.0 * settings.layer.max(0.0) + eps).collect();
    let theta: Vec<f64> = na
        .iter()
        .map(|(nrm, area)| if judge_overhangs && *area > 0.0 && nrm[2] < 0.0 { (-nrm[2]).min(1.0).asin().to_degrees() } else { 0.0 })
        .collect();
    let is_flat: Vec<bool> = (0..n).map(|i| theta[i] >= FLAT_FROM && !on_plate[i]).collect();

    // Group the ceilings into connected regions.
    let mut region_of: Vec<i32> = vec![-1; n];
    let mut regions: Vec<Flat> = Vec::new();
    for start in 0..n {
        if !is_flat[start] || region_of[start] >= 0 {
            continue;
        }
        let id = regions.len() as i32;
        let mut faces = Vec::new();
        let mut queue = vec![start as u32];
        region_of[start] = id;
        while let Some(f) = queue.pop() {
            faces.push(f);
            let t = mesh.tris[f as usize];
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                if let Some(nb) = edges.get(&(a.min(b), a.max(b))) {
                    for &g in nb {
                        if is_flat[g as usize] && region_of[g as usize] < 0 {
                            region_of[g as usize] = id;
                            queue.push(g);
                        }
                    }
                }
            }
        }
        let area = faces.iter().map(|f| na[*f as usize].1).sum();
        regions.push(Flat { faces, area, bridge: None, reach: None });
    }

    // For points on each ceiling, look along 12 lines in the plane of the face.
    // Two rays go each way: one in the air just under the ceiling, one in the
    // material just above it. A wall that the air ray meets while the material
    // above still goes on is a support. Where the material ends first, the
    // ceiling has a free edge. Rays do not need the triangles to join, so the
    // hairline cracks of a converted STEP file do no harm.
    let off = (FLAT_PROBE).max(eps * 50.0);
    let flat_total: usize = regions.iter().filter(|r| r.area >= 0.25).map(|r| r.faces.len()).sum();
    let step = (flat_total / 3000).max(1);
    for region in regions.iter_mut() {
        if region.area < 0.25 {
            continue;
        }
        let mut bridge: Option<Spot> = None;
        let mut reach: Option<Spot> = None;
        for &f in region.faces.iter().step_by(step) {
            let (nrm, area) = na[f as usize];
            if area <= 0.0 {
                continue;
            }
            let c = mesh.centroid(f as usize);
            let (air, solid) = (add(c, scale(nrm, off)), sub(c, scale(nrm, off)));
            let mut best_span = f64::MAX;
            let mut nearest = f64::MAX;
            let mut free = f64::MAX;
            for d in 0..12 {
                let ang = (15.0 * d as f64).to_radians();
                let flat_dir = [ang.cos(), ang.sin(), 0.0];
                let dir = mesh::unit(sub(flat_dir, scale(nrm, dot(flat_dir, nrm))));
                if mesh::length(dir) < 0.5 {
                    continue;
                }
                let mut side = [None, None];
                for (k, sense) in [1.0, -1.0].into_iter().enumerate() {
                    let dd = scale(dir, sense);
                    // The wall under the ceiling: the first face that the air ray enters.
                    let wall = bvh.ray(mesh, air, dd, eps, f64::MAX, f).filter(|h| dot(dd, na[h.tri as usize].0) < -0.1).map(|h| h.t);
                    // The end of the material above: the first face that the inner ray leaves through.
                    let end = bvh.ray(mesh, solid, dd, eps, f64::MAX, f).filter(|h| dot(dd, na[h.tri as usize].0) > 0.1).map(|h| h.t);
                    match (wall, end) {
                        (Some(a), Some(b)) if a <= b + 4.0 * off => side[k] = Some(a),
                        (Some(a), None) => side[k] = Some(a),
                        (_, Some(b)) => free = free.min(b),
                        _ => {}
                    }
                    if let Some(a) = side[k] {
                        nearest = nearest.min(a);
                    }
                }
                if let (Some(a), Some(b)) = (side[0], side[1]) {
                    best_span = best_span.min(a + b);
                }
            }
            if best_span < f64::MAX {
                if bridge.as_ref().map(|s| best_span > s.value).unwrap_or(true) {
                    bridge = Some(Spot { value: best_span, at: round3(c) });
                }
            } else {
                // No pair of supports: this point hangs past its nearest support.
                let r = if nearest < f64::MAX { nearest } else if free < f64::MAX { free } else { region.area.sqrt() };
                if reach.as_ref().map(|s| r > s.value).unwrap_or(true) {
                    reach = Some(Spot { value: r, at: round3(c) });
                }
            }
        }
        region.bridge = bridge;
        region.reach = reach;
    }

    // A ceiling that is bridged (or reaches less than the tolerance) is not an overhang.
    let bridged = |r: &Flat| r.area < 0.25 || r.reach.as_ref().map(|s| s.value <= FLAT_TOLERANCE).unwrap_or(true);
    for r in &regions {
        if let Some(b) = &r.bridge {
            if rep.max_bridge.as_ref().map(|m| b.value > m.value).unwrap_or(true) {
                rep.max_bridge = Some(b.clone());
            }
        }
        if !bridged(r) {
            if let Some(x) = &r.reach {
                if rep.flat_overhang.as_ref().map(|m| x.value > m.value).unwrap_or(true) {
                    rep.flat_overhang = Some(x.clone());
                }
            }
        }
    }
    let mut hangs: Vec<Sample> = Vec::new();
    for i in 0..n {
        if theta[i] <= 0.0 || on_plate[i] {
            continue;
        }
        if region_of[i] >= 0 && bridged(&regions[region_of[i] as usize]) {
            continue;
        }
        hangs.push(Sample { value: theta[i], area: na[i].1, at: mesh.centroid(i) });
        if theta[i] > settings.overhang_limit + 0.5 {
            rep.overhang_area_mm2 += na[i].1;
        }
    }
    if judge_overhangs {
        rep.max_overhang = Some(match robust(&mut hangs, threshold, true) {
            Some(s) => Spot { value: (s.value * 10.0).round() / 10.0, at: round3(s.at) },
            None => Spot { value: 0.0, at: [0.0; 3] },
        });
    }

    // --- Clearance between bodies ---
    if body_count >= 2 {
        // True when the point lies inside the body `label`: a ray from it
        // crosses that body's surface an odd number of times.
        let inside_body = |q: V3, label: u32| -> bool {
            let mut crossings = 0;
            bvh.ray_each(mesh, q, mesh::unit([0.1234, 0.4567, 0.8811]), 0.0, |_, tri| {
                if labels[tri as usize] == label {
                    crossings += 1;
                }
            });
            crossings % 2 == 1
        };
        let cell = (total_area / GAP_SAMPLES).max(1e-9);
        let mut gaps: Vec<Sample> = Vec::new();
        let (mut inside, mut touching) = (0.0, 0.0);
        for i in 0..n {
            let (normal, area) = na[i];
            if area <= 0.0 || !is_body[labels[i] as usize] {
                continue;
            }
            let k = ((area / cell).ceil() as u32).clamp(1, 32);
            points_on(&mesh.tri(i), k, &mut pts);
            for p in pts.iter() {
                let origin = add(*p, scale(normal, eps));
                let Some(h) = bvh.ray(mesh, origin, normal, eps * 0.5, f64::MAX, i as u32) else { continue };
                let other = labels[h.tri as usize];
                if other == labels[i] || !is_body[other as usize] {
                    continue;
                }
                let facing = dot(normal, na[h.tri as usize].0);
                if facing < -0.2 {
                    if h.t + eps <= TOUCH {
                        touching += area / k as f64;
                    } else {
                        gaps.push(Sample { value: h.t + eps, area: area / k as f64, at: *p });
                    }
                } else if facing > 0.2 {
                    // The ray left the other body, so this point is on or in it.
                    // A point a little behind this face tells a seat from a clash.
                    if inside_body(sub(*p, scale(normal, 2.5 * TOUCH)), other) {
                        inside += area / k as f64;
                    } else {
                        touching += area / k as f64;
                    }
                }
            }
        }
        rep.contact_mm2 = (touching * 0.5).round();
        if inside > threshold {
            rep.overlap = true;
            rep.notes.push("Two bodies overlap: part of one lies inside another. The clearance there is zero.".into());
            rep.min_clearance = Some(Spot { value: 0.0, at: [0.0; 3] });
        } else if !gaps.is_empty() {
            let sampled: f64 = gaps.iter().map(|s| s.area).sum();
            rep.min_clearance = robust(&mut gaps, threshold.min(sampled * 0.25), false).map(|s| Spot { value: s.value, at: round3(s.at) });
        }
        if rep.contact_mm2 > 0.0 {
            rep.notes.push(format!(
                "The bodies touch over about {} mm2 (faces that rest on each other). The clearance is the smallest gap that is open.",
                rep.contact_mm2
            ));
        }
    }


    // --- The numbers to judge ---
    let r2 = |v: f64| (v * 100.0).round() / 100.0;
    let m = |name: &str, value: f64, unit: &str| Measurement { name: name.into(), value, unit: Some(unit.into()) };
    if let Some(w) = &rep.min_wall {
        rep.measurements.push(m("minimum wall", r2(w.value), "mm"));
    }
    if let Some(o) = &rep.max_overhang {
        rep.measurements.push(m("maximum unsupported overhang", o.value, "deg"));
    }
    if let Some(b) = &rep.max_bridge {
        rep.measurements.push(m("maximum bridge", r2(b.value), "mm"));
    }
    // The largest side of one body: a part must fit the printer, an assembly need not.
    let mut largest: f64 = 0.0;
    for (pi, tris) in part_tris.iter().enumerate() {
        if !is_body[pi] && body_count > 0 {
            continue;
        }
        let (mut plo, mut phi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for &t in tris {
            for &v in &mesh.tris[t] {
                let p = mesh.p(v);
                for k in 0..3 {
                    plo[k] = plo[k].min(p[k]);
                    phi[k] = phi[k].max(p[k]);
                }
            }
        }
        for k in 0..3 {
            largest = largest.max(phi[k] - plo[k]);
        }
    }
    rep.measurements.push(m("maximum part size", r2(largest), "mm"));
    rep
}

/// What to check and against which rules.
#[derive(Clone, Debug, Default)]
pub struct Args {
    /// Language ID or name. Without it, only the hard rules judge.
    pub language: Option<String>,
    pub family: Option<String>,
    /// Project name or ID, for its rule sets.
    pub project: Option<String>,
    /// Build direction: +z (default), -z, +y, -y, +x or -x.
    pub up: Option<String>,
    /// The rule that the gap between bodies is compared with (default "sliding clearance").
    pub clearance: Option<String>,
    /// Numbers from another tool. A number with the name of a measured value replaces it.
    pub measurements: Vec<Measurement>,
    /// ID (or its first characters) of an output to save the report with.
    pub output: Option<String>,
}

fn norm(s: &str) -> String {
    s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

fn rule_value(sets: &[RuleSet], name: &str, unit: &str) -> Option<f64> {
    match_rule(sets, name).filter(|(_, r)| r.unit.as_deref() == Some(unit)).and_then(|(_, r)| r.value)
}

/// Measures a model file and judges the numbers. The hard rules are those of
/// `project` plus the sets that apply everywhere.
pub fn check_file(lib: &crate::Library, path: &Path, args: &Args) -> Result<GeometryReport> {
    let started = std::time::Instant::now();
    if !path.is_absolute() {
        bail!("The path of the model must be absolute.");
    }
    let up = Up::parse(args.up.as_deref().unwrap_or(""))?;
    let project = match args.project.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => Some(lib.find_project(p)?),
        None => None,
    };
    let sets = lib.effective_rule_sets(project.as_ref())?;
    let loaded = mesh::load(path)?;
    let settings = Settings {
        overhang_limit: rule_value(&sets, "maximum unsupported overhang", "deg").unwrap_or(DEFAULT_OVERHANG),
        layer: rule_value(&sets, "layer height", "mm").unwrap_or(DEFAULT_LAYER),
        // A build direction that the caller names means: judge the model as it is placed.
        overhangs: args.up.as_deref().map(str::trim).filter(|u| !u.is_empty()).map(|_| true),
        solid_source: loaded.format == "step",
    };
    let mesh = loaded.mesh.oriented(up);
    let mut rep = analyze(&mesh, &settings);
    rep.file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    rep.format = loaded.format;
    rep.up = up.label().to_string();
    rep.notes.splice(0..0, loaded.notes);
    if let Some(c) = &rep.min_clearance {
        let name = args.clearance.as_deref().map(str::trim).filter(|c| !c.is_empty()).unwrap_or("sliding clearance");
        rep.measurements.push(Measurement { name: name.to_string(), value: (c.value * 100.0).round() / 100.0, unit: Some("mm".into()) });
    }
    // Numbers from another tool win over the ones measured here.
    for extra in &args.measurements {
        if extra.name.trim().is_empty() || !extra.value.is_finite() {
            bail!("Every measurement needs a name and a number.");
        }
        match rep.measurements.iter_mut().find(|m| norm(&m.name) == norm(&extra.name)) {
            Some(own) => {
                rep.notes.push(format!(
                    "\"{}\" uses the number that you sent ({}); this check measured {}.",
                    own.name,
                    fmt(extra.value, extra.unit.as_deref()),
                    fmt(own.value, own.unit.as_deref())
                ));
                own.value = extra.value;
                if extra.unit.is_some() {
                    own.unit = extra.unit.clone();
                }
            }
            None => rep.measurements.push(extra.clone()),
        }
    }
    let log = lib.print_results_for(&sets)?;
    let report = match args.language.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
        Some(key) => {
            let lang = lib.find_language(key)?;
            let content = lib
                .read_latest(&lang.id)?
                .ok_or_else(|| anyhow!("\"{}\" has no version yet, so there is nothing to check against.", lang.name))?;
            if content.info.schema < 2 {
                bail!("Version {} of \"{}\" uses the older format. Synthesize a new version first.", content.info.version, lang.name);
            }
            conformance::check_with_log(&lang.name, content.info.version, &content.tokens, &sets, args.family.as_deref(), &rep.measurements, &log)?
        }
        None => conformance::check_with_log("", 0, &Value::Null, &sets, None, &rep.measurements, &log)?,
    };
    rep.check = Some(report);
    rep.seconds = (started.elapsed().as_secs_f64() * 10.0).round() / 10.0;
    if let Some(key) = args.output.as_deref().map(str::trim).filter(|o| !o.is_empty()) {
        let out = lib.find_output(key)?;
        lib.save_output_geometry(&out.id, &rep)?;
    }
    Ok(rep)
}

fn fmt(v: f64, unit: Option<&str>) -> String {
    let n = if (v - v.round()).abs() < 1e-9 { format!("{}", v.round() as i64) } else { format!("{:.2}", v).trim_end_matches('0').trim_end_matches('.').to_string() };
    match unit {
        Some(u) if !u.is_empty() => format!("{n} {u}"),
        _ => n,
    }
}

fn at(s: &Spot) -> String {
    format!("near x {}, y {}, z {}", fmt(s.at[0], None), fmt(s.at[1], None), fmt(s.at[2], None))
}

/// The report as text for an agent.
pub fn report_text(r: &GeometryReport) -> String {
    let mut s = format!(
        "GEOMETRY of {} ({}, build direction {}): {} triangles, {} bod{}, size {} x {} x {} mm{}.\n\n",
        r.file,
        r.format.to_uppercase(),
        r.up,
        r.triangles,
        r.bodies,
        if r.bodies == 1 { "y" } else { "ies" },
        fmt(r.size[0], None),
        fmt(r.size[1], None),
        fmt(r.size[2], None),
        r.volume_cm3.map(|v| format!(", volume {} cm3", fmt(v, None))).unwrap_or_default()
    );
    s.push_str("Measured:\n");
    if let Some(w) = &r.min_wall {
        s.push_str(&format!(
            "- Thinnest wall: {} {}{}.\n",
            fmt(w.value, Some("mm")),
            at(w),
            r.median_wall.map(|m| format!(" (typical wall {})", fmt(m, Some("mm")))).unwrap_or_default()
        ));
    }
    if let (Some(t), Some(w)) = (&r.thinnest, &r.min_wall) {
        if t.value < w.value * 0.9 {
            s.push_str(&format!("- Thinnest tapered section (a chamfer or a tapered edge, not a wall): {} {}.\n", fmt(t.value, Some("mm")), at(t)));
        }
    }
    if let Some(o) = &r.max_overhang {
        if o.value <= 0.0 {
            s.push_str("- Overhang: none above the first layer.\n");
        } else {
            s.push_str(&format!(
                "- Steepest overhang: {} from the vertical {}; {} mm2 of faces are beyond {}.\n",
                fmt(o.value, Some("deg")),
                at(o),
                fmt(r.overhang_area_mm2.round(), None),
                fmt(r.overhang_limit, Some("deg"))
            ));
        }
    }
    if let Some(f) = &r.flat_overhang {
        s.push_str(&format!("- Flat overhang: a ceiling reaches {} past its support {}. It needs support or a redesign.\n", fmt(f.value, Some("mm")), at(f)));
    }
    if let Some(b) = &r.max_bridge {
        s.push_str(&format!("- Longest bridge: {} between two supports {}.\n", fmt(b.value, Some("mm")), at(b)));
    }
    if let Some(c) = &r.min_clearance {
        s.push_str(&format!("- Smallest gap between bodies: {} {}.\n", fmt(c.value, Some("mm")), at(c)));
    }
    for n in &r.notes {
        s.push_str(&format!("- Note: {n}\n"));
    }
    s.push_str("\nA wall is the material between two faces that are parallel within about 25 degrees; ribs and lettering count as walls. Faces in the first two layers above the build plate are not overhangs. Overhangs and bridges depend on the build direction: pass \"up\" when the part prints in another orientation.\n\n");
    if let Some(c) = &r.check {
        s.push_str(&conformance::report_text(c));
    }
    s
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::mesh::tests::{binary_stl, box_soup};
    use crate::models::Rule;

    /// A closed prism: a polygon in the X-Z plane (counter-clockwise), pulled
    /// along Y. `cap` lists the triangles of the polygon.
    pub fn prism(poly: &[[f32; 2]], cap: &[[usize; 3]], y0: f32, y1: f32) -> Vec<[f32; 3]> {
        let mut out = Vec::new();
        let n = poly.len();
        for i in 0..n {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            let (a0, a1) = ([a[0], y0, a[1]], [a[0], y1, a[1]]);
            let (b0, b1) = ([b[0], y0, b[1]], [b[0], y1, b[1]]);
            out.extend([a0, a1, b1, a0, b1, b0]);
        }
        for t in cap {
            let p = |i: usize, y: f32| [poly[i][0], y, poly[i][1]];
            out.extend([p(t[0], y0), p(t[1], y0), p(t[2], y0)]);
            out.extend([p(t[0], y1), p(t[2], y1), p(t[1], y1)]);
        }
        out
    }

    /// An open channel: walls and floor 1 mm thick, 20 wide, 10 high, 30 long.
    pub fn channel() -> Vec<[f32; 3]> {
        let poly = [[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [19.0, 10.0], [19.0, 1.0], [1.0, 1.0], [1.0, 10.0], [0.0, 10.0]];
        prism(&poly, &[[0, 1, 5], [1, 4, 5], [1, 2, 4], [2, 3, 4], [0, 5, 7], [5, 6, 7]], 0.0, 30.0)
    }

    /// A T: a stem with a bar on top that hangs 8 mm past the stem on each side.
    fn tee() -> Vec<[f32; 3]> {
        let poly = [[8.0, 0.0], [12.0, 0.0], [12.0, 10.0], [20.0, 10.0], [20.0, 12.0], [0.0, 12.0], [0.0, 10.0], [8.0, 10.0]];
        prism(&poly, &[[0, 1, 2], [0, 2, 7], [7, 2, 4], [2, 3, 4], [7, 4, 5], [6, 7, 5]], 0.0, 10.0)
    }

    /// An arch: two legs with a slab across, 6 mm between the legs.
    fn arch() -> Vec<[f32; 3]> {
        let poly = [[0.0, 0.0], [2.0, 0.0], [2.0, 10.0], [8.0, 10.0], [8.0, 0.0], [10.0, 0.0], [10.0, 12.0], [0.0, 12.0]];
        prism(&poly, &[[0, 1, 2], [0, 2, 7], [2, 3, 7], [3, 6, 7], [3, 4, 5], [3, 5, 6]], 0.0, 10.0)
    }

    fn value(r: &GeometryReport, name: &str) -> f64 {
        r.measurements.iter().find(|m| m.name == name).unwrap_or_else(|| panic!("no {name}")).value
    }

    #[test]
    fn solid_block() {
        let mesh = Mesh::from_soup(&box_soup([0.0; 3], [20.0, 30.0, 10.0]));
        let r = analyze(&mesh, &Settings::default());
        assert_eq!((r.bodies, r.cavities, r.open_edges), (1, 0, 0));
        assert!((r.volume_cm3.unwrap() - 6.0).abs() < 1e-9);
        assert!((value(&r, "minimum wall") - 10.0).abs() < 0.01, "{:?}", r.min_wall);
        assert_eq!(value(&r, "maximum unsupported overhang"), 0.0);
        assert!(r.max_bridge.is_none() && r.flat_overhang.is_none());
        assert!((value(&r, "maximum part size") - 30.0).abs() < 1e-6);
        assert!(r.notes.is_empty(), "{:?}", r.notes);
    }

    #[test]
    fn thin_walls_and_bridges_by_orientation() {
        let mesh = Mesh::from_soup(&channel());
        assert!(mesh.signed_volume(0..mesh.tris.len()) > 0.0);
        let r = analyze(&mesh, &Settings::default());
        assert_eq!(r.open_edges, 0);
        assert!((value(&r, "minimum wall") - 1.0).abs() < 0.01, "{:?}", r.min_wall);
        // Open side up: nothing hangs.
        assert_eq!(value(&r, "maximum unsupported overhang"), 0.0);
        assert!(r.max_bridge.is_none());
        // Upside down: the floor is a ceiling across 18 mm, held by both walls.
        let r = analyze(&mesh.clone().oriented(Up::NegZ), &Settings::default());
        assert!((value(&r, "maximum bridge") - 18.0).abs() < 0.05, "{:?}", r.max_bridge);
        assert_eq!(value(&r, "maximum unsupported overhang"), 0.0, "a bridge is not an overhang");
    }

    #[test]
    fn a_chamfered_edge_is_not_a_thin_wall() {
        // A wall 2 mm thick and 10 mm high with a 1 mm chamfer on one top edge:
        // the top land is 1 mm wide, but the wall is still 2 mm.
        let poly = [[0.0, 0.0], [2.0, 0.0], [2.0, 9.0], [1.0, 10.0], [0.0, 10.0]];
        let soup = prism(&poly, &[[0, 1, 2], [0, 2, 3], [0, 3, 4]], 0.0, 40.0);
        let r = analyze(&Mesh::from_soup(&soup), &Settings::default());
        assert_eq!(r.open_edges, 0);
        assert!((value(&r, "minimum wall") - 2.0).abs() < 0.01, "{:?}", r.min_wall);
        let t = r.thinnest.as_ref().unwrap().value;
        assert!(t < 1.6, "the tapered section under the chamfer is thinner: {t}");
        assert!(report_text(&r).contains("Thinnest tapered section"));
    }

    #[test]
    fn first_layers_are_not_overhangs() {
        // A block with a 0.3 mm chamfer on its bottom edges, modelled as a 45 degree
        // skirt 0.3 mm high: it lies inside the first two layers.
        let poly = [[0.3, 0.0], [9.7, 0.0], [10.0, 0.3], [10.0, 5.0], [0.0, 5.0], [0.0, 0.3]];
        let soup = prism(&poly, &[[0, 1, 2], [0, 2, 5], [5, 2, 3], [5, 3, 4]], 0.0, 20.0);
        let mut steep = Settings::default();
        steep.overhang_limit = 30.0;
        let r = analyze(&Mesh::from_soup(&soup), &steep);
        assert_eq!(value(&r, "maximum unsupported overhang"), 0.0, "{:?}", r.max_overhang);
        // With a thin layer, the same skirt is above the first two layers and counts.
        steep.layer = 0.05;
        let r = analyze(&Mesh::from_soup(&soup), &steep);
        assert!((value(&r, "maximum unsupported overhang") - 45.0).abs() < 0.2, "{:?}", r.max_overhang);
    }

    #[test]
    fn cantilever_is_an_overhang_and_an_arch_is_a_bridge() {
        let r = analyze(&Mesh::from_soup(&tee()), &Settings::default());
        assert_eq!(r.open_edges, 0);
        assert!(value(&r, "maximum unsupported overhang") > 89.0, "{:?}", r.max_overhang);
        let reach = r.flat_overhang.as_ref().unwrap().value;
        assert!(reach > 3.0 && reach <= 8.01, "{reach}");
        assert!((r.overhang_area_mm2 - 160.0).abs() < 1.0, "{}", r.overhang_area_mm2);

        let r = analyze(&Mesh::from_soup(&arch()), &Settings::default());
        assert!((value(&r, "maximum bridge") - 6.0).abs() < 0.05, "{:?}", r.max_bridge);
        assert_eq!(value(&r, "maximum unsupported overhang"), 0.0);
        assert!(r.flat_overhang.is_none());
    }

    #[test]
    fn clearance_between_bodies() {
        let mut soup = box_soup([0.0; 3], [10.0, 10.0, 10.0]);
        soup.extend(box_soup([10.25, 0.0, 0.0], [20.0, 10.0, 10.0]));
        let r = analyze(&Mesh::from_soup(&soup), &Settings::default());
        assert_eq!(r.bodies, 2);
        assert!((r.min_clearance.as_ref().unwrap().value - 0.25).abs() < 0.01, "{:?}", r.min_clearance);
        assert!(!r.overlap);
        // The largest side of one body, not of the pair.
        assert!((value(&r, "maximum part size") - 10.0).abs() < 1e-4);
        // An assembly is not in print orientation: no overhang verdict, and a note that says so.
        assert!(r.max_overhang.is_none() && !r.measurements.iter().any(|m| m.name.contains("overhang")));
        assert!(r.notes.iter().any(|n| n.contains("2 bodies")), "{:?}", r.notes);
        // With a build direction, the bodies are judged as they stand.
        let r = analyze(&Mesh::from_soup(&soup), &Settings { overhangs: Some(true), ..Default::default() });
        assert_eq!(value(&r, "maximum unsupported overhang"), 0.0);
        // A lid that rests on a base: the seat touches, the open gap beside it is the clearance.
        let base = box_soup([0.0; 3], [20.0, 20.0, 5.0]);
        let lid = box_soup([0.0, 0.0, 5.0], [20.0, 20.0, 8.0]);
        let pin = box_soup([20.4, 0.0, 0.0], [24.0, 20.0, 8.0]);
        let r = analyze(&Mesh::from_groups(&[&base, &lid, &pin]), &Settings::default());
        assert_eq!(r.bodies, 3);
        assert!(!r.overlap, "{:?}", r.notes);
        assert!((r.contact_mm2 - 400.0).abs() < 8.0, "{}", r.contact_mm2);
        assert!((r.min_clearance.as_ref().unwrap().value - 0.4).abs() < 0.01, "{:?}", r.min_clearance);
        // A lid that sinks 1 mm into the base is a clash.
        let sunk = box_soup([2.0, 2.0, 4.0], [18.0, 18.0, 8.0]);
        let r = analyze(&Mesh::from_groups(&[&base, &sunk]), &Settings::default());
        assert!(r.overlap);
        assert_eq!(r.min_clearance.as_ref().unwrap().value, 0.0);

        // A closed void inside a block is a cavity, not a body.
        let mut soup = box_soup([0.0; 3], [10.0, 10.0, 10.0]);
        soup.extend(crate::mesh::tests::flipped(&box_soup([3.0; 3], [7.0, 7.0, 7.0])));
        let r = analyze(&Mesh::from_soup(&soup), &Settings::default());
        assert_eq!((r.bodies, r.cavities), (1, 1));
        assert!((value(&r, "minimum wall") - 3.0).abs() < 0.01, "{:?}", r.min_wall);
        assert!(r.notes.iter().any(|n| n.contains("closed void")));
    }

    #[test]
    fn file_check_against_rules() {
        let dir = tempfile::tempdir().unwrap();
        let lib = crate::Library::open(dir.path()).unwrap();
        // The starter set applies only when a project selects it: make one that applies everywhere.
        let rule = |n: &str, v: f64, u: &str| Rule { name: n.into(), value: Some(v), unit: Some(u.into()), note: String::new() };
        lib.create_rule_set(&crate::models::RuleSetInput {
            name: "Test printer".into(),
            description: String::new(),
            always: true,
            rules: vec![rule("Minimum wall", 1.2, "mm"), rule("Maximum unsupported overhang", 45.0, "deg"), rule("Maximum bridge", 10.0, "mm"), rule("Maximum part size", 250.0, "mm"), rule("Sliding clearance", 0.3, "mm")],
        })
        .unwrap();
        let path = dir.path().join("channel.stl");
        std::fs::write(&path, binary_stl(&channel())).unwrap();

        let r = check_file(&lib, &path, &Args::default()).unwrap();
        let c = r.check.as_ref().unwrap();
        let verdict = |name: &str| c.findings.iter().find(|f| f.measure == name).map(|f| f.verdict.clone()).unwrap();
        assert_eq!(verdict("minimum wall"), "fail"); // 1.0 < 1.2
        assert_eq!(verdict("maximum unsupported overhang"), "pass");
        assert_eq!(verdict("maximum part size"), "pass");
        let text = report_text(&r);
        assert!(text.contains("GEOMETRY of channel.stl") && text.contains("Thinnest wall: 1 mm"), "{text}");

        // Upside down, the bridge is too long; a number from another tool replaces the wall.
        let args = Args { up: Some("-z".into()), measurements: vec![Measurement { name: "Minimum wall".into(), value: 1.5, unit: Some("mm".into()) }], ..Default::default() };
        let r = check_file(&lib, &path, &args).unwrap();
        let c = r.check.as_ref().unwrap();
        assert_eq!(c.findings.iter().find(|f| f.measure == "maximum bridge").unwrap().verdict, "fail");
        assert_eq!(c.findings.iter().find(|f| f.measure == "minimum wall").unwrap().verdict, "pass");
        assert!(r.notes.iter().any(|n| n.contains("the number that you sent")));

        // Two bodies 0.25 mm apart break the sliding clearance.
        let mut soup = box_soup([0.0; 3], [10.0, 10.0, 10.0]);
        soup.extend(box_soup([10.25, 0.0, 0.0], [20.0, 10.0, 10.0]));
        let pair = dir.path().join("pair.stl");
        std::fs::write(&pair, binary_stl(&soup)).unwrap();
        let r = check_file(&lib, &pair, &Args::default()).unwrap();
        assert_eq!(r.check.unwrap().findings.iter().find(|f| f.measure == "sliding clearance").unwrap().verdict, "fail");

        assert!(check_file(&lib, Path::new("relative.stl"), &Args::default()).is_err());
        assert!(check_file(&lib, &dir.path().join("missing.stl"), &Args::default()).is_err());
        assert!(check_file(&lib, &path, &Args { up: Some("sideways".into()), ..Default::default() }).is_err());
    }
}
