//! Key frames from a video, with AVFoundation (macOS).
//!
//! "Key frame" here means a distinct view, not a codec I-frame: the app samples
//! the video at a steady interval with small images, cuts it into segments where
//! the picture changes (a cut, or a pan that drifts far enough), and keeps the
//! middle frame of each segment at full size. Very dark or flat frames (fades),
//! one-sample transitions and repeats of an earlier view are left out.

use anyhow::{bail, Result};
use std::path::Path;

/// Extensions that AVFoundation reads.
pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "m4v"];
/// Video extensions that AVFoundation cannot read.
pub const UNSUPPORTED_VIDEO_EXTENSIONS: &[&str] = &["webm", "mkv", "avi", "flv", "wmv"];

/// Most frames from one video.
pub const MAX_FRAMES: usize = 20;
/// Longest side of a saved frame (pixels).
const FRAME_SIZE: f64 = 2048.0;
/// Longest side of a scan sample (pixels).
const SCAN_SIZE: f64 = 160.0;
/// Most scan samples; longer videos get a longer interval.
const MAX_SAMPLES: usize = 300;
/// Shortest interval between scan samples (seconds).
const MIN_INTERVAL: f64 = 0.5;
/// Side of the grayscale signature of a sample.
const SIG: usize = 24;
/// Change to the previous sample that counts as a cut.
const CUT: f32 = 0.12;
/// Change to the first sample of a segment that starts a new segment (pans).
const DRIFT: f32 = 0.18;
/// A chosen frame closer than this to an earlier one is a repeat.
const REPEAT: f32 = 0.06;

