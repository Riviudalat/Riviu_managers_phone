//! Group-sync timing & offset policy for fleet fan-out (feature A1, ported from xiaowei
//! `delaySync` / `delayOffset`).
//!
//! When one gesture is mirrored to a whole group (`group_input`), acting in perfect lockstep
//! at identical coordinates is exactly the signature behavioural detection looks for. This
//! module decides, per device, **how long to wait before its action** and **how far to nudge
//! the tap/swipe coordinates**. It is deliberately *pure*: the sleeping and the coordinate
//! mutation happen in the command layer — here we only compute *what* to apply, so the policy
//! is unit-testable without hardware or a real clock.
//!
//! Determinism follows the same pattern the nurture engine already uses
//! (`StdRng::seed_from_u64`, see `human_behavior.rs`): given a seed the plan is reproducible,
//! yet each device draws independently. The command layer picks a fresh seed per operation
//! (`StdRng::from_entropy`), so successive group actions differ while any single one stays
//! testable.

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};

/// How long each device waits before performing its share of a group action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", tag = "mode")]
pub enum DelayPolicy {
    /// No inter-device delay. Every phone acts as soon as the loop reaches it.
    #[default]
    None,
    /// Each phone waits a delay drawn uniformly from `[min_ms, max_ms]`. If `max_ms < min_ms`
    /// the range collapses to `min_ms` rather than panicking on an empty range.
    // `rename_all` on the enum renames the *variants*; the fields inside a struct variant
    // need their own rename_all to reach the frontend as camelCase.
    #[serde(rename_all = "camelCase")]
    Random { min_ms: u64, max_ms: u64 },
    /// The phone at ordinal `i` waits `i * step_ms` — a fixed staircase so the fleet does not
    /// fire in one instant. Ordinal 0 waits nothing.
    #[serde(rename_all = "camelCase")]
    Staggered { step_ms: u64 },
}

/// Random pixel jitter applied to coordinate-bearing actions (tap/swipe).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OffsetPolicy {
    /// Maximum absolute pixel jitter, applied independently to x and y. `0` disables offset.
    pub max_px: u32,
}

/// The full group-sync policy: a delay schedule plus coordinate jitter. Both default to
/// "do nothing", so an absent policy is exactly the old lockstep behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GroupSyncPolicy {
    #[serde(default)]
    pub delay: DelayPolicy,
    #[serde(default)]
    pub offset: OffsetPolicy,
}

/// What to apply to one device: computed once, then consumed by the command layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DevicePlan {
    /// Milliseconds to sleep before this device's action.
    pub delay_ms: u64,
    /// Pixel offset to add to x (may be negative).
    pub dx: f64,
    /// Pixel offset to add to y (may be negative).
    pub dy: f64,
}

/// Distinct sub-streams so delay, dx and dy do not correlate for a given seed+ordinal.
const SALT_ORDINAL: u64 = 0x9E37_79B9_7F4A_7C15;
const SALT_DELAY: u64 = 0x1;
const SALT_DX: u64 = 0x2;
const SALT_DY: u64 = 0x3;

fn stream_rng(seed: u64, ordinal: usize, salt: u64) -> StdRng {
    // Mix the seed with the ordinal and a per-axis salt so each (device, axis) draw is
    // independent yet reproducible. Same shape as the seeded RNG the nurture path uses.
    let mixed = seed
        .wrapping_add((ordinal as u64).wrapping_mul(SALT_ORDINAL))
        .wrapping_add(salt.wrapping_mul(0x2545_F491_4F6C_DD1D));
    StdRng::seed_from_u64(mixed)
}

impl DelayPolicy {
    fn delay_for(&self, ordinal: usize, seed: u64) -> u64 {
        match *self {
            DelayPolicy::None => 0,
            DelayPolicy::Staggered { step_ms } => (ordinal as u64).saturating_mul(step_ms),
            DelayPolicy::Random { min_ms, max_ms } => {
                let (lo, hi) = if min_ms <= max_ms {
                    (min_ms, max_ms)
                } else {
                    // A misconfigured range is the operator's slip, not a crash.
                    (min_ms, min_ms)
                };
                stream_rng(seed, ordinal, SALT_DELAY).gen_range(lo..=hi)
            }
        }
    }
}

impl OffsetPolicy {
    fn offset_for(&self, ordinal: usize, seed: u64) -> (f64, f64) {
        if self.max_px == 0 {
            return (0.0, 0.0);
        }
        let m = self.max_px as i64;
        let dx = stream_rng(seed, ordinal, SALT_DX).gen_range(-m..=m) as f64;
        let dy = stream_rng(seed, ordinal, SALT_DY).gen_range(-m..=m) as f64;
        (dx, dy)
    }
}

impl GroupSyncPolicy {
    /// Returns true when there is nothing to do — lets the command layer skip the whole
    /// machinery (and the per-device seeding) on the common no-policy path.
    pub fn is_noop(&self) -> bool {
        matches!(self.delay, DelayPolicy::None) && self.offset.max_px == 0
    }

    /// Compute the plan for the device at `ordinal` (0-based position in the group), given a
    /// per-operation `seed`. `_count` is accepted for symmetry / future policies (e.g. easing
    /// the stagger across the fleet) and is currently unused.
    pub fn plan(&self, ordinal: usize, _count: usize, seed: u64) -> DevicePlan {
        let delay_ms = self.delay.delay_for(ordinal, seed);
        let (dx, dy) = self.offset.offset_for(ordinal, seed);
        DevicePlan { delay_ms, dx, dy }
    }
}

