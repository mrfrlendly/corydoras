//! Triangle meshes: reading STL, 3MF, OBJ and (through FreeCAD) STEP files, and
//! the queries that the geometry check and the renderer need.
//!
//! STL, 3MF and OBJ are read here. STEP is a B-rep format that needs a CAD
//! kernel; when FreeCAD is installed, its console program converts the file to
//! triangles. Without FreeCAD, a STEP file gets a clear message.

use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

pub type V3 = [f64; 3];

pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub fn length(a: V3) -> f64 {
    dot(a, a).sqrt()
}
pub fn unit(a: V3) -> V3 {
    let l = length(a);
    if l > 0.0 {
        scale(a, 1.0 / l)
    } else {
        [0.0, 0.0, 0.0]
    }
}

/// File types that Corydoras reads as 3D models.
pub const MODEL_EXTENSIONS: &[&str] = &["stl", "3mf", "obj", "step", "stp"];
/// Largest model file (bytes).
pub const MAX_MODEL_BYTES: u64 = 400 * 1024 * 1024;
/// Most triangles in one model.
pub const MAX_TRIANGLES: usize = 6_000_000;
/// Seconds that the STEP conversion may take.
const STEP_TIMEOUT_SECS: u64 = 180;

pub fn is_model_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    MODEL_EXTENSIONS.iter().any(|e| lower.ends_with(&format!(".{e}")))
}

/// The build direction of a model: the axis that points up on the printer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Up {
    PosX,
    NegX,
    PosY,
    NegY,
    PosZ,
    NegZ,
}

impl Up {
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s.trim().to_lowercase().replace(' ', "").as_str() {
            "" | "z" | "+z" | "zup" | "z-up" => Up::PosZ,
            "-z" => Up::NegZ,
            "y" | "+y" | "yup" | "y-up" => Up::PosY,
            "-y" => Up::NegY,
            "x" | "+x" => Up::PosX,
            "-x" => Up::NegX,
            other => bail!("Unknown build direction \"{other}\". Use +z, -z, +y, -y, +x or -x."),
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Up::PosX => "+x",
            Up::NegX => "-x",
            Up::PosY => "+y",
            Up::NegY => "-y",
            Up::PosZ => "+z",
            Up::NegZ => "-z",
        }
    }

    /// The same point in a frame where the build direction is +Z (a rotation).
    fn to_z(self, p: [f32; 3]) -> [f32; 3] {
        let [x, y, z] = p;
        match self {
            Up::PosZ => [x, y, z],
            Up::NegZ => [x, -y, -z],
            Up::PosY => [x, -z, y],
            Up::NegY => [x, z, -y],
            Up::PosX => [-z, y, x],
            Up::NegX => [z, y, -x],
        }
    }
}

/// A triangle mesh with shared vertices. Lengths are millimetres.
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub tris: Vec<[u32; 3]>,
}

/// A model that was read from a file.
#[derive(Clone, Debug)]
pub struct Loaded {
    pub mesh: Mesh,
    /// "stl", "3mf", "obj" or "step"
    pub format: String,
    /// What the reader did that the user must know (unit conversion, the converter).
    pub notes: Vec<String>,
}

impl Mesh {
    /// Makes a mesh from loose triangles (three points each). Points at the
    /// same place become one vertex; triangles without area are dropped.
    pub fn from_soup(soup: &[[f32; 3]]) -> Mesh {
        Mesh::from_groups(&[soup])
    }

    /// The same for several bodies. Each group keeps its own vertices, so two
    /// bodies that touch stay two bodies.
    pub fn from_groups(groups: &[&[[f32; 3]]]) -> Mesh {
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in groups.iter().flat_map(|g| g.iter()) {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        let diag = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt().max(1e-6);
        // Far below anything a printer can make, and wide enough to close the
        // hairline gaps that CAD exports leave between faces.
        let tol = (diag as f64 * 5e-6).clamp(1e-6, 1e-3);
        Mesh::from_groups_tol(groups, lo, tol)
    }

    fn from_groups_tol(groups: &[&[[f32; 3]]], lo: [f32; 3], tol: f64) -> Mesh {
        let total: usize = groups.iter().map(|g| g.len()).sum();
        let mut positions: Vec<[f32; 3]> = Vec::new();
        let mut tris: Vec<[u32; 3]> = Vec::with_capacity(total / 3);
        for soup in groups {
            // Points with the same bits are the common case: find them first.
            let mut exact: HashMap<[u32; 3], u32> = HashMap::with_capacity(soup.len() / 4);
            let mut grid: HashMap<[i64; 3], Vec<u32>> = HashMap::with_capacity(soup.len() / 4);
            let cell = |p: [f32; 3]| -> [i64; 3] {
                [
                    ((p[0] - lo[0]) as f64 / tol).floor() as i64,
                    ((p[1] - lo[1]) as f64 / tol).floor() as i64,
                    ((p[2] - lo[2]) as f64 / tol).floor() as i64,
                ]
            };
            let mut index = |p: [f32; 3], positions: &mut Vec<[f32; 3]>| -> u32 {
                let bits = [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()];
                if let Some(i) = exact.get(&bits) {
                    return *i;
                }
                let c = cell(p);
                let mut found = None;
                'search: for dx in -1..=1 {
                    for dy in -1..=1 {
                        for dz in -1..=1 {
                            if let Some(list) = grid.get(&[c[0] + dx, c[1] + dy, c[2] + dz]) {
                                for &i in list {
                                    let q = positions[i as usize];
                                    let d2 = (q[0] - p[0]) as f64 * (q[0] - p[0]) as f64 + (q[1] - p[1]) as f64 * (q[1] - p[1]) as f64 + (q[2] - p[2]) as f64 * (q[2] - p[2]) as f64;
                                    if d2 <= tol * tol {
                                        found = Some(i);
                                        break 'search;
                                    }
                                }
                            }
                        }
                    }
                }
                let i = found.unwrap_or_else(|| {
                    positions.push(p);
                    let i = (positions.len() - 1) as u32;
                    grid.entry(c).or_default().push(i);
                    i
                });
                exact.insert(bits, i);
                i
            };
            for t in soup.chunks_exact(3) {
                let (a, b, c) = (index(t[0], &mut positions), index(t[1], &mut positions), index(t[2], &mut positions));
                if a != b && b != c && a != c {
                    tris.push([a, b, c]);
                }
            }
        }
        Mesh { positions, tris }
    }

