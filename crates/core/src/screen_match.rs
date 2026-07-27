//! Template matching for on-screen UI probes (zero-mean normalized
//! cross-correlation, the `TM_CCOEFF_NORMED` of OpenCV).
//!
//! Colour-fingerprint probes (sampling a few pixels, or comparing region
//! brightness) misfire whenever TikTok changes a gradient or the video behind a
//! sheet happens to be bright. Matching the shape of the close button instead is
//! stable: measured on this iPhone 8, the "Add phone" sheet scores 0.988 while
//! the interest picker scores 0.46 and a plain feed 0.43, so a 0.85 threshold
//! separates them with a wide margin.
//!
//! Speed matters because the popup watcher runs off the live frame stream, in
//! debug builds too. Three things keep it cheap:
//!
//! 1. Callers crop to a region of interest before matching (see [`crate::screen`]).
//! 2. A coarse pass at half resolution picks candidate positions; only those get
//!    scored at full resolution.
//! 3. The inner loops index raw `&[u8]` rows instead of `GrayImage::get_pixel`,
//!    whose bounds checks and `Luma` wrapper dominated the old profile.

use image::imageops::FilterType;
use image::{GrayImage, RgbImage};

/// Score + centre of the best match, in the coordinate space of the haystack
/// passed to [`find_template`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Match {
    pub score: f64,
    pub cx: f64,
    pub cy: f64,
}

/// Scale factors tried for the needle. A template cropped on a @2x device still
/// has to match a @3x screenshot (and vice versa), so probe around 1.0.
const SCALES: [f64; 3] = [1.0, 0.667, 1.5];

/// Candidate positions carried from the coarse pass to the refine pass. More
/// candidates cost little (each is a single window score) and protect against
/// the true peak losing to a blur artefact at half resolution.
const COARSE_CANDIDATES: usize = 12;

/// Half-width of the full-resolution search box around each coarse candidate.
/// The coarse pass locates the peak to ±1 coarse pixel = ±2 full pixels; 3 gives
/// margin for the resample shifting the peak.
const REFINE_RADIUS: i64 = 3;

/// Best match of `needle` inside `haystack`, or `None` when the needle does not
/// fit. Both images must already be grayscale and at the same nominal scale.
pub fn find_template(haystack: &GrayImage, needle: &GrayImage) -> Option<Match> {
    let mut best: Option<Match> = None;
    for factor in SCALES {
        let nw = ((needle.width() as f64 * factor).round() as u32).max(1);
        let nh = ((needle.height() as f64 * factor).round() as u32).max(1);
        if nw >= haystack.width() || nh >= haystack.height() {
            continue;
        }
        let scaled = if (factor - 1.0).abs() < f64::EPSILON {
            needle.clone()
        } else {
            image::imageops::resize(needle, nw, nh, FilterType::Triangle)
        };
        if let Some(m) = correlate(haystack, &scaled) {
            if best.map_or(true, |b| m.score > b.score) {
                best = Some(m);
            }
        }
    }
    best
}

/// Zero-mean needle statistics, reused across every window.
struct Needle {
    dev: Vec<f64>,
    norm: f64,
    w: usize,
    h: usize,
}

impl Needle {
    fn new(img: &GrayImage) -> Option<Self> {
        let (w, h) = (img.width() as usize, img.height() as usize);
        let len = (w * h) as f64;
        if len == 0.0 {
            return None;
        }
        let pixels: Vec<f64> = img.as_raw().iter().map(|&v| v as f64).collect();
        let mean = pixels.iter().sum::<f64>() / len;
        let dev: Vec<f64> = pixels.iter().map(|v| v - mean).collect();
        let norm = dev.iter().map(|v| v * v).sum::<f64>().sqrt();
        if norm <= f64::EPSILON {
            return None;
        }
        Some(Self { dev, norm, w, h })
    }
}

/// Integral image of values and squares, for O(1) window sum/variance.
struct Integrals {
    sum: Vec<f64>,
    sq: Vec<f64>,
    stride: usize,
}

