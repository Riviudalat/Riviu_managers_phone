//! Desired-state Save primitive shared by hierarchy and pixel callers.

use crate::driver::StatefulElementBox;
use crate::screen::ActionRail;
use crate::types::TapPoint;
use serde::{Deserialize, Serialize};

/// The measured state of TikTok's bookmark toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BookmarkState {
    Saved,
    Unsaved,
    Unreadable,
}

/// Stable, typed identity evidence for the card that may receive the Save effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveCardIdentity {
    Hierarchy {
        author: String,
        sound: Option<String>,
    },
    Pixel {
        author: String,
        caption: Option<String>,
    },
}

/// One atomic observation supplied by either the hierarchy or pixel adapter.
#[derive(Debug, Clone)]
pub struct SaveObservation {
    pub identity: Option<SaveCardIdentity>,
    /// Monotonic observation number owned by the adapter (hierarchy read or frame sequence).
    pub sequence: u64,
    pub state: BookmarkState,
    pub tap_point: Option<TapPoint>,
}

/// The strongest conclusion one Save attempt can prove.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SaveVerdict {
    Saved,
    AlreadySaved,
    NoControl,
    StateUnreadable,
    CardChangedBeforeEffect,
    FailedBeforeEffect,
    CardChangedAfterEffect,
    NotConfirmed,
    UncertainAfterEffect,
}

/// Evidence returned on every branch, including failures.
#[derive(Debug, Clone)]
pub struct SaveEvidence {
    pub verdict: SaveVerdict,
    pub initial: Option<SaveObservation>,
    pub final_observation: Option<SaveObservation>,
    pub effect_boundary_crossed: bool,
    pub error: Option<String>,
}

impl SaveEvidence {
    pub fn effect_may_have_gone_out(&self) -> bool {
        self.effect_boundary_crossed
    }

    pub fn retry_is_safe(&self) -> bool {
        !self.effect_boundary_crossed
    }
}

/// Device-facing operations required by the state machine.
#[async_trait::async_trait]
pub trait SaveAdapter: Send {
    async fn observe(&mut self) -> anyhow::Result<SaveObservation>;
    async fn tap(&mut self, point: TapPoint) -> anyhow::Result<()>;
}

fn evidence(
    verdict: SaveVerdict,
    initial: Option<SaveObservation>,
    final_observation: Option<SaveObservation>,
    effect_boundary_crossed: bool,
    error: Option<String>,
) -> SaveEvidence {
    SaveEvidence {
        verdict,
        initial,
        final_observation,
        effect_boundary_crossed,
        error,
    }
}

fn pre_effect_verdict(observation: &SaveObservation) -> Option<SaveVerdict> {
    if observation.tap_point.is_none() {
        return Some(SaveVerdict::NoControl);
    }
    if observation.identity.is_none() || observation.state == BookmarkState::Unreadable {
        return Some(SaveVerdict::StateUnreadable);
    }
    (observation.state == BookmarkState::Saved).then_some(SaveVerdict::AlreadySaved)
}