    pub fn p(&self, i: u32) -> V3 {
        let v = self.positions[i as usize];
        [v[0] as f64, v[1] as f64, v[2] as f64]
    }

    pub fn tri(&self, i: usize) -> [V3; 3] {
        let t = self.tris[i];
        [self.p(t[0]), self.p(t[1]), self.p(t[2])]
    }

    /// The unit normal (right-hand rule) and the area of a triangle.
    pub fn normal_area(&self, i: usize) -> (V3, f64) {
        let [a, b, c] = self.tri(i);
        let n = cross(sub(b, a), sub(c, a));
        let l = length(n);
        if l > 0.0 {
            (scale(n, 1.0 / l), l * 0.5)
        } else {
            ([0.0, 0.0, 0.0], 0.0)
        }
    }

    pub fn centroid(&self, i: usize) -> V3 {
        let [a, b, c] = self.tri(i);
        scale(add(add(a, b), c), 1.0 / 3.0)
    }

    /// The corners of the bounding box.
    pub fn bounds(&self) -> (V3, V3) {
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for p in &self.positions {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k] as f64);
                hi[k] = hi[k].max(p[k] as f64);
            }
        }
        if self.positions.is_empty() {
            return ([0.0; 3], [0.0; 3]);
        }
        (lo, hi)
    }

    /// Turns the mesh so that the build direction is +Z.
    pub fn oriented(mut self, up: Up) -> Mesh {
        if up != Up::PosZ {
            for p in self.positions.iter_mut() {
                *p = up.to_z(*p);
            }
        }
        self
    }

    pub fn area(&self) -> f64 {
        (0..self.tris.len()).map(|i| self.normal_area(i).1).sum()
    }

    /// Connected parts of the mesh (triangles that share a vertex). Returns the
    /// part number of each triangle and the number of parts.
    pub fn components(&self) -> (Vec<u32>, usize) {
        let n = self.positions.len();
        let mut parent: Vec<u32> = (0..n as u32).collect();
        fn find(parent: &mut [u32], mut x: u32) -> u32 {
            while parent[x as usize] != x {
                parent[x as usize] = parent[parent[x as usize] as usize];
                x = parent[x as usize];
            }
            x
        }
        for t in &self.tris {
            let a = find(&mut parent, t[0]);
            for &v in &t[1..] {
                let b = find(&mut parent, v);
                if a != b {
                    parent[b as usize] = a;
                }
            }
        }
        let mut ids: HashMap<u32, u32> = HashMap::new();
        let mut labels = Vec::with_capacity(self.tris.len());
        for t in &self.tris {
            let root = find(&mut parent, t[0]);
            let next = ids.len() as u32;
            labels.push(*ids.entry(root).or_insert(next));
        }
        (labels, ids.len())
    }

    /// For each undirected edge, the triangles that use it.
    pub fn edge_faces(&self) -> HashMap<(u32, u32), Vec<u32>> {
        let mut map: HashMap<(u32, u32), Vec<u32>> = HashMap::with_capacity(self.tris.len() * 3 / 2);
        for (i, t) in self.tris.iter().enumerate() {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                map.entry((a.min(b), a.max(b))).or_default().push(i as u32);
            }
        }
        map
    }

    /// Signed volume of a set of triangles (positive when the normals point out).
    pub fn signed_volume(&self, tris: impl Iterator<Item = usize>) -> f64 {
        let mut v = 0.0;
        for i in tris {
            let [a, b, c] = self.tri(i);
            v += dot(a, cross(b, c)) / 6.0;
        }
        v
    }
}

// ----------------------------------------------------------------------
// Ray casts
// ----------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Node {
    lo: [f32; 3],
    hi: [f32; 3],
    /// Inner node: the two children. Leaf: start and count in `order`.
    a: u32,
    b: u32,
    leaf: bool,
}

/// A bounding-volume tree over the triangles of a mesh, for ray casts.
pub struct Bvh {
    nodes: Vec<Node>,
    order: Vec<u32>,
}

pub struct Hit {
    pub t: f64,
    pub tri: u32,
}

