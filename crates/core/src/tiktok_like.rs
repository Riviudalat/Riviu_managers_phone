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
) -> anyhow::Result<LikeVerdict> {
    if present(session, labels, TikTokControl::Liked).await {
        return Ok(LikeVerdict::AlreadyLiked);
    }
    let Some(element) = locate(session, labels, TikTokControl::Like).await? else {
        return Ok(LikeVerdict::NoControl);
    };
    // Placement is the caller's — a sync closure, so it can hold the touch planner mutably
    // while the session stays borrowed here. The tap itself belongs to this routine, because
    // the proof that follows only means anything if the tap it is proving happened first.
    let point = place(&element);
    session.tap(point).await?;

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
            .await?
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
        ] {
            let reason = verdict.reason();
            assert!(!reason.is_empty());
            assert!(
                !reason.is_ascii(),
                "{verdict:?} should read as Vietnamese, got {reason:?}"
            );
        }
    }
}