impl Integrals {
    fn new(img: &GrayImage) -> Self {
        let (w, h) = (img.width() as usize, img.height() as usize);
        let stride = w + 1;
        let mut sum = vec![0.0f64; stride * (h + 1)];
        let mut sq = vec![0.0f64; stride * (h + 1)];
        let raw = img.as_raw();
        for y in 0..h {
            let mut row = 0.0f64;
            let mut row_sq = 0.0f64;
            let src = &raw[y * w..y * w + w];
            let prev = y * stride;
            let cur = (y + 1) * stride;
            for x in 0..w {
                let v = src[x] as f64;
                row += v;
                row_sq += v * v;
                sum[cur + x + 1] = sum[prev + x + 1] + row;
                sq[cur + x + 1] = sq[prev + x + 1] + row_sq;
            }
        }
        Self { sum, sq, stride }
    }

    fn window(&self, table: &[f64], x: usize, y: usize, w: usize, h: usize) -> f64 {
        table[(y + h) * self.stride + x + w] - table[y * self.stride + x + w]
            - table[(y + h) * self.stride + x]
            + table[y * self.stride + x]
    }
}

/// NCC of `needle` at one position, or `None` for a flat (signal-free) window.
fn score_at(
    hay: &GrayImage,
    integ: &Integrals,
    needle: &Needle,
    x: usize,
    y: usize,
) -> Option<f64> {
    let n_len = (needle.w * needle.h) as f64;
    let w_sum = integ.window(&integ.sum, x, y, needle.w, needle.h);
    let w_sq = integ.window(&integ.sq, x, y, needle.w, needle.h);
    let w_mean = w_sum / n_len;
    let w_var = w_sq - w_sum * w_mean;
    if w_var <= 1e-6 {
        return None;
    }
    let hw = hay.width() as usize;
    let raw = hay.as_raw();
    let mut cross = 0.0f64;
    for ny in 0..needle.h {
        let row = &raw[(y + ny) * hw + x..(y + ny) * hw + x + needle.w];
        let dev = &needle.dev[ny * needle.w..ny * needle.w + needle.w];
        for nx in 0..needle.w {
            cross += (row[nx] as f64 - w_mean) * dev[nx];
        }
    }
    Some(cross / (w_var.sqrt() * needle.norm))
}

/// Single-scale NCC over the whole haystack, coarse-to-fine.
fn correlate(haystack: &GrayImage, needle_img: &GrayImage) -> Option<Match> {
    let needle = Needle::new(needle_img)?;
    let (hw, hh) = (haystack.width() as usize, haystack.height() as usize);
    if needle.w == 0 || needle.h == 0 || needle.w > hw || needle.h > hh {
        return None;
    }

    // Small searches are cheaper scanned directly than pyramided.
    let positions = (hw - needle.w + 1) * (hh - needle.h + 1);
    if positions <= 4_096 || needle.w < 8 || needle.h < 8 {
        return exhaustive(haystack, &needle);
    }

    let half = |img: &GrayImage| {
        image::imageops::resize(
            img,
            (img.width() / 2).max(1),
            (img.height() / 2).max(1),
            FilterType::Triangle,
        )
    };
    let hay_small = half(haystack);
    let needle_small_img = half(needle_img);
    let Some(needle_small) = Needle::new(&needle_small_img) else {
        return exhaustive(haystack, &needle);
    };
    if needle_small.w > hay_small.width() as usize || needle_small.h > hay_small.height() as usize {
        return exhaustive(haystack, &needle);
    }

    // Coarse pass: keep the best few positions rather than only the argmax.
    let integ_small = Integrals::new(&hay_small);
    let (sw, sh) = (hay_small.width() as usize, hay_small.height() as usize);
    let mut coarse: Vec<(f64, usize, usize)> = Vec::new();
    for y in 0..=(sh - needle_small.h) {
        for x in 0..=(sw - needle_small.w) {
            let Some(score) = score_at(&hay_small, &integ_small, &needle_small, x, y) else {
                continue;
            };
            if coarse.len() < COARSE_CANDIDATES {
                coarse.push((score, x, y));
                if coarse.len() == COARSE_CANDIDATES {
                    coarse.sort_by(|a, b| b.0.total_cmp(&a.0));
                }
            } else if score > coarse[COARSE_CANDIDATES - 1].0 {
                coarse[COARSE_CANDIDATES - 1] = (score, x, y);
                coarse.sort_by(|a, b| b.0.total_cmp(&a.0));
            }
        }
    }
    if coarse.is_empty() {
        return None;
    }

    // Refine each candidate at full resolution.
    let integ = Integrals::new(haystack);
    let max_x = (hw - needle.w) as i64;
    let max_y = (hh - needle.h) as i64;
    let mut best: Option<Match> = None;
    let mut seen: Vec<(usize, usize)> = Vec::new();
    for (_, cx, cy) in coarse {
        let bx = (cx * 2) as i64;
        let by = (cy * 2) as i64;
        for y in (by - REFINE_RADIUS).max(0)..=(by + REFINE_RADIUS).min(max_y) {
            for x in (bx - REFINE_RADIUS).max(0)..=(bx + REFINE_RADIUS).min(max_x) {
                let (x, y) = (x as usize, y as usize);
                if seen.contains(&(x, y)) {
                    continue;
                }
                seen.push((x, y));
                let Some(score) = score_at(haystack, &integ, &needle, x, y) else {
                    continue;
                };
                if best.map_or(true, |b| score > b.score) {
                    best = Some(Match {
                        score,
                        cx: x as f64 + needle.w as f64 / 2.0,
                        cy: y as f64 + needle.h as f64 / 2.0,
                    });
                }
            }
        }
    }
    best
}