impl Bvh {
    pub fn build(mesh: &Mesh) -> Bvh {
        let n = mesh.tris.len();
        let mut order: Vec<u32> = (0..n as u32).collect();
        let cents: Vec<[f32; 3]> = (0..n)
            .map(|i| {
                let c = mesh.centroid(i);
                [c[0] as f32, c[1] as f32, c[2] as f32]
            })
            .collect();
        let boxes: Vec<([f32; 3], [f32; 3])> = mesh
            .tris
            .iter()
            .map(|t| {
                let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                for &v in t {
                    let p = mesh.positions[v as usize];
                    for k in 0..3 {
                        lo[k] = lo[k].min(p[k]);
                        hi[k] = hi[k].max(p[k]);
                    }
                }
                (lo, hi)
            })
            .collect();
        let mut nodes: Vec<Node> = Vec::with_capacity(n / 2 + 1);
        // (node index, start, end)
        let mut stack: Vec<(usize, usize, usize)> = Vec::new();
        nodes.push(Node { lo: [0.0; 3], hi: [0.0; 3], a: 0, b: 0, leaf: true });
        stack.push((0, 0, n));
        while let Some((ni, start, end)) = stack.pop() {
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            let (mut clo, mut chi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for &t in &order[start..end] {
                let (bl, bh) = boxes[t as usize];
                let c = cents[t as usize];
                for k in 0..3 {
                    lo[k] = lo[k].min(bl[k]);
                    hi[k] = hi[k].max(bh[k]);
                    clo[k] = clo[k].min(c[k]);
                    chi[k] = chi[k].max(c[k]);
                }
            }
            let count = end - start;
            let ext = [chi[0] - clo[0], chi[1] - clo[1], chi[2] - clo[2]];
            let axis = if ext[0] >= ext[1] && ext[0] >= ext[2] { 0 } else if ext[1] >= ext[2] { 1 } else { 2 };
            if count <= 6 || ext[axis] <= 0.0 {
                nodes[ni] = Node { lo, hi, a: start as u32, b: count as u32, leaf: true };
                continue;
            }
            let mid = start + count / 2;
            order[start..end].select_nth_unstable_by(count / 2, |x, y| {
                cents[*x as usize][axis].partial_cmp(&cents[*y as usize][axis]).unwrap_or(std::cmp::Ordering::Equal)
            });
            let (left, right) = (nodes.len(), nodes.len() + 1);
            nodes.push(Node { lo: [0.0; 3], hi: [0.0; 3], a: 0, b: 0, leaf: true });
            nodes.push(Node { lo: [0.0; 3], hi: [0.0; 3], a: 0, b: 0, leaf: true });
            nodes[ni] = Node { lo, hi, a: left as u32, b: right as u32, leaf: false };
            stack.push((left, start, mid));
            stack.push((right, mid, end));
        }
        Bvh { nodes, order }
    }

    /// Calls `f` for every triangle that the ray meets after `tmin`, in no
    /// order. Use it to count crossings; `ray` gives only the nearest one.
    pub fn ray_each(&self, mesh: &Mesh, origin: V3, dir: V3, tmin: f64, mut f: impl FnMut(f64, u32)) {
        if self.order.is_empty() {
            return;
        }
        let inv = [1.0 / dir[0], 1.0 / dir[1], 1.0 / dir[2]];
        let mut stack: Vec<u32> = vec![0];
        while let Some(ni) = stack.pop() {
            let node = &self.nodes[ni as usize];
            let (mut t0, mut t1) = (tmin, f64::MAX);
            let mut miss = false;
            for k in 0..3 {
                let (lo, hi) = (node.lo[k] as f64 - 1e-7, node.hi[k] as f64 + 1e-7);
                if dir[k] == 0.0 {
                    if origin[k] < lo || origin[k] > hi {
                        miss = true;
                        break;
                    }
                    continue;
                }
                let (mut a, mut b) = ((lo - origin[k]) * inv[k], (hi - origin[k]) * inv[k]);
                if a > b {
                    std::mem::swap(&mut a, &mut b);
                }
                t0 = t0.max(a);
                t1 = t1.min(b);
                if t0 > t1 {
                    miss = true;
                    break;
                }
            }
            if miss {
                continue;
            }
            if !node.leaf {
                stack.push(node.a);
                stack.push(node.b);
                continue;
            }
            for &t in &self.order[node.a as usize..(node.a + node.b) as usize] {
                let [p0, p1, p2] = mesh.tri(t as usize);
                let (e1, e2) = (sub(p1, p0), sub(p2, p0));
                let h = cross(dir, e2);
                let det = dot(e1, h);
                if det.abs() < 1e-14 {
                    continue;
                }
                let inv_det = 1.0 / det;
                let s = sub(origin, p0);
                let u = inv_det * dot(s, h);
                if !(0.0..=1.0).contains(&u) {
                    continue;
                }
                let q = cross(s, e1);
                let v = inv_det * dot(dir, q);
                if v < 0.0 || u + v > 1.0 {
                    continue;
                }
                let tt = inv_det * dot(e2, q);
                if tt > tmin {
                    f(tt, t);
                }
            }
        }
    }

    /// The nearest triangle that the ray meets between `tmin` and `tmax`.
    /// `skip` is a triangle to ignore (the one that the ray starts on).
    pub fn ray(&self, mesh: &Mesh, origin: V3, dir: V3, tmin: f64, tmax: f64, skip: u32) -> Option<Hit> {
        if self.order.is_empty() {
            return None;
        }
        let inv = [1.0 / dir[0], 1.0 / dir[1], 1.0 / dir[2]];
        let mut best: Option<Hit> = None;
        let mut limit = tmax;
        let mut stack = [0u32; 96];
        let mut sp = 1;
        while sp > 0 {
            sp -= 1;
            let node = &self.nodes[stack[sp] as usize];
            // Slab test.
            let (mut t0, mut t1) = (tmin, limit);
            let mut miss = false;
            for k in 0..3 {
                let (lo, hi) = (node.lo[k] as f64 - 1e-7, node.hi[k] as f64 + 1e-7);
                if dir[k] == 0.0 {
                    if origin[k] < lo || origin[k] > hi {
                        miss = true;
                        break;
                    }
                    continue;
                }
                let (mut a, mut b) = ((lo - origin[k]) * inv[k], (hi - origin[k]) * inv[k]);
                if a > b {
                    std::mem::swap(&mut a, &mut b);
                }
                t0 = t0.max(a);
                t1 = t1.min(b);
                if t0 > t1 {
                    miss = true;
                    break;
                }
            }
            if miss {
                continue;
            }
            if node.leaf {
                for &t in &self.order[node.a as usize..(node.a + node.b) as usize] {
                    if t == skip {
                        continue;
                    }
                    let [p0, p1, p2] = mesh.tri(t as usize);
                    let (e1, e2) = (sub(p1, p0), sub(p2, p0));
                    let h = cross(dir, e2);
                    let det = dot(e1, h);
                    if det.abs() < 1e-14 {
                        continue;
                    }
                    let f = 1.0 / det;
                    let s = sub(origin, p0);
                    let u = f * dot(s, h);
                    if !(-1e-9..=1.0 + 1e-9).contains(&u) {
                        continue;
                    }
                    let q = cross(s, e1);
                    let v = f * dot(dir, q);
                    if v < -1e-9 || u + v > 1.0 + 1e-9 {
                        continue;
                    }
                    let tt = f * dot(e2, q);
                    if tt > tmin && tt < limit {
                        limit = tt;
                        best = Some(Hit { t: tt, tri: t });
                    }
                }
            } else if sp + 2 <= stack.len() {
                stack[sp] = node.a;
                stack[sp + 1] = node.b;
                sp += 2;
            }
        }
        best
    }
}

// ----------------------------------------------------------------------
// Readers
// ----------------------------------------------------------------------

fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Triangles of an STL file, binary or text, as loose points.
pub fn parse_stl(bytes: &[u8]) -> Result<Vec<[f32; 3]>> {
    let binary_count = if bytes.len() >= 84 { Some(u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize) } else { None };
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_lowercase();
    let looks_text = head.trim_start().starts_with("solid") && (head.contains("facet") || head.contains("endsolid"));
    if let Some(n) = binary_count {
        // A binary file has exactly this size. Some writers start a binary file with "solid".
        if 84 + n * 50 == bytes.len() || (!looks_text && n > 0 && 84 + n * 50 <= bytes.len()) {
            if n > MAX_TRIANGLES {
                bail!("The model has {n} triangles; the limit is {MAX_TRIANGLES}. Export it with a coarser mesh.");
            }
            let mut out = Vec::with_capacity(n * 3);
            for i in 0..n {
                let at = 84 + i * 50 + 12;
                for k in 0..3 {
                    let p = at + k * 12;
                    out.push([f32_at(bytes, p), f32_at(bytes, p + 4), f32_at(bytes, p + 8)]);
                }
            }
            return Ok(out);
        }
    }
    if !looks_text {
        bail!("The file is not a valid STL: it is not a text STL, and its size does not match its triangle count.");
    }
    let text = String::from_utf8_lossy(bytes);
    let mut out: Vec<[f32; 3]> = Vec::new();
    let mut words = text.split_ascii_whitespace();
    while let Some(w) = words.next() {
        if w.eq_ignore_ascii_case("vertex") {
            let mut p = [0f32; 3];
            for v in p.iter_mut() {
                *v = words
                    .next()
                    .and_then(|x| x.parse::<f32>().ok())
                    .ok_or_else(|| anyhow!("The text STL has a vertex without three numbers."))?;
            }
            out.push(p);
            if out.len() > MAX_TRIANGLES * 3 {
                bail!("The model has more than {MAX_TRIANGLES} triangles. Export it with a coarser mesh.");
            }
        }
    }
    if out.len() % 3 != 0 {
        out.truncate(out.len() - out.len() % 3);
    }
    Ok(out)
}

/// Triangles of a Wavefront OBJ file. Faces with more than three corners are
/// split into a fan.
pub fn parse_obj(text: &str) -> Result<Vec<[f32; 3]>> {
    let mut verts: Vec<[f32; 3]> = Vec::new();
    let mut out: Vec<[f32; 3]> = Vec::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("v") => {
                let nums: Vec<f32> = parts.take(3).filter_map(|x| x.parse().ok()).collect();
                if nums.len() == 3 {
                    verts.push([nums[0], nums[1], nums[2]]);
                }
            }
            Some("f") => {
                let mut idx: Vec<usize> = Vec::new();
                for corner in parts {
                    let first = corner.split('/').next().unwrap_or("");
                    let i: i64 = first.parse().map_err(|_| anyhow!("The OBJ file has a face corner that is not a number: \"{corner}\"."))?;
                    let at = if i < 0 { verts.len() as i64 + i } else { i - 1 };
                    if at < 0 || at as usize >= verts.len() {
                        bail!("The OBJ file has a face that points at vertex {i}, which does not exist.");
                    }
                    idx.push(at as usize);
                }
                for k in 1..idx.len().saturating_sub(1) {
                    out.extend([verts[idx[0]], verts[idx[k]], verts[idx[k + 1]]]);
                }
                if out.len() > MAX_TRIANGLES * 3 {
                    bail!("The model has more than {MAX_TRIANGLES} triangles. Export it with a coarser mesh.");
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

// --- A small XML tag reader (3MF models are plain, machine-written XML) ---

pub(crate) struct Tag<'a> {
    /// The name without a namespace prefix.
    pub name: &'a str,
    pub closing: bool,
    pub self_closing: bool,
    body: &'a str,
}

impl<'a> Tag<'a> {
    /// The value of an attribute. A namespace prefix on the attribute is ignored.
    pub fn attr(&self, key: &str) -> Option<String> {
        let b = self.body.as_bytes();
        let mut i = 0;
        while i < b.len() {
            while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
                i += 1;
            }
            let start = i;
            while i < b.len() && b[i] != b'=' && !b[i].is_ascii_whitespace() {
                i += 1;
            }
            let name = &self.body[start..i];
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= b.len() || b[i] != b'=' {
                continue;
            }
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= b.len() || (b[i] != b'"' && b[i] != b'\'') {
                continue;
            }
            let quote = b[i];
            i += 1;
            let vstart = i;
            while i < b.len() && b[i] != quote {
                i += 1;
            }
            let value = &self.body[vstart..i.min(b.len())];
            i += 1;
            let local = name.rsplit(':').next().unwrap_or(name);
            if local == key {
                return Some(
                    value
                        .replace("&quot;", "\"")
                        .replace("&apos;", "'")
                        .replace("&lt;", "<")
                        .replace("&gt;", ">")
                        .replace("&amp;", "&"),
                );
            }
        }
        None
    }
}

/// Calls `f` for each element tag of an XML text, in order.
pub(crate) fn each_tag<'a>(xml: &'a str, mut f: impl FnMut(Tag<'a>) -> Result<()>) -> Result<()> {
    let b = xml.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        if xml[i..].starts_with("<!--") {
            i = xml[i..].find("-->").map(|e| i + e + 3).unwrap_or(b.len());
            continue;
        }
        if xml[i..].starts_with("<?") || xml[i..].starts_with("<!") {
            i = xml[i..].find('>').map(|e| i + e + 1).unwrap_or(b.len());
            continue;
        }
        // Find the end of the tag; a ">" inside quotes does not end it.
        let mut j = i + 1;
        let mut quote = 0u8;
        while j < b.len() {
            if quote != 0 {
                if b[j] == quote {
                    quote = 0;
                }
            } else if b[j] == b'"' || b[j] == b'\'' {
                quote = b[j];
            } else if b[j] == b'>' {
                break;
            }
            j += 1;
        }
        if j >= b.len() {
            break;
        }
        let inner = &xml[i + 1..j];
        let closing = inner.starts_with('/');
        let inner = inner.trim_start_matches('/');
        let self_closing = inner.ends_with('/');
        let name_end = inner.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(inner.len());
        let full = &inner[..name_end];
        let name = full.rsplit(':').next().unwrap_or(full);
        f(Tag { name, closing, self_closing, body: &inner[name_end..] })?;
        i = j + 1;
    }
    Ok(())
}

/// A 4 x 3 transform of 3MF: the point is a row vector, the last row moves it.
type Xf = [f64; 12];
const IDENTITY: Xf = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];

