//! What is currently on the iPhone screen, decided from one captured frame.
//!
//! Every probe here works on frames from the device screen stream (native
//! screen pixels, any capture scale) and reports positions as **screen
//! fractions**, so callers can map them to WDA points without knowing the
//! capture geometry.
//!
//! The geometry constants were measured on the live iPhone 8 (750×1334 @2x)
//! this project targets; see `docs/LIVE_NURTURE_REPORT_2026-07-26.md` for the
//! captures they came from. Fractions rather than pixels keep them meaningful
//! if the stream is rescaled.

use image::RgbImage;

use crate::screen_match::{find_template, to_gray};

/// A screen class whose detector constants in this file have actually been
/// measured on a physical device.
///
/// Every geometry constant below is a fraction anchored to a fixed point
/// distance from a screen edge, not a proportion of the whole. They therefore
/// do **not** transfer by arithmetic to a different screen: `COMMENT_INPUT.1`
/// is `640/667`, which is 27pt up from the bottom of an iPhone 8 but lands 35pt
/// up on an 844pt-tall iPhone — before counting the 34pt home indicator that an
/// iPhone 8 does not have. A new class has to be re-measured, not divided.
pub struct CalibratedLayout {
    pub id: &'static str,
    pub logical_width: f64,
    pub logical_height: f64,
}

/// Exactly the screen classes that have been calibrated. Adding one is a
/// measurement exercise (AGENTS.md section 6), not an edit to this table.
pub const CALIBRATED_LAYOUTS: &[CalibratedLayout] = &[CalibratedLayout {
    id: "iphone8-portrait-v1",
    logical_width: 375.0,
    logical_height: 667.0,
}];

/// The device's own logical screen size, or a refusal that says why.
///
/// **Never substitute a default.** Every geometry constant in this file is a *fraction*, so
/// the screen size is the multiplier that turns a fraction into a tap. Get the multiplier
/// wrong and the fraction is still perfectly valid — it just points somewhere else on the
/// glass, and nothing downstream can tell.
///
/// Four call sites used to write `window_size().await.unwrap_or((375.0, 667.0))`. That
/// fallback is the one calibrated layout this file has, an iPhone 8; the Android fleet this
/// project drives reports **1080x2220**. So a single failed `window_size()` moved every
/// derived point into the top-left corner: `x * 375/1080` and `y * 667/2220`, about 35% and
/// 30% of the way in. Two of those call sites then tapped composer and Send, which is a
/// comment published against whatever control happened to be under the fabricated point.
///
/// `run_session` already refuses unknown geometry (see `nurture/mod.rs`, the
/// `calibrated_layout(..) else` arm) and `docs/agents/10-thiet-bi-moi.md` records that
/// refusal as covering both paths. It did not: the Interaction entry points acquire their
/// own geometry *after* that protection, and kept the fallback. Found by an independent
/// review on 27/08/2026.
///
/// A refusal costs one attempt. A fabricated tap costs an action on a real account that
/// nobody can take back.
pub async fn measured_screen_size(session: &dyn crate::UiSession) -> anyhow::Result<(f64, f64)> {
    let size = session
        .window_size()
        .await
        .map_err(|error| anyhow::anyhow!("screen_size_unavailable: {error}"))?;
    // A zero or negative dimension multiplies every fraction to zero, which lands on the
    // top-left pixel rather than failing — the same class of wrong as the old fallback.
    anyhow::ensure!(
        size.0 > 0.0 && size.1 > 0.0,
        "screen_size_unavailable: máy báo kích thước {}x{}",
        size.0,
        size.1
    );
    Ok(size)
}

/// Half a point of slack, because the size arrives as a float over the wire.
const LAYOUT_MATCH_SLACK: f64 = 0.5;

/// The calibrated layout for a live screen size, or `None` when this screen
/// class has never been measured.
///
/// `None` must mean refuse. Multiplying these fractions against an unmeasured
/// screen produces tap points that look plausible and land on the wrong
/// controls — the failure AGENTS.md §3.12 names directly: *"chua qualify
/// profile moi thi fail closed ... khong tap toa do iPhone 8 len may moi"*.
pub fn calibrated_layout(
    logical_width: f64,
    logical_height: f64,
) -> Option<&'static CalibratedLayout> {
    CALIBRATED_LAYOUTS.iter().find(|layout| {
        (layout.logical_width - logical_width).abs() <= LAYOUT_MATCH_SLACK
            && (layout.logical_height - logical_height).abs() <= LAYOUT_MATCH_SLACK
    })
}

/// TikTok's close-button glyph (grey ✕ on a light disc), cropped at @2x.
static CLOSE_X_TEMPLATE: &[u8] = include_bytes!("../assets/tiktok_close_x.png");

/// Device pixel ratio the close-button template was cropped at.
const TEMPLATE_SCALE: f64 = 2.0;

/// Minimum NCC score to accept the close button. Measured on this iPhone 8:
/// Add-phone sheet 0.99, interest picker 0.46, plain FYP feed 0.59.
pub const CLOSE_X_THRESHOLD: f64 = 0.85;

/// Search window for the close button, as screen fractions. TikTok puts it in
/// the upper-right of whatever sheet is showing; restricting the search keeps
/// the match cheap and avoids false hits in the video itself.
///
/// The top edge used to be 0.08, which put it *below* [`LIVE_EXIT`] at 0.069 —
/// and the match centre has to clear the crop by half a template besides. So
/// the one ✕ the engine most needs to find was the one ✕ this search could
/// never reach, and a LIVE room whose follow pill is absent (host already
/// followed) had no second way out at all. 0.045 leaves room for the template
/// and still starts below the status bar.
const CLOSE_X_REGION: (f64, f64, f64, f64) = (0.68, 0.045, 1.0, 0.92);

/// The close-button search band, so a test can assert [`LIVE_EXIT`] is inside
/// it rather than the two constants drifting apart again unnoticed.
pub fn close_x_region() -> (f64, f64, f64, f64) {
    CLOSE_X_REGION
}

#[cfg(test)]
mod calibrated_layout_tests {
    use super::*;

    #[test]
    fn the_measured_iphone_8_screen_is_calibrated() {
        let layout = calibrated_layout(375.0, 667.0).expect("iPhone 8 is the measured class");
        assert_eq!(layout.id, "iphone8-portrait-v1");
        // Half a point of slack, because the size arrives as a float.
        assert!(calibrated_layout(375.2, 666.8).is_some());
    }

    #[test]
    fn screens_nobody_has_measured_are_refused() {
        // Each of these would otherwise be tapped with iPhone 8 fractions.
        for (width, height, what) in [
            (390.0, 844.0, "iPhone 14"),
            (393.0, 852.0, "iPhone 15"),
            (320.0, 568.0, "iPhone SE 1"),
            (414.0, 896.0, "iPhone 11"),
            (1080.0, 2220.0, "Galaxy S8+ in device pixels"),
            (667.0, 375.0, "iPhone 8 rotated to landscape"),
        ] {
            assert!(
                calibrated_layout(width, height).is_none(),
                "{what} ({width}x{height}) has never been measured and must be refused"
            );
        }
    }

    #[test]
    fn a_missing_or_absurd_size_is_refused_rather_than_rounded_to_the_nearest_class() {
        assert!(calibrated_layout(0.0, 0.0).is_none());
        assert!(calibrated_layout(f64::NAN, f64::NAN).is_none());
        assert!(calibrated_layout(376.0, 668.0).is_none());
    }
}

/// A TikTok promo card can float from the upper-left and keep the compose bar
/// visible. Its close control is a dark circular button rather than the light
/// X used by sheets, so it has a separate colour/shape detector.
const PROMO_CLOSE_REGION: (f64, f64, f64, f64) = (0.15, 0.15, 0.25, 0.22);

/// Screen-fraction x of TikTok's right-hand action rail (avatar → share).
pub const RAIL_X: f64 = 0.919;

/// TikTok ships two sidebar layouts whose icons differ by 36 logical points.
/// Within a layout the spacing is fixed, so locating any one icon locates the
/// rest. Measured on this iPhone 8 (750×1334) and cross-checked against the
/// reference tool's tables, which were tuned on the same screen size:
///
/// | icon    | layout 1 | layout 2 | offset from follow |
/// |---------|---------:|---------:|-------------------:|
/// | follow  |      223 |      259 |                 +0 |
/// | like    |      277 |      313 |                +54 |
/// | comment |      335 |      371 |               +112 |
///
/// (logical points; ÷667 for the fractions used here.) Measuring the glyphs
/// directly on this device's capture put the badge at 263, the heart at 312 and
/// the bubble at 377 — offsets of +49 and +114. The two sources agree to within
/// a few points, comfortably inside a 23-point icon, so the offsets below split
/// the difference and hit either way.
const FOLLOW_TO_LIKE: f64 = 51.0 / 667.0;
const FOLLOW_TO_COMMENT: f64 = 113.0 / 667.0;
/// Save is one measured rail pitch below Comment. The known layout offsets derive targets at
/// 403 and 439 logical points; the layout-2 capture's actual white glyph was centred at 443,
/// four points from the derived target and well inside the same 23-point control.
const FOLLOW_TO_SAVE: f64 = 180.0 / 667.0;

/// Follow-badge centre for each known layout, as screen fractions.
const FOLLOW_Y_LAYOUT1: f64 = 223.0 / 667.0;
const FOLLOW_Y_LAYOUT2: f64 = 259.0 / 667.0;

/// Centre y of the like heart, layout 2.
pub const LIKE_Y: f64 = FOLLOW_Y_LAYOUT2 + FOLLOW_TO_LIKE;

/// Redness at the heart separating "filled" from "outline".
///
/// Measured over 11 live feed frames from `05101fdb`:
///
/// | state | readings |
/// |---|---|
/// | filled (liked) | 111.0, 121.8, 122.4, 122.6 |
/// | outline | −25.9 … 58.7 |
///
/// The gap is wide, and 90 sits in it. This replaced a `before + 40` relative
/// test that failed both ways: a reddish video lifted the baseline to 42 and a
/// genuine fill to 60 read as "no change", while an outline over red at 58.7
/// tripped the old `> 45` "already liked" check and skipped the like entirely.
pub const LIKE_FILLED_REDNESS: f64 = 90.0;
/// The author "+ Follow" pill a LIVE room shows top-left, in screen fractions.
const LIVE_FOLLOW_PILL: (f64, f64, f64, f64) = (0.36, 0.052, 0.55, 0.088);
/// Colour margins for that pill. Measured: LIVE rooms R−G 152–176 and R−B
/// 121–140; a feed frame with video behind the same box read 34 and 13.
const LIVE_PILL_RG_MIN: f64 = 90.0;
const LIVE_PILL_RB_MIN: f64 = 80.0;

