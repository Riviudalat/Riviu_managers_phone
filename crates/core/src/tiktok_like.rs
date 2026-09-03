//! Liking a TikTok post by hierarchy, and proving it.
//!
//! Extracted rather than copied, for the reason [`crate::tiktok_drawer`] gives about the
//! comment drawer: this contract was **measured**, and two copies of "the liked label
//! appearing is the proof" would drift. Drift here means a run that reports likes it did not
//! land, or refuses ones it did.
//!
//! The nurture feed loop had this as a private method; the Interaction campaign now needs the
//! same thing on a post opened from a link. Same steps, same evidence, one implementation.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::driver::{ElementBox, UiSession};
use crate::nurture::sleep_interruptible;
use crate::tiktok_labels::{TikTokControl, TikTokControls};
use crate::types::TapPoint;
use crate::ActionFailure;

/// How long the like state gets to flip after the tap, and how often to look.
const LIKE_CONFIRM_WINDOW: Duration = Duration::from_millis(2_500);
const LIKE_CONFIRM_POLL: Duration = Duration::from_millis(250);

/// The outcome of one like attempt, named for what was actually proved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LikeVerdict {
    /// The not-liked label went away while the rail stayed — the state flipped.
    Liked,
    /// The measured liked-label was already present before we touched anything.
    AlreadyLiked,
    /// No like control on this card. A LIVE card genuinely has none.
    NoControl,
    /// The tap was delivered and nothing observable changed.
    NotConfirmed,
    /// The already-liked state could not be read, so **nothing was tapped**.
    ///
    /// This is the fail-closed answer to a query error on the pre-tap `Liked` check. It
    /// exists because the alternative — treating an unreadable state as "not liked" and
    /// tapping — is the one mistake this whole module must never make: on the fleet build
    /// the `Like` node is present in both states, so a tap on an already-liked post
    /// **removes** the like. "I could not tell" must not become "so I tapped".
    StateUnreadable,
}

impl LikeVerdict {
    /// Did this attempt leave the post liked?
    ///
    /// `AlreadyLiked` counts as liked — the post carries our like either way — but a caller
    /// keeping statistics should not count it as a *new* one, which is why the variants stay
    /// distinct instead of collapsing to a bool.
    pub fn is_liked(&self) -> bool {
        matches!(self, Self::Liked | Self::AlreadyLiked)
    }

    pub fn reason(&self) -> &'static str {
        match self {
            Self::Liked => "đã thả tim (nhãn đổi trạng thái)",
            Self::AlreadyLiked => "bài này đã tim từ trước",
            Self::NoControl => "thẻ này không có nút tim",
            Self::NotConfirmed => "tap gửi được nhưng nhãn không đổi — không tính là đã tim",
            Self::StateUnreadable => {
                "không đọc được trạng thái đã-tim — KHÔNG tap, vì tap nhầm lên bài đã tim là gỡ mất tim"
            }
        }
    }
}

/// Like the post on screen, and return what could be proved about it.
///
/// `place` is the caller's own tap-placement function, so the shared touch history and this
/// device's hand stay with the caller rather than being re-invented here — see
/// `crate::nurture::touch`.
///
/// The proof is the **label state**, not the tap. Two ways it can show, both measured on an
/// SM-N950F: the `Liked` label appears, or the not-liked label goes while the action rail
/// stays. The second matters because the rail leaving as well means the card changed under
/// us, which is not a like.
pub async fn like_post(
    session: &dyn UiSession,
    labels: TikTokControls,
    // `+ Send` because the whole nurture session runs in a spawned task: without it the
    // returned future is not `Send` and the Tauri command that drives a session will not
    // compile. The same bound is on the report callback for the same reason.
    place: &mut (dyn FnMut(&ElementBox) -> TapPoint + Send),
    stop: &AtomicBool,
) -> Result<LikeVerdict, ActionFailure> {
    like_post_with_gate(session, labels, place, stop, |_| Ok(())).await
}

