//! A small software renderer: draws a mesh from four directions into one
//! image, so that a 3D model can be a reference on the board like a picture.
//!
//! The render is neutral clay on a light background, without perspective. It
//! shows form, proportion and edges; it does not show colour or material.

use crate::mesh::{cross, dot, sub, unit, Mesh, V3};
use image::{Rgb, RgbImage};

/// Side of one view of the sheet (pixels), before the sheet is put together.
pub const VIEW_W: u32 = 760;
pub const VIEW_H: u32 = 570;
const GAP: u32 = 12;
const BACKGROUND: [u8; 3] = [246, 245, 242];
/// Soft clay tones; each body of an assembly gets the next one.
const TONES: [[f32; 3]; 4] = [[0.80, 0.80, 0.82], [0.86, 0.79, 0.71], [0.72, 0.80, 0.85], [0.79, 0.84, 0.75]];

/// The four views of the sheet, as the direction from the model to the camera
/// and the direction that is up in the picture.
pub const VIEWS: [(&str, V3, V3); 4] = [
    ("three-quarter front", [1.0, -1.25, 0.8], [0.0, 0.0, 1.0]),
    ("three-quarter back", [-1.0, 1.25, 0.8], [0.0, 0.0, 1.0]),
    ("front", [0.0, -1.0, 0.0], [0.0, 0.0, 1.0]),
    ("top", [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
];

/// What the analysis must know about a rendered sheet.
pub const SHEET_NOTE: &str = "This image is a sheet of four automatic renders of a 3D model: top left a three-quarter view from the front, top right a three-quarter view from the back, bottom left the front, bottom right the top. The renders are neutral grey without perspective. They show form, proportions and edges. They do not show colour, material or finish, so report no colours and no materials for this item.";

/// Draws one view at `w` x `h` pixels.
fn draw_view(mesh: &Mesh, normals: &[(V3, f64)], labels: &[u32], dir: V3, up: V3, w: u32, h: u32) -> RgbImage {
    let zc = unit(dir);
    let xc = unit(cross(up, zc));
    let yc = cross(zc, xc);
    let (lo, hi) = mesh.bounds();
    let centre = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5];
    // Camera coordinates of every vertex.
    let cam: Vec<[f64; 3]> = mesh
        .positions
        .iter()
        .map(|p| {
            let v = sub([p[0] as f64, p[1] as f64, p[2] as f64], centre);
            [dot(v, xc), dot(v, yc), dot(v, zc)]
        })
        .collect();
    let (mut bx, mut by) = (1e-9f64, 1e-9f64);
    let (mut zlo, mut zhi) = (f64::MAX, f64::MIN);
    for c in &cam {
        bx = bx.max(c[0].abs());
        by = by.max(c[1].abs());
        zlo = zlo.min(c[2]);
        zhi = zhi.max(c[2]);
    }
    let s = ((w as f64 * 0.44) / bx).min((h as f64 * 0.44) / by);
    let (cx, cy) = (w as f64 * 0.5, h as f64 * 0.5);
    let n = (w * h) as usize;
    let mut depth = vec![f32::MIN; n];
    let mut shade = vec![0f32; n];
    let mut body = vec![u32::MAX; n];
    let mut facing = vec![[0f32; 3]; n];
    // Key light from the upper left, a weaker fill from the right.
    let (key, fill) = (unit([-0.45, 0.6, 0.66]), unit([0.7, 0.1, 0.45]));
    for (i, t) in mesh.tris.iter().enumerate() {
        let (nrm, area) = normals[i];
        if area <= 0.0 {
            continue;
        }
        let nc = [dot(nrm, xc), dot(nrm, yc), dot(nrm, zc)];
        // The inside of an open shell is drawn darker.
        let lit = if nc[2] >= 0.0 {
            (0.30 + 0.62 * dot(nc, key).max(0.0) + 0.22 * dot(nc, fill).max(0.0)).min(1.0)
        } else {
            0.22 + 0.25 * dot([-nc[0], -nc[1], -nc[2]], key).max(0.0)
        };
        let p: [[f64; 3]; 3] = [cam[t[0] as usize], cam[t[1] as usize], cam[t[2] as usize]];
        let sx = [cx + p[0][0] * s, cx + p[1][0] * s, cx + p[2][0] * s];
        let sy = [cy - p[0][1] * s, cy - p[1][1] * s, cy - p[2][1] * s];
        let det = (sx[1] - sx[0]) * (sy[2] - sy[0]) - (sx[2] - sx[0]) * (sy[1] - sy[0]);
        if det.abs() < 1e-12 {
            continue;
        }
        let x0 = sx.iter().cloned().fold(f64::MAX, f64::min).floor().max(0.0) as u32;
        let x1 = (sx.iter().cloned().fold(f64::MIN, f64::max).ceil().min(w as f64 - 1.0)).max(0.0) as u32;
        let y0 = sy.iter().cloned().fold(f64::MAX, f64::min).floor().max(0.0) as u32;
        let y1 = (sy.iter().cloned().fold(f64::MIN, f64::max).ceil().min(h as f64 - 1.0)).max(0.0) as u32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let b1 = ((px - sx[0]) * (sy[2] - sy[0]) - (sx[2] - sx[0]) * (py - sy[0])) / det;
                let b2 = ((sx[1] - sx[0]) * (py - sy[0]) - (px - sx[0]) * (sy[1] - sy[0])) / det;
                let b0 = 1.0 - b1 - b2;
                if b0 < -1e-9 || b1 < -1e-9 || b2 < -1e-9 {
                    continue;
                }
                let z = (b0 * p[0][2] + b1 * p[1][2] + b2 * p[2][2]) as f32;
                let at = (y * w + x) as usize;
                if z > depth[at] {
                    depth[at] = z;
                    shade[at] = lit as f32;
                    body[at] = labels[i];
                    facing[at] = [nc[0] as f32, nc[1] as f32, nc[2] as f32];
                }
            }
        }
    }
    // Lines at the outline, at creases and where the depth jumps.
    let jump = ((zhi - zlo).max(1e-9) * 0.012) as f32;
    let mut img = RgbImage::from_pixel(w, h, Rgb(BACKGROUND));
    for y in 0..h {
        for x in 0..w {
            let at = (y * w + x) as usize;
            let mut line = 0f32;
            for (dx, dy) in [(1i32, 0i32), (0, 1), (-1, 0), (0, -1)] {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                    continue;
                }
                let o = (ny as u32 * w + nx as u32) as usize;
                let (here, there) = (body[at] != u32::MAX, body[o] != u32::MAX);
                if here != there {
                    line = line.max(1.0);
                } else if here {
                    if body[at] != body[o] || (depth[at] - depth[o]).abs() > jump {
                        line = line.max(0.8);
                    } else {
                        let d = facing[at][0] * facing[o][0] + facing[at][1] * facing[o][1] + facing[at][2] * facing[o][2];
                        if d < 0.82 {
                            line = line.max(0.5);
                        }
                    }
                }
            }
            if body[at] == u32::MAX {
                if line > 0.0 {
                    img.put_pixel(x, y, Rgb([120, 120, 124]));
                }
                continue;
            }
            let tone = TONES[body[at] as usize % TONES.len()];
            let k = shade[at] * (1.0 - 0.62 * line);
            img.put_pixel(x, y, Rgb([(tone[0] * k * 255.0) as u8, (tone[1] * k * 255.0) as u8, (tone[2] * k * 255.0) as u8]));
        }
    }
    img
}

