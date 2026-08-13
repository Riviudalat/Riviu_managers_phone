//! Picking up settings an operator changed while a session is running.
//!
//! Its own module because both feed loops need it and they must not each have their
//! own version. The pixel loop lives in [`super`], the hierarchy loop in
//! [`super::hierarchy`], and a live change has to mean the same thing in both — which
//! it did not: the hierarchy loop took `&NurtureSettings` once and kept it for the
//! whole session, so on Android every switch and slider in the panel was inert.
//! Measured on an SM-N950F on 12/08/2026: the settings row was saved with
//! `likeProb 100`, `fatigue false`, `frenzyProb 0` while a session ran, and the next
//! 16 posts were driven by the numbers from before the save.
//!
//! The subtler half is why re-reading the row was not enough even where it happened.
//! `HumanBehavior` and `HumanSessionPolicy` are constructed *before* the loop and hold
//! their own copies of the rhythm switches and the per-hour ceilings, so a refreshed
//! `NurtureSettings` reached the struct and stopped there. A unit test of
//! [`NurtureSettings::absorb_live_changes`] alone passes happily against that bug,
//! which is exactly what happened — hence [`apply_live_settings`] does all three steps
//! together, in one place, with a test that asserts the *decision* changed.

use crate::human_behavior::{HumanBehavior, HumanSessionPolicy, MoodCycle};
use crate::types::NurtureSettings;

/// Where a running session goes to pick up settings saved after it started.
///
/// A trait because the loops have no business knowing about a database: the app hands
/// them one of these backed by the settings row, while the Android G2 probe hands them
/// nothing and keeps the numbers it was started with — which is what a gate should do.
pub trait LiveSettings: Send + Sync {
    /// Fold whatever has been saved since the session started into `settings`.
    ///
    /// Implementors must go through [`NurtureSettings::absorb_live_changes`] so that
    /// which fields are live, and which need a restart, is decided in exactly one
    /// place.
    fn refresh(&self, settings: &mut NurtureSettings);
}

/// How many posts a session is aiming for.
///
/// Shared by both loops so the answer cannot differ between them, and it is here rather
/// than in either loop for the same reason [`apply_live_settings`] is.
///
/// **This used to be ignored on the only path an operator uses.** Both loops computed
/// `u32::MAX` whenever a run duration was set, and
/// `NurturePopup.tsx` starts a manual run with no duration, which makes
/// `nurture_start` fill in a randomised 2–3 hour horizon — so a duration was *always*
/// set and the video count never bound anything. Measured through the app on
/// 12/08/2026: `GIỚI HẠN VIDEO = 15` produced runs that reached video **68** and
/// **36**. The panel showed the field with a "needs a restart" badge, as if it bounded
/// the run.
///
/// Both bounds apply now and the run ends at whichever arrives first. The duration keeps
/// doing its job — it is the ceiling that stops a forgotten session, and its randomised
/// default still means two phones started together do not stop at the same instant — and
/// the count means what the panel says it means.
pub(super) fn video_target(settings: &NurtureSettings) -> u32 {
    settings
        .num_videos
        .max(1)
        .saturating_mul(settings.num_rounds.max(1))
}