/// Like with a durable callback at the last instruction before the public-effect tap.
///
/// All state/control reads happen before `durable_intent`. There is no await between a
/// successful callback and [`UiSession::tap`], so a callback error is a before-effect failure
/// with zero tap while process loss after the callback is conservatively reconcilable.
pub async fn like_post_with_gate<F>(
    session: &dyn UiSession,
    labels: TikTokControls,
    place: &mut (dyn FnMut(&ElementBox) -> TapPoint + Send),
    stop: &AtomicBool,
    durable_intent: F,
) -> Result<LikeVerdict, ActionFailure>
where
    F: FnOnce(&ElementBox) -> anyhow::Result<()> + Send,
{
    // **Fail closed on the pre-tap read.** `present` folds a query error to `false`, which
    // here would mean "not liked, go ahead and tap" — and a tap on an already-liked post
    // removes the like. So the already-liked check reads the three states apart: liked
    // (stop, already ours), genuinely not liked (proceed), and unreadable (refuse to tap).
    match locate(session, labels, TikTokControl::Liked).await {
        Ok(Some(_)) => return Ok(LikeVerdict::AlreadyLiked),
        Ok(None) => {}
        Err(_) => return Ok(LikeVerdict::StateUnreadable),
    }
    let Some(element) = locate(session, labels, TikTokControl::Like)
        .await
        .map_err(ActionFailure::before)?
    else {
        return Ok(LikeVerdict::NoControl);
    };
    // Placement is the caller's — a sync closure, so it can hold the touch planner mutably
    // while the session stays borrowed here. The tap itself belongs to this routine, because
    // the proof that follows only means anything if the tap it is proving happened first.
    let point = place(&element);
    durable_intent(&element).map_err(ActionFailure::before)?;
    session.tap(point).await.map_err(ActionFailure::after)?;

    let deadline = Instant::now() + LIKE_CONFIRM_WINDOW;
    while Instant::now() < deadline {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        sleep_interruptible(LIKE_CONFIRM_POLL, stop).await;
        if present(session, labels, TikTokControl::Liked).await {
            return Ok(LikeVerdict::Liked);
        }
        let not_liked_gone = locate(session, labels, TikTokControl::Like)
            .await
            .map_err(ActionFailure::after)?
            .is_none();
        let rail_still_here = present(session, labels, TikTokControl::Comments).await;
        if not_liked_gone && rail_still_here {
            return Ok(LikeVerdict::Liked);
        }
    }
    Ok(LikeVerdict::NotConfirmed)
}

/// Locate a control, or `None` when the label for it was never measured.
///
/// The two `None`s are different and both matter: no measured label means *do not look*,
/// while a measured label that finds nothing means the control is not on this card.
async fn locate(
    session: &dyn UiSession,
    labels: TikTokControls,
    control: TikTokControl,
) -> anyhow::Result<Option<ElementBox>> {
    let Some(label) = labels.label(control) else {
        return Ok(None);
    };
    session.locate(label.to_query()).await
}

async fn present(session: &dyn UiSession, labels: TikTokControls, control: TikTokControl) -> bool {
    matches!(locate(session, labels, control).await, Ok(Some(_)))
}

