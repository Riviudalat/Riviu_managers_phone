use crate::types::{SwipePath, SwipeStep, TapPoint};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

/// How many moves a planned swipe is cut into.
///
/// Enough that the velocity profile is visible in the gesture rather than implied, few
/// enough that the whole path still travels in one `/actions` round trip. Twelve puts a
/// sample roughly every 25 ms of a 300 ms flick, which is finer than the ~16 ms frame the
/// app can observe.
const SWIPE_STEPS: usize = 12;

/// How far the path may bow away from the straight line, as a fraction of its length.
///
/// A finger pivots from a knuckle, so a "vertical" flick is a shallow arc, not a line. Kept
/// small: a few percent reads as a hand, while a large bow starts crossing UI it was not
/// aimed at.
const SWIPE_BOW: (f64, f64) = (0.012, 0.045);

/// How far the endpoints may wander from what the caller asked for, in pixels.
///
/// The caller's points are a *target*, not a coordinate. Without this every feed swipe on
/// every device starts and ends on exactly the same two pixels forever, which is the single
/// most mechanical property the old gesture had.
const SWIPE_ENDPOINT_JITTER: f64 = 18.0;

/// How long the finger keeps contact after the last move.
const SWIPE_SETTLE_MS: (u64, u64) = (12, 45);

/// What a planned gesture is meant to *be*, which decides three things about its shape.
///
/// Not a style knob: [`TouchPointPlanner::plan_flick`] carries the measurement that says a
/// pager ignores a drag, and a caller picking the wrong one here gets a gesture the app
/// quietly declines to act on rather than an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Curve {
    /// A finger dragging something: bowed, decelerating, resting a moment before it lifts.
    Drag,
    /// A finger throwing something: straight, still speeding up, gone the instant it stops.
    Flick,
}

// Native XCTest ultimately synthesizes integer-ish logical points. Keeping the
// planner on that grid prevents two distinct floats from landing on one device
// coordinate after transport rounding.
const GRID_SCALE: f64 = 1.0;

/// How tight the tap cluster is, as a fraction of the control's radius.
///
/// A third, so about 99,7 % of draws land inside the control before clamping and the bulk
/// sit near the middle. Uniform-over-the-rectangle put as many taps in the extreme corners
/// of a hit area as in the centre, which no thumb does.
const TAP_SPREAD: f64 = 1.0 / 3.0;

/// How far this device's hand sits off the geometric centre, in screen pixels.
///
/// Drawn **once per device** and then constant, because that is what makes it a hand rather
/// than noise: the same grip produces the same lean on every control, all session. A number
/// re-drawn per tap would just be more randomness.
///
/// Pixels rather than a fraction of the target, deliberately — a thumb's offset does not
/// know how big the button is.
const HAND_BIAS: f64 = 7.0;

/// Where a tap lands, for one device.
///
/// **The distribution is the point, and it used to be the wrong one.** This drew uniformly
/// across the control's whole rectangle, refused to return a coordinate it had already used,
/// and required every tap to be at least 3 px from the last 96 — three rules whose combined
/// effect is a spread *more even than chance*. Sampling without replacement over a small
/// rectangle is a recognisable pattern in its own right, and a hit area worked through
/// coordinate by coordinate is not what a finger produces.
///
/// A finger produces a cluster: roughly normal, centred a few pixels off the visual middle
/// in whichever direction the hand leans, with repeats. So that is what this draws — a
/// truncated bivariate normal around centre plus [`Self::bias`], and no anti-repeat rule at
/// all, because tapping the same pixel twice in fifty taps is what actually happens.
///
/// The one rule kept from the old version is the load-bearing one: **the tap never leaves
/// the control's rectangle.** An earlier version widened the rectangle up to 5x when it ran
/// out of fresh coordinates, which for the like heart's (10, 12) radius is ±60 logical
/// points — a full rail icon pitch, so it tapped comment and opened the drawer.
#[derive(Debug)]
pub struct TouchPointPlanner {
    width: f64,
    height: f64,
    rng: StdRng,
    /// This device's hand, in pixels. Constant for the planner's life.
    bias: (f64, f64),
}

impl TouchPointPlanner {
    pub fn new(screen_size: (f64, f64)) -> Self {
        let mut rng = StdRng::from_entropy();
        let bias = (
            rng.gen_range(-HAND_BIAS..=HAND_BIAS),
            rng.gen_range(-HAND_BIAS..=HAND_BIAS),
        );
        Self {
            width: screen_size.0.max(1.0),
            height: screen_size.1.max(1.0),
            rng,
            bias,
        }
    }