fn extension(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// True for a file name that is a video (readable or not).
pub fn is_video_name(name: &str) -> bool {
    let ext = extension(Path::new(name));
    VIDEO_EXTENSIONS.contains(&ext.as_str()) || UNSUPPORTED_VIDEO_EXTENSIONS.contains(&ext.as_str())
}

/// One chosen frame.
pub struct Frame {
    /// Position in the video (seconds).
    pub time: f64,
    pub jpeg: Vec<u8>,
}

pub struct Extracted {
    pub duration: f64,
    pub frames: Vec<Frame>,
}

/// Grayscale signature of a sample, values 0..1.
#[derive(Clone)]
pub struct Signature {
    px: Vec<f32>,
    mean: f32,
    spread: f32,
}

impl Signature {
    pub fn of(img: &image::RgbaImage) -> Self {
        let small = image::imageops::resize(img, SIG as u32, SIG as u32, image::imageops::FilterType::Triangle);
        let px: Vec<f32> = small
            .pixels()
            .map(|p| (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32) / 255.0)
            .collect();
        let mean = px.iter().sum::<f32>() / px.len() as f32;
        let spread = (px.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / px.len() as f32).sqrt();
        Signature { px, mean, spread }
    }

    pub fn diff(&self, other: &Signature) -> f32 {
        self.px.iter().zip(&other.px).map(|(a, b)| (a - b).abs()).sum::<f32>() / self.px.len() as f32
    }

    /// A fade, a black frame or a flat colour.
    fn is_blank(&self) -> bool {
        self.mean < 0.04 || self.spread < 0.02
    }
}

/// The scan times for a video of `duration` seconds.
pub fn sample_times(duration: f64) -> Vec<f64> {
    if duration <= 0.0 {
        return vec![];
    }
    let interval = (duration / MAX_SAMPLES as f64).max(MIN_INTERVAL);
    if duration < interval {
        return vec![duration / 2.0];
    }
    let n = (duration / interval).floor() as usize;
    (0..n).map(|i| (i as f64 + 0.5) * interval).collect()
}

/// Indexes of the samples to keep: the middle sample of each segment, best
/// `max` segments by length, in time order.
pub fn choose(sigs: &[Signature], max: usize) -> Vec<usize> {
    if sigs.is_empty() {
        return vec![];
    }
    // Segments as (start, end) sample ranges, end exclusive.
    let mut segments: Vec<(usize, usize)> = Vec::new();
    let mut start = 0;
    for i in 1..sigs.len() {
        if sigs[i].diff(&sigs[i - 1]) > CUT || sigs[i].diff(&sigs[start]) > DRIFT {
            segments.push((start, i));
            start = i;
        }
    }
    segments.push((start, sigs.len()));

    // A one-sample segment between two others is usually a transition.
    let several = segments.len() > 2;
    let mut picks: Vec<(usize, usize)> = segments
        .iter()
        .filter(|(s, e)| !(several && e - s == 1))
        .map(|&(s, e)| ((s + e - 1) / 2, e - s))
        .filter(|&(mid, _)| !sigs[mid].is_blank())
        .collect();
    if picks.is_empty() {
        // Everything is dark or flat: keep the middle of the longest segment.
        let (s, e) = *segments.iter().max_by_key(|(s, e)| e - s).expect("one segment");
        picks.push(((s + e - 1) / 2, e - s));
    }
    // Longest segments first, so repeats and the limit drop the short ones.
    picks.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut kept: Vec<usize> = Vec::new();
    for (mid, _) in picks {
        if kept.len() >= max {
            break;
        }
        if kept.iter().all(|&k| sigs[k].diff(&sigs[mid]) > REPEAT) {
            kept.push(mid);
        }
    }
    kept.sort_unstable();
    kept
}

/// Reads the video and returns its key frames as JPEG.
pub fn extract(path: &Path, max: usize) -> Result<Extracted> {
    let ext = extension(path);
    if UNSUPPORTED_VIDEO_EXTENSIONS.contains(&ext.as_str()) {
        bail!(
            "macOS cannot read .{ext} videos. Convert \"{}\" to MP4 or MOV first (for example with QuickTime Player › File › Export As).",
            path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
        );
    }
    platform::extract(path, max.clamp(1, MAX_FRAMES))
}

fn jpeg_of(img: &image::RgbaImage) -> Result<Vec<u8>> {
    let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
    let mut out = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90).encode_image(&rgb)?;
    Ok(out.into_inner())
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use anyhow::anyhow;
    use objc2_av_foundation::{AVAssetImageGenerator, AVURLAsset};
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_core_graphics::{CGBitmapContextCreate, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo};
    use objc2_core_media::CMTime;
    use objc2_foundation::{NSString, NSURL};

    const TIMESCALE: i32 = 600;

    /// Draws a CGImage into an RGBA buffer.
    fn rgba_of(img: &CGImage) -> Result<image::RgbaImage> {
        let w = CGImage::width(Some(img));
        let h = CGImage::height(Some(img));
        if w == 0 || h == 0 {
            bail!("The video frame is empty.");
        }
        let mut buf = vec![0u8; w * h * 4];
        let space = CGColorSpace::new_device_rgb().ok_or_else(|| anyhow!("No RGB colour space."))?;
        // SAFETY: buf outlives the context, and has w * h * 4 bytes for 8-bit RGBA rows of w * 4 bytes.
        let ctx = unsafe {
            CGBitmapContextCreate(
                buf.as_mut_ptr().cast(),
                w,
                h,
                8,
                w * 4,
                Some(&space),
                CGImageAlphaInfo::PremultipliedLast.0,
            )
        }
        .ok_or_else(|| anyhow!("Cannot make a bitmap for the video frame."))?;
        let rect = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(w as f64, h as f64));
        CGContext::draw_image(Some(&ctx), rect, Some(img));
        drop(ctx);
        image::RgbaImage::from_raw(w as u32, h as u32, buf).ok_or_else(|| anyhow!("Bad frame buffer."))
    }

    fn generator(asset: &AVURLAsset, size: f64, tolerance: f64) -> objc2::rc::Retained<AVAssetImageGenerator> {
        // SAFETY: plain AVFoundation calls on objects that we own.
        unsafe {
            let g = AVAssetImageGenerator::assetImageGeneratorWithAsset(asset);
            g.setAppliesPreferredTrackTransform(true);
            g.setMaximumSize(CGSize::new(size, size));
            let tol = CMTime::with_seconds(tolerance, TIMESCALE);
            g.setRequestedTimeToleranceBefore(tol);
            g.setRequestedTimeToleranceAfter(tol);
            g
        }
    }

    #[allow(deprecated)] // copyCGImageAtTime is synchronous, which suits a worker thread.
    fn frame_at(g: &AVAssetImageGenerator, t: f64) -> Result<image::RgbaImage> {
        // SAFETY: a null actual_time pointer is allowed.
        let img = unsafe { g.copyCGImageAtTime_actualTime_error(CMTime::with_seconds(t, TIMESCALE), std::ptr::null_mut()) }
            .map_err(|e| anyhow!("{}", e.localizedDescription()))?;
        rgba_of(&img)
    }

    #[allow(deprecated)] // AVAsset.duration and isReadable are synchronous; fine off the main thread.
    pub fn extract(path: &Path, max: usize) -> Result<Extracted> {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !path.is_file() {
            bail!("The video \"{name}\" does not exist.");
        }
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        // SAFETY: plain AVFoundation calls.
        let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&url, None) };
        let (readable, duration) = unsafe { (asset.isReadable(), asset.duration().seconds()) };
        if !readable || !duration.is_finite() || duration <= 0.0 {
            bail!("macOS cannot read the video \"{name}\". Try an MP4 or MOV file.");
        }
        let times = sample_times(duration);
        let interval = if times.len() > 1 { times[1] - times[0] } else { duration };
        let scan = generator(&asset, SCAN_SIZE, interval / 2.0);
        let mut sigs = Vec::new();
        let mut sig_times = Vec::new();
        let mut last_err = None;
        for &t in &times {
            match frame_at(&scan, t) {
                Ok(img) => {
                    sigs.push(Signature::of(&img));
                    sig_times.push(t);
                }
                Err(e) => last_err = Some(e),
            }
        }
        if sigs.is_empty() {
            return Err(last_err.unwrap_or_else(|| anyhow!("No frames in \"{name}\".")))
                .map_err(|e| anyhow!("Cannot read frames from \"{name}\": {e}"));
        }
        // Full-size frames, as exact as the decoder allows.
        let full = generator(&asset, FRAME_SIZE, 0.05);
        let mut frames = Vec::new();
        for i in choose(&sigs, max) {
            let t = sig_times[i];
            let img = frame_at(&full, t)?;
            frames.push(Frame { time: t, jpeg: jpeg_of(&img)? });
        }
        Ok(Extracted { duration, frames })
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::*;
    pub fn extract(_path: &Path, _max: usize) -> Result<Extracted> {
        bail!("Video import needs macOS (AVFoundation).")
    }
}