/// Half-width of the sampled rail column, as a screen fraction.
const RAIL_HALF_WIDTH: f64 = 0.032;

/// Fraction of a sampled rail row that must read as TikTok red to count as
/// part of the follow badge.
const BADGE_ROW_COVERAGE: f64 = 0.30;

/// Where the action buttons are on *this* frame.
///
/// Hard-coding one set of fractions is what made likes miss: the previous
/// values fell in the dead space between two icons, so every "like" tapped
/// nothing and the counter still went up. Locating the rail per frame fixes
/// that and adapts to whichever layout TikTok is showing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActionRail {
    pub x: f64,
    pub follow_y: f64,
    pub like_y: f64,
    pub comment_y: f64,
    /// Save centre when this rail was located from the current frame. Fallback geometry never
    /// fills this field because an unproved coordinate must not authorize a toggle.
    pub save_y: Option<f64>,
    /// True when the follow badge was actually found in this frame; false when
    /// these are the fallback constants.
    pub located: bool,
}

impl ActionRail {
    /// Layout 2 constants — the safe default for this device.
    pub fn fallback() -> Self {
        Self::from_follow(FOLLOW_Y_LAYOUT2, false)
    }

    fn from_follow(follow_y: f64, located: bool) -> Self {
        Self {
            x: RAIL_X,
            follow_y,
            like_y: follow_y + FOLLOW_TO_LIKE,
            comment_y: follow_y + FOLLOW_TO_COMMENT,
            save_y: located.then_some(follow_y + FOLLOW_TO_SAVE),
            located,
        }
    }

    /// Which of the two known layouts this rail matches, for logging.
    pub fn layout(&self) -> u8 {
        let d1 = (self.follow_y - FOLLOW_Y_LAYOUT1).abs();
        let d2 = (self.follow_y - FOLLOW_Y_LAYOUT2).abs();
        if d1 < d2 {
            1
        } else {
            2
        }
    }
}

/// Is this pixel TikTok red?
fn is_tiktok_red(p: &image::Rgb<u8>) -> bool {
    let (r, g, b) = (p[0] as f64, p[1] as f64, p[2] as f64);
    r > 180.0 && g < 110.0 && (r - g) > 90.0 && (b - g).abs() < 90.0 && b < 170.0
}

/// Find the follow "+" badge and derive the rest of the rail from it.
///
/// The badge is the only saturated red disc in the upper part of the rail. A
/// liked heart is red too, which is why the search band stops above it and why
/// the topmost run wins: follow always sits above like.
///
/// Returns `None` when no badge is visible — the usual reason is that this
/// author is already followed, in which case the caller keeps its last known
/// rail rather than guessing.
pub fn find_action_rail(img: &RgbImage) -> Option<ActionRail> {
    let (w, h) = (img.width() as f64, img.height() as f64);
    // Narrow enough that the ~40 px badge dominates the sampled row: it covers
    // 80 % of this band at its widest, so a stray red pixel from the video
    // cannot reach the threshold.
    let x0 = ((RAIL_X - RAIL_HALF_WIDTH) * w) as u32;
    let x1 = (((RAIL_X + RAIL_HALF_WIDTH) * w) as u32).min(img.width());
    // Band covering both layouts' badge positions with margin, stopping short
    // of layout 2's heart so a liked heart cannot be mistaken for the badge.
    let y0 = (0.300 * h) as u32;
    let y1 = ((0.440 * h) as u32).min(img.height());
    if x1 <= x0 || y1 <= y0 {
        return None;
    }

    let width = (x1 - x0) as f64;
    let mut runs: Vec<(u32, u32)> = Vec::new();
    let mut run_start: Option<u32> = None;
    for y in y0..y1 {
        let mut red = 0.0;
        for x in x0..x1 {
            if is_tiktok_red(img.get_pixel(x, y)) {
                red += 1.0;
            }
        }
        // Measured on a real capture: 0.83 at the badge's widest, dipping to
        // 0.44 across the white "+" glyph, and 0 elsewhere in the band.
        let is_badge_row = red / width >= BADGE_ROW_COVERAGE;
        match (is_badge_row, run_start) {
            (true, None) => run_start = Some(y),
            (false, Some(start)) => {
                runs.push((start, y));
                run_start = None;
            }
            _ => {}
        }
    }
    if let Some(start) = run_start {
        runs.push((start, y1));
    }

    // The badge measured 42 px tall (≈21 logical points) on this device.
    // Reject specks and anything tall enough to be a red banner in the video.
    let min_h = (0.018 * h) as u32;
    let max_h = (0.055 * h) as u32;
    let badge = runs
        .into_iter()
        .find(|(a, b)| b.saturating_sub(*a) >= min_h && b.saturating_sub(*a) <= max_h)?;
    let centre = (badge.0 + badge.1) as f64 / 2.0 / h;
    Some(ActionRail::from_follow(centre, true))
}

/// Does this frame carry a real action rail?
///
/// [`find_action_rail`] keys on the red follow badge, which is absent whenever
/// the author is already followed — so "no badge" cannot mean "do not act", and
/// the engine used to fall back to layout 2 and tap blind. That is wrong on two
/// screens that still show the compose bar and so still classify as `Feed`:
///
/// * a **LIVE preview card** in the FYP ("Đang LIVE / Nhấn để xem LIVE"), which
///   has no rail at all;
/// * a **mid-swipe transition**, where the rail is half faded out.
///
/// A live run tapped 14 of these in a row for 0 likes. What separates them is
/// structure, not colour: the real rail is a column of white glyphs spaced
/// evenly down the right edge. Measured centres, in logical points:
///
/// | frame | chain |
/// |---|---|
/// | normal video | 312, 377, 443, 512 (spacing 65–69) |
/// | already-followed author, heart filled | 382, 443, 511 |
/// | LIVE card | one stray run, no chain |
/// | mid-swipe | one stray run, no chain |
///
/// So: at least two white runs whose spacing matches the icon pitch.
pub fn rail_icons_present(img: &RgbImage) -> bool {
    rail_icon_centres(img).len() >= RAIL_MIN_ICONS
}

/// Locate the rail, cross-checking the two independent readings of it.
///
/// [`find_action_rail`] keys on the red follow badge and is the more precise
/// landmark when it is right, but it is a single red run and the topmost one
/// in its band wins — so anything red the video puts above the badge takes the
/// whole rail with it. Measured on `feed-same-card-2.jpg`, where a sponsored
/// card's pink "LIVE 8.8 Sale" ribbon produced a 27 px run above the real
/// 41 px badge and moved the like target 76 px, over half an icon pitch. The
/// tap then lands on the ribbon, and — worse — the redness probe reads the
/// ribbon at 79.7 instead of the heart at 3.1, close enough to the filled
/// threshold to also fake an "already liked" skip. That is the "tapped 14 in a
/// row for 0 likes" failure by another door.
///
/// The white-glyph chain is the other reading, and it did not move: all three
/// frames of that card give the same four centres. So the two are compared,
/// and when they disagree the one with more independent support wins — a chain
/// of three or more glyphs at a measured pitch outranks one red run, while a
/// two-glyph chain (which video content can imitate, see `feed-heart-liked`)
/// does not.
///
/// The chain reading also stands alone when the author is already followed and
/// there is no badge at all. The scan only sees *unfilled* icons, so a liked
/// heart is red and drops out, and the chain then starts at the comment bubble
/// one pitch below the heart. A redness probe one pitch above the first glyph
/// tells the two cases apart.
pub fn locate_action_rail(img: &RgbImage) -> Option<ActionRail> {
    match (find_action_rail(img), rail_from_icon_chain(img)) {
        (Some(badge), Some(chain)) => {
            let disagreement = (badge.like_y - chain.rail.like_y).abs();
            if disagreement <= chain.pitch * RAIL_AGREEMENT_PITCH_FRACTION
                || chain.icons < RAIL_CHAIN_OUTRANKS_BADGE
            {
                Some(badge)
            } else {
                Some(chain.rail)
            }
        }
        (Some(badge), None) => Some(badge),
        (None, Some(chain)) => Some(chain.rail),
        (None, None) => None,
    }
}

/// The chain reading together with what backs it, so it can be ranked against
/// the badge's single red run.
struct RailReading {
    rail: ActionRail,
    /// Icon spacing this chain measured, in screen fractions.
    pitch: f64,
    /// Glyph detections in the chain.
    icons: usize,
}

/// Derive the rail from the evenly spaced white glyphs alone.
fn rail_from_icon_chain(img: &RgbImage) -> Option<RailReading> {
    let centres = rail_icon_centres(img);
    if centres.len() < RAIL_MIN_ICONS {
        return None;
    }
    // Pitch from the first gap of the white-glyph chain.
    let pitch = centres[1] - centres[0];
    if !(RAIL_ICON_PITCH.0..=RAIL_ICON_PITCH.1).contains(&pitch) {
        return None;
    }
    // If a filled heart sits one pitch above the first white run, that run is
    // the comment bubble and the heart was excluded for being red; otherwise
    // the first run is the (unfilled) heart itself. Same primitive do_like uses
    // to confirm a like, so "filled" means exactly what it means there.
    let heart_above = centres[0] - pitch;
    let (like_y, comment_y) =
        if heart_above > 0.0 && icon_redness(img, RAIL_X, heart_above) > LIKE_FILLED_REDNESS {
            (heart_above, centres[0])
        } else {
            (centres[0], centres[1])
        };
    // The rail is a fixed control, not something that can be anywhere. A chain
    // claiming the heart is half an icon outside both known layouts is some
    // other row of bright things — measured live at 199 pt against a real 314,
    // on a frame still animating after a back gesture, where no follow badge
    // existed to contradict it. The tap missed by 115 pt.
    if !(LIKE_Y_LAYOUT1 - RAIL_LIKE_Y_SLACK..=LIKE_Y_LAYOUT2 + RAIL_LIKE_Y_SLACK).contains(&like_y)
    {
        return None;
    }
    Some(RailReading {
        rail: ActionRail {
            x: RAIL_X,
            follow_y: like_y - FOLLOW_TO_LIKE,
            like_y,
            comment_y,
            save_y: Some(comment_y + pitch),
            located: true,
        },
        pitch,
        icons: centres.len(),
    })
}