fn parse_xf(s: Option<String>) -> Xf {
    let Some(s) = s else { return IDENTITY };
    let nums: Vec<f64> = s.split_whitespace().filter_map(|x| x.parse().ok()).collect();
    if nums.len() == 12 {
        let mut m = IDENTITY;
        m.copy_from_slice(&nums);
        m
    } else {
        IDENTITY
    }
}

fn xf_point(m: &Xf, p: [f32; 3]) -> [f32; 3] {
    let (x, y, z) = (p[0] as f64, p[1] as f64, p[2] as f64);
    [
        (x * m[0] + y * m[3] + z * m[6] + m[9]) as f32,
        (x * m[1] + y * m[4] + z * m[7] + m[10]) as f32,
        (x * m[2] + y * m[5] + z * m[8] + m[11]) as f32,
    ]
}

/// First `a`, then `b`.
fn xf_then(a: &Xf, b: &Xf) -> Xf {
    let mut m = [0.0; 12];
    for r in 0..4 {
        for c in 0..3 {
            let mut v = 0.0;
            for k in 0..3 {
                v += a[r * 3 + k] * b[k * 3 + c];
            }
            if r == 3 {
                v += b[9 + c];
            }
            m[r * 3 + c] = v;
        }
    }
    m
}

#[derive(Default)]
struct Obj3mf {
    verts: Vec<[f32; 3]>,
    tris: Vec<[u32; 3]>,
    /// (model file of the part, object ID, transform)
    components: Vec<(Option<String>, String, Xf)>,
    printable_type: bool,
}