    /// A planner whose hand is given rather than drawn.
    ///
    /// Test-only. A test that measures the *shape* of the distribution has to hold the lean
    /// still, or it is measuring the lean instead — which is what made
    /// `taps_cluster_near_the_middle_instead_of_filling_the_rectangle` fail about one run in
    /// six: a large lean pins the draws against one edge of a small control and the sample
    /// stops looking like the normal it is.
    #[cfg(test)]
    pub fn with_bias(screen_size: (f64, f64), bias: (f64, f64)) -> Self {
        let mut planner = Self::new(screen_size);
        planner.bias = bias;
        planner
    }

    pub(crate) fn next(&mut self, center: TapPoint, radius: (f64, f64)) -> TapPoint {
        let rx = radius.0.abs().max(1.0);
        let ry = radius.1.abs().max(1.0);
        let (min_x, max_x, min_y, max_y) = self.bounds(&center, rx, ry);
        // The hand leans, then the finger scatters around where it aimed. Clamped into the
        // rectangle rather than re-drawn, so a control small enough that the bias would miss
        // it still gets tapped — on the edge nearest the lean, which is where a thumb that
        // leans would land anyway.
        let x = center.x + self.bias.0 + self.normal() * rx * TAP_SPREAD;
        let y = center.y + self.bias.1 + self.normal() * ry * TAP_SPREAD;
        // Quantize **then** clamp, on rounded bounds. The other order is a bug and it is
        // easy to miss: clamping to a `.5` edge and rounding afterwards pushes the point
        // back outside the rectangle. With the old uniform draw that was measure-zero, but a
        // clamped normal saturates on the edge constantly, so it fired at once.
        Self::quantize_into(x, y, (min_x, max_x, min_y, max_y))
    }

    /// Round onto the transport's grid, guaranteed to stay within `bounds`.
    ///
    /// The rounded window can be a fraction of a pixel wider than the real rectangle when a
    /// control sits between two integers. That is below what the transport can express — it
    /// casts to whole pixels either way — so the alternative would be a coordinate the device
    /// rounds outside the bound regardless.
    fn quantize_into(x: f64, y: f64, bounds: (f64, f64, f64, f64)) -> TapPoint {
        let (min_x, max_x, min_y, max_y) = bounds;
        let low_x = (min_x * GRID_SCALE).ceil();
        let high_x = (max_x * GRID_SCALE).floor().max(low_x);
        let low_y = (min_y * GRID_SCALE).ceil();
        let high_y = (max_y * GRID_SCALE).floor().max(low_y);
        TapPoint {
            x: (x * GRID_SCALE).round().clamp(low_x, high_x) / GRID_SCALE,
            y: (y * GRID_SCALE).round().clamp(low_y, high_y) / GRID_SCALE,
        }
    }

    /// One draw from a standard normal, by Box–Muller.
    ///
    /// Written out rather than pulling in `rand_distr` for six lines. The uniform is opened
    /// away from zero because `ln(0)` is not finite.
    fn normal(&mut self) -> f64 {
        let u1: f64 = self.rng.gen_range(f64::EPSILON..1.0);
        let u2: f64 = self.rng.gen_range(0.0..1.0);
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }

    /// Turn two target points into a path a finger could have drawn.
    ///
    /// Three things the old single-segment swipe could not express, each of them measurable
    /// by anything watching the touch stream:
    ///
    /// * **The endpoints move.** They were fixed fractions of the screen, so every feed
    ///   swipe in every session started and ended on the same two pixels.
    /// * **The line bows.** A finger pivots from a knuckle; a "vertical" flick is a shallow
    ///   arc. The bow direction is chosen per gesture, so consecutive swipes are not even
    ///   curved the same way.
    /// * **The velocity changes.** One `pointerMove` is constant speed from first pixel to
    ///   last. A flick accelerates, runs, and eases — here the duration is spread across
    ///   the steps by that profile rather than divided evenly.
    ///
    /// `total_ms` stays the caller's number: this changes the shape of the gesture, not how
    /// long it takes, so nothing that tuned a duration has to be re-tuned. The endpoints are
    /// clamped inside the screen, so jitter can never push a gesture off the display.
    pub fn plan_swipe(&mut self, from: TapPoint, to: TapPoint, total_ms: u64) -> SwipePath {
        self.plan(from, to, total_ms, Curve::Drag)
    }