/// Minimum number of evenly spaced glyphs that counts as a rail.
const RAIL_MIN_ICONS: usize = 2;
/// Glyphs a chain needs before it overrules the follow badge. Two is enough to
/// *see* a rail but not to overrule one, because video content produces stray
/// two-run pairs at a plausible pitch; three is three independent detections.
const RAIL_CHAIN_OUTRANKS_BADGE: usize = 3;
/// How far the badge-derived like position may sit from the chain-derived one
/// and still be the same icon, as a fraction of the measured pitch. Half a
/// pitch is where a tap lands on the neighbouring button, so a quarter leaves a
/// full icon of margin.
const RAIL_AGREEMENT_PITCH_FRACTION: f64 = 0.25;
/// Where the like heart sits under each known layout, and how far outside that
/// span a chain-derived reading may still be believed. Half an icon pitch is
/// the point at which a tap lands on the neighbouring button, so it is the
/// widest slack that can still be called the same control.
const LIKE_Y_LAYOUT1: f64 = FOLLOW_Y_LAYOUT1 + FOLLOW_TO_LIKE;
const LIKE_Y_LAYOUT2: f64 = FOLLOW_Y_LAYOUT2 + FOLLOW_TO_LIKE;
const RAIL_LIKE_Y_SLACK: f64 = 40.0 / 667.0;
/// Vertical gap between neighbouring rail icons, in screen fractions. Measured
/// 65–69 logical points; the window is widened to absorb JPEG blur.
const RAIL_ICON_PITCH: (f64, f64) = (55.0 / 667.0, 80.0 / 667.0);
/// Band searched for glyphs: below the header, above the caption block.
const RAIL_ICON_BAND: (f64, f64) = (0.28, 0.85);
/// A row is part of a glyph when this much of the rail column is white.
const RAIL_ICON_COVERAGE: f64 = 0.35;
/// A bright run this tall, as a fraction of frame height, is not an icon. An
/// icon measured ~24 logical points (0.036 of the frame); this is four of them,
/// which nothing in the rail can reach but a washed-out video can.
const RAIL_SATURATED_RUN: f64 = 0.15;

/// Centres of the longest evenly spaced chain of white glyphs in the rail
/// column, as screen fractions.
fn rail_icon_centres(img: &RgbImage) -> Vec<f64> {
    let h = img.height();
    // Drop specks: a glyph is at least a few rows tall.
    let min_h = (0.0045 * h as f64) as u32;
    let centres: Vec<f64> = rail_glyph_runs(img)
        .into_iter()
        .filter(|(a, b)| b.saturating_sub(*a) >= min_h)
        .map(|(a, b)| (a + b) as f64 / 2.0 / h as f64)
        .collect();

    // Longest chain whose consecutive gaps match the icon pitch.
    let mut best: Vec<f64> = Vec::new();
    for i in 0..centres.len() {
        let mut chain = vec![centres[i]];
        for &c in &centres[i + 1..] {
            let gap = c - chain[chain.len() - 1];
            if gap >= RAIL_ICON_PITCH.0 && gap <= RAIL_ICON_PITCH.1 {
                chain.push(c);
            }
        }
        if chain.len() > best.len() {
            best = chain;
        }
    }
    best
}

/// Is the rail column washed out rather than empty?
///
/// [`rail_icons_present`] answers "is there an icon chain", and a frame whose
/// video goes near-white behind the right edge — a white product background, a
/// sky, a flash cut — answers no for a reason that has nothing to do with the
/// rail: every row in the band passes the white test at once, so the scan
/// returns one continuous run instead of a chain of glyphs.
///
/// That matters because the swipe check treats "no rail" as "the feed is
/// between cards". One blown-out frame would otherwise latch that conclusion
/// and let the *next* ordinary frame of the *same* card count as a new one —
/// reinstating exactly the false advance the rail check exists to remove.
///
/// A run taller than several icons cannot be an icon, so it is this instead.
pub fn rail_column_saturated(img: &RgbImage) -> bool {
    let h = img.height() as f64;
    let limit = (RAIL_SATURATED_RUN * h) as u32;
    rail_glyph_runs(img)
        .into_iter()
        .any(|(a, b)| b.saturating_sub(a) >= limit)
}

/// Runs of rows in the rail column that read as bright glyph material.
fn rail_glyph_runs(img: &RgbImage) -> Vec<(u32, u32)> {
    let (w, h) = (img.width(), img.height());
    let x0 = ((RAIL_X - RAIL_HALF_WIDTH) * w as f64) as u32;
    let x1 = (((RAIL_X + RAIL_HALF_WIDTH) * w as f64) as u32).min(w);
    let y0 = (RAIL_ICON_BAND.0 * h as f64) as u32;
    let y1 = ((RAIL_ICON_BAND.1 * h as f64) as u32).min(h);
    if x1 <= x0 || y1 <= y0 {
        return Vec::new();
    }
    let width = (x1 - x0) as f64;
    // Glyph rows, then runs of them.
    let mut runs: Vec<(u32, u32)> = Vec::new();
    let mut start: Option<u32> = None;
    for y in y0..y1 {
        let mut white = 0.0;
        for x in x0..x1 {
            let p = img.get_pixel(x, y).0;
            let lo = p[0].min(p[1]).min(p[2]);
            let hi = p[0].max(p[1]).max(p[2]);
            if lo > 190 && hi - lo < 40 {
                white += 1.0;
            }
        }
        let glyph = white / width >= RAIL_ICON_COVERAGE;
        match (glyph, start) {
            (true, None) => start = Some(y),
            (false, Some(s)) => {
                runs.push((s, y));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        runs.push((s, y1));
    }
    runs
}

/// Mean "redness" of a small box centred on a rail icon: high when TikTok has
/// filled it (liked heart, armed Send button), near zero for the white glyph.
fn icon_redness(img: &RgbImage, x: f64, y: f64) -> f64 {
    let (r, g, b) = region_rgb(img, x - 0.040, y - 0.021, x + 0.040, y + 0.021);
    r - (g + b) / 2.0
}

/// Redness of the like heart on this rail.
pub fn like_redness_at(img: &RgbImage, rail: &ActionRail) -> f64 {
    icon_redness(img, rail.x, rail.like_y)
}

/// Is the author's follow badge still showing? Used to confirm a follow tap.
pub fn follow_badge_present(img: &RgbImage, rail: &ActionRail) -> bool {
    icon_redness(img, rail.x, rail.follow_y) > 45.0
}

/// Where the compose "+" button sits in TikTok's bottom bar. The button is a
/// white pill with a cyan left edge and a pink right edge — three independent
/// colour facts in a fixed order, which a video frame does not reproduce by
/// accident the way a plain brightness test would.
const PLUS_BAND: (f64, f64) = (0.9430, 0.9760);
const PLUS_CYAN_X: (f64, f64) = (0.4440, 0.4533);
const PLUS_WHITE_X: (f64, f64) = (0.4700, 0.5330);
const PLUS_PINK_X: (f64, f64) = (0.5453, 0.5547);

/// Margins for the compose-bar test. Measured on the live feed: cyan B−R ≈ 170,
/// white min channel ≈ 227, pink R−G ≈ 129 — every threshold has ≥2× headroom.
const PLUS_CYAN_MIN: f64 = 60.0;
const PLUS_WHITE_MIN: f64 = 190.0;
const PLUS_PINK_MIN: f64 = 60.0;

/// The close-button template, decoded once and normalised to logical points.
/// Decoding the PNG per frame showed up as pure overhead in the watcher, which
/// runs this several times a second.
fn close_x_needle() -> Option<&'static image::GrayImage> {
    static NEEDLE: std::sync::OnceLock<Option<image::GrayImage>> = std::sync::OnceLock::new();
    NEEDLE
        .get_or_init(|| {
            let rgb = image::load_from_memory(CLOSE_X_TEMPLATE).ok()?.to_rgb8();
            let logical = image::imageops::resize(
                &rgb,
                ((rgb.width() as f64 / TEMPLATE_SCALE).round() as u32).max(1),
                ((rgb.height() as f64 / TEMPLATE_SCALE).round() as u32).max(1),
                image::imageops::FilterType::Triangle,
            );
            Some(to_gray(&logical))
        })
        .as_ref()
}

/// What is covering the screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScreenKind {
    /// TikTok with its compose bar visible — the feed is reachable.
    Feed,
    /// A sheet with a close button; payload is the ✕ centre in screen fractions.
    ClosableSheet { x: f64, y: f64, score: f64 },
    /// The "Chọn chủ đề bạn thích" onboarding page — no ✕, skipped with a pill.
    InterestPicker,
    /// A TikTok LIVE room. Swiping does not leave one — it scrolls the room's
    /// own content — so a session that drifts in gets stuck until it taps the
    /// ✕. Its layout has nothing in common with the feed, so the action rail
    /// positions are meaningless here.
    LiveRoom,
    /// An iOS system alert (`UIAlertController`) on the dimmed backdrop —
    /// SpringBoard's, not TikTok's. Payload is the dismissive button's centre.
    ///
    /// A SIM-less device raises "iPhone chưa được Kích hoạt" on its own every
    /// few minutes, and while it is up nothing underneath can be driven, so a
    /// session that cannot close this one stalls at zero videos forever.
    SystemAlert { x: f64, y: f64 },
    /// TikTok's bar is not visible and no known overlay matched. Never tap on
    /// this: it covers "some other app", "mid-transition", and "capture noise".
    Unknown,
}

impl ScreenKind {
    pub fn label(&self) -> &'static str {
        match self {
            ScreenKind::Feed => "feed",
            ScreenKind::ClosableSheet { .. } => "sheet",
            ScreenKind::InterestPicker => "interest-picker",
            ScreenKind::LiveRoom => "live-room",
            ScreenKind::SystemAlert { .. } => "system-alert",
            ScreenKind::Unknown => "unknown",
        }
    }
}

/// A classification plus the measurements behind it.
#[derive(Debug, Clone, Copy)]
pub struct ScreenObservation {
    pub kind: ScreenKind,
    pub evidence: Evidence,
}

/// Kind of feed card currently visible. This is intentionally conservative:
/// uncertain cards are treated as ordinary video and the engine only performs
/// a LIVE-specific gesture after a positive visual marker.
///
/// There is deliberately no `PhotoCarousel` here. There was, keyed on the page
/// dots, and measured against 40 real cards it fired on 10 of them: one photo
/// post and nine videos, while missing three photo posts. The dots cannot carry
/// it — six of the seven are dim grey at ~50% opacity over the photo, so on a
/// real capture only the *active* dot clears any brightness threshold, and every
/// rule tried in their place (evenly spaced local maxima; uniform-width evenly
/// spaced blobs) matched caption text instead, at 19 false positives out of 36.
/// A line of same-size letters at a fixed font is exactly a row of evenly spaced
/// uniform blobs.
///
/// What separates a photo post from a video is not on any single frame: the
/// photo post does not change. See `NurtureEngine::card_is_still`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedCardKind {
    Video,
    LivePreview,
    TransitionOrUnknown,
}