/// Bring a running session up to date: the row, then the two objects that decide.
///
/// Call once per post, not per action. A probability that changed between rolling an
/// action and confirming it would make that action's own record unexplainable.
///
/// Returns whether a live source was wired at all, so a caller can tell "nothing
/// changed" from "nobody is listening".
pub(super) fn apply_live_settings(
    live: Option<&dyn LiveSettings>,
    settings: &mut NurtureSettings,
    human: &mut HumanBehavior,
    policy: &mut HumanSessionPolicy,
    moods: &mut MoodCycle,
) -> bool {
    let Some(live) = live else {
        return false;
    };
    live.refresh(settings);
    // `refresh` has already folded the per-feature switches into the probabilities, so
    // these two see the effective numbers rather than the operator's raw ones.
    human.retune(settings.fatigue, settings.time_of_day, settings.pause_swipe);
    policy.retune(
        settings.like_prob,
        settings.comment_prob,
        settings.follow_prob,
        settings.human_limits,
    );
    // Third object with its own copy of the answer. `Mood::Skimming` zeroes the like
    // probability outright, so leaving the cycle alone would keep a 100 % setting at zero
    // for most posts even with every ceiling lifted.
    moods.retune(settings.human_limits);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::human_behavior::{MoodCycle, PolicyAction};
    use std::time::Duration;

    /// A live source that hands over a whole row, the way the database does.
    struct Saved(NurtureSettings);

    impl LiveSettings for Saved {
        fn refresh(&self, settings: &mut NurtureSettings) {
            settings.absorb_live_changes(&self.0);
            // The engine re-folds the switches after absorbing, because
            // `absorb_live_changes` copies the operator's raw numbers. Mirrored here so
            // the test exercises the same two steps the app performs.
            *settings = std::mem::take(settings).into_effective();
        }
    }

    fn off_at_start() -> NurtureSettings {
        NurtureSettings {
            like_prob: 0,
            comment_prob: 0,
            follow_prob: 0,
            fatigue: false,
            time_of_day: false,
            pause_swipe: false,
            // These tests are about the per-hour ceilings, so they ask for them. The
            // shipped default is off — see `full_control_lets_the_configured_numbers_rule`.
            human_limits: true,
            ..NurtureSettings::default()
        }
    }

    #[test]
    fn full_control_lets_the_configured_numbers_rule() {
        // The shipped default, and the operator's decision on 12/08/2026. What it removes
        // is asserted here rather than described, because each of these silently overrode
        // a number set in the panel:
        //
        // * a per-hour ceiling of 8–16 likes — so "Thích 100%" stopped at 8;
        // * at most two of the last five cards may be interacted with — so 100% could
        //   never exceed 40% of posts, whatever else was configured;
        // * a 12–35 s wait after every action;
        // * a 15–90 s rest every 7–13 videos, the `nghỉ tự nhiên 85s` in the log.
        let settings = NurtureSettings {
            like_prob: 100,
            human_limits: false,
            ..NurtureSettings::default()
        };
        let mut policy = HumanSessionPolicy::new(
            settings.like_prob,
            settings.comment_prob,
            settings.follow_prob,
            settings.human_limits,
        );

        // No per-hour ceiling: a hundred attempts in a row are all allowed, and each one is
        // recorded, so this is not passing merely because nothing was counted.
        for attempt in 1..=100 {
            assert!(
                policy.can_attempt(PolicyAction::Like),
                "attempt {attempt} was refused with the limits off"
            );
            policy.record_attempt(PolicyAction::Like);
        }
        // No two-of-five rule: every card can be interacted with, back to back.
        for _ in 0..10 {
            policy.begin_post();
            assert!(policy.can_interact_with_post());
            policy.mark_post_interacted();
        }
        assert!(policy.can_interact_with_post(), "the fifth in a row too");
        // No imposed rests, however many videos go by.
        for _ in 0..40 {
            assert_eq!(policy.rest_after_video(), None);
        }
        assert!(!policy.should_take_home_break());
        assert!(!policy.should_take_block_break());
        assert!(!policy.should_cold_restart());
        // The gap left is a settle, not pacing: enough for a screen to stop animating
        // before the next gesture, and far below anything a human-pacing argument asks for.
        let gap = policy.min_action_gap();
        assert!(
            gap < Duration::from_secs(2),
            "the settle should be sub-second-ish, got {gap:?}"
        );
        assert!(gap > Duration::ZERO, "zero would tap into a transition");
    }

    #[test]
    fn full_control_also_neutralises_the_mood_multipliers() {
        // The third layer, and the one that survived the first attempt at this switch.
        // `Mood::Skimming` multiplies the like probability by **0.0** and takes about 60 % of
        // videos, so lifting every ceiling still left a 100 % setting producing nothing on
        // most posts. Measured: a twelve-video run at `like_prob = 100` with the ceilings
        // already gone reported `tim 0/0`, every post logged `(lướt)`.
        use crate::human_behavior::{roll_feed_action_in_mood, FeedAction, Mood};

        assert_eq!(Mood::Skimming.watch_mult(), 0.55);
        assert_eq!(
            Mood::Neutral.watch_mult(),
            1.0,
            "the configured watch window has to be the real one too"
        );
        // Neutral is not "a mood that likes a lot" — it is the absence of scaling, so a
        // configured 100 % lands on every post and a configured 0 % on none.
        for _ in 0..50 {
            assert!(matches!(
                roll_feed_action_in_mood(100, 0, Mood::Neutral),
                FeedAction::Like
            ));
            assert!(matches!(
                roll_feed_action_in_mood(0, 0, Mood::Neutral),
                FeedAction::None
            ));
        }
        // And a skim run at the same setting interacts with nothing, which is what the
        // operator was hitting.
        assert!(matches!(
            roll_feed_action_in_mood(100, 0, Mood::Skimming),
            FeedAction::None
        ));

        // The cycle follows the switch, in both directions, mid-session.
        let mut moods = MoodCycle::new();
        moods.retune(false);
        for _ in 0..20 {
            assert_eq!(moods.next().0, Mood::Neutral);
        }
        moods.retune(true);
        assert_ne!(
            moods.current(),
            Mood::Neutral,
            "turning the pacing back on has to resume varying"
        );
    }

    #[test]
    fn the_pacing_switch_takes_effect_mid_session() {
        // It is a switch, so it is live-tunable, so it has to reach the policy object the
        // way every other live switch does — the same defect class as the rhythm switches.
        let mut settings = NurtureSettings {
            like_prob: 100,
            human_limits: true,
            ..NurtureSettings::default()
        }
        .into_effective();
        let mut human = HumanBehavior::new("casual", false, false, false);
        let mut policy = HumanSessionPolicy::new(100, 0, 0, true);
        let mut moods = MoodCycle::new();

        // Burn through the hourly ceiling, whatever it rolled.
        for _ in 0..20 {
            if policy.can_attempt(PolicyAction::Like) {
                policy.record_attempt(PolicyAction::Like);
            }
        }
        assert!(
            !policy.can_attempt(PolicyAction::Like),
            "the ceiling should be reached after twenty attempts"
        );

        let saved = Saved(NurtureSettings {
            like_prob: 100,
            human_limits: false,
            ..NurtureSettings::default()
        });
        apply_live_settings(
            Some(&saved),
            &mut settings,
            &mut human,
            &mut policy,
            &mut moods,
        );

        assert!(!settings.human_limits);
        assert!(
            policy.can_attempt(PolicyAction::Like),
            "switching the pacing off has to lift the ceiling on this post, not the next run"
        );
        assert_eq!(policy.rest_after_video(), None);
    }

    #[test]
    fn the_video_count_bounds_a_run_that_also_has_a_duration() {
        // The regression: both loops used to return `u32::MAX` whenever a duration was
        // set, and the app always sets one. So this asserts the number itself, not a
        // branch — there is no longer a case where the operator's count is discarded.
        assert_eq!(
            video_target(&NurtureSettings {
                num_videos: 15,
                num_rounds: 1,
                ..NurtureSettings::default()
            }),
            15
        );
        // Rounds multiply, the way the panel's two fields read.
        assert_eq!(
            video_target(&NurtureSettings {
                num_videos: 15,
                num_rounds: 4,
                ..NurtureSettings::default()
            }),
            60
        );
        // A zero from an old or hand-edited row must not turn into "stop immediately".
        assert_eq!(
            video_target(&NurtureSettings {
                num_videos: 0,
                num_rounds: 0,
                ..NurtureSettings::default()
            }),
            1
        );
        // And a big count cannot overflow into a small one.
        assert_eq!(
            video_target(&NurtureSettings {
                num_videos: u32::MAX,
                num_rounds: 100,
                ..NurtureSettings::default()
            }),
            u32::MAX
        );
    }

    #[test]
    fn switching_a_feature_on_mid_session_lets_it_actually_fire() {
        // The regression this module exists for. A feature that was off when the session
        // started has a closed per-hour ceiling, and `can_attempt` reads a closed ceiling
        // as *never* — so before this, turning it on left it on in every part of the UI
        // and unable to fire for the rest of the run, silently.
        let mut settings = off_at_start().into_effective();
        let mut human = HumanBehavior::new("casual", false, false, false);
        let mut policy = HumanSessionPolicy::new(
            settings.like_prob,
            settings.comment_prob,
            settings.follow_prob,
            settings.human_limits,
        );
        let mut moods = MoodCycle::new();
        assert!(
            !policy.can_attempt(PolicyAction::Like),
            "a feature off at start must not fire"
        );

        let saved = Saved(NurtureSettings {
            like_prob: 60,
            like_enabled: true,
            ..off_at_start()
        });
        assert!(apply_live_settings(
            Some(&saved),
            &mut settings,
            &mut human,
            &mut policy,
            &mut moods,
        ));

        assert_eq!(settings.like_prob, 60, "the row reached the settings");
        assert!(
            policy.can_attempt(PolicyAction::Like),
            "…and reached the object that decides, which is the part that was broken"
        );
    }

    #[test]
    fn switching_a_feature_off_mid_session_stops_it_on_this_post() {
        // Not merely "the probability roll stops selecting it": the ceiling shuts too, so
        // the switch takes effect even on a post whose roll already wanted the action.
        let mut settings = NurtureSettings {
            like_prob: 80,
            follow_prob: 50,
            human_limits: true,
            ..NurtureSettings::default()
        }
        .into_effective();
        let mut human = HumanBehavior::new("casual", true, true, true);
        let mut policy = HumanSessionPolicy::new(
            settings.like_prob,
            settings.comment_prob,
            settings.follow_prob,
            settings.human_limits,
        );
        let mut moods = MoodCycle::new();
        assert!(policy.can_attempt(PolicyAction::Like));

        let saved = Saved(NurtureSettings {
            like_prob: 80,
            like_enabled: false,
            follow_prob: 50,
            follow_enabled: true,
            human_limits: true,
            ..NurtureSettings::default()
        });
        apply_live_settings(
            Some(&saved),
            &mut settings,
            &mut human,
            &mut policy,
            &mut moods,
        );

        assert_eq!(
            settings.like_prob, 0,
            "the switch is folded into the probability"
        );
        assert!(!policy.can_attempt(PolicyAction::Like));
        // The operator switched one feature off, not the session.
        assert!(policy.can_attempt(PolicyAction::Follow));
    }

    #[test]
    fn the_rhythm_switches_reach_the_behaviour_model() {
        // `fatigue`, `time_of_day` and `pause_swipe` are consumed only by
        // `HumanBehavior::new`, so before `retune` existed these three were the clearest
        // case of a saved change that reached `NurtureSettings` and went no further.
        let mut settings = off_at_start().into_effective();
        let mut human = HumanBehavior::new("casual", false, false, false);
        let mut policy = HumanSessionPolicy::new(0, 0, 0, true);
        let mut moods = MoodCycle::new();
        assert!(!human.fatigue_is_on());

        let saved = Saved(NurtureSettings {
            fatigue: true,
            time_of_day: true,
            pause_swipe: true,
            ..off_at_start()
        });
        apply_live_settings(
            Some(&saved),
            &mut settings,
            &mut human,
            &mut policy,
            &mut moods,
        );

        assert!(human.fatigue_is_on());
        assert!(human.time_of_day_is_on());
        assert!(human.pause_swipe_is_on());
    }

    #[test]
    fn a_session_with_no_live_source_keeps_the_numbers_it_started_with() {
        // The G2 probe's case, and it must stay this way: a gate whose settings could
        // move underneath it is not measuring the run it reported.
        let mut settings = NurtureSettings {
            like_prob: 25,
            ..NurtureSettings::default()
        }
        .into_effective();
        let mut human = HumanBehavior::new("casual", true, false, false);
        let mut policy = HumanSessionPolicy::new(25, 0, 0, true);
        let mut moods = MoodCycle::new();

        assert!(!apply_live_settings(
            None,
            &mut settings,
            &mut human,
            &mut policy,
            &mut moods,
        ));
        assert_eq!(settings.like_prob, 25);
        assert!(human.fatigue_is_on());
    }

    #[test]
    fn repeated_application_does_not_move_an_open_ceiling() {
        // Safe to call every post is a requirement, not a nicety: re-rolling the per-hour
        // ceiling once a post would make "at most 8–16 an hour" mean nothing at all.
        let mut settings = NurtureSettings {
            like_prob: 40,
            human_limits: true,
            ..NurtureSettings::default()
        }
        .into_effective();
        let mut human = HumanBehavior::new("casual", false, false, false);
        let mut policy = HumanSessionPolicy::new(40, 0, 0, true);
        let mut moods = MoodCycle::new();
        let saved = Saved(NurtureSettings {
            like_prob: 40,
            human_limits: true,
            ..NurtureSettings::default()
        });

        let first = policy.like_ceiling();
        for _ in 0..25 {
            apply_live_settings(
                Some(&saved),
                &mut settings,
                &mut human,
                &mut policy,
                &mut moods,
            );
        }
        assert_eq!(policy.like_ceiling(), first);
    }
}