#[derive(Default)]
struct Model3mf {
    objects: HashMap<String, Obj3mf>,
    /// (model file, object ID, transform)
    build: Vec<(Option<String>, String, Xf)>,
    unit: f64,
}

fn parse_3mf_model(xml: &str) -> Result<Model3mf> {
    let mut m = Model3mf { unit: 1.0, ..Default::default() };
    let mut current: Option<String> = None;
    let mut in_build = false;
    each_tag(xml, |t| {
        if t.closing {
            match t.name {
                "object" => current = None,
                "build" => in_build = false,
                _ => {}
            }
            return Ok(());
        }
        match t.name {
            "model" => {
                m.unit = match t.attr("unit").as_deref().unwrap_or("millimeter") {
                    "micron" => 0.001,
                    "centimeter" => 10.0,
                    "inch" => 25.4,
                    "foot" => 304.8,
                    "meter" => 1000.0,
                    _ => 1.0,
                };
            }
            "object" => {
                let id = t.attr("id").unwrap_or_default();
                let kind = t.attr("type").unwrap_or_else(|| "model".into());
                m.objects.insert(id.clone(), Obj3mf { printable_type: kind == "model" || kind == "solidsupport", ..Default::default() });
                current = if t.self_closing { None } else { Some(id) };
            }
            "vertex" => {
                if let Some(o) = current.as_ref().and_then(|id| m.objects.get_mut(id)) {
                    let num = |k: &str| t.attr(k).and_then(|v| v.trim().parse::<f32>().ok());
                    match (num("x"), num("y"), num("z")) {
                        (Some(x), Some(y), Some(z)) => o.verts.push([x, y, z]),
                        _ => bail!("The 3MF file has a vertex without x, y and z."),
                    }
                }
            }
            "triangle" => {
                if let Some(o) = current.as_ref().and_then(|id| m.objects.get_mut(id)) {
                    let num = |k: &str| t.attr(k).and_then(|v| v.trim().parse::<u32>().ok());
                    match (num("v1"), num("v2"), num("v3")) {
                        (Some(a), Some(b), Some(c)) => o.tris.push([a, b, c]),
                        _ => bail!("The 3MF file has a triangle without v1, v2 and v3."),
                    }
                }
            }
            "component" => {
                if let Some(o) = current.as_ref().and_then(|id| m.objects.get_mut(id)) {
                    if let Some(id) = t.attr("objectid") {
                        o.components.push((t.attr("path"), id, parse_xf(t.attr("transform"))));
                    }
                }
            }
            "build" => in_build = !t.self_closing,
            "item" if in_build => {
                if t.attr("printable").as_deref() != Some("0") {
                    if let Some(id) = t.attr("objectid") {
                        m.build.push((t.attr("path"), id, parse_xf(t.attr("transform"))));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    Ok(m)
}

fn zip_text<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>, name: &str) -> Option<String> {
    let name = name.trim_start_matches('/');
    let mut f = zip.by_name(name).ok()?;
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    Some(s)
}

/// Triangles of a 3MF file: the items of the build, with their transforms, one
/// list for each mesh. Parts in separate model files (the production extension
/// that Bambu Studio and OrcaSlicer write) are followed.
pub fn parse_3mf(bytes: &[u8]) -> Result<Vec<Vec<[f32; 3]>>> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| anyhow!("The file is not a valid 3MF (it is not a zip archive)."))?;
    // The package relationships name the main model; most writers use 3D/3dmodel.model.
    let mut root = "3D/3dmodel.model".to_string();
    if let Some(rels) = zip_text(&mut zip, "_rels/.rels") {
        each_tag(&rels, |t| {
            if t.name == "Relationship" && t.attr("Type").map(|x| x.ends_with("/3dmodel")).unwrap_or(false) {
                if let Some(target) = t.attr("Target") {
                    root = target.trim_start_matches('/').to_string();
                }
            }
            Ok(())
        })?;
    }
    let mut models: HashMap<String, Model3mf> = HashMap::new();
    let load = |zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>, models: &mut HashMap<String, Model3mf>, path: &str| -> Result<()> {
        if !models.contains_key(path) {
            let xml = zip_text(zip, path).ok_or_else(|| anyhow!("The 3MF file has no model part \"{path}\"."))?;
            models.insert(path.to_string(), parse_3mf_model(&xml)?);
        }
        Ok(())
    };
    load(&mut zip, &mut models, &root)?;
    let unit = models[&root].unit;
    let mut groups: Vec<Vec<[f32; 3]>> = Vec::new();
    let mut count = 0usize;
    // (model file, object ID, transform, depth)
    let mut todo: Vec<(String, String, Xf, usize)> = Vec::new();
    {
        let m = &models[&root];
        if m.build.is_empty() {
            for (id, o) in &m.objects {
                if o.printable_type && !o.tris.is_empty() {
                    todo.push((root.clone(), id.clone(), IDENTITY, 0));
                }
            }
        } else {
            for (path, id, xf) in &m.build {
                todo.push((path.clone().map(|p| p.trim_start_matches('/').to_string()).unwrap_or_else(|| root.clone()), id.clone(), *xf, 0));
            }
        }
    }
    while let Some((path, id, xf, depth)) = todo.pop() {
        if depth > 8 {
            bail!("The 3MF file nests its parts too deep.");
        }
        load(&mut zip, &mut models, &path)?;
        let Some(o) = models[&path].objects.get(&id) else { continue };
        let mut out: Vec<[f32; 3]> = Vec::with_capacity(o.tris.len() * 3);
        for t in &o.tris {
            for &v in t {
                let p = *o.verts.get(v as usize).ok_or_else(|| anyhow!("The 3MF file has a triangle that points at a missing vertex."))?;
                let q = xf_point(&xf, p);
                out.push([(q[0] as f64 * unit) as f32, (q[1] as f64 * unit) as f32, (q[2] as f64 * unit) as f32]);
            }
        }
        count += out.len();
        if count > MAX_TRIANGLES * 3 {
            bail!("The model has more than {MAX_TRIANGLES} triangles. Export it with a coarser mesh.");
        }
        if !out.is_empty() {
            groups.push(out);
        }
        for (cpath, cid, cxf) in &o.components {
            let next = cpath.clone().map(|p| p.trim_start_matches('/').to_string()).unwrap_or_else(|| path.clone());
            todo.push((next, cid.clone(), xf_then(cxf, &xf), depth + 1));
        }
    }
    Ok(groups)
}

// --- STEP through FreeCAD ---

const STEP_SCRIPT: &str = r#"import FreeCAD
import os
import Part, MeshPart

src = os.environ["CORY_STEP_IN"]
dst = os.environ["CORY_STL_DIR"]
try:
    shape = Part.Shape()
    shape.read(src)
    if shape.isNull():
        raise RuntimeError("The STEP file has no shape.")
    box = shape.BoundBox
    deflection = max(0.01, min(0.1, box.DiagonalLength * 0.0004))
    # One mesh for each solid, so that bodies that touch stay separate.
    parts = shape.Solids if len(shape.Solids) > 0 else [shape]
    count = 0
    facets = 0
    for part in parts:
        mesh = MeshPart.meshFromShape(Shape=part, LinearDeflection=deflection, AngularDeflection=0.3, Relative=False)
        if mesh.CountFacets == 0:
            continue
        mesh.write(os.path.join(dst, "part_%04d.stl" % count))
        count += 1
        facets += mesh.CountFacets
    if count == 0:
        raise RuntimeError("The STEP file gave no triangles.")
    FreeCAD.Console.PrintMessage("CORY_OK %d %d\n" % (count, facets))
except Exception as e:
    FreeCAD.Console.PrintMessage("CORY_ERR %s\n" % str(e).encode("ascii", "replace").decode("ascii"))
"#;

/// The FreeCAD console program, if FreeCAD is installed. CORYDORAS_FREECADCMD
/// names another path.
pub fn freecad_cmd() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CORYDORAS_FREECADCMD").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    let mut candidates: Vec<PathBuf> = vec![
        PathBuf::from("/Applications/FreeCAD.app/Contents/Resources/bin/freecadcmd"),
        PathBuf::from("/Applications/FreeCAD.app/Contents/MacOS/FreeCADCmd"),
    ];
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join("Applications/FreeCAD.app/Contents/Resources/bin/freecadcmd"));
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            for name in ["freecadcmd", "FreeCADCmd"] {
                candidates.push(dir.join(name));
            }
        }
    }
    for dir in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        candidates.push(Path::new(dir).join("freecadcmd"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Converts a STEP file to triangles: one list for each solid. FreeCAD runs
/// in a temporary folder with its own empty settings, so a FreeCAD that you
/// have open is not touched.
fn step_to_groups(path: &Path) -> Result<Vec<Vec<[f32; 3]>>> {
    let cmd = freecad_cmd().ok_or_else(|| {
        anyhow!("Corydoras reads STEP files through FreeCAD, and FreeCAD is not installed. Install FreeCAD (free, freecad.org), or export the part as STL or 3MF.")
    })?;
    let dir = std::env::temp_dir().join(format!("corydoras-step-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(dir.join("home"))?;
    let result = (|| -> Result<Vec<Vec<[f32; 3]>>> {
        let script = dir.join("cory_step_to_stl.py");
        std::fs::write(&script, STEP_SCRIPT)?;
        let out = dir.join("parts");
        std::fs::create_dir_all(&out)?;
        let log = dir.join("log.txt");
        let mut child = std::process::Command::new(&cmd)
            .arg(&script)
            .current_dir(&dir)
            .env("CORY_STEP_IN", path)
            .env("CORY_STL_DIR", &out)
            .env("HOME", dir.join("home"))
            .stdin(std::process::Stdio::null())
            .stdout(std::fs::File::create(&log)?)
            .stderr(std::process::Stdio::null())
            .spawn()
            .with_context(|| format!("Cannot start {}", cmd.display()))?;
        let start = std::time::Instant::now();
        loop {
            match child.try_wait()? {
                Some(_) => break,
                None if start.elapsed().as_secs() > STEP_TIMEOUT_SECS => {
                    let _ = child.kill();
                    let _ = child.wait();
                    bail!("FreeCAD did not finish the STEP file in {STEP_TIMEOUT_SECS} s. Export the part as STL or 3MF.");
                }
                None => std::thread::sleep(std::time::Duration::from_millis(60)),
            }
        }
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        if let Some(line) = text.lines().find(|l| l.contains("CORY_ERR")) {
            bail!("FreeCAD could not read the STEP file: {}", line.split("CORY_ERR").nth(1).unwrap_or("").trim());
        }
        if !text.contains("CORY_OK") {
            bail!("FreeCAD could not convert the STEP file. Export the part as STL or 3MF.");
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(&out)?.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == "stl").unwrap_or(false)).collect();
        files.sort();
        let mut groups = Vec::new();
        for f in files {
            groups.push(parse_stl(&std::fs::read(&f)?)?);
        }
        Ok(groups)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// Reads a model file. The type comes from the file name.
pub fn load(path: &Path) -> Result<Loaded> {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let ext = Path::new(&name).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    if !MODEL_EXTENSIONS.contains(&ext.as_str()) {
        bail!("\"{name}\" is not a model type that Corydoras reads. Use STL, 3MF, OBJ or STEP.");
    }
    let meta = std::fs::metadata(path).with_context(|| format!("Cannot read {}", path.display()))?;
    if meta.len() > MAX_MODEL_BYTES {
        bail!("The model file is larger than {} MB.", MAX_MODEL_BYTES / 1024 / 1024);
    }
    let mut notes = Vec::new();
    let (groups, format) = match ext.as_str() {
        "stl" => (vec![parse_stl(&std::fs::read(path)?)?], "stl"),
        "3mf" => (parse_3mf(&std::fs::read(path)?)?, "3mf"),
        "obj" => (vec![parse_obj(&String::from_utf8_lossy(&std::fs::read(path)?))?], "obj"),
        _ => {
            notes.push("FreeCAD converted the STEP file to triangles; curved faces are approximated within about 0.05 mm.".to_string());
            (step_to_groups(path)?, "step")
        }
    };
    if groups.iter().all(|g| g.is_empty()) {
        bail!("\"{name}\" has no triangles.");
    }
    if groups.iter().flatten().any(|p| p.iter().any(|v| !v.is_finite())) {
        bail!("\"{name}\" has coordinates that are not numbers.");
    }
    let refs: Vec<&[[f32; 3]]> = groups.iter().map(|g| g.as_slice()).collect();
    let mesh = Mesh::from_groups(&refs);
    if mesh.tris.is_empty() {
        bail!("\"{name}\" has no triangles with an area.");
    }
    Ok(Loaded { mesh, format: format.to_string(), notes })
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A closed box as loose triangles, with normals that point out.
    pub fn box_soup(lo: [f32; 3], hi: [f32; 3]) -> Vec<[f32; 3]> {
        let p = |x: u8, y: u8, z: u8| [if x == 0 { lo[0] } else { hi[0] }, if y == 0 { lo[1] } else { hi[1] }, if z == 0 { lo[2] } else { hi[2] }];
        let quads = [
            [p(0, 0, 0), p(0, 1, 0), p(1, 1, 0), p(1, 0, 0)], // bottom, normal -z
            [p(0, 0, 1), p(1, 0, 1), p(1, 1, 1), p(0, 1, 1)], // top, +z
            [p(0, 0, 0), p(1, 0, 0), p(1, 0, 1), p(0, 0, 1)], // front, -y
            [p(0, 1, 0), p(0, 1, 1), p(1, 1, 1), p(1, 1, 0)], // back, +y
            [p(0, 0, 0), p(0, 0, 1), p(0, 1, 1), p(0, 1, 0)], // left, -x
            [p(1, 0, 0), p(1, 1, 0), p(1, 1, 1), p(1, 0, 1)], // right, +x
        ];
        let mut out = Vec::new();
        for q in quads {
            out.extend([q[0], q[1], q[2], q[0], q[2], q[3]]);
        }
        out
    }

    /// The same triangles with the opposite winding (normals point in).
    pub fn flipped(soup: &[[f32; 3]]) -> Vec<[f32; 3]> {
        soup.chunks_exact(3).flat_map(|t| [t[0], t[2], t[1]]).collect()
    }

    pub fn binary_stl(soup: &[[f32; 3]]) -> Vec<u8> {
        let mut out = vec![0u8; 80];
        out.extend(((soup.len() / 3) as u32).to_le_bytes());
        for t in soup.chunks_exact(3) {
            out.extend([0u8; 12]);
            for p in t {
                for v in p {
                    out.extend(v.to_le_bytes());
                }
            }
            out.extend([0u8; 2]);
        }
        out
    }

    #[test]
    fn stl_binary_and_text() {
        let soup = box_soup([0.0; 3], [10.0, 20.0, 30.0]);
        let mesh = Mesh::from_soup(&parse_stl(&binary_stl(&soup)).unwrap());
        assert_eq!(mesh.tris.len(), 12);
        assert_eq!(mesh.positions.len(), 8);
        assert!((mesh.signed_volume(0..12) - 6000.0).abs() < 1e-6);
        assert!((mesh.area() - 2.0 * (200.0 + 300.0 + 600.0)).abs() < 1e-6);
        let (_, parts) = mesh.components();
        assert_eq!(parts, 1);
        assert!(mesh.edge_faces().values().all(|f| f.len() == 2));

        // Two boxes that share a face stay two bodies when they come as two groups,
        // and points a hair apart become one vertex.
        let (a, b) = (box_soup([0.0; 3], [1.0, 1.0, 1.0]), box_soup([1.0, 0.0, 0.0], [2.0, 1.0, 1.0]));
        assert_eq!(Mesh::from_groups(&[&a, &b]).components().1, 2);
        let mut joined = a.clone();
        joined.extend(b.iter().copied());
        assert_eq!(Mesh::from_soup(&joined).components().1, 1);
        let mut hair = a.clone();
        hair[0][0] += 1e-7;
        assert_eq!(Mesh::from_soup(&hair).positions.len(), 8);

        let text = "solid a\n facet normal 0 0 1\n  outer loop\n   vertex 0 0 0\n   vertex 1 0 0\n   vertex 0 1 0\n  endloop\n endfacet\nendsolid a\n";
        assert_eq!(parse_stl(text.as_bytes()).unwrap().len(), 3);
        assert!(parse_stl(b"not an stl at all").is_err());
    }

    #[test]
    fn obj_faces() {
        let soup = parse_obj("v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\nf -4/1/1 -3/2/1 -2/3/1\n").unwrap();
        assert_eq!(soup.len(), 9);
        assert!(parse_obj("v 0 0 0\nf 1 2 3\n").is_err());
    }

    pub fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
        use std::io::Write;
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, text) in files {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(text.as_bytes()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn threemf_with_components() {
        let part = r#"<?xml version="1.0"?><model unit="millimeter" xmlns="x"><resources>
            <object id="1" type="model"><mesh><vertices>
              <vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/><vertex x="0" y="0" z="1"/>
            </vertices><triangles>
              <triangle v1="0" v2="2" v3="1"/><triangle v1="0" v2="1" v3="3"/><triangle v1="0" v2="3" v3="2"/><triangle v1="1" v2="2" v3="3"/>
            </triangles></mesh></object></resources></model>"#;
        let root = r#"<model unit="centimeter" xmlns:p="prod"><resources>
            <object id="2" type="model"><components>
              <component p:path="/3D/Objects/part.model" objectid="1" transform="1 0 0 0 1 0 0 0 1 5 0 0"/>
              <component p:path="/3D/Objects/part.model" objectid="1"/>
            </components></object></resources>
            <build><item objectid="2" transform="1 0 0 0 1 0 0 0 1 0 0 2"/><item objectid="2" printable="0"/></build></model>"#;
        let rels = r#"<Relationships><Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>"#;
        let bytes = zip_of(&[("_rels/.rels", rels), ("3D/3dmodel.model", root), ("3D/Objects/part.model", part)]);
        let groups = parse_3mf(&bytes).unwrap();
        assert_eq!(groups.len(), 2, "one mesh for each placed part");
        let refs: Vec<&[[f32; 3]]> = groups.iter().map(|g| g.as_slice()).collect();
        let mesh = Mesh::from_groups(&refs);
        assert_eq!(mesh.tris.len(), 8);
        assert_eq!(mesh.components().1, 2);
        let (lo, hi) = mesh.bounds();
        // Centimetres become millimetres; the transforms add up: x 0..60, z 20..30.
        assert!((lo[0] - 0.0).abs() < 1e-4 && (hi[0] - 60.0).abs() < 1e-4, "{lo:?} {hi:?}");
        assert!((lo[2] - 20.0).abs() < 1e-4 && (hi[2] - 30.0).abs() < 1e-4);
        assert!(parse_3mf(b"nope").is_err());
    }

    #[test]
    fn rays_and_orientation() {
        let mesh = Mesh::from_soup(&box_soup([0.0; 3], [10.0, 20.0, 30.0]));
        let bvh = Bvh::build(&mesh);
        let hit = bvh.ray(&mesh, [5.0, 5.0, -10.0], [0.0, 0.0, 1.0], 1e-9, f64::MAX, u32::MAX).unwrap();
        assert!((hit.t - 10.0).abs() < 1e-9);
        assert!(bvh.ray(&mesh, [50.0, 5.0, -10.0], [0.0, 0.0, 1.0], 1e-9, f64::MAX, u32::MAX).is_none());
        // +y up: the 20 mm side becomes the height.
        let turned = mesh.clone().oriented(Up::PosY);
        let (lo, hi) = turned.bounds();
        assert!((hi[2] - lo[2] - 20.0).abs() < 1e-6);
        assert!((turned.signed_volume(0..12) - 6000.0).abs() < 1e-6, "a rotation keeps the volume sign");
        assert_eq!(Up::parse("-x").unwrap(), Up::NegX);
        assert!(Up::parse("up").is_err());
        assert!(is_model_name("Part.STEP") && !is_model_name("part.png"));
    }
}