    /// [`Self::plan_swipe`]'s gesture, shaped so the app reads it as a **fling**.
    ///
    /// Three of `plan_swipe`'s properties are right for a drag and were each measured to stop
    /// a pager turning. A flick drops all three and keeps everything else:
    ///
    /// * The **pause before the lift**. [`SwipePath::settle_ms`] exists because a real flick
    ///   keeps contact for a few milliseconds after the motion stops. Android reads the
    ///   release velocity out of the recent motion history, so a pause with no movement in it
    ///   describes a finger that had already stopped.
    /// * The **bow**, which is perpendicular to travel — so on a *horizontal* gesture it is
    ///   vertical, into the axis the feed's own pager is watching.
    /// * The **deceleration**. [`Self::ease`] is a smoothstep, whose slope at the end is
    ///   zero: the last leg of a 600 px path crawls about ten pixels. That is what a finger
    ///   coming to rest looks like, and it is the opposite of a flick, where the hand is
    ///   still speeding up as it leaves the glass.
    ///
    /// **Measured on the feed, TikTok 38.3.2, one component at a time on the same card**, on
    /// ce021822e3f548f40b / ce03171392f9390c01 / ce031713dd735a1103 / ce0417145199e0490c /
    /// ce0517151215a00304, 18/08/2026. Turns that advanced the page counter, counting only
    /// turns whose previous reading was known and whose post had images left:
    ///
    /// | gesture | turns that paged |
    /// |---|---|
    /// | the full planned path | 13 of 40 |
    /// | the bow removed | 6 of 15 |
    /// | the pause removed | 7 of 12 |
    /// | bow and pause removed, still decelerating | 18 of 27 |
    /// | **all three removed — this** | **19 of 19** |
    /// | a plain straight swipe, for reference | 31 of 32 |
    ///
    /// Removing any one of them leaves a gesture that works between a third and two thirds of
    /// the time, which the loop reads as "no further image" — and that is exactly how a photo
    /// post used to end a session at two images. All three had to go, and the order they were
    /// found in is the order they are listed: each looked like the answer on its own.
    ///
    /// What survives is what the pager does not object to: the endpoints still wander
    /// ([`SWIPE_ENDPOINT_JITTER`]), the path is still cut into [`SWIPE_STEPS`] legs rather
    /// than one, and the leg spacing still varies. So this is not the fixed-pixel straight
    /// line the planner exists to replace.
    pub fn plan_flick(&mut self, from: TapPoint, to: TapPoint, total_ms: u64) -> SwipePath {
        self.plan(from, to, total_ms, Curve::Flick)
    }

    /// The one path builder. [`Curve`] is the only thing the two gestures disagree about.
    fn plan(&mut self, from: TapPoint, to: TapPoint, total_ms: u64, curve: Curve) -> SwipePath {
        let start = self.jitter_endpoint(&from);
        let end = self.jitter_endpoint(&to);

        let dx = end.x - start.x;
        let dy = end.y - start.y;
        let length = (dx * dx + dy * dy).sqrt();
        // Perpendicular to the direction of travel, so the bow is across the swipe rather
        // than along it. Sign per gesture: a hand does not curve the same way twice.
        let bow = match curve {
            Curve::Drag => {
                let bow = length * self.rng.gen_range(SWIPE_BOW.0..=SWIPE_BOW.1);
                if self.rng.gen_bool(0.5) {
                    bow
                } else {
                    -bow
                }
            }
            Curve::Flick => 0.0,
        };
        let (nx, ny) = if length > f64::EPSILON {
            (-dy / length, dx / length)
        } else {
            (0.0, 0.0)
        };
        // Quadratic Bézier control point: the midpoint pushed sideways by the bow. Placed
        // slightly off-centre so the arc is not symmetric, which a wrist is not either. With
        // no bow the control sits on the line, so the curve degenerates to it — and the lean
        // still varies where along it the legs fall.
        let lean = self.rng.gen_range(0.40..=0.60);
        let control = TapPoint {
            x: start.x + dx * lean + nx * bow,
            y: start.y + dy * lean + ny * bow,
        };

        let total_ms = total_ms.max(SWIPE_STEPS as u64);
        let mut steps = Vec::with_capacity(SWIPE_STEPS);
        let mut spent = 0u64;
        let mut previous_t = 0.0;
        for step in 1..=SWIPE_STEPS {
            let raw = step as f64 / SWIPE_STEPS as f64;
            let t = match curve {
                Curve::Drag => Self::ease(raw),
                Curve::Flick => Self::ease_in(raw),
            };
            let point = Self::bezier(&start, &control, &end, t);
            // The time for this leg is the *eased* time it represents, so a leg that covers
            // more distance gets proportionally less of the clock — which is what "moving
            // faster through the middle" means on the wire.
            let share = (t - previous_t).max(0.0);
            previous_t = t;
            let mut duration_ms = (total_ms as f64 * share).round() as u64;
            if step == SWIPE_STEPS {
                // Give the remainder to the last leg so the total is exactly the caller's.
                duration_ms = total_ms.saturating_sub(spent);
            }
            let duration_ms = duration_ms.max(1);
            spent = spent.saturating_add(duration_ms);
            steps.push(SwipeStep {
                point: self.clamp(point),
                duration_ms,
            });
        }

        SwipePath {
            start: self.clamp(start),
            steps,
            settle_ms: match curve {
                Curve::Drag => self.rng.gen_range(SWIPE_SETTLE_MS.0..=SWIPE_SETTLE_MS.1),
                Curve::Flick => 0,
            },
        }
    }