/// Add an offset to a point, clamping into `[0, bound-1]` on any axis whose bound is given
/// (image-space taps must stay on-screen). Axes with no bound are only floored at 0.
pub fn apply_offset(
    x: f64,
    y: f64,
    dx: f64,
    dy: f64,
    max_w: Option<f64>,
    max_h: Option<f64>,
) -> (f64, f64) {
    let clamp = |v: f64, bound: Option<f64>| {
        let v = v.max(0.0);
        match bound {
            Some(b) if b > 0.0 => v.min(b - 1.0),
            _ => v,
        }
    };
    (clamp(x + dx, max_w), clamp(y + dy, max_h))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: u64 = 0xDEAD_BEEF;

    #[test]
    fn default_policy_is_noop() {
        let p = GroupSyncPolicy::default();
        assert!(p.is_noop());
        for ordinal in 0..5 {
            let plan = p.plan(ordinal, 5, SEED);
            assert_eq!(plan.delay_ms, 0);
            assert_eq!(plan.dx, 0.0);
            assert_eq!(plan.dy, 0.0);
        }
    }

    #[test]
    fn staggered_delay_is_ordinal_times_step() {
        let p = GroupSyncPolicy {
            delay: DelayPolicy::Staggered { step_ms: 100 },
            offset: OffsetPolicy::default(),
        };
        assert!(!p.is_noop());
        assert_eq!(p.plan(0, 4, SEED).delay_ms, 0);
        assert_eq!(p.plan(1, 4, SEED).delay_ms, 100);
        assert_eq!(p.plan(3, 4, SEED).delay_ms, 300);
    }

    #[test]
    fn staggered_delay_saturates_instead_of_overflowing() {
        let p = DelayPolicy::Staggered { step_ms: u64::MAX };
        // ordinal 2 * u64::MAX must not panic in debug (overflow) — it saturates.
        assert_eq!(p.delay_for(2, SEED), u64::MAX);
    }

    #[test]
    fn random_delay_stays_in_range_and_is_deterministic() {
        let p = DelayPolicy::Random {
            min_ms: 50,
            max_ms: 150,
        };
        for ordinal in 0..50 {
            let d = p.delay_for(ordinal, SEED);
            assert!((50..=150).contains(&d), "ordinal {ordinal} gave {d}");
        }
        // Same seed + ordinal reproduces; the command layer relies on this for testability.
        assert_eq!(p.delay_for(7, SEED), p.delay_for(7, SEED));
    }

    #[test]
    fn random_delay_survives_inverted_range() {
        let p = DelayPolicy::Random {
            min_ms: 200,
            max_ms: 100,
        };
        // Collapses to min_ms rather than panicking on an empty gen_range.
        assert_eq!(p.delay_for(0, SEED), 200);
        assert_eq!(p.delay_for(9, SEED), 200);
    }

    #[test]
    fn random_delay_equal_bounds_is_that_value() {
        let p = DelayPolicy::Random {
            min_ms: 120,
            max_ms: 120,
        };
        assert_eq!(p.delay_for(3, SEED), 120);
    }

    #[test]
    fn offset_stays_within_max_px() {
        let off = OffsetPolicy { max_px: 5 };
        for ordinal in 0..50 {
            let (dx, dy) = off.offset_for(ordinal, SEED);
            assert!((-5.0..=5.0).contains(&dx), "dx {dx}");
            assert!((-5.0..=5.0).contains(&dy), "dy {dy}");
        }
    }

    #[test]
    fn offset_zero_is_no_jitter() {
        let off = OffsetPolicy { max_px: 0 };
        assert_eq!(off.offset_for(4, SEED), (0.0, 0.0));
    }

    #[test]
    fn dx_and_dy_do_not_lockstep() {
        // Independent sub-streams: across ordinals, dx == dy on every device would betray a
        // shared draw. Require at least one device where they differ.
        let off = OffsetPolicy { max_px: 8 };
        let any_diff = (0..32).any(|o| {
            let (dx, dy) = off.offset_for(o, SEED);
            dx != dy
        });
        assert!(any_diff);
    }

    #[test]
    fn apply_offset_clamps_into_image_bounds() {
        // Negative pushed back to 0.
        assert_eq!(
            apply_offset(2.0, 2.0, -5.0, -5.0, Some(100.0), Some(200.0)),
            (0.0, 0.0)
        );
        // Over the top clamped to bound-1.
        assert_eq!(
            apply_offset(99.0, 199.0, 10.0, 10.0, Some(100.0), Some(200.0)),
            (99.0, 199.0)
        );
        // No bound: only floored at 0, no upper clamp.
        assert_eq!(
            apply_offset(50.0, 50.0, 5.0, -5.0, None, None),
            (55.0, 45.0)
        );
    }

    #[test]
    fn policy_serde_round_trips_camel_case() {
        let p = GroupSyncPolicy {
            delay: DelayPolicy::Random {
                min_ms: 10,
                max_ms: 20,
            },
            offset: OffsetPolicy { max_px: 3 },
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"mode\":\"random\""), "{json}");
        assert!(json.contains("\"minMs\":10"), "{json}");
        assert!(json.contains("\"maxPx\":3"), "{json}");
        let back: GroupSyncPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn absent_fields_default_to_noop() {
        // Frontend may send `{}` — must deserialize to the do-nothing policy.
        let p: GroupSyncPolicy = serde_json::from_str("{}").unwrap();
        assert!(p.is_noop());
        // And a delay-only policy leaves offset at zero.
        let p: GroupSyncPolicy =
            serde_json::from_str(r#"{"delay":{"mode":"staggered","stepMs":40}}"#).unwrap();
        assert_eq!(p.delay, DelayPolicy::Staggered { step_ms: 40 });
        assert_eq!(p.offset.max_px, 0);
    }
}