/// True only when TikTok's feed chrome is reachable and no transient notice is
/// covering the content. A toast is still a feed frame, but waiting for it to
/// fade avoids tapping while its UI text is in the vision evidence.
pub fn is_actionable_feed(observation: &ScreenObservation) -> bool {
    observation.kind == ScreenKind::Feed && !observation.evidence.ad_feedback_notice
}

/// Classify a frame and require an actionable feed rather than just a visible
/// compose bar.
pub fn feed_ready(img: &RgbImage, logical_width: Option<f64>) -> bool {
    is_actionable_feed(&classify(img, logical_width))
}

impl ScreenObservation {
    /// Compact one-line form for debug logs and frame-dump filenames.
    pub fn debug_line(&self) -> String {
        let e = &self.evidence;
        format!(
            "{} bar={} notice={} cyan={:.0} white={:.0} pink={:.0} x_score={:.3} light={:.0} neutral={:.1} cta={:.0} live={:.0}",
            self.kind.label(),
            e.compose_bar,
            e.ad_feedback_notice,
            e.cyan,
            e.white,
            e.pink,
            e.close_x_score,
            e.picker_light,
            e.picker_neutral,
            e.picker_cta,
            e.live_pill,
        )
    }
}

/// Per-feature evidence behind a classification, for debug logs and threshold
/// calibration against real captures.
#[derive(Debug, Clone, Copy, Default)]
pub struct Evidence {
    pub compose_bar: bool,
    pub cyan: f64,
    pub white: f64,
    pub pink: f64,
    pub close_x_score: f64,
    pub picker_light: f64,
    pub picker_neutral: f64,
    pub picker_cta: f64,
    pub live_pill: f64,
    pub alert_blue: f64,
    /// TikTok's transient ad-feedback toast. It has no safe dismiss button and
    /// fades by itself, so the engine waits instead of tapping a guess.
    pub ad_feedback_notice: bool,
}

/// Mean (R, G, B) of a fractional region, sampled every other pixel.
fn region_rgb(img: &RgbImage, x0: f64, y0: f64, x1: f64, y1: f64) -> (f64, f64, f64) {
    let (w, h) = (img.width() as f64, img.height() as f64);
    let px0 = (x0 * w).max(0.0) as u32;
    let py0 = (y0 * h).max(0.0) as u32;
    let px1 = ((x1 * w) as u32).min(img.width());
    let py1 = ((y1 * h) as u32).min(img.height());
    let (mut rs, mut gs, mut bs, mut n) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let mut y = py0;
    while y < py1 {
        let mut x = px0;
        while x < px1 {
            let p = img.get_pixel(x, y);
            rs += p[0] as f64;
            gs += p[1] as f64;
            bs += p[2] as f64;
            n += 1.0;
            x += 2;
        }
        y += 2;
    }
    if n == 0.0 {
        (0.0, 0.0, 0.0)
    } else {
        (rs / n, gs / n, bs / n)
    }
}

fn region_brightness(img: &RgbImage, x0: f64, y0: f64, x1: f64, y1: f64) -> f64 {
    let (r, g, b) = region_rgb(img, x0, y0, x1, y1);
    (r + g + b) / 3.0
}

/// Mean **per-pixel** channel spread of a region — near 0 for greys and whites,
/// high for saturated colour. Averaging the spread rather than spreading the
/// average matters: a frame split between red and blue content averages to a
/// neutral grey, and would pass a mean-based test it has no business passing.
fn region_colourfulness(img: &RgbImage, x0: f64, y0: f64, x1: f64, y1: f64) -> f64 {
    let (w, h) = (img.width() as f64, img.height() as f64);
    let px0 = (x0 * w).max(0.0) as u32;
    let py0 = (y0 * h).max(0.0) as u32;
    let px1 = ((x1 * w) as u32).min(img.width());
    let py1 = ((y1 * h) as u32).min(img.height());
    let (mut total, mut n) = (0.0f64, 0.0f64);
    let mut y = py0;
    while y < py1 {
        let mut x = px0;
        while x < px1 {
            let p = img.get_pixel(x, y);
            let max = p[0].max(p[1]).max(p[2]) as f64;
            let min = p[0].min(p[1]).min(p[2]) as f64;
            total += max - min;
            n += 1.0;
            x += 2;
        }
        y += 2;
    }
    if n == 0.0 {
        0.0
    } else {
        total / n
    }
}

/// Is TikTok's compose bar (the "+" pill) on screen? True on any TikTok tab,
/// false on the Home screen and behind any full-screen sheet.
pub fn compose_bar_visible(img: &RgbImage) -> (bool, f64, f64, f64) {
    let (cr, cg, cb) = region_rgb(img, PLUS_CYAN_X.0, PLUS_BAND.0, PLUS_CYAN_X.1, PLUS_BAND.1);
    let cyan = (cb - cr).min(cg - cr);
    let (wr, wg, wb) = region_rgb(
        img,
        PLUS_WHITE_X.0,
        PLUS_BAND.0,
        PLUS_WHITE_X.1,
        PLUS_BAND.1,
    );
    let white = wr.min(wg).min(wb);
    let (pr, pg, pb) = region_rgb(img, PLUS_PINK_X.0, PLUS_BAND.0, PLUS_PINK_X.1, PLUS_BAND.1);
    let pink = (pr - pg).min(pr - pb);
    let ok = cyan >= PLUS_CYAN_MIN && white >= PLUS_WHITE_MIN && pink >= PLUS_PINK_MIN;
    (ok, cyan, white, pink)
}

/// Mean luma, luma standard deviation and per-pixel colourfulness for a region.
/// The toast detector uses all three so a dark video band is not enough to make
/// a frame look blocked.
fn region_luma_stats(img: &RgbImage, x0: f64, y0: f64, x1: f64, y1: f64) -> (f64, f64, f64) {
    let (w, h) = (img.width() as f64, img.height() as f64);
    let px0 = (x0 * w).max(0.0) as u32;
    let py0 = (y0 * h).max(0.0) as u32;
    let px1 = ((x1 * w) as u32).min(img.width());
    let py1 = ((y1 * h) as u32).min(img.height());
    let (mut sum, mut sum_sq, mut colour, mut n) = (0.0, 0.0, 0.0, 0.0);
    let mut y = py0;
    while y < py1 {
        let mut x = px0;
        while x < px1 {
            let p = img.get_pixel(x, y);
            let luma = (p[0] as f64 + p[1] as f64 + p[2] as f64) / 3.0;
            sum += luma;
            sum_sq += luma * luma;
            colour += (p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2])) as f64;
            n += 1.0;
            x += 2;
        }
        y += 2;
    }
    if n == 0.0 {
        return (0.0, 0.0, 0.0);
    }
    let mean = sum / n;
    let variance = (sum_sq / n - mean * mean).max(0.0);
    (mean, variance.sqrt(), colour / n)
}

/// TikTok's "Bạn sẽ thấy ít quảng cáo như thế này hơn" notice is a dark,
/// rounded, short-lived toast over the upper feed. It has no reliable close
/// target; identifying it lets nurture pause until TikTok removes it itself.
pub fn ad_feedback_notice_present(img: &RgbImage) -> bool {
    if !compose_bar_visible(img).0 {
        return false;
    }

    let (core_luma, core_sd, core_colour) = region_luma_stats(img, 0.12, 0.12, 0.88, 0.18);
    let above = region_brightness(img, 0.12, 0.09, 0.88, 0.12);
    let below = region_brightness(img, 0.12, 0.18, 0.88, 0.21);
    let left = region_brightness(img, 0.02, 0.12, 0.08, 0.18);
    let right = region_brightness(img, 0.92, 0.12, 0.98, 0.18);
    let surrounding = (above + below + left + right) / 4.0;
    let capsule_contrast = surrounding - core_luma;

    (95.0..=160.0).contains(&core_luma)
        && (18.0..=46.0).contains(&core_sd)
        && (8.0..=35.0).contains(&core_colour)
        && capsule_contrast >= 20.0
}

/// Locate TikTok's close button, returning its centre as screen fractions plus
/// the NCC score. Shape match inside [`CLOSE_X_REGION`] only — cheap, and a
/// match in the video area would be a false positive anyway.
///
/// `logical_width` is the device's width in WDA points; it tells us how much
/// the capture is scaled up so the template can be normalised to match. Pass
/// `None` to assume the common @2x case.
pub fn find_close_button(img: &RgbImage, logical_width: Option<f64>) -> Option<(f64, f64, f64)> {
    let (w, h) = (img.width(), img.height());
    let (rx0, ry0, rx1, ry1) = CLOSE_X_REGION;
    let px0 = (rx0 * w as f64) as u32;
    let py0 = (ry0 * h as f64) as u32;
    let px1 = ((rx1 * w as f64) as u32).min(w);
    let py1 = ((ry1 * h as f64) as u32).min(h);
    if px1 <= px0 || py1 <= py0 {
        return None;
    }
    let crop = image::imageops::crop_imm(img, px0, py0, px1 - px0, py1 - py0).to_image();

    // Normalise the capture to logical points by its own device pixel ratio;
    // the template is pre-normalised the same way. Matching in a shared space
    // is what lets one template serve @2x and @3x devices, and it is also ~4×
    // less correlation work than matching at native resolution.
    let capture_ratio = match logical_width {
        Some(lw) if lw > 0.0 => (w as f64 / lw).max(1.0),
        _ => 2.0,
    };
    let scaled = if capture_ratio <= 1.01 {
        crop
    } else {
        image::imageops::resize(
            &crop,
            ((crop.width() as f64 / capture_ratio).round() as u32).max(1),
            ((crop.height() as f64 / capture_ratio).round() as u32).max(1),
            image::imageops::FilterType::Triangle,
        )
    };
    let haystack = to_gray(&scaled);
    let needle = close_x_needle()?;

    let m = find_template(&haystack, needle)?;
    let cx = px0 as f64 + m.cx * capture_ratio;
    let cy = py0 as f64 + m.cy * capture_ratio;
    Some((cx / w as f64, cy / h as f64, m.score))
}

fn is_promo_close_dark(p: &image::Rgb<u8>) -> bool {
    let max = p[0].max(p[1]).max(p[2]);
    let min = p[0].min(p[1]).min(p[2]);
    max <= 190 && max.saturating_sub(min) >= 35
}

