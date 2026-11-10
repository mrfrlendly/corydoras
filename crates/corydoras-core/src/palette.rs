//! Colour palettes: the main colours of an image, and of a whole board.
//!
//! The colours are grouped in the OKLab space, where equal distances look
//! equally different. The result is the same every time for the same image.

use image::DynamicImage;
use serde::{Deserialize, Serialize};

/// One colour of a palette and the share of the image (or board) that it covers.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Swatch {
    pub hex: String,
    /// 0 to 1.
    pub share: f64,
}

/// Colours in one image palette.
pub const IMAGE_COLOURS: usize = 5;
/// Colours in a board palette.
pub const BOARD_COLOURS: usize = 8;
/// Two groups closer than this look the same and are joined.
const MERGE_DISTANCE: f32 = 0.055;

fn to_linear(c: u8) -> f32 {
    let v = c as f32 / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn to_srgb(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round().clamp(0.0, 255.0) as u8
}

pub fn rgb_to_oklab(rgb: [u8; 3]) -> [f32; 3] {
    let (r, g, b) = (to_linear(rgb[0]), to_linear(rgb[1]), to_linear(rgb[2]));
    let l = (0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

pub fn oklab_to_rgb(lab: [f32; 3]) -> [u8; 3] {
    let l = (lab[0] + 0.396_337_78 * lab[1] + 0.215_803_76 * lab[2]).powi(3);
    let m = (lab[0] - 0.105_561_346 * lab[1] - 0.063_854_17 * lab[2]).powi(3);
    let s = (lab[0] - 0.089_484_18 * lab[1] - 1.291_485_5 * lab[2]).powi(3);
    [
        to_srgb(4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s),
        to_srgb(-1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s),
        to_srgb(-0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s),
    ]
}

pub fn hex_of(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

pub fn parse_hex(hex: &str) -> Option<[u8; 3]> {
    let h = hex.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    Some([u8::from_str_radix(&h[0..2], 16).ok()?, u8::from_str_radix(&h[2..4], 16).ok()?, u8::from_str_radix(&h[4..6], 16).ok()?])
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

/// Groups weighted colours into at most `k` groups. Returns each group's
/// colour and weight, largest first.
fn cluster(points: &[([f32; 3], f32)], k: usize) -> Vec<([f32; 3], f32)> {
    if points.is_empty() {
        return Vec::new();
    }
    // Start with the heaviest point, then take the point farthest from the centres so far.
    let mut centres: Vec<[f32; 3]> = Vec::new();
    let first = points.iter().fold(points[0], |best, p| if p.1 > best.1 { *p } else { best });
    centres.push(first.0);
    while centres.len() < k {
        let mut best = (0f32, points[0].0);
        for p in points {
            let d = centres.iter().map(|c| dist2(*c, p.0)).fold(f32::MAX, f32::min) * p.1.sqrt();
            if d > best.0 {
                best = (d, p.0);
            }
        }
        if best.0 <= 1e-9 {
            break;
        }
        centres.push(best.1);
    }
    let mut weights = vec![0f32; centres.len()];
    for _ in 0..14 {
        let mut sums = vec![[0f32; 3]; centres.len()];
        weights.iter_mut().for_each(|w| *w = 0.0);
        for p in points {
            let mut at = 0;
            let mut best = f32::MAX;
            for (i, c) in centres.iter().enumerate() {
                let d = dist2(*c, p.0);
                if d < best {
                    best = d;
                    at = i;
                }
            }
            for j in 0..3 {
                sums[at][j] += p.0[j] * p.1;
            }
            weights[at] += p.1;
        }
        for (i, c) in centres.iter_mut().enumerate() {
            if weights[i] > 0.0 {
                *c = [sums[i][0] / weights[i], sums[i][1] / weights[i], sums[i][2] / weights[i]];
            }
        }
    }
    let mut groups: Vec<([f32; 3], f32)> = centres.into_iter().zip(weights).filter(|g| g.1 > 0.0).collect();
    // Join groups that look the same.
    let mut i = 0;
    while i < groups.len() {
        let mut j = i + 1;
        while j < groups.len() {
            if dist2(groups[i].0, groups[j].0).sqrt() < MERGE_DISTANCE {
                let (a, b) = (groups[i], groups[j]);
                let w = a.1 + b.1;
                groups[i] = ([(a.0[0] * a.1 + b.0[0] * b.1) / w, (a.0[1] * a.1 + b.0[1] * b.1) / w, (a.0[2] * a.1 + b.0[2] * b.1) / w], w);
                groups.remove(j);
            } else {
                j += 1;
            }
        }
        i += 1;
    }
    groups.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    groups
}

fn swatches(groups: Vec<([f32; 3], f32)>, keep: usize) -> Vec<Swatch> {
    let total: f32 = groups.iter().map(|g| g.1).sum();
    groups
        .into_iter()
        .take(keep)
        .map(|(lab, w)| Swatch { hex: hex_of(oklab_to_rgb(lab)), share: ((w / total.max(1e-9)) as f64 * 1000.0).round() / 1000.0 })
        .collect()
}

/// The main colours of an image, largest share first.
pub fn of_image(img: &DynamicImage) -> Vec<Swatch> {
    let small = img.resize(72, 72, image::imageops::FilterType::Triangle).to_rgba8();
    // Bucket the pixels first: thousands of pixels become a few hundred weighted colours.
    let mut buckets: std::collections::HashMap<[u8; 3], ([f32; 3], f32)> = std::collections::HashMap::new();
    for px in small.pixels() {
        if px.0[3] < 128 {
            continue;
        }
        let rgb = [px.0[0], px.0[1], px.0[2]];
        let key = [rgb[0] >> 3, rgb[1] >> 3, rgb[2] >> 3];
        let lab = rgb_to_oklab(rgb);
        let e = buckets.entry(key).or_insert(([0.0; 3], 0.0));
        for j in 0..3 {
            e.0[j] += lab[j];
        }
        e.1 += 1.0;
    }
    let mut points: Vec<([f32; 3], f32)> = buckets.into_values().map(|(sum, n)| ([sum[0] / n, sum[1] / n, sum[2] / n], n)).collect();
    // A fixed order makes the result the same on every run.
    points.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0[0].partial_cmp(&b.0[0]).unwrap_or(std::cmp::Ordering::Equal)));
    swatches(cluster(&points, IMAGE_COLOURS + 2), IMAGE_COLOURS)
}

/// One palette from many: each input is a palette and its weight.
pub fn merged(palettes: &[(Vec<Swatch>, f64)]) -> Vec<Swatch> {
    let mut points: Vec<([f32; 3], f32)> = Vec::new();
    for (palette, weight) in palettes {
        for s in palette {
            if let Some(rgb) = parse_hex(&s.hex) {
                points.push((rgb_to_oklab(rgb), (s.share * weight) as f32));
            }
        }
    }
    points.retain(|p| p.1 > 0.0);
    swatches(cluster(&points, BOARD_COLOURS + 2), BOARD_COLOURS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_round_trip() {
        for rgb in [[0, 0, 0], [255, 255, 255], [232, 228, 220], [220, 30, 30], [12, 90, 200]] {
            let back = oklab_to_rgb(rgb_to_oklab(rgb));
            for k in 0..3 {
                assert!((back[k] as i32 - rgb[k] as i32).abs() <= 1, "{rgb:?} -> {back:?}");
            }
        }
        assert_eq!(parse_hex("#E8E4DC"), Some([232, 228, 220]));
        assert_eq!(hex_of([232, 228, 220]), "#E8E4DC");
        assert!(parse_hex("red").is_none());
    }

    #[test]
    fn image_palette() {
        // Three quarters warm white, a quarter orange, with a thin dark line.
        let img = image::RgbImage::from_fn(120, 80, |x, y| {
            if y == 40 {
                image::Rgb([20, 20, 22])
            } else if x < 90 {
                image::Rgb([236, 232, 224])
            } else {
                image::Rgb([232, 120, 30])
            }
        });
        let p = of_image(&DynamicImage::ImageRgb8(img.clone()));
        assert!(p.len() >= 2 && p.len() <= IMAGE_COLOURS, "{p:?}");
        let near = |hex: &str, rgb: [i32; 3]| {
            let c = parse_hex(hex).unwrap();
            (0..3).all(|k| (c[k] as i32 - rgb[k]).abs() < 18)
        };
        assert!(near(&p[0].hex, [236, 232, 224]) && p[0].share > 0.6, "{p:?}");
        assert!(near(&p[1].hex, [232, 120, 30]) && p[1].share > 0.15, "{p:?}");
        // The same image gives the same palette.
        assert_eq!(p, of_image(&DynamicImage::ImageRgb8(img)));

        // A board palette joins the same colour from two images and keeps the weights.
        let a = vec![Swatch { hex: "#ECE8E0".into(), share: 0.8 }, Swatch { hex: "#E8781E".into(), share: 0.2 }];
        let b = vec![Swatch { hex: "#EDE9E1".into(), share: 0.5 }, Swatch { hex: "#1E5AC8".into(), share: 0.5 }];
        let m = merged(&[(a, 3.0), (b, 3.0)]);
        assert_eq!(m.len(), 3, "{m:?}");
        assert!(near(&m[0].hex, [236, 232, 224]) && (m[0].share - 0.65).abs() < 0.01, "{m:?}");
        assert!(merged(&[]).is_empty());
    }
}