/// Reach Saved state with at most one tap.
///
/// `durable_intent` is called once, after the immediate identity re-proof and immediately before
/// the tap. From that callback onward every inconclusive branch is after-effect and unsafe for an
/// automatic retry. A caller may explicitly retry because the first observation is desired-state:
/// an already-saved card returns without crossing the boundary or toggling it back off.
pub async fn tiktok_save<A, F>(adapter: &mut A, durable_intent: F) -> SaveEvidence
where
    A: SaveAdapter + ?Sized,
    F: FnOnce(&SaveObservation) -> anyhow::Result<()>,
{
    let initial = match adapter.observe().await {
        Ok(observation) => observation,
        Err(error) => {
            return evidence(
                SaveVerdict::FailedBeforeEffect,
                None,
                None,
                false,
                Some(error.to_string()),
            )
        }
    };
    if let Some(verdict) = pre_effect_verdict(&initial) {
        return evidence(verdict, Some(initial), None, false, None);
    }

    let reproved = match adapter.observe().await {
        Ok(observation) => observation,
        Err(error) => {
            return evidence(
                SaveVerdict::FailedBeforeEffect,
                Some(initial),
                None,
                false,
                Some(error.to_string()),
            )
        }
    };
    if reproved.sequence <= initial.sequence {
        return evidence(
            SaveVerdict::FailedBeforeEffect,
            Some(initial),
            Some(reproved),
            false,
            Some("the immediate re-proof was not newer than the initial observation".to_owned()),
        );
    }
    if reproved.identity.is_none() || reproved.identity != initial.identity {
        return evidence(
            SaveVerdict::CardChangedBeforeEffect,
            Some(initial),
            Some(reproved),
            false,
            None,
        );
    }
    if let Some(verdict) = pre_effect_verdict(&reproved) {
        return evidence(verdict, Some(initial), Some(reproved), false, None);
    }

    if let Err(error) = durable_intent(&reproved) {
        return evidence(
            SaveVerdict::FailedBeforeEffect,
            Some(initial),
            Some(reproved),
            false,
            Some(error.to_string()),
        );
    }

    let point = reproved
        .tap_point
        .clone()
        .expect("pre_effect_verdict returned for a missing Save control");
    if let Err(error) = adapter.tap(point).await {
        return evidence(
            SaveVerdict::UncertainAfterEffect,
            Some(initial),
            Some(reproved),
            true,
            Some(error.to_string()),
        );
    }

    let confirmed = match adapter.observe().await {
        Ok(observation) => observation,
        Err(error) => {
            return evidence(
                SaveVerdict::UncertainAfterEffect,
                Some(initial),
                Some(reproved),
                true,
                Some(error.to_string()),
            )
        }
    };
    let verdict = if confirmed.identity.is_none() || confirmed.identity != reproved.identity {
        SaveVerdict::CardChangedAfterEffect
    } else if confirmed.sequence <= reproved.sequence {
        SaveVerdict::NotConfirmed
    } else if confirmed.state == BookmarkState::Saved {
        SaveVerdict::Saved
    } else {
        SaveVerdict::NotConfirmed
    };
    evidence(verdict, Some(initial), Some(confirmed), true, None)
}

/// Pure hierarchy adapter. The invariant Bookmark label locates the control but contributes no
/// state: only `checked` or `selected` can distinguish the two toggle states.
pub fn hierarchy_save_observation(
    identity: SaveCardIdentity,
    sequence: u64,
    control: Option<StatefulElementBox>,
) -> SaveObservation {
    let Some(control) = control else {
        return SaveObservation {
            identity: Some(identity),
            sequence,
            state: BookmarkState::Unreadable,
            tap_point: None,
        };
    };
    let state = if control.checked == Some(true) || control.selected == Some(true) {
        BookmarkState::Saved
    } else if control.checked == Some(false) || control.selected == Some(false) {
        BookmarkState::Unsaved
    } else {
        BookmarkState::Unreadable
    };
    SaveObservation {
        identity: Some(identity),
        sequence,
        state,
        tap_point: Some(control.element.centre()),
    }
}