    /// A drag's profile: the finger builds speed, runs, and slows before it lifts.
    ///
    /// `raw` is the fraction of the way through the steps; the return is the fraction of the
    /// way along the path. The two differ, and that difference *is* the velocity profile.
    fn ease(raw: f64) -> f64 {
        // Smoothstep, whose slope at both ends is zero. Right for a drag, and the reason a
        // drag cannot double as a flick: see [`Self::ease_in`].
        raw * raw * (3.0 - 2.0 * raw)
    }

    /// A flick's profile: still accelerating when the finger leaves the glass.
    ///
    /// [`Self::ease`]'s slope at the end is zero, which on a 600 px path leaves the last leg
    /// covering about ten pixels — a finger coming to rest. Android reads the release
    /// velocity out of the recent motion history, so a path that ends that way is a drag
    /// however fast its middle was. Quadratic rather than anything steeper: the gesture still
    /// has to start from a standstill.
    fn ease_in(raw: f64) -> f64 {
        raw * raw
    }

    fn bezier(start: &TapPoint, control: &TapPoint, end: &TapPoint, t: f64) -> TapPoint {
        let inv = 1.0 - t;
        TapPoint {
            x: inv * inv * start.x + 2.0 * inv * t * control.x + t * t * end.x,
            y: inv * inv * start.y + 2.0 * inv * t * control.y + t * t * end.y,
        }
    }

    /// Move a swipe endpoint the way the same hand would.
    ///
    /// Normal rather than uniform, and it carries [`Self::bias`] — the same lean the taps
    /// get, because it is the same hand. Uniform jitter spread the start point evenly across
    /// a 36 px box, which reads as a box rather than as a person reaching for the same
    /// comfortable spot each time.
    fn jitter_endpoint(&mut self, point: &TapPoint) -> TapPoint {
        let spread = SWIPE_ENDPOINT_JITTER / 2.0;
        let x = point.x + self.bias.0 + self.normal() * spread;
        let y = point.y + self.bias.1 + self.normal() * spread;
        self.clamp(TapPoint { x, y })
    }

    fn clamp(&self, point: TapPoint) -> TapPoint {
        TapPoint {
            x: point.x.clamp(1.0, self.width - 1.0),
            y: point.y.clamp(1.0, self.height - 1.0),
        }
    }