fn is_promo_close_cross(p: &image::Rgb<u8>) -> bool {
    let min = p[0].min(p[1]).min(p[2]);
    let max = p[0].max(p[1]).max(p[2]);
    min >= 145 && max >= 180 && max.saturating_sub(min) <= 120
}

fn is_promo_warm(p: &image::Rgb<u8>) -> bool {
    let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
    r > 150 && r > g + 45 && r > b + 30
}

/// Locate the dismissive X on TikTok's floating upper-left promo card.
///
/// This overlay leaves TikTok's compose bar visible, so the normal feed
/// short-circuit would otherwise hide it. The match requires three independent
/// signals: a dark circular button, a light diagonal cross inside it, and a
/// warm/red promo surface immediately below-left. A video frame with a random
/// dark patch or white glyph does not satisfy all three.
pub fn find_promo_close(img: &RgbImage) -> Option<(f64, f64, f64)> {
    let (w, h) = (img.width() as i32, img.height() as i32);
    if w < 160 || h < 240 {
        return None;
    }
    let radius = ((0.021 * w as f64).round() as i32).max(8);
    let r2 = radius * radius;
    let ann_lo = ((radius as f64 * 1.18).powi(2)) as i32;
    let ann_hi = ((radius as f64 * 1.65).powi(2)) as i32;
    let x0 = (PROMO_CLOSE_REGION.0 * w as f64) as i32;
    let x1 = (PROMO_CLOSE_REGION.2 * w as f64) as i32;
    let y0 = (PROMO_CLOSE_REGION.1 * h as f64) as i32;
    let y1 = (PROMO_CLOSE_REGION.3 * h as f64) as i32;
    // The close control is stable on the iPhone 8 layout; an 8-pixel grid is
    // enough to cover its 16-pixel radius without scanning every candidate.
    let step = (radius / 2).max(4) as usize;
    let mut best: Option<(f64, f64, f64)> = None;

    for cy in (y0..=y1).step_by(step) {
        for cx in (x0..=x1).step_by(step) {
            let mut inner_n = 0u32;
            let mut dark_n = 0u32;
            let mut cross_n = 0u32;
            let mut inner_max_sum = 0.0f64;
            for dy in (-radius..=radius).step_by(2) {
                for dx in (-radius..=radius).step_by(2) {
                    if dx * dx + dy * dy > r2 {
                        continue;
                    }
                    let x = cx + dx;
                    let y = cy + dy;
                    if x < 0 || y < 0 || x >= w || y >= h {
                        continue;
                    }
                    let p = img.get_pixel(x as u32, y as u32);
                    inner_n += 1;
                    inner_max_sum += p[0].max(p[1]).max(p[2]) as f64;
                    dark_n += is_promo_close_dark(p) as u32;
                    cross_n += is_promo_close_cross(p) as u32;
                }
            }
            if inner_n == 0 {
                continue;
            }

            let mut ann_n = 0u32;
            let mut ann_max_sum = 0.0f64;
            for dy in (-radius * 2..=radius * 2).step_by(3) {
                for dx in (-radius * 2..=radius * 2).step_by(3) {
                    let d2 = dx * dx + dy * dy;
                    if d2 < ann_lo || d2 > ann_hi {
                        continue;
                    }
                    let x = cx + dx;
                    let y = cy + dy;
                    if x < 0 || y < 0 || x >= w || y >= h {
                        continue;
                    }
                    let p = img.get_pixel(x as u32, y as u32);
                    ann_n += 1;
                    ann_max_sum += p[0].max(p[1]).max(p[2]) as f64;
                }
            }
            if ann_n == 0 {
                continue;
            }

            // The gift/promo artwork sits below-left of its close control.
            let px0 = (cx - radius * 5).max(0);
            let px1 = (cx - radius).min(w);
            let py0 = (cy + radius).min(h);
            let py1 = (cy + radius * 6).min(h);
            let mut promo_n = 0u32;
            let mut warm_n = 0u32;
            for y in (py0..py1).step_by(3) {
                for x in (px0..px1).step_by(3) {
                    promo_n += 1;
                    warm_n += is_promo_warm(img.get_pixel(x as u32, y as u32)) as u32;
                }
            }
            if promo_n == 0 {
                continue;
            }

            let dark = dark_n as f64 / inner_n as f64;
            let cross = cross_n as f64 / inner_n as f64;
            let contrast = ann_max_sum / ann_n as f64 - inner_max_sum / inner_n as f64;
            let warm = warm_n as f64 / promo_n as f64;
            if dark < 0.65 || cross < 0.04 || contrast < 18.0 || warm < 0.10 {
                continue;
            }
            let score = (0.86
                + (dark - 0.65).min(0.25) * 0.15
                + (cross - 0.04).min(0.15) * 0.20
                + (warm - 0.10).min(0.50) * 0.05)
                .min(0.99);
            if best.is_none_or(|(_, _, old)| score > old) {
                best = Some((cx as f64 / w as f64, cy as f64 / h as f64, score));
            }
        }
    }
    best
}

/// Does this look like the "Chọn chủ đề bạn thích" onboarding page?
///
/// Deliberately more than a brightness test — a white video frame passes that.
/// Three independent facts must hold at once: the page is light, it is close to
/// neutral grey over a large area (video rarely is), and a saturated call-to-
/// action pill sits at the bottom on white.
fn interest_picker_evidence(img: &RgbImage) -> (bool, f64, f64, f64) {
    let light = region_brightness(img, 0.10, 0.20, 0.90, 0.62);
    let neutral = region_colourfulness(img, 0.10, 0.20, 0.90, 0.62);
    let (cr, cg, cb) = region_rgb(img, 0.20, 0.885, 0.80, 0.945);
    let cta = (cr - cg).min(cr - cb);
    let ok = light > 215.0 && neutral < 18.0 && cta > 45.0;
    (ok, light, neutral, cta)
}

/// Is this a LIVE room? Keyed on the author follow pill, which sits in a fixed
/// spot at the top of every LIVE and is TikTok red — a saturation the feed does
/// not reach in that box even with colourful video behind it.
fn live_room_evidence(img: &RgbImage) -> (bool, f64) {
    let (r, g, b) = region_rgb(
        img,
        LIVE_FOLLOW_PILL.0,
        LIVE_FOLLOW_PILL.1,
        LIVE_FOLLOW_PILL.2,
        LIVE_FOLLOW_PILL.3,
    );
    let rg = r - g;
    let rb = r - b;
    (rg >= LIVE_PILL_RG_MIN && rb >= LIVE_PILL_RB_MIN, rg.min(rb))
}

fn live_preview_label_present(img: &RgbImage) -> bool {
    let (w, h) = (img.width() as f64, img.height() as f64);
    let x0 = (0.02 * w) as u32;
    let x1 = (0.27 * w) as u32;
    let y0 = (0.76 * h) as u32;
    let y1 = (0.87 * h) as u32;
    if x1 <= x0 || y1 <= y0 {
        return false;
    }
    let mut hits = 0u32;
    let mut total = 0u32;
    for y in (y0..y1).step_by(2) {
        for x in (x0..x1).step_by(2) {
            let p = img.get_pixel(x, y).0;
            total += 1;
            if p[0] > 175 && p[1] < 120 && p[2] > 70 && p[0] as i16 - p[1] as i16 > 50 {
                hits += 1;
            }
        }
    }
    total > 0 && hits as f64 / total as f64 >= 0.035
}

pub fn feed_card_kind(img: &RgbImage) -> FeedCardKind {
    if !compose_bar_visible(img).0 {
        return FeedCardKind::TransitionOrUnknown;
    }
    if live_preview_label_present(img) && !rail_icons_present(img) {
        return FeedCardKind::LivePreview;
    }
    if !rail_icons_present(img) {
        return FeedCardKind::TransitionOrUnknown;
    }
    FeedCardKind::Video
}

/// One row of the composer's emoji grid, as screen fractions.
pub type EmojiRow = Vec<(f64, f64)>;