/// Pure pixel adapter. Geometry and state are independent proofs: both are required.
pub fn pixel_save_observation(
    identity: SaveCardIdentity,
    sequence: u64,
    rail: Option<ActionRail>,
    screen_size: (f64, f64),
    measured_state: Option<BookmarkState>,
) -> SaveObservation {
    let save_y = rail.and_then(|rail| {
        rail.located
            .then_some(rail)
            .and_then(|rail| rail.save_y.map(|save_y| (rail.x, save_y)))
    });
    SaveObservation {
        identity: Some(identity),
        sequence,
        state: if save_y.is_some() {
            measured_state.unwrap_or(BookmarkState::Unreadable)
        } else {
            BookmarkState::Unreadable
        },
        tap_point: save_y.map(|(x, y)| TapPoint {
            x: x * screen_size.0,
            y: y * screen_size.1,
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use crate::driver::{ElementBox, StatefulElementBox};
    use crate::screen::ActionRail;
    use crate::types::TapPoint;

    struct FixtureAdapter {
        observations: VecDeque<Result<SaveObservation, &'static str>>,
        taps: usize,
        tap_error: Option<&'static str>,
    }

    #[async_trait::async_trait]
    impl SaveAdapter for FixtureAdapter {
        async fn observe(&mut self) -> anyhow::Result<SaveObservation> {
            match self.observations.pop_front().expect("scripted observation") {
                Ok(observation) => Ok(observation),
                Err(message) => anyhow::bail!(message),
            }
        }

        async fn tap(&mut self, _point: TapPoint) -> anyhow::Result<()> {
            self.taps += 1;
            match self.tap_error {
                Some(message) => anyhow::bail!(message),
                None => Ok(()),
            }
        }
    }

    fn identity(name: &str) -> SaveCardIdentity {
        SaveCardIdentity::Hierarchy {
            author: name.to_owned(),
            sound: Some("sound-7".to_owned()),
        }
    }

    fn observation(name: &str, sequence: u64, state: BookmarkState) -> SaveObservation {
        SaveObservation {
            identity: Some(identity(name)),
            sequence,
            state,
            tap_point: Some(TapPoint {
                x: 918.0,
                y: 1460.0,
            }),
        }
    }

    #[test]
    fn bookmark_state_and_save_verdict_have_stable_camel_case_wire_names() {
        let bookmark_states = [
            (BookmarkState::Saved, "saved"),
            (BookmarkState::Unsaved, "unsaved"),
            (BookmarkState::Unreadable, "unreadable"),
        ];
        for (value, wire) in bookmark_states {
            assert_eq!(serde_json::to_value(value).expect("serialize state"), wire);
            assert_eq!(
                serde_json::from_value::<BookmarkState>(wire.into()).expect("deserialize state"),
                value
            );
        }

        let verdicts = [
            (SaveVerdict::Saved, "saved"),
            (SaveVerdict::AlreadySaved, "alreadySaved"),
            (SaveVerdict::NoControl, "noControl"),
            (SaveVerdict::StateUnreadable, "stateUnreadable"),
            (
                SaveVerdict::CardChangedBeforeEffect,
                "cardChangedBeforeEffect",
            ),
            (SaveVerdict::FailedBeforeEffect, "failedBeforeEffect"),
            (
                SaveVerdict::CardChangedAfterEffect,
                "cardChangedAfterEffect",
            ),
            (SaveVerdict::NotConfirmed, "notConfirmed"),
            (SaveVerdict::UncertainAfterEffect, "uncertainAfterEffect"),
        ];
        let typescript = include_str!("../../../apps/desktop/src/types.ts");
        for (value, wire) in verdicts {
            assert_eq!(
                serde_json::to_value(value).expect("serialize verdict"),
                wire
            );
            assert_eq!(
                serde_json::from_value::<SaveVerdict>(wire.into()).expect("deserialize verdict"),
                value
            );
            assert!(
                typescript.contains(&format!("\"{wire}\"")),
                "TypeScript SaveVerdict is missing {wire}"
            );
        }
        for (_, wire) in bookmark_states {
            assert!(
                typescript.contains(&format!("\"{wire}\"")),
                "TypeScript BookmarkState is missing {wire}"
            );
        }
    }

    fn adapter(observations: Vec<Result<SaveObservation, &'static str>>) -> FixtureAdapter {
        FixtureAdapter {
            observations: observations.into(),
            taps: 0,
            tap_error: None,
        }
    }

    fn located_rail(layout: u8) -> ActionRail {
        let follow_y = if layout == 1 {
            223.0 / 667.0
        } else {
            259.0 / 667.0
        };
        ActionRail {
            x: 0.919,
            follow_y,
            like_y: follow_y + 51.0 / 667.0,
            comment_y: follow_y + 113.0 / 667.0,
            save_y: Some(follow_y + 180.0 / 667.0),
            located: true,
        }
    }

    #[tokio::test]
    async fn already_saved_is_a_zero_tap_success() {
        let mut fixture = adapter(vec![Ok(observation("author-a", 1, BookmarkState::Saved))]);
        let mut intents = 0;
        let evidence = tiktok_save(&mut fixture, |_| {
            intents += 1;
            Ok(())
        })
        .await;

        assert_eq!(evidence.verdict, SaveVerdict::AlreadySaved);
        assert_eq!(fixture.taps, 0);
        assert_eq!(intents, 0);
        assert!(!evidence.effect_may_have_gone_out());
    }

    #[tokio::test]
    async fn unsaved_to_saved_crosses_one_boundary_and_taps_once() {
        let mut fixture = adapter(vec![
            Ok(observation("author-a", 1, BookmarkState::Unsaved)),
            Ok(observation("author-a", 2, BookmarkState::Unsaved)),
            Ok(observation("author-a", 3, BookmarkState::Saved)),
        ]);
        let mut intents = 0;
        let evidence = tiktok_save(&mut fixture, |proved| {
            intents += 1;
            assert_eq!(proved.identity.as_ref(), Some(&identity("author-a")));
            assert_eq!(proved.sequence, 2);
            Ok(())
        })
        .await;

        assert_eq!(evidence.verdict, SaveVerdict::Saved);
        assert_eq!(fixture.taps, 1);
        assert_eq!(intents, 1);
        assert!(evidence.effect_boundary_crossed);
        assert!(evidence.effect_may_have_gone_out());
    }

    #[tokio::test]
    async fn no_control_and_unreadable_state_both_refuse_without_a_tap() {
        for (initial, expected) in [
            (
                SaveObservation {
                    identity: Some(identity("author-a")),
                    sequence: 1,
                    state: BookmarkState::Unreadable,
                    tap_point: None,
                },
                SaveVerdict::NoControl,
            ),
            (
                observation("author-a", 1, BookmarkState::Unreadable),
                SaveVerdict::StateUnreadable,
            ),
        ] {
            let mut fixture = adapter(vec![Ok(initial)]);
            let evidence = tiktok_save(&mut fixture, |_| Ok(())).await;
            assert_eq!(evidence.verdict, expected);
            assert_eq!(fixture.taps, 0);
        }
    }

    #[tokio::test]
    async fn same_card_is_reproved_but_a_changed_card_stops_before_effect() {
        let mut same = adapter(vec![
            Ok(observation("author-a", 10, BookmarkState::Unsaved)),
            Ok(observation("author-a", 11, BookmarkState::Unsaved)),
            Ok(observation("author-a", 12, BookmarkState::Saved)),
        ]);
        assert_eq!(
            tiktok_save(&mut same, |_| Ok(())).await.verdict,
            SaveVerdict::Saved
        );
        assert_eq!(same.taps, 1);

        let mut changed = adapter(vec![
            Ok(observation("author-a", 20, BookmarkState::Unsaved)),
            Ok(observation("author-b", 21, BookmarkState::Unsaved)),
        ]);
        let evidence = tiktok_save(&mut changed, |_| panic!("intent must not be recorded")).await;
        assert_eq!(evidence.verdict, SaveVerdict::CardChangedBeforeEffect);
        assert_eq!(changed.taps, 0);
        assert!(!evidence.effect_boundary_crossed);
    }

    #[tokio::test]
    async fn card_change_after_tap_is_distinct_and_uncertain() {
        let mut fixture = adapter(vec![
            Ok(observation("author-a", 1, BookmarkState::Unsaved)),
            Ok(observation("author-a", 2, BookmarkState::Unsaved)),
            Ok(observation("author-b", 3, BookmarkState::Saved)),
        ]);
        let evidence = tiktok_save(&mut fixture, |_| Ok(())).await;

        assert_eq!(evidence.verdict, SaveVerdict::CardChangedAfterEffect);
        assert_eq!(fixture.taps, 1);
        assert!(evidence.effect_may_have_gone_out());
        assert!(!evidence.retry_is_safe());
    }

    #[tokio::test]
    async fn durable_intent_failure_is_before_effect_and_zero_tap() {
        let mut fixture = adapter(vec![
            Ok(observation("author-a", 1, BookmarkState::Unsaved)),
            Ok(observation("author-a", 2, BookmarkState::Unsaved)),
        ]);
        let evidence = tiktok_save(&mut fixture, |_| anyhow::bail!("ledger unavailable")).await;

        assert_eq!(evidence.verdict, SaveVerdict::FailedBeforeEffect);
        assert_eq!(evidence.error.as_deref(), Some("ledger unavailable"));
        assert_eq!(fixture.taps, 0);
        assert!(!evidence.effect_boundary_crossed);
    }

    #[tokio::test]
    async fn observation_transport_failure_before_boundary_is_retryable_and_zero_tap() {
        let mut initial_failed = adapter(vec![Err("initial hierarchy read failed")]);
        let evidence = tiktok_save(&mut initial_failed, |_| Ok(())).await;
        assert_eq!(evidence.verdict, SaveVerdict::FailedBeforeEffect);
        assert_eq!(initial_failed.taps, 0);
        assert!(evidence.retry_is_safe());

        let mut reproof_failed = adapter(vec![
            Ok(observation("author-a", 1, BookmarkState::Unsaved)),
            Err("immediate re-proof failed"),
        ]);
        let evidence = tiktok_save(&mut reproof_failed, |_| Ok(())).await;
        assert_eq!(evidence.verdict, SaveVerdict::FailedBeforeEffect);
        assert_eq!(reproof_failed.taps, 0);
        assert!(!evidence.effect_boundary_crossed);
    }

    #[tokio::test]
    async fn tap_and_readback_transport_failures_are_uncertain_after_boundary() {
        let mut tap_failed = adapter(vec![
            Ok(observation("author-a", 1, BookmarkState::Unsaved)),
            Ok(observation("author-a", 2, BookmarkState::Unsaved)),
        ]);
        tap_failed.tap_error = Some("tap response lost");
        let evidence = tiktok_save(&mut tap_failed, |_| Ok(())).await;
        assert_eq!(evidence.verdict, SaveVerdict::UncertainAfterEffect);
        assert_eq!(evidence.error.as_deref(), Some("tap response lost"));
        assert!(evidence.effect_boundary_crossed);

        let mut readback_failed = adapter(vec![
            Ok(observation("author-a", 1, BookmarkState::Unsaved)),
            Ok(observation("author-a", 2, BookmarkState::Unsaved)),
            Err("hierarchy transport dropped"),
        ]);
        let evidence = tiktok_save(&mut readback_failed, |_| Ok(())).await;
        assert_eq!(evidence.verdict, SaveVerdict::UncertainAfterEffect);
        assert_eq!(
            evidence.error.as_deref(),
            Some("hierarchy transport dropped")
        );
        assert!(evidence.effect_may_have_gone_out());
    }

    #[tokio::test]
    async fn explicit_retry_observes_saved_and_never_repeats_the_tap() {
        let mut fixture = adapter(vec![
            Ok(observation("author-a", 1, BookmarkState::Unsaved)),
            Ok(observation("author-a", 2, BookmarkState::Unsaved)),
        ]);
        fixture.tap_error = Some("tap response lost");
        assert_eq!(
            tiktok_save(&mut fixture, |_| Ok(())).await.verdict,
            SaveVerdict::UncertainAfterEffect
        );
        fixture.tap_error = None;
        fixture
            .observations
            .push_back(Ok(observation("author-a", 3, BookmarkState::Saved)));
        assert_eq!(
            tiktok_save(&mut fixture, |_| panic!("retry must not cross intent"))
                .await
                .verdict,
            SaveVerdict::AlreadySaved
        );
        assert_eq!(fixture.taps, 1);
    }

    #[tokio::test]
    async fn confirmation_must_be_newer_and_still_saved() {
        let mut stale = adapter(vec![
            Ok(observation("author-a", 1, BookmarkState::Unsaved)),
            Ok(observation("author-a", 2, BookmarkState::Unsaved)),
            Ok(observation("author-a", 2, BookmarkState::Saved)),
        ]);
        assert_eq!(
            tiktok_save(&mut stale, |_| Ok(())).await.verdict,
            SaveVerdict::NotConfirmed
        );

        let mut unchanged = adapter(vec![
            Ok(observation("author-a", 1, BookmarkState::Unsaved)),
            Ok(observation("author-a", 2, BookmarkState::Unsaved)),
            Ok(observation("author-a", 3, BookmarkState::Unsaved)),
        ]);
        assert_eq!(
            tiktok_save(&mut unchanged, |_| Ok(())).await.verdict,
            SaveVerdict::NotConfirmed
        );
    }

    fn stateful(checked: Option<bool>, selected: Option<bool>) -> StatefulElementBox {
        StatefulElementBox {
            element: ElementBox {
                x: 900.0,
                y: 1400.0,
                width: 80.0,
                height: 80.0,
                description: Some("Bookmark".to_owned()),
                enabled: true,
                clickable: true,
            },
            checked,
            selected,
        }
    }

    #[test]
    fn hierarchy_adapter_uses_measured_state_not_the_invariant_bookmark_label() {
        let card = identity("author-a");
        let saved = hierarchy_save_observation(card.clone(), 1, Some(stateful(Some(true), None)));
        let unsaved =
            hierarchy_save_observation(card.clone(), 2, Some(stateful(Some(false), None)));
        let selected =
            hierarchy_save_observation(card.clone(), 3, Some(stateful(None, Some(true))));
        let unreadable = hierarchy_save_observation(card, 4, Some(stateful(None, None)));

        assert_eq!(saved.state, BookmarkState::Saved);
        assert_eq!(unsaved.state, BookmarkState::Unsaved);
        assert_eq!(selected.state, BookmarkState::Saved);
        assert_eq!(unreadable.state, BookmarkState::Unreadable);
        assert!(
            unreadable.tap_point.is_some(),
            "control exists but state is unreadable"
        );
    }

    #[test]
    fn hierarchy_and_pixel_adapters_keep_no_control_distinct_from_unreadable() {
        let card = identity("author-a");
        let hierarchy = hierarchy_save_observation(card.clone(), 1, None);
        let pixel = pixel_save_observation(
            card,
            2,
            None,
            (1080.0, 2220.0),
            Some(BookmarkState::Unsaved),
        );
        assert!(hierarchy.tap_point.is_none());
        assert!(pixel.tap_point.is_none());
        assert_eq!(hierarchy.state, BookmarkState::Unreadable);
        assert_eq!(pixel.state, BookmarkState::Unreadable);
    }

    #[test]
    fn pixel_adapter_requires_both_a_located_rail_and_a_measured_state_signal() {
        let card = SaveCardIdentity::Pixel {
            author: "author-a".to_owned(),
            caption: Some("caption-a".to_owned()),
        };
        let fallback = pixel_save_observation(
            card.clone(),
            1,
            Some(ActionRail::fallback()),
            (1080.0, 2220.0),
            Some(BookmarkState::Unsaved),
        );
        assert!(fallback.tap_point.is_none());

        let mut forged_fallback = ActionRail::fallback();
        forged_fallback.save_y = Some(0.65);
        let forged = pixel_save_observation(
            card.clone(),
            2,
            Some(forged_fallback),
            (1080.0, 2220.0),
            Some(BookmarkState::Unsaved),
        );
        assert!(
            forged.tap_point.is_none(),
            "a coordinate without a freshly located rail must not authorize Save"
        );

        let located = located_rail(2);
        let unreadable =
            pixel_save_observation(card.clone(), 3, Some(located), (1080.0, 2220.0), None);
        assert_eq!(unreadable.state, BookmarkState::Unreadable);
        assert!(unreadable.tap_point.is_some());

        let measured = pixel_save_observation(
            card,
            4,
            Some(located),
            (1080.0, 2220.0),
            Some(BookmarkState::Unsaved),
        );
        assert_eq!(measured.state, BookmarkState::Unsaved);
        let point = measured.tap_point.expect("scaled Save point");
        assert!((point.x - 992.52).abs() < 0.01);
        assert!((point.y - (439.0 / 667.0 * 2220.0)).abs() < 0.01);
    }
}