/// "m:ss" or "h:mm:ss" for a position in seconds.
pub fn timecode(t: f64) -> String {
    let s = t.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(level: u8, stripe: bool) -> Signature {
        let img = image::RgbaImage::from_fn(64, 36, |x, _| {
            let v = if stripe && x % 16 < 8 { 255 - level } else { level };
            image::Rgba([v, v, v, 255])
        });
        Signature::of(&img)
    }

    #[test]
    fn sample_times_cover_the_video() {
        assert!(sample_times(0.0).is_empty());
        assert_eq!(sample_times(0.2), vec![0.1]);
        let t = sample_times(10.0);
        assert_eq!(t.len(), 20);
        assert!((t[0] - 0.25).abs() < 1e-9);
        assert!(sample_times(3600.0).len() <= MAX_SAMPLES);
    }

    #[test]
    fn choose_keeps_one_frame_per_shot() {
        // Shot A (6 samples), a one-sample flash, shot B (5), a black fade (3), shot A again (4).
        let a = sig(60, true);
        let b = sig(200, true);
        let flash = sig(250, false);
        let black = sig(2, false);
        let mut sigs = vec![a.clone(); 6];
        sigs.push(flash);
        sigs.extend(vec![b.clone(); 5]);
        sigs.extend(vec![black; 3]);
        sigs.extend(vec![a.clone(); 4]);
        let kept = choose(&sigs, 20);
        // The middle of shot A and of shot B; the flash, the fade and the repeat of A are left out.
        assert_eq!(kept, vec![2, 9]);
        assert_eq!(choose(&sigs, 1), vec![2]);
    }

    #[test]
    fn choose_handles_a_dark_video() {
        let sigs = vec![sig(1, false); 5];
        assert_eq!(choose(&sigs, 20), vec![2]);
    }

    /// Manual check on a real file:
    /// CORY_TEST_VIDEO=clip.mp4 CORY_TEST_OUT=/tmp/frames cargo test -p corydoras-core real_video -- --ignored --nocapture
    #[test]
    #[ignore]
    fn real_video() {
        let path = std::env::var("CORY_TEST_VIDEO").expect("CORY_TEST_VIDEO");
        let out = std::path::PathBuf::from(std::env::var("CORY_TEST_OUT").expect("CORY_TEST_OUT"));
        std::fs::create_dir_all(&out).unwrap();
        let t0 = std::time::Instant::now();
        let x = extract(Path::new(&path), MAX_FRAMES).unwrap();
        println!("duration {:.1} s, {} frames, {:.1} s", x.duration, x.frames.len(), t0.elapsed().as_secs_f64());
        for f in &x.frames {
            let name = format!("{:07.2}.jpg", f.time);
            let img = image::load_from_memory(&f.jpeg).unwrap();
            println!("{} {}x{}", timecode(f.time), img.width(), img.height());
            std::fs::write(out.join(name), &f.jpeg).unwrap();
        }
        // Contact sheet: 5 columns of 240 px wide thumbnails.
        let thumbs: Vec<image::RgbImage> = x
            .frames
            .iter()
            .map(|f| image::load_from_memory(&f.jpeg).unwrap().thumbnail(240, 240).to_rgb8())
            .collect();
        let (cw, ch) = (240u32, 240u32);
        let cols = 5u32;
        let rows = (thumbs.len() as u32).div_ceil(cols).max(1);
        let mut sheet = image::RgbImage::from_pixel(cols * cw, rows * ch, image::Rgb([30, 30, 30]));
        for (i, t) in thumbs.iter().enumerate() {
            let (cx, cy) = ((i as u32 % cols) * cw, (i as u32 / cols) * ch);
            image::imageops::overlay(&mut sheet, t, (cx + (cw - t.width()) / 2) as i64, (cy + (ch - t.height()) / 2) as i64);
        }
        sheet.save(out.join("sheet.jpg")).unwrap();
    }

    #[test]
    fn timecodes() {
        assert_eq!(timecode(0.4), "0:00");
        assert_eq!(timecode(75.6), "1:16");
        assert_eq!(timecode(3725.0), "1:02:05");
    }
}