    fn bounds(&self, center: &TapPoint, rx: f64, ry: f64) -> (f64, f64, f64, f64) {
        let min_x = (center.x - rx).clamp(0.5, self.width - 0.5);
        let max_x = (center.x + rx).clamp(min_x, self.width - 0.5);
        let min_y = (center.y - ry).clamp(0.5, self.height - 0.5);
        let max_y = (center.y + ry).clamp(min_y, self.height - 0.5);
        (min_x, max_x, min_y, max_y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SwipePath;
    use std::collections::HashSet;

    /// Mean and standard deviation of a sample, for asserting a *distribution* rather
    /// than an individual draw.
    fn stats(values: &[f64]) -> (f64, f64) {
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
        (mean, variance.sqrt())
    }

    #[test]
    fn taps_cluster_near_the_middle_instead_of_filling_the_rectangle() {
        // The complaint this answers: the old sampling was uniform across the whole hit
        // area, so a tap was as likely in an extreme corner as in the centre. A finger is
        // not uniform — it clusters.
        //
        // The like heart's measured radius. With a uniform draw, half the taps would land
        // outside the middle half of the box; with a cluster, far fewer do.
        let centre = TapPoint { x: 344.0, y: 307.0 };
        // The lean is held at zero so this measures the *shape*. `each_device_taps_with_its_
        // own_lean` covers the lean being drawn, and `a_leaning_hand_still_lands_on_the_
        // control` covers what a large one does.
        let mut planner = TouchPointPlanner::with_bias((375.0, 667.0), (0.0, 0.0));
        let mut xs = Vec::new();
        for _ in 0..600 {
            let point = planner.next(centre.clone(), (10.0, 12.0));
            // The load-bearing rule, unchanged: never leave the control.
            assert!(
                (334.0..=354.0).contains(&point.x) && (295.0..=319.0).contains(&point.y),
                "tap left the heart: {point:?}"
            );
            xs.push(point.x);
        }
        // Measured against the sample's **own** centre, not the geometric one — the hand
        // leans on purpose, so "clustered around the middle of the button" is the wrong
        // property and asserting it made this test fail whenever the lean came out large.
        // What is being claimed is that the draws are clustered *at all*.
        let (mean, sd) = stats(&xs);
        // Uniform across a 20 px-wide box has sd = 20/√12 ≈ 5,8. A cluster is far tighter,
        // and clamping against an edge only tightens it further.
        assert!(sd < 4.5, "the spread is still uniform-wide: sd {sd:.2}");
        let within = xs.iter().filter(|x| (*x - mean).abs() <= 5.0).count();
        // Uniform would put about half the draws inside ±5 px of the mean; a normal with
        // this spread puts the large majority there.
        assert!(
            within > 420,
            "only {within}/600 sit near the sample centre — that is a uniform spread"
        );
    }

    #[test]
    fn a_leaning_hand_still_lands_on_the_control() {
        // The safety rule under the worst lean the model can draw. A tap that leaves the
        // rectangle is the expensive failure — the like heart and the comment button are one
        // rail pitch apart, so a miss opens the drawer instead of liking.
        let centre = TapPoint { x: 344.0, y: 307.0 };
        for bias in [(7.0, 7.0), (-7.0, -7.0), (7.0, -7.0), (-7.0, 7.0)] {
            let mut planner = TouchPointPlanner::with_bias((375.0, 667.0), bias);
            for _ in 0..300 {
                let point = planner.next(centre.clone(), (10.0, 12.0));
                assert!(
                    (334.0..=354.0).contains(&point.x) && (295.0..=319.0).contains(&point.y),
                    "lean {bias:?} put the tap outside the heart: {point:?}"
                );
            }
        }
    }

    #[test]
    fn a_tap_may_land_where_a_previous_tap_landed() {
        // Deliberately the *opposite* of the old guarantee. Refusing to reuse a coordinate,
        // and forcing every tap 3 px clear of the last 96, produced a spread more even than
        // chance — sampling without replacement is itself a pattern. A person tapping one
        // button fifty times hits the same pixel more than once.
        let mut planner = TouchPointPlanner::new((375.0, 667.0));
        let centre = TapPoint { x: 344.0, y: 307.0 };
        let mut seen = HashSet::new();
        let mut repeats = 0;
        for _ in 0..200 {
            let point = planner.next(centre.clone(), (10.0, 12.0));
            let key = (point.x as i32, point.y as i32);
            if !seen.insert(key) {
                repeats += 1;
            }
        }
        assert!(
            repeats > 0,
            "200 taps on one control with no repeat at all is the old anti-human rule"
        );
    }

    #[test]
    fn each_device_taps_with_its_own_lean() {
        // What makes the offset a hand rather than more noise: it is drawn once per device
        // and then constant, so every control on that phone is approached the same way, and
        // two phones do not share the bias.
        let centre = TapPoint {
            x: 540.0,
            y: 1100.0,
        };
        // 4000 draws, not 400, and the reason is arithmetic rather than caution: on a
        // 60 px radius the spread is 20 px, so a 400-draw mean has a standard error of 1 px
        // and a ±2 px tolerance is only two standard errors — which fails about one run in
        // ten by construction. At 4000 the standard error is 0,32 px and the same tolerance
        // is five, so the test measures the lean instead of measuring luck.
        let mean_offset = |planner: &mut TouchPointPlanner| {
            let points: Vec<f64> = (0..4_000)
                .map(|_| planner.next(centre.clone(), (60.0, 60.0)).x)
                .collect();
            stats(&points).0 - centre.x
        };
        // A lean that was given is the lean that shows up, and it does not move between
        // batches — that is what "the same grip all session" means.
        let mut known = TouchPointPlanner::with_bias((1080.0, 2220.0), (5.0, -4.0));
        let first = mean_offset(&mut known);
        let again = mean_offset(&mut known);
        assert!(
            (first - 5.0).abs() < 1.6 && (again - 5.0).abs() < 1.6,
            "the given lean did not show up: {first:.2} then {again:.2}"
        );

        // And each device draws its own. Asserted across a dozen planners rather than by
        // comparing two, because two draws from the same range coincide often enough to
        // matter: an earlier version of this compared exactly two and failed about one run
        // in eight for no reason but luck.
        let leans: Vec<f64> = (0..12)
            .map(|_| mean_offset(&mut TouchPointPlanner::new((1080.0, 2220.0))))
            .collect();
        let (_, spread) = stats(&leans);
        assert!(
            spread > 1.5,
            "a dozen devices should not share one hand, spread {spread:.2}: {leans:?}"
        );
    }

    #[test]
    fn dwells_and_swipe_lengths_have_no_holes_and_may_repeat() {
        // Both of these carried the same two faults as the old tap sampling, and both were
        // visible in a real log rather than only in theory.
        use crate::human_behavior::HumanBehavior;

        let mut human = HumanBehavior::new("casual", false, false, false);

        // 1. Consecutive dwells were forced apart by 15 % of the window. A 3–5 s window
        //    produced 2,5 · 3,6 · 2,8 · 3,2 · … — alternating, never close twice.
        let dwells: Vec<f64> = (0..400).map(|_| human.watch_seconds(3.0, 5.0)).collect();
        let close = dwells
            .windows(2)
            .filter(|pair| (pair[0] - pair[1]).abs() < 0.3)
            .count();
        assert!(
            close > 20,
            "only {close} of 399 consecutive dwells were within 0,3 s of each other — that \
             is still the forced-apart rule"
        );
        for dwell in &dwells {
            assert!(
                (3.0..=5.0).contains(dwell),
                "dwell left the configured window: {dwell}"
            );
        }
        // Skewed short: the median sits below the middle of the window, because most posts
        // get a glance rather than a full watch.
        let mut sorted = dwells.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
        let median = sorted[sorted.len() / 2];
        assert!(
            median < 4.0,
            "the dwell distribution is not skewed short: {median:.2}"
        );

        // 2. Swipe durations came from three disjoint ranges, so nothing ever landed in
        //    281–299 ms. A gap in the histogram is its own signal.
        let lengths: Vec<u64> = (0..4_000).map(|_| human.swipe_duration_ms(false)).collect();
        let in_gap = lengths
            .iter()
            .filter(|ms| (281..=299u64).contains(ms))
            .count();
        assert!(in_gap > 0, "the 281–299 ms hole is still there");
        assert!(
            lengths.iter().all(|ms| (190..=820).contains(ms)),
            "a swipe length left the intended range"
        );
    }

    /// The straight line between the path's ends, at the same fraction along.
    fn on_chord(path: &SwipePath, t: f64) -> TapPoint {
        let end = path.end();
        TapPoint {
            x: path.start.x + (end.x - path.start.x) * t,
            y: path.start.y + (end.y - path.start.y) * t,
        }
    }

    #[test]
    fn a_planned_swipe_is_curved_rather_than_a_straight_line() {
        // The tell this replaces: one `pointerMove` is a dead-straight line, and every feed
        // swipe was the same one. A finger pivots from a knuckle, so the path bows.
        let mut planner = TouchPointPlanner::new((1080.0, 2220.0));
        let mut bowed = 0;
        for _ in 0..40 {
            let path = planner.plan_swipe(
                TapPoint {
                    x: 540.0,
                    y: 1598.0,
                },
                TapPoint { x: 540.0, y: 621.0 },
                280,
            );
            let middle = &path.steps[path.steps.len() / 2 - 1].point;
            let chord = on_chord(&path, 0.5);
            let off = ((middle.x - chord.x).powi(2) + (middle.y - chord.y).powi(2)).sqrt();
            if off > 2.0 {
                bowed += 1;
            }
        }
        assert!(
            bowed >= 36,
            "almost every path should leave the chord, only {bowed}/40 did"
        );
    }

    #[test]
    fn a_planned_swipe_does_not_travel_at_a_constant_speed() {
        // A single `pointerMove` covers its whole distance at one velocity. Here the
        // duration is spread by an easing curve, so the legs differ — that difference *is*
        // the acceleration the touch stream shows.
        let mut planner = TouchPointPlanner::new((1080.0, 2220.0));
        let path = planner.plan_swipe(
            TapPoint {
                x: 540.0,
                y: 1598.0,
            },
            TapPoint { x: 540.0, y: 621.0 },
            300,
        );
        let durations: Vec<u64> = path.steps.iter().map(|step| step.duration_ms).collect();
        let shortest = *durations.iter().min().expect("steps");
        let longest = *durations.iter().max().expect("steps");
        assert!(
            longest >= shortest * 2,
            "the velocity profile should be visible, got {durations:?}"
        );
        // The caller's duration is not silently changed: something that tuned a flick
        // length keeps the length it tuned.
        assert_eq!(path.travel_ms(), 300, "{durations:?}");
        // And the finger stays down a moment after the motion, which a fling needs.
        assert!(path.settle_ms >= 12 && path.settle_ms <= 45);
    }

    #[test]
    fn two_swipes_asked_for_identically_are_not_the_same_gesture() {
        // The property that matters most: repetition. Same request, different gesture.
        let mut planner = TouchPointPlanner::new((1080.0, 2220.0));
        let ask = || {
            (
                TapPoint {
                    x: 540.0,
                    y: 1598.0,
                },
                TapPoint { x: 540.0, y: 621.0 },
            )
        };
        let (a_from, a_to) = ask();
        let (b_from, b_to) = ask();
        let a = planner.plan_swipe(a_from, a_to, 280);
        let b = planner.plan_swipe(b_from, b_to, 280);
        assert!(
            (a.start.x - b.start.x).abs() > f64::EPSILON
                || (a.start.y - b.start.y).abs() > f64::EPSILON,
            "the endpoints should move between gestures"
        );
        let differs = a
            .steps
            .iter()
            .zip(&b.steps)
            .any(|(x, y)| x.point.x != y.point.x || x.point.y != y.point.y);
        assert!(differs, "the paths should not be identical");
    }

    #[test]
    fn a_planned_swipe_never_leaves_the_screen() {
        // Jitter and bow are applied to points that may already sit at an edge — the
        // carousel's page turn starts at 78% of the width and the feed flick ends near the
        // top. A gesture off the display does nothing at all.
        let mut planner = TouchPointPlanner::new((1080.0, 2220.0));
        for _ in 0..60 {
            for (from, to) in [
                (
                    TapPoint {
                        x: 1079.0,
                        y: 2219.0,
                    },
                    TapPoint { x: 1.0, y: 1.0 },
                ),
                (
                    TapPoint { x: 0.0, y: 0.0 },
                    TapPoint {
                        x: 1080.0,
                        y: 2220.0,
                    },
                ),
                (
                    TapPoint { x: 842.0, y: 888.0 },
                    TapPoint { x: 237.0, y: 888.0 },
                ),
            ] {
                let path = planner.plan_swipe(from, to, 320);
                for point in std::iter::once(&path.start).chain(path.steps.iter().map(|s| &s.point))
                {
                    assert!(
                        (1.0..=1079.0).contains(&point.x) && (1.0..=2219.0).contains(&point.y),
                        "left the screen: {point:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn planner_clamps_points_inside_screen_bounds() {
        let mut planner = TouchPointPlanner::new((375.0, 667.0));
        for _ in 0..32 {
            let point = planner.next(TapPoint { x: 0.0, y: 667.0 }, (30.0, 30.0));
            assert!((0.5..=374.5).contains(&point.x));
            assert!((0.5..=666.5).contains(&point.y));
        }
    }

    #[test]
    fn a_flick_leaves_the_pager_nothing_to_snap_back_to() {
        // Both properties go, and they go together: removing either one on its own was
        // measured to leave a page turn that lands about a third of the time, which the
        // nurture loop reads as "no further image". See `plan_flick` for the numbers.
        let mut planner = TouchPointPlanner::new((1080.0, 2220.0));
        let from = TapPoint { x: 842.0, y: 888.0 };
        let to = TapPoint { x: 237.0, y: 888.0 };
        for _ in 0..40 {
            let flick = planner.plan_flick(from.clone(), to.clone(), 320);
            assert_eq!(
                flick.settle_ms, 0,
                "a pause with no movement in it describes a finger that had already stopped"
            );
            let end = flick.end();
            let (dx, dy) = (end.x - flick.start.x, end.y - flick.start.y);
            let span = dx * dx + dy * dy;
            for step in &flick.steps {
                let along = (((step.point.x - flick.start.x) * dx
                    + (step.point.y - flick.start.y) * dy)
                    / span)
                    .clamp(0.0, 1.0);
                let (lx, ly) = (flick.start.x + dx * along, flick.start.y + dy * along);
                let off = ((step.point.x - lx).powi(2) + (step.point.y - ly).powi(2)).sqrt();
                assert!(
                    off < 0.5,
                    "a horizontal flick must not bow into the vertical axis the feed's own \
                     pager is watching, but this step sits {off:.1}px off the line"
                );
            }
        }
    }

    #[test]
    fn a_flick_keeps_everything_the_pager_does_not_object_to() {
        // `plan_swipe` minus two measured obstacles, not a return to the fixed-pixel
        // straight line the planner exists to replace.
        let mut planner = TouchPointPlanner::new((1080.0, 2220.0));
        let from = TapPoint { x: 842.0, y: 888.0 };
        let to = TapPoint { x: 237.0, y: 888.0 };
        let first = planner.plan_flick(from.clone(), to.clone(), 320);
        let second = planner.plan_flick(from, to, 320);
        assert!(
            (first.start.x - second.start.x).abs() > f64::EPSILON
                || (first.start.y - second.start.y).abs() > f64::EPSILON,
            "two flicks aimed at the same target should not start on the same pixel"
        );
        let durations: Vec<u64> = first.steps.iter().map(|step| step.duration_ms).collect();
        let shortest = *durations.iter().min().expect("steps");
        let longest = *durations.iter().max().expect("steps");
        assert!(
            longest >= shortest * 2,
            "the velocity profile survives being straightened: {durations:?}"
        );
        assert_eq!(
            durations.iter().sum::<u64>(),
            320,
            "the caller's duration is still the caller's"
        );
    }

    #[test]
    fn a_flick_is_still_speeding_up_when_the_finger_leaves() {
        // The property the pager actually reads. `plan_swipe`'s smoothstep has zero slope at
        // the end, so its last leg crawls; a flick's last leg must be its longest, or the
        // release velocity describes a finger that had already stopped.
        let mut planner = TouchPointPlanner::new((1080.0, 2220.0));
        let from = TapPoint { x: 842.0, y: 888.0 };
        let to = TapPoint { x: 237.0, y: 888.0 };
        for _ in 0..40 {
            let flick = planner.plan_flick(from.clone(), to.clone(), 320);
            let hops: Vec<f64> = std::iter::once(&flick.start)
                .chain(flick.steps.iter().map(|step| &step.point))
                .collect::<Vec<_>>()
                .windows(2)
                .map(|pair| {
                    ((pair[1].x - pair[0].x).powi(2) + (pair[1].y - pair[0].y).powi(2)).sqrt()
                })
                .collect();
            let last = *hops.last().expect("legs");
            let longest = hops.iter().cloned().fold(0.0_f64, f64::max);
            assert!(
                (last - longest).abs() < f64::EPSILON,
                "the last leg should be the longest, got {last:.1}px against {longest:.1}px"
            );
        }
        // And the drag it is derived from must still decelerate, or the two are the same
        // gesture and the measurement that separated them has been undone.
        let drag = planner.plan_swipe(from, to, 320);
        let hops: Vec<f64> = std::iter::once(&drag.start)
            .chain(drag.steps.iter().map(|step| &step.point))
            .collect::<Vec<_>>()
            .windows(2)
            .map(|pair| ((pair[1].x - pair[0].x).powi(2) + (pair[1].y - pair[0].y).powi(2)).sqrt())
            .collect();
        let last = *hops.last().expect("legs");
        let longest = hops.iter().cloned().fold(0.0_f64, f64::max);
        assert!(
            last < longest / 2.0,
            "a drag still eases to a stop: last {last:.1}px against longest {longest:.1}px"
        );
    }
}