fn exhaustive(haystack: &GrayImage, needle: &Needle) -> Option<Match> {
    let integ = Integrals::new(haystack);
    let (hw, hh) = (haystack.width() as usize, haystack.height() as usize);
    let mut best: Option<Match> = None;
    for y in 0..=(hh - needle.h) {
        for x in 0..=(hw - needle.w) {
            let Some(score) = score_at(haystack, &integ, needle, x, y) else {
                continue;
            };
            if best.map_or(true, |b| score > b.score) {
                best = Some(Match {
                    score,
                    cx: x as f64 + needle.w as f64 / 2.0,
                    cy: y as f64 + needle.h as f64 / 2.0,
                });
            }
        }
    }
    best
}

/// Rec709 luma, matching OpenCV's `COLOR_RGB2GRAY`.
pub fn to_gray(img: &RgbImage) -> GrayImage {
    let mut out = GrayImage::new(img.width(), img.height());
    for (dst, px) in out.as_mut().iter_mut().zip(img.pixels()) {
        let v = 0.299 * px[0] as f64 + 0.587 * px[1] as f64 + 0.114 * px[2] as f64;
        *dst = v.round().clamp(0.0, 255.0) as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-noise — non-repeating so a crop has one true home.
    fn noise(w: u32, h: u32, seed: u32) -> GrayImage {
        GrayImage::from_fn(w, h, |x, y| {
            let mut v = x
                .wrapping_mul(2_654_435_761)
                .wrapping_add(y.wrapping_mul(40_503))
                .wrapping_add(seed.wrapping_mul(2_246_822_519));
            v ^= v >> 13;
            v = v.wrapping_mul(1_274_126_177);
            v ^= v >> 16;
            image::Luma([(v & 0xFF) as u8])
        })
    }

    #[test]
    fn finds_a_patch_it_was_cut_from() {
        let scene = noise(60, 60, 7);
        let needle = image::imageops::crop_imm(&scene, 20, 24, 12, 12).to_image();
        let m = find_template(&scene, &needle).expect("match");
        assert!(m.score > 0.95, "score {}", m.score);
        assert!((m.cx - 26.0).abs() <= 1.0, "cx {}", m.cx);
        assert!((m.cy - 30.0).abs() <= 1.0, "cy {}", m.cy);
    }

    #[test]
    fn scores_low_on_unrelated_content() {
        let scene = noise(60, 60, 7);
        let needle = noise(12, 12, 999);
        let m = find_template(&scene, &needle).expect("match");
        assert!(m.score < 0.85, "score {}", m.score);
    }

    /// The pyramid must not cost accuracy on a haystack big enough to trigger
    /// it — same crop, same answer as the exhaustive path.
    #[test]
    fn coarse_to_fine_agrees_with_exhaustive_on_a_large_scene() {
        let scene = noise(400, 500, 3);
        let needle = image::imageops::crop_imm(&scene, 137, 291, 24, 24).to_image();
        let m = find_template(&scene, &needle).expect("match");
        assert!(m.score > 0.95, "score {}", m.score);
        assert!((m.cx - 149.0).abs() <= 1.0, "cx {}", m.cx);
        assert!((m.cy - 303.0).abs() <= 1.0, "cy {}", m.cy);
    }
}