/// The sheet of four views of a mesh whose up direction is +Z.
pub fn sheet(mesh: &Mesh) -> RgbImage {
    let normals: Vec<(V3, f64)> = (0..mesh.tris.len()).map(|i| mesh.normal_area(i)).collect();
    let (labels, _) = mesh.components();
    let (w, h) = (VIEW_W * 2 + GAP * 3, VIEW_H * 2 + GAP * 3);
    let mut out = RgbImage::from_pixel(w, h, Rgb([255, 255, 255]));
    for (i, (_, dir, up)) in VIEWS.iter().enumerate() {
        // Draw at twice the size and scale down: smooth edges without a slow sampler.
        let big = draw_view(mesh, &normals, &labels, *dir, *up, VIEW_W * 2, VIEW_H * 2);
        let view = image::imageops::resize(&big, VIEW_W, VIEW_H, image::imageops::FilterType::Triangle);
        let (col, row) = ((i % 2) as u32, (i / 2) as u32);
        image::imageops::replace(&mut out, &view, (GAP + col * (VIEW_W + GAP)) as i64, (GAP + row * (VIEW_H + GAP)) as i64);
    }
    out
}

/// The sheet as PNG bytes.
pub fn sheet_png(mesh: &Mesh) -> anyhow::Result<Vec<u8>> {
    let mut buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(sheet(mesh)).write_to(&mut buf, image::ImageFormat::Png)?;
    Ok(buf.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::tests::box_soup;

    fn is_background(c: [u8; 3]) -> bool {
        (0..3).all(|k| (c[k] as i32 - BACKGROUND[k] as i32).abs() <= 1)
    }

    #[test]
    fn four_views_of_a_box() {
        let mesh = Mesh::from_soup(&box_soup([0.0; 3], [40.0, 20.0, 10.0]));
        let img = sheet(&mesh);
        assert_eq!((img.width(), img.height()), (VIEW_W * 2 + GAP * 3, VIEW_H * 2 + GAP * 3));
        // Each view has the model in its middle and the background at its corner.
        for (col, row) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let (x0, y0) = (GAP + col * (VIEW_W + GAP), GAP + row * (VIEW_H + GAP));
            let mid = img.get_pixel(x0 + VIEW_W / 2, y0 + VIEW_H / 2).0;
            let corner = img.get_pixel(x0 + 4, y0 + 4).0;
            assert!(is_background(corner), "view {col},{row}: {corner:?}");
            assert!(!is_background(mid), "view {col},{row}: {mid:?}");
        }
        // The top view of a 40 x 20 box is wider than it is high: the model fills
        // the width of its view, and the rows above and below the middle are empty.
        let (x0, y0) = (GAP + VIEW_W + GAP, GAP + VIEW_H + GAP);
        assert!(!is_background(img.get_pixel(x0 + VIEW_W / 8, y0 + VIEW_H / 2).0));
        assert!(is_background(img.get_pixel(x0 + VIEW_W / 2, y0 + VIEW_H / 12).0));
        assert!(sheet_png(&mesh).unwrap().starts_with(&[0x89, b'P', b'N', b'G']));
    }
}
