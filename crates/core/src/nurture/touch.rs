use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::collections::HashSet;

use crate::types::TapPoint;

// Native XCTest ultimately synthesizes integer-ish logical points. Keeping the
// planner on that grid prevents two distinct floats from landing on one device
// coordinate after transport rounding.
const GRID_SCALE: f64 = 1.0;
const RECENT_MIN_DISTANCE: f64 = 3.0;

/// Per-device tap point history. Coordinates are quantized to one logical point
/// because the native route may round sub-point values before synthesis.
/// A point is never returned twice while this planner is alive.
#[derive(Debug)]
pub(super) struct TouchPointPlanner {
    width: f64,
    height: f64,
    rng: StdRng,
    used: HashSet<(i32, i32)>,
    recent: Vec<TapPoint>,
}

impl TouchPointPlanner {
    pub(super) fn new(screen_size: (f64, f64)) -> Self {
        Self {
            width: screen_size.0.max(1.0),
            height: screen_size.1.max(1.0),
            rng: StdRng::from_entropy(),
            used: HashSet::new(),
            recent: Vec::new(),
        }
    }

    pub(super) fn next(&mut self, center: TapPoint, radius: (f64, f64)) -> TapPoint {
        let rx = radius.0.abs().max(1.0);
        let ry = radius.1.abs().max(1.0);
        let bounds = self.bounds(&center, rx, ry);

        // Prefer a fresh point that is also separated from the recent touch
        // trail. The exact uniqueness check is global to this device session.
        for _ in 0..128 {
            let point = self.random_point(bounds);
            if self.accept(&point, true) {
                return point;
            }
        }

        // A narrow control can eventually exhaust its preferred rectangle.
        // Scan it deterministically before widening, so the tap remains inside
        // the intended hit area whenever a fresh coordinate is available.
        if let Some(point) = self.scan(bounds, true) {
            return point;
        }
        for scale in [1.5, 2.0, 3.0, 5.0] {
            let expanded = self.bounds(&center, rx * scale, ry * scale);
            if let Some(point) = self.scan(expanded, false) {
                tracing::debug!(
                    "touch point area widened after {} used coordinates",
                    self.used.len()
                );
                return point;
            }
        }

        // The full logical screen still has a large finite pool. Reaching this
        // branch means an unusually long standalone interaction exhausted all
        // nearby points; preserve uniqueness rather than returning a duplicate.
        let full = (0.5, self.width - 0.5, 0.5, self.height - 0.5);
        self.scan(full, false).unwrap_or_else(|| {
            panic!(
                "touch point planner exhausted the logical screen ({} coordinates)",
                self.used.len()
            )
        })
    }

    fn bounds(&self, center: &TapPoint, rx: f64, ry: f64) -> (f64, f64, f64, f64) {
        let min_x = (center.x - rx).clamp(0.5, self.width - 0.5);
        let max_x = (center.x + rx).clamp(min_x, self.width - 0.5);
        let min_y = (center.y - ry).clamp(0.5, self.height - 0.5);
        let max_y = (center.y + ry).clamp(min_y, self.height - 0.5);
        (min_x, max_x, min_y, max_y)
    }

    fn random_point(&mut self, bounds: (f64, f64, f64, f64)) -> TapPoint {
        let (min_x, max_x, min_y, max_y) = bounds;
        Self::quantize(
            self.rng.gen_range(min_x..=max_x),
            self.rng.gen_range(min_y..=max_y),
        )
    }

    fn scan(&mut self, bounds: (f64, f64, f64, f64), require_recent_gap: bool) -> Option<TapPoint> {
        let (min_x, max_x, min_y, max_y) = bounds;
        let start_x = self.rng.gen_range(0..=3);
        let start_y = self.rng.gen_range(0..=3);
        let x0 = (min_x * GRID_SCALE).ceil() as i32;
        let x1 = (max_x * GRID_SCALE).floor() as i32;
        let y0 = (min_y * GRID_SCALE).ceil() as i32;
        let y1 = (max_y * GRID_SCALE).floor() as i32;
        for y_offset in 0..=(y1 - y0).max(0) {
            let y = y0 + ((y_offset + start_y) % (y1 - y0 + 1).max(1));
            for x_offset in 0..=(x1 - x0).max(0) {
                let x = x0 + ((x_offset + start_x) % (x1 - x0 + 1).max(1));
                let point = TapPoint {
                    x: x as f64 / GRID_SCALE,
                    y: y as f64 / GRID_SCALE,
                };
                if self.accept(&point, require_recent_gap) {
                    return Some(point);
                }
            }
        }
        None
    }

    fn accept(&mut self, point: &TapPoint, require_recent_gap: bool) -> bool {
        let key = (
            (point.x * GRID_SCALE).round() as i32,
            (point.y * GRID_SCALE).round() as i32,
        );
        if self.used.contains(&key) {
            return false;
        }
        if require_recent_gap
            && self.recent.iter().rev().take(96).any(|previous| {
                let dx = previous.x - point.x;
                let dy = previous.y - point.y;
                dx * dx + dy * dy < RECENT_MIN_DISTANCE * RECENT_MIN_DISTANCE
            })
        {
            return false;
        }
        self.used.insert(key);
        self.recent.push(point.clone());
        true
    }

    fn quantize(x: f64, y: f64) -> TapPoint {
        TapPoint {
            x: (x * GRID_SCALE).round() / GRID_SCALE,
            y: (y * GRID_SCALE).round() / GRID_SCALE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn planner_never_repeats_quantized_points_in_a_hit_area() {
        let mut planner = TouchPointPlanner::new((375.0, 667.0));
        let mut seen = HashSet::new();
        for _ in 0..400 {
            let point = planner.next(TapPoint { x: 312.0, y: 307.0 }, (18.0, 18.0));
            let key = ((point.x * GRID_SCALE) as i32, (point.y * GRID_SCALE) as i32);
            assert!(seen.insert(key), "repeated point: {point:?}");
            assert!((294.0..=330.0).contains(&point.x));
            assert!((289.0..=325.0).contains(&point.y));
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
}