/// The centre of a rectangle, for a caller with no touch planner of its own.
///
/// Deliberately **not** the default: tapping the exact centre of a control every time is the
/// pattern `crate::nurture::touch` exists to avoid. This is here for a caller that has no
/// planner to hand, and such a caller should say why.
pub fn centre_of(element: &ElementBox) -> TapPoint {
    TapPoint {
        x: element.x + element.width / 2.0,
        y: element.y + element.height / 2.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn already_liked_counts_as_liked_but_is_a_distinct_answer() {
        // A caller keeping counts needs the difference: the post carries our like either
        // way, but only one of these is a like this run performed.
        assert!(LikeVerdict::AlreadyLiked.is_liked());
        assert!(LikeVerdict::Liked.is_liked());
        assert_ne!(LikeVerdict::AlreadyLiked, LikeVerdict::Liked);
    }

    #[test]
    fn an_unproved_or_absent_like_is_not_liked() {
        // `NotConfirmed` is the one that matters: the tap went out and nothing changed, so
        // reporting it as a like would inflate every count that uses this.
        assert!(!LikeVerdict::NotConfirmed.is_liked());
        assert!(!LikeVerdict::NoControl.is_liked());
    }

    #[test]
    fn every_verdict_explains_itself_in_the_operator_s_language() {
        for verdict in [
            LikeVerdict::Liked,
            LikeVerdict::AlreadyLiked,
            LikeVerdict::NoControl,
            LikeVerdict::NotConfirmed,
            LikeVerdict::StateUnreadable,
        ] {
            let reason = verdict.reason();
            assert!(!reason.is_empty());
            assert!(
                !reason.is_ascii(),
                "{verdict:?} should read as Vietnamese, got {reason:?}"
            );
        }
    }

    #[test]
    fn the_unreadable_state_is_not_a_like() {
        assert!(!LikeVerdict::StateUnreadable.is_liked());
    }

    use crate::driver::ElementQuery;
    use crate::tiktok_labels::controls_for;
    use crate::types::SwipeGesture;
    use parking_lot::Mutex as PlMutex;

    /// A phone whose `Liked` query can be made to error, with the `Like` node present in
    /// both states — the exact fleet shape where a mis-read removes a like.
    struct FlakyLikePhone {
        controls: TikTokControls,
        liked_present: bool,
        liked_errors: bool,
        like_errors: bool,
        tap_errors: bool,
        taps: PlMutex<usize>,
    }

    #[async_trait::async_trait]
    impl UiSession for FlakyLikePhone {
        async fn tap(&self, _point: TapPoint) -> anyhow::Result<()> {
            *self.taps.lock() += 1;
            if self.tap_errors {
                anyhow::bail!("agent dropped the tap response");
            }
            Ok(())
        }
        async fn swipe(&self, _gesture: SwipeGesture) -> anyhow::Result<()> {
            Ok(())
        }
        async fn type_text(&self, _text: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn home(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn find_and_tap(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn assert_visible(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
        async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            let liked_q = self
                .controls
                .label(TikTokControl::Liked)
                .map(|l| l.to_query());
            let like_q = self
                .controls
                .label(TikTokControl::Like)
                .map(|l| l.to_query());
            let here = ElementBox {
                description: None,
                enabled: true,
                clickable: true,
                x: 500.0,
                y: 1900.0,
                width: 80.0,
                height: 80.0,
            };
            if Some(query) == liked_q {
                if self.liked_errors {
                    anyhow::bail!("agent không trả lời khi hỏi trạng thái đã-tim");
                }
                return Ok(self.liked_present.then(|| here.clone()));
            }
            if Some(query) == like_q {
                if self.like_errors {
                    anyhow::bail!("agent failed before returning the Like control");
                }
                // Present in BOTH states on this build — the whole reason a mis-read is
                // dangerous.
                return Ok(Some(here));
            }
            Ok(None)
        }
    }

    fn fleet_controls() -> TikTokControls {
        controls_for("com.ss.android.ugc.trill", "en", "38.3.2")
            .expect("the fleet build is catalogued")
    }

    /// **A query error on the already-liked check must NOT tap.** Reading it as "not liked"
    /// and tapping removes a real like on this build — the one mistake the module exists to
    /// avoid. Measured shape: `Like` present in both states.
    #[tokio::test]
    async fn an_unreadable_liked_state_refuses_to_tap() {
        let phone = FlakyLikePhone {
            controls: fleet_controls(),
            liked_present: true,
            liked_errors: true,
            like_errors: false,
            tap_errors: false,
            taps: PlMutex::new(0),
        };
        let stop = AtomicBool::new(false);
        let verdict = like_post(&phone, fleet_controls(), &mut centre_of, &stop)
            .await
            .expect("no transport error out of like_post");
        assert_eq!(verdict, LikeVerdict::StateUnreadable);
        assert_eq!(
            *phone.taps.lock(),
            0,
            "an unreadable like state was tapped anyway"
        );
    }

    /// The other direction stays intact: a genuinely not-liked post is tapped once.
    #[tokio::test]
    async fn a_genuinely_unliked_post_is_tapped() {
        let phone = FlakyLikePhone {
            controls: fleet_controls(),
            liked_present: false,
            liked_errors: false,
            like_errors: false,
            tap_errors: false,
            taps: PlMutex::new(0),
        };
        let stop = AtomicBool::new(false);
        let _ = like_post(&phone, fleet_controls(), &mut centre_of, &stop).await;
        assert_eq!(
            *phone.taps.lock(),
            1,
            "a not-liked post should be tapped exactly once"
        );
    }

    #[tokio::test]
    async fn a_like_control_read_failure_is_before_effect() {
        let phone = FlakyLikePhone {
            controls: fleet_controls(),
            liked_present: false,
            liked_errors: false,
            like_errors: true,
            tap_errors: false,
            taps: PlMutex::new(0),
        };

        let failure = like_post(
            &phone,
            fleet_controls(),
            &mut centre_of,
            &AtomicBool::new(false),
        )
        .await
        .expect_err("the scripted locate must fail");

        assert!(!failure.effect_may_have_gone_out());
        assert_eq!(*phone.taps.lock(), 0);
    }

    #[tokio::test]
    async fn an_ambiguous_like_tap_failure_is_after_effect() {
        let phone = FlakyLikePhone {
            controls: fleet_controls(),
            liked_present: false,
            liked_errors: false,
            like_errors: false,
            tap_errors: true,
            taps: PlMutex::new(0),
        };

        let failure = like_post(
            &phone,
            fleet_controls(),
            &mut centre_of,
            &AtomicBool::new(false),
        )
        .await
        .expect_err("the scripted tap response must fail");

        assert!(failure.effect_may_have_gone_out());
        assert_eq!(*phone.taps.lock(), 1);
    }

    #[tokio::test]
    async fn durable_intent_is_written_immediately_before_the_like_tap() {
        let phone = FlakyLikePhone {
            controls: fleet_controls(),
            liked_present: false,
            liked_errors: false,
            like_errors: false,
            tap_errors: false,
            taps: PlMutex::new(0),
        };
        let boundary_taps = &phone.taps;
        let mut callbacks = 0;

        let _ = like_post_with_gate(
            &phone,
            fleet_controls(),
            &mut centre_of,
            &AtomicBool::new(false),
            |_| {
                callbacks += 1;
                assert_eq!(
                    *boundary_taps.lock(),
                    0,
                    "the callback must be the final instruction before tap"
                );
                Ok(())
            },
        )
        .await;

        assert_eq!(callbacks, 1);
        assert_eq!(*phone.taps.lock(), 1);
    }

    #[tokio::test]
    async fn like_intent_write_failure_is_before_effect_and_zero_tap() {
        let phone = FlakyLikePhone {
            controls: fleet_controls(),
            liked_present: false,
            liked_errors: false,
            like_errors: false,
            tap_errors: false,
            taps: PlMutex::new(0),
        };

        let failure = like_post_with_gate(
            &phone,
            fleet_controls(),
            &mut centre_of,
            &AtomicBool::new(false),
            |_| anyhow::bail!("interaction action ledger unavailable"),
        )
        .await
        .expect_err("the intent callback must fail closed");

        assert!(!failure.effect_may_have_gone_out());
        assert_eq!(*phone.taps.lock(), 0);
    }

    #[tokio::test]
    async fn already_liked_never_arms_an_effect_intent() {
        let phone = FlakyLikePhone {
            controls: fleet_controls(),
            liked_present: true,
            liked_errors: false,
            like_errors: false,
            tap_errors: false,
            taps: PlMutex::new(0),
        };

        let verdict = like_post_with_gate(
            &phone,
            fleet_controls(),
            &mut centre_of,
            &AtomicBool::new(false),
            |_| panic!("an already-liked card must not arm a tap"),
        )
        .await
        .expect("already liked");

        assert_eq!(verdict, LikeVerdict::AlreadyLiked);
        assert_eq!(*phone.taps.lock(), 0);
    }
}