/// Locate the emoji grid inside the open composer.
///
/// The grid **moves**: TikTok inserts a "Đã sử dụng gần đây" section above it
/// once the account has used an emoji, which shifts every cell down. Hard-coded
/// cells miss after the first successful comment, so the grid is found per
/// frame instead.
///
/// Emoji are large saturated-yellow blobs on a near-white panel, which makes
/// them cheap to find without decoding glyphs. Rows with fewer than
/// [`MIN_EMOJI_PER_ROW`] blobs are dropped — that is what excludes the partial
/// "recently used" row and leaves the full grid.
pub fn find_emoji_grid(img: &RgbImage) -> Vec<EmojiRow> {
    let (w, h) = (img.width(), img.height());
    let y_start = (0.55 * h as f64) as u32;

    // Row bands: count yellow pixels per scanline, then split into runs.
    let mut counts = vec![0u32; h as usize];
    for y in y_start..h {
        let mut n = 0;
        let mut x = 0;
        while x < w {
            let p = img.get_pixel(x, y);
            if is_emoji_yellow(p) {
                n += 1;
            }
            x += 2;
        }
        counts[y as usize] = n;
    }

    let row_min = (0.012 * w as f64) as u32; // a row of emoji is wide
    let mut rows: Vec<(u32, u32)> = Vec::new();
    let mut start: Option<u32> = None;
    for y in y_start..h {
        let hit = counts[y as usize] >= row_min;
        match (hit, start) {
            (true, None) => start = Some(y),
            (false, Some(s)) => {
                if y - s >= (0.012 * h as f64) as u32 {
                    rows.push((s, y));
                }
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        rows.push((s, h));
    }

    // Within each band, split yellow into horizontal runs — one per emoji.
    let mut grid = Vec::new();
    for (y0, y1) in rows {
        let mid = (y0 + y1) / 2;
        let mut cells: EmojiRow = Vec::new();
        let mut run: Option<u32> = None;
        let mut gap = 0u32;
        for x in 0..w {
            let mut hit = false;
            let mut y = y0;
            while y < y1 {
                if is_emoji_yellow(img.get_pixel(x, y)) {
                    hit = true;
                    break;
                }
                y += 2;
            }
            match (hit, run) {
                (true, None) => {
                    run = Some(x);
                    gap = 0;
                }
                (true, Some(_)) => gap = 0,
                (false, Some(s)) => {
                    gap += 1;
                    // Tolerate the gaps inside one glyph (eyes, mouth).
                    if gap > (0.012 * w as f64) as u32 {
                        let end = x - gap;
                        if end > s && end - s >= (0.02 * w as f64) as u32 {
                            cells.push(((s + end) as f64 / 2.0 / w as f64, mid as f64 / h as f64));
                        }
                        run = None;
                    }
                }
                _ => {}
            }
        }
        if let Some(s) = run {
            if w > s {
                cells.push(((s + w) as f64 / 2.0 / w as f64, mid as f64 / h as f64));
            }
        }
        if cells.len() >= MIN_EMOJI_PER_ROW {
            grid.push(cells);
        }
    }
    grid
}

/// Rows below this are partial ("recently used"), not the full grid.
const MIN_EMOJI_PER_ROW: usize = 5;

/// Emoji glyphs are a saturated yellow that the near-white panel never reaches.
fn is_emoji_yellow(p: &image::Rgb<u8>) -> bool {
    let (r, g, b) = (p[0] as f64, p[1] as f64, p[2] as f64);
    r > 200.0 && g > 150.0 && b < 120.0 && (r - b) > 90.0
}

/// Is the comment composer open? Keyed on the send arrow, which only exists
/// there — the drawer's comment list has nothing at that spot.
pub fn composer_send_redness(img: &RgbImage) -> f64 {
    icon_redness(img, COMPOSER_SEND.0, COMPOSER_SEND.1)
}

/// Classify one captured frame.
pub fn classify(img: &RgbImage, logical_width: Option<f64>) -> ScreenObservation {
    let mut ev = Evidence::default();
    let (bar, cyan, white, pink) = compose_bar_visible(img);
    ev.compose_bar = bar;
    ev.cyan = cyan;
    ev.white = white;
    ev.pink = pink;
    ev.ad_feedback_notice = ad_feedback_notice_present(img);

    let done = |kind, evidence| ScreenObservation { kind, evidence };

    // System alerts before everything else that taps: they sit above TikTok on
    // a dimmed backdrop, so any coordinate derived from the app underneath is
    // both wrong and unreachable.
    if let Some((x, y, blue)) = find_system_alert(img) {
        ev.alert_blue = blue;
        return done(ScreenKind::SystemAlert { x, y }, ev);
    }

    // Floating promo cards keep the compose bar visible but place a dark ✕ in
    // the upper-left, so this must run before the feed short-circuit.
    if let Some((x, y, score)) = find_promo_close(img) {
        ev.close_x_score = score;
        return done(ScreenKind::ClosableSheet { x, y, score }, ev);
    }

    // A visible compose bar means nothing is covering TikTok's chrome, so no
    // full-screen sheet can be up. Checking it after the strict system-alert
    // signature keeps an iOS alert from being mistaken for a reachable feed
    // when its dimmed backdrop leaves enough of the bar visible.
    // Measured before the feed short-circuit, not after. The LIVE reading used
    // to be taken only on frames that had already failed the compose-bar test,
    // so `live_pill` was 0.0 on every feed frame — and `RIVIU_FRAME_DUMP`, the
    // one tool for calibrating the threshold against real captures, could
    // therefore never show how close an ordinary feed frame gets to it.
    let (live, pill) = live_room_evidence(img);
    ev.live_pill = pill;

    if bar {
        return done(ScreenKind::Feed, ev);
    }

    // A LIVE room before the ✕ search: its product cards carry a grey ✕ that
    // scores above threshold, and tapping that only closes the card while the
    // session stays stuck in the room.
    if live {
        return done(ScreenKind::LiveRoom, ev);
    }

    if let Some((x, y, score)) = find_close_button(img, logical_width) {
        ev.close_x_score = score;
        if score >= CLOSE_X_THRESHOLD {
            return done(ScreenKind::ClosableSheet { x, y, score }, ev);
        }
    }

    let (picker, light, neutral, cta) = interest_picker_evidence(img);
    ev.picker_light = light;
    ev.picker_neutral = neutral;
    ev.picker_cta = cta;
    if picker {
        return done(ScreenKind::InterestPicker, ev);
    }

    done(ScreenKind::Unknown, ev)
}

// ── iOS system alert ─────────────────────────────────────────────────────
//
// Every `UIAlertController` looks the same: a light rounded panel centred on a
// backdrop iOS dims to near-black, with tinted-blue button labels in a row at
// the bottom. Those three things together are what we match — the panel alone
// would also fire on TikTok's own white sheets, and blue text alone on any
// link. Measured on the activation alert of `05101fdb` (750×1334):
//
//   panel rows           y 0.394 – 0.606
//   button label rows    y 0.564 – 0.587   (centre 0.575)
//   left / right button  x 0.320 / 0.679
//   inside the panel     min-channel mean 175
//   backdrop at x < 0.08 max-channel mean   2
//   blue share in band   0.180
//
// Only the *dismissive* button is ever returned. On a two-button alert that is
// the left one, which is where iOS puts Cancel / Bỏ qua / Not Now; the right
// one is the affirmative action and must never be pressed blind.
const ALERT_SEARCH_Y: (f64, f64) = (0.22, 0.86);
const ALERT_BAND_H: f64 = 0.030;
const ALERT_PANEL_X: (f64, f64) = (0.20, 0.80);
const ALERT_BACKDROP_X: f64 = 0.08;
/// Panel interior must be this light (mean of the darkest channel).
const ALERT_PANEL_MIN: f64 = 140.0;
/// Backdrop must be this dark (mean of the brightest channel).
const ALERT_BACKDROP_MAX: f64 = 70.0;
/// Share of pixels in the band that are tinted-blue label strokes.
const ALERT_BLUE_MIN: f64 = 0.04;

fn is_alert_blue(r: i32, g: i32, b: i32) -> bool {
    b - r > 50 && b > 150 && b > g
}

/// Locate the dismissive button of an iOS system alert, as screen fractions
/// plus the blue share that carried the match.
pub fn find_system_alert(img: &RgbImage) -> Option<(f64, f64, f64)> {
    let (w, h) = (img.width(), img.height());
    if w < 32 || h < 32 {
        return None;
    }
    let band_h = ((ALERT_BAND_H * h as f64) as u32).max(4);
    let step = (band_h / 2).max(2);
    let px0 = (ALERT_PANEL_X.0 * w as f64) as u32;
    let px1 = ((ALERT_PANEL_X.1 * w as f64) as u32).min(w);
    let bx1 = ((ALERT_BACKDROP_X * w as f64) as u32).max(1);

    let mut best: Option<(f64, f64, f64)> = None;
    let mut y0 = (ALERT_SEARCH_Y.0 * h as f64) as u32;
    let y_end = ((ALERT_SEARCH_Y.1 * h as f64) as u32).min(h);
    while y0 + band_h <= y_end {
        let y1 = y0 + band_h;
        let (mut panel_dark_sum, mut panel_n) = (0u64, 0u64);
        let (mut back_bright_sum, mut back_n) = (0u64, 0u64);
        let (mut blue_n, mut blue_total) = (0u64, 0u64);
        // Columns of blue label pixels, so a two-button row can be split.
        let mut blue_cols: Vec<u32> = Vec::new();

        for y in (y0..y1).step_by(2) {
            for x in (px0..px1).step_by(2) {
                let p = img.get_pixel(x, y).0;
                let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
                panel_dark_sum += r.min(g).min(b) as u64;
                panel_n += 1;
                blue_total += 1;
                if is_alert_blue(r, g, b) {
                    blue_n += 1;
                    blue_cols.push(x);
                }
            }
            for x in (0..bx1).step_by(2) {
                let p = img.get_pixel(x, y).0;
                back_bright_sum += (p[0].max(p[1]).max(p[2])) as u64;
                back_n += 1;
            }
        }
        if panel_n == 0 || back_n == 0 || blue_total == 0 {
            y0 += step;
            continue;
        }
        let panel = panel_dark_sum as f64 / panel_n as f64;
        let backdrop = back_bright_sum as f64 / back_n as f64;
        let blue = blue_n as f64 / blue_total as f64;
        if panel >= ALERT_PANEL_MIN && backdrop <= ALERT_BACKDROP_MAX && blue >= ALERT_BLUE_MIN {
            if let Some(x) = dismissive_button_x(&blue_cols, w) {
                let cy = (y0 + y1) as f64 / 2.0 / h as f64;
                if best.is_none_or(|(_, _, b)| blue > b) {
                    best = Some((x, cy, blue));
                }
            }
        }
        y0 += step;
    }
    best
}

/// Centre of the dismissive button given the blue label columns in one band.
///
/// Labels are split into clusters by the widest horizontal gap; a gap that wide
/// only exists between two buttons, never inside one word's letter spacing. One
/// cluster means a single-button alert, where the only button is dismissive.
fn dismissive_button_x(cols: &[u32], w: u32) -> Option<f64> {
    if cols.is_empty() {
        return None;
    }
    let mut sorted = cols.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    // A gap must exceed a letter-spacing run to count as a button boundary.
    let min_gap = (w as f64 * 0.06) as u32;
    let mut split = None;
    let mut widest = 0u32;
    for pair in sorted.windows(2) {
        let gap = pair[1] - pair[0];
        if gap > widest {
            widest = gap;
            split = Some(pair[0]);
        }
    }
    let cluster: Vec<u32> = match split {
        Some(edge) if widest >= min_gap => sorted.iter().copied().filter(|c| *c <= edge).collect(),
        _ => sorted,
    };
    let lo = *cluster.first()? as f64;
    let hi = *cluster.last()? as f64;
    Some((lo + hi) / 2.0 / w as f64)
}

/// "Redness" of the like heart at the fallback rail position.
pub fn like_redness(img: &RgbImage) -> f64 {
    icon_redness(img, RAIL_X, LIKE_Y)
}

// ── Comment drawer ───────────────────────────────────────────────────────
//
// Geometry from the reference tool's tables, which were tuned on this same
// 375×667 screen, converted to fractions. The decisive signal is the Send
// button: it is TikTok red only once there is text to post, at a fixed spot.
// The lighter checks around it distinguish "drawer up" from "keyboard up".

/// Send button centre, once the keyboard is up (322, 425 logical).
pub const SEND_BUTTON: (f64, f64) = (322.0 / 375.0, 425.0 / 667.0);
/// Comment input field, live-verified at (120, 640) logical on iPhone 8.
pub const COMMENT_INPUT: (f64, f64) = (120.0 / 375.0, 640.0 / 667.0);
/// A point above the drawer; tapping it dismisses the drawer (180, 100).
pub const DRAWER_DISMISS: (f64, f64) = (180.0 / 375.0, 100.0 / 667.0);

/// The emoji/sticker icon used by the stock-WDA fallback. Stock synthetic taps
/// cannot focus the "Thêm bình luận…" pill, while RT-MMO uses `COMMENT_INPUT`
/// for text. Measured at (299, 639) logical.
pub const DRAWER_EMOJI_ICON: (f64, f64) = (299.0 / 375.0, 639.0 / 667.0);

/// Send arrow inside the composer (337, 307 logical). Light pink while the
/// field is empty, solid TikTok red once there is something to post.
pub const COMPOSER_SEND: (f64, f64) = (337.0 / 375.0, 307.0 / 667.0);

/// Redness at [`COMPOSER_SEND`] separating "armed" from "disabled".
/// Measured on this device: disabled 62.8, armed 156.2.
pub const SEND_ARMED_REDNESS: f64 = 100.0;

/// The ✕ that leaves a LIVE room, top-right.
pub const LIVE_EXIT: (f64, f64) = (0.945, 0.069);

/// Redness at the Send button above which it counts as armed.
const SEND_ARMED_MIN: f64 = 60.0;
/// Mean brightness of the drawer body that marks the sheet as present.
const DRAWER_LIGHT_MIN: f64 = 170.0;

/// How far the comment drawer has progressed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CommentDrawer {
    /// No drawer — the feed's compose bar is visible.
    Closed,
    /// Drawer is up but nothing is typed; the keyboard may or may not be up.
    Open,
    /// Send is armed: there is text ready to post.
    SendArmed,
    /// Cannot tell — do not act on this.
    Unknown,
}

/// Measurements behind a [`CommentDrawer`] reading.
#[derive(Debug, Clone, Copy, Default)]
pub struct DrawerEvidence {
    pub compose_bar: bool,
    pub send_redness: f64,
    pub body_light: f64,
}

impl DrawerEvidence {
    pub fn debug_line(&self) -> String {
        format!(
            "bar={} send_red={:.0} body_light={:.0}",
            self.compose_bar, self.send_redness, self.body_light
        )
    }
}

/// Classify the comment drawer from one frame.
///
/// The drawer covers TikTok's compose bar, so a visible compose bar is a
/// conclusive "closed" and costs three region means to establish.
pub fn comment_drawer_state(img: &RgbImage) -> (CommentDrawer, DrawerEvidence) {
    let mut ev = DrawerEvidence::default();
    let (bar, _, _, _) = compose_bar_visible(img);
    ev.compose_bar = bar;
    if bar {
        return (CommentDrawer::Closed, ev);
    }

    ev.send_redness = icon_redness(img, SEND_BUTTON.0, SEND_BUTTON.1);
    ev.body_light = region_brightness(img, 0.05, 0.62, 0.95, 0.88);

    if ev.send_redness >= SEND_ARMED_MIN {
        return (CommentDrawer::SendArmed, ev);
    }
    if ev.body_light >= DRAWER_LIGHT_MIN {
        return (CommentDrawer::Open, ev);
    }
    (CommentDrawer::Unknown, ev)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// iPhone 8 @2x, the geometry every measured constant here came from.
    const W: u32 = 750;
    const H: u32 = 1334;

    #[test]
    fn comment_input_uses_verified_iphone_8_field_center() {
        assert!((COMMENT_INPUT.0 * 375.0 - 120.0).abs() < f64::EPSILON);
        assert!((COMMENT_INPUT.1 * 667.0 - 640.0).abs() < f64::EPSILON);
    }

    /// Dark "video" pixels — textured, since a flat fill has no variance to
    /// correlate against and would pass any test for free.
    fn feed_backdrop() -> RgbImage {
        RgbImage::from_fn(W, H, |x, y| {
            let v = (20 + ((x / 7 + y / 11) % 24)) as u8;
            image::Rgb([v, v, v])
        })
    }

    /// Paint TikTok's compose pill: cyan edge, white core, pink edge.
    fn with_compose_bar(img: &mut RgbImage) {
        let band = (
            (PLUS_BAND.0 * H as f64) as u32,
            (PLUS_BAND.1 * H as f64) as u32,
        );
        let paint = |img: &mut RgbImage, xr: (f64, f64), c: [u8; 3]| {
            let x0 = (xr.0 * W as f64) as u32;
            let x1 = (xr.1 * W as f64) as u32;
            for y in band.0..band.1 {
                for x in x0..x1 {
                    img.put_pixel(x, y, image::Rgb(c));
                }
            }
        };
        paint(img, PLUS_CYAN_X, [44, 203, 231]);
        paint(img, PLUS_WHITE_X, [254, 254, 254]);
        paint(img, PLUS_PINK_X, [223, 59, 113]);
    }

    fn feed_scene() -> RgbImage {
        let mut img = feed_backdrop();
        with_compose_bar(&mut img);
        img
    }

    /// Paint a solid block down the rail column.
    fn paint_rail_block(img: &mut RgbImage, y0: u32, y1: u32, colour: [u8; 3]) {
        let x0 = ((RAIL_X - RAIL_HALF_WIDTH) * W as f64) as u32;
        let x1 = ((RAIL_X + RAIL_HALF_WIDTH) * W as f64) as u32;
        for y in y0..y1 {
            for x in x0..x1 {
                img.put_pixel(x, y, image::Rgb(colour));
            }
        }
    }

    /// A filled (liked) heart is red, so the white-glyph scan cannot see it and
    /// the chain starts at the comment bubble. `locate_action_rail` must probe
    /// for the red heart one pitch up and keep the like target on it — not on
    /// the comment bubble one pitch below, which was the silent misfire.
    #[test]
    fn locate_action_rail_keeps_the_like_target_on_a_filled_heart() {
        const WHITE: [u8; 3] = [254, 254, 254];
        const RED: [u8; 3] = [254, 44, 85];

        // Filled heart: red heart slot + white comment/save/share below it.
        let mut liked = feed_backdrop();
        paint_rail_block(&mut liked, 600, 670, RED); // heart ~624px, tall for the redness probe
        paint_rail_block(&mut liked, 757, 771, WHITE); // comment ~764px
        paint_rail_block(&mut liked, 879, 893, WHITE); // save ~886px
        paint_rail_block(&mut liked, 1015, 1029, WHITE); // share ~1022px
        let rail = locate_action_rail(&liked).expect("liked-card rail");
        let redness = like_redness_at(&liked, &rail);
        assert!(
            redness > LIKE_FILLED_REDNESS,
            "like target (y={:.3}) landed off the red heart: redness {redness:.1}",
            rail.like_y
        );
        assert!(
            rail.like_y < rail.comment_y,
            "the heart must sit above the comment bubble"
        );

        // Unfilled heart: a white heart is already the first run, so the label
        // stays put and the like target lands on it.
        let mut unliked = feed_backdrop();
        paint_rail_block(&mut unliked, 617, 631, WHITE); // white heart ~624px
        paint_rail_block(&mut unliked, 757, 771, WHITE); // comment ~764px
        paint_rail_block(&mut unliked, 879, 893, WHITE); // save ~886px
        let rail = locate_action_rail(&unliked).expect("unliked-card rail");
        assert!(
            (rail.like_y * H as f64 - 624.0).abs() < 20.0,
            "unfilled heart like target at {} px, expected ~624",
            rail.like_y * H as f64
        );
        assert!(
            (rail.comment_y * H as f64 - 764.0).abs() < 20.0,
            "comment target at {} px, expected ~764",
            rail.comment_y * H as f64
        );
    }

    #[test]
    fn both_measured_rail_layouts_derive_the_save_coordinate() {
        const RED: [u8; 3] = [254, 44, 85];
        for (follow_y, expected_layout, expected_save) in
            [(FOLLOW_Y_LAYOUT1, 1, 403.0), (FOLLOW_Y_LAYOUT2, 2, 439.0)]
        {
            let mut frame = feed_backdrop();
            let centre = follow_y * H as f64;
            paint_rail_block(
                &mut frame,
                (centre - 20.0) as u32,
                (centre + 20.0) as u32,
                RED,
            );
            let rail = find_action_rail(&frame).expect("the fresh frame locates its rail");
            assert!(rail.located);
            assert_eq!(rail.layout(), expected_layout);
            assert!((rail.save_y.expect("located Save") * 667.0 - expected_save).abs() <= 5.0);
        }

        let layout_2 = ActionRail::from_follow(FOLLOW_Y_LAYOUT2, true);
        assert!(
            (layout_2.save_y.expect("layout 2 Save") * 667.0 - 443.0).abs() < 12.0,
            "derived layout-2 target must land inside the captured Save glyph centred at 443pt"
        );
    }

    #[test]
    fn fallback_rail_never_authorizes_save() {
        let fallback = ActionRail::fallback();
        assert!(!fallback.located);
        assert_eq!(fallback.save_y, None);
    }

    /// Feed with a white sheet covering everything below `sheet_top`, the shape
    /// the Add-phone popup takes. The glyph template includes the sheet's white
    /// surround, so it only correlates when pasted onto white.
    fn sheet_scene(sheet_top: u32, glyph_at: (u32, u32)) -> RgbImage {
        let mut img = feed_backdrop();
        for y in sheet_top..H {
            for x in 0..W {
                img.put_pixel(x, y, image::Rgb([252, 252, 252]));
            }
        }
        let glyph = image::load_from_memory(CLOSE_X_TEMPLATE).unwrap().to_rgb8();
        image::imageops::overlay(&mut img, &glyph, glyph_at.0 as i64, glyph_at.1 as i64);
        img
    }

    #[test]
    fn finds_close_button_and_reports_its_centre() {
        // Sheet edge and glyph position measured off a real Add-phone capture.
        let screen = sheet_scene(700, (673, 735));
        let (fx, fy, score) = find_close_button(&screen, Some(375.0)).expect("close button");
        assert!(score >= CLOSE_X_THRESHOLD, "score {score}");
        assert!((fx - 693.0 / W as f64).abs() < 0.02, "fx {fx}");
        assert!((fy - 755.0 / H as f64).abs() < 0.02, "fy {fy}");
    }

    #[test]
    fn no_close_button_on_a_plain_feed() {
        let m = find_close_button(&feed_backdrop(), Some(375.0));
        assert!(
            m.is_none_or(|(_, _, s)| s < CLOSE_X_THRESHOLD),
            "feed matched the ✕: {m:?}"
        );
    }

    #[test]
    fn ignores_a_close_button_outside_the_search_region() {
        // Same glyph on the same white sheet, but in the top-left corner:
        // outside CLOSE_X_REGION, so it must not become a tap target.
        let screen = sheet_scene(0, (10, 10));
        let m = find_close_button(&screen, Some(375.0));
        assert!(m.is_none_or(|(_, _, s)| s < CLOSE_X_THRESHOLD), "{m:?}");
    }

    #[test]
    fn compose_bar_marks_the_feed() {
        let obs = classify(&feed_scene(), Some(375.0));
        assert_eq!(obs.kind, ScreenKind::Feed, "{}", obs.debug_line());
        assert!(is_actionable_feed(&obs));
    }

    #[test]
    fn recognises_the_transient_ad_feedback_notice_and_waits_for_it() {
        let mut screen = feed_scene();
        for y in 80..280 {
            for x in 0..W {
                screen.put_pixel(x, y, image::Rgb([180, 185, 195]));
            }
        }
        // A compact neutral toast with a few light text strokes, matching the
        // live "ít quảng cáo hơn" notice without depending on its wording.
        for y in 160..225 {
            for x in 75..675 {
                screen.put_pixel(x, y, image::Rgb([108, 114, 128]));
            }
        }
        for y in 184..192 {
            for (x0, x1) in [(120, 198), (214, 268), (288, 365), (388, 470)] {
                for x in x0..x1 {
                    screen.put_pixel(x, y, image::Rgb([238, 240, 244]));
                }
            }
        }

        assert!(ad_feedback_notice_present(&screen));
        let obs = classify(&screen, Some(375.0));
        assert_eq!(obs.kind, ScreenKind::Feed, "{}", obs.debug_line());
        assert!(obs.evidence.ad_feedback_notice);
        assert!(!is_actionable_feed(&obs));
    }

    fn promo_scene() -> RgbImage {
        let mut img = RgbImage::from_fn(W, H, |x, y| {
            let v = ((x / 11 + y / 17) % 8) as u8;
            image::Rgb([48 + v, 108 + v, 236 + v])
        });
        with_compose_bar(&mut img);

        // The observed floating card has a warm gift/promo surface below-left
        // of a dark blue close button.
        for y in 260..340 {
            for x in 58..122 {
                img.put_pixel(x, y, image::Rgb([220, 80, 62]));
            }
        }

        let (cx, cy, radius) = (136i32, 244i32, 16i32);
        for y in cy - radius..=cy + radius {
            for x in cx - radius..=cx + radius {
                let dx = x - cx;
                let dy = y - cy;
                if dx * dx + dy * dy <= radius * radius {
                    img.put_pixel(x as u32, y as u32, image::Rgb([30, 72, 158]));
                }
            }
        }
        for y in cy - 8..=cy + 8 {
            for x in cx - 8..=cx + 8 {
                let dx = (x - cx).abs();
                let dy = (y - cy).abs();
                if (dx - dy).abs() <= 1 {
                    img.put_pixel(x as u32, y as u32, image::Rgb([245, 245, 245]));
                }
            }
        }
        img
    }

    #[test]
    fn finds_upper_left_promo_close_before_feed_short_circuit() {
        let screen = promo_scene();
        let (x, y, score) = find_promo_close(&screen).expect("promo close");
        assert!(score >= 0.86, "score {score}");
        assert!((x - 136.0 / W as f64).abs() < 0.03, "x {x}");
        assert!((y - 244.0 / H as f64).abs() < 0.03, "y {y}");

        let obs = classify(&screen, Some(375.0));
        match obs.kind {
            ScreenKind::ClosableSheet { x, y, .. } => {
                assert!((x - 136.0 / W as f64).abs() < 0.03, "x {x}");
                assert!((y - 244.0 / H as f64).abs() < 0.03, "y {y}");
            }
            other => panic!("expected promo sheet, got {other:?} ({})", obs.debug_line()),
        }
    }

    #[test]
    fn no_upper_left_promo_close_on_plain_feed() {
        assert!(find_promo_close(&feed_scene()).is_none());
    }

    #[test]
    fn a_sheet_hides_the_compose_bar_and_is_classified_closable() {
        // The sheet covers the bottom bar, exactly as on device.
        let screen = sheet_scene(700, (673, 735));
        let obs = classify(&screen, Some(375.0));
        match obs.kind {
            ScreenKind::ClosableSheet { x, y, .. } => {
                assert!((x - 0.924).abs() < 0.02, "x {x}");
                assert!((y - 0.566).abs() < 0.02, "y {y}");
            }
            other => panic!(
                "expected a closable sheet, got {other:?} ({})",
                obs.debug_line()
            ),
        }
    }

    /// A near-white video frame is the classic false positive for a brightness
    /// rule. It must read as the feed while the bar shows, and must still not
    /// be called onboarding once the bar is gone.
    #[test]
    fn a_bright_white_video_is_not_an_onboarding_page() {
        let white_video = |with_bar: bool| {
            let mut img = RgbImage::from_fn(W, H, |x, y| {
                let v = (225 + ((x / 5 + y / 9) % 25)) as u8;
                image::Rgb([v, (v as f64 * 0.86) as u8, (v as f64 * 0.72) as u8])
            });
            if with_bar {
                with_compose_bar(&mut img);
            }
            img
        };
        let obs = classify(&white_video(true), Some(375.0));
        assert_eq!(obs.kind, ScreenKind::Feed, "{}", obs.debug_line());

        let obs = classify(&white_video(false), Some(375.0));
        assert_ne!(obs.kind, ScreenKind::InterestPicker, "{}", obs.debug_line());
    }

    #[test]
    fn recognises_the_interest_picker_layout() {
        // Light neutral page, no compose bar, pink "Tiếp theo" pill at the foot.
        let mut img = RgbImage::from_fn(W, H, |x, y| {
            let v = (243 + ((x / 11 + y / 13) % 10)) as u8;
            image::Rgb([v, v, v])
        });
        for y in (0.89 * H as f64) as u32..(0.94 * H as f64) as u32 {
            for x in (0.22 * W as f64) as u32..(0.78 * W as f64) as u32 {
                img.put_pixel(x, y, image::Rgb([254, 44, 85]));
            }
        }
        let obs = classify(&img, Some(375.0));
        assert_eq!(obs.kind, ScreenKind::InterestPicker, "{}", obs.debug_line());
    }

    #[test]
    fn home_screen_is_unknown_not_feed() {
        // Light dock, no compose pill, no ✕, colourful icons — must not be
        // mistaken for the feed, and must not be tapped as an overlay either.
        let img = RgbImage::from_fn(W, H, |x, y| {
            let r = (120 + ((x / 3) % 120)) as u8;
            let g = (90 + ((y / 4) % 140)) as u8;
            let b = (60 + ((x / 5 + y / 6) % 160)) as u8;
            image::Rgb([r, g, b])
        });
        let obs = classify(&img, Some(375.0));
        assert_eq!(obs.kind, ScreenKind::Unknown, "{}", obs.debug_line());
    }
    /// **No tap may be computed from a screen size the phone did not report.**
    ///
    /// Every geometry constant in this module is a *fraction*. The screen size is the
    /// multiplier that turns a fraction into a point on glass, so a wrong multiplier keeps
    /// the fraction perfectly valid and moves the tap somewhere else entirely. Nothing
    /// downstream can detect it: the tap succeeds, the gesture is recorded, and the action
    /// lands on whatever control happened to be there.
    ///
    /// Six call sites wrote `window_size().await.unwrap_or((375.0, 667.0))`. That fallback is
    /// this file's only calibrated layout, an iPhone 8; the Android fleet reports 1080x2220,
    /// measured 27/08/2026. So one failed `window_size()` moved every derived point to roughly
    /// 35% across and 30% down from where it belonged. Two of the six then tapped composer and
    /// Send, which is a comment published against an unknown control on a real account.
    ///
    /// `run_session` had already been fixed to refuse unknown geometry, and
    /// `docs/agents/10-thiet-bi-moi.md` recorded that refusal as covering the whole product.
    /// It did not — the Interaction entry points took their own geometry after that check.
    /// An independent review found them; this gate is what stops the seventh.
    ///
    /// A source scan rather than a type, for the same reason `wda.rs` scans for
    /// `Client::new()`: the danger is a constructor that exists and is easy to reach for, not
    /// a shape the compiler can rule out.
    #[test]
    fn no_tap_geometry_comes_from_a_fabricated_screen_size() {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if entry.file_name() != "target" {
                        walk(&path, out);
                    }
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    out.push(path);
                }
            }
        }

        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("the repo root resolves from the crate manifest");
        let mut files = Vec::new();
        walk(&repo.join("crates"), &mut files);
        walk(&repo.join("apps/desktop/src-tauri/src"), &mut files);

        let mut scanned = 0usize;
        let mut sizes_read = 0usize;
        let mut fabricated: Vec<String> = Vec::new();

        for path in &files {
            let rel = path
                .strip_prefix(&repo)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            let Ok(body) = std::fs::read_to_string(path) else {
                continue;
            };
            scanned += 1;
            for (idx, line) in body.lines().enumerate() {
                let code = line.trim_start();
                // Doc comments here quote the forbidden shape on purpose.
                if code.starts_with("//") {
                    continue;
                }
                if !line.contains("window_size()") {
                    continue;
                }
                sizes_read += 1;
                // The whole point is that a failure must not become a value. `?`, `map_err`,
                // an `else` arm and a `match` all keep the failure; `unwrap_or*` and
                // `unwrap_or_default` do not. And rustfmt is allowed to wrap the chain — a
                // review pointed out that `window_size().await` on one line with
                // `.unwrap_or(...)` on the next passed the old same-line check — so the scan
                // follows the chain across continuation lines (the ones rustfmt starts with
                // a `.`), which is exactly where a wrapped `unwrap_or` can live and nowhere
                // an unrelated expression can.
                let mut chain_fabricates = line.contains("unwrap_or");
                for follow in body.lines().skip(idx + 1) {
                    let follow = follow.trim_start();
                    if !follow.starts_with('.') {
                        break;
                    }
                    if follow.contains("unwrap_or") {
                        chain_fabricates = true;
                    }
                }
                if chain_fabricates {
                    fabricated.push(format!("{rel}:{}", idx + 1));
                }
            }
        }

        // A scanner that reads nothing passes every assertion below it.
        assert!(
            scanned >= 40,
            "only {scanned} source files scanned; the walk is broken"
        );
        assert!(
            sizes_read >= 8,
            "only {sizes_read} `window_size()` sites found; the scan is broken"
        );
        assert!(
            fabricated.is_empty(),
            "these substitute a screen size the phone never reported, so every tap derived \
             from them lands somewhere nobody chose: {fabricated:?}\n\
             \n\
             Use `screen::measured_screen_size`, which refuses and says why. A refused attempt \
             costs one retry; a fabricated tap costs an action on a real account."
        );
    }
}
