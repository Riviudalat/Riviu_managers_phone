//! The two ways an Interaction campaign can drive a device, behind one interface.
//!
//! An Interaction assignment does three things to a phone: open the target post, send a
//! root comment, or send a reply under a known parent. Every one of them is done
//! differently depending on whether the device can report where its controls are — and
//! the *rest* of the campaign is identical either way.
//!
//! **Why a trait and not a branch inside the two send functions.** The branch is not two
//! functions, it is six: `open_target_confirmed`, `open_comment_for_ocr`,
//! `scroll_to_parent`, `stable_parent_match`, the send itself, and `discover_after_send`.
//! Branching only inside the sends would leave the other four pixel-only, and
//! `open_target_confirmed` alone kills an Android run before any send is reached.
//!
//! **What deliberately stays outside.** Every database write, the `effect_intent` flag
//! and the `Uncertain`-versus-`Failed` distinction it decides, the evidence artifact, the
//! identity map, `chain_broken_at`, the per-message cancellation check, and retry
//! scoping. One loop, one set of writes, two implementations of three methods. Those
//! rules were expensive to get right and they are not platform-specific.
//!
//! The choice is made **once per assignment**, from
//! `UiSession::supports_element_bounds()` — the runtime authority, asked of the session
//! actually in hand rather than predicted from a platform name.

//!
//! Lives in `riviu-core` beside the campaign that uses it. It began in the desktop crate
//! only because the campaign did, and neither has ever needed anything from Tauri.

use std::sync::atomic::AtomicBool;

use crate::{
    CommentLocatorIdentity, PreparedThreadMessage, ResolvedTikTokTarget, ThreadSendEvidence,
    UiSession,
};

use crate::interaction_campaign::TargetProof;

/// One-shot durable boundary for Like/Save effects.
pub(crate) struct ActionEffectGate<'a> {
    callback: Option<Box<dyn FnOnce() -> anyhow::Result<bool> + Send + 'a>>,
    crossed: bool,
}

impl<'a> ActionEffectGate<'a> {
    pub fn new(callback: impl FnOnce() -> anyhow::Result<bool> + Send + 'a) -> Self {
        Self {
            callback: Some(Box::new(callback)),
            crossed: false,
        }
    }

    pub fn cross(&mut self) -> Result<(), crate::ActionFailure> {
        let callback = self.callback.take().ok_or_else(|| {
            crate::ActionFailure::before(anyhow::anyhow!("action effect gate already consumed"))
        })?;
        if !callback().map_err(crate::ActionFailure::before)? {
            return Err(crate::ActionFailure::before(anyhow::anyhow!(
                "action ownership was lost before effect"
            )));
        }
        self.crossed = true;
        Ok(())
    }

    pub fn crossed(&self) -> bool {
        self.crossed
    }
}

/// What one send produced: the evidence row, and the posted comment if it could be read
/// back.
///
/// `identity` is `None` when the comment could not be found unambiguously afterwards.
/// That breaks the reply chain — deliberately, because the alternative is replying under
/// a row nobody confirmed is ours.
pub(crate) struct SendOutcome {
    pub evidence: ThreadSendEvidence,
    pub identity: Option<CommentLocatorIdentity>,
    /// What became of the `@` tags, when the campaign asked for any.
    ///
    /// Carried the same way the like note is, and for the same reason: a tag that TikTok
    /// never offered goes out as plain text, the account is not notified, and nothing else on
    /// screen would tell the operator that happened.
    pub mention_note: Option<String>,
    /// Whether this reply was attached under a **folded** comment.
    ///
    /// Travels with the send for the same reason `mention_note` does, and it was being lost:
    /// the hierarchy send set it, but `SendOutcome` had no slot and `finish` dropped it, so a
    /// reply posted and confirmed under TikTok's folded section — one nobody will ever see —
    /// was recorded as an ordinary success. "It went out" and "it is invisible" have to
    /// arrive together, or the operator cannot tell the second from a normal reply. Always
    /// `false` on the pixel/root paths, which have no folded concept.
    pub parent_was_folded: bool,
}

/// A send that did not succeed, and **which side of the effect line it fell on**.
///
/// This distinction is the whole reason the trait returns a typed error instead of a
/// bare `anyhow::Error`, and it was learned the hard way: the first version of this
/// refactor moved the parent hunt (open the drawer, scroll, match the row) from *above*
/// the caller's `effect_intent = true` to *inside* `send_reply`, which is below it. Every
/// one of those steps can fail with nothing typed and nothing tapped — the parent simply
/// is not in the list, because each reply is sent from a different device and TikTok
/// re-ranks the comments. Recording that as `Uncertain` makes it **permanently
/// unretryable** (`retryable_assignments` excludes `Uncertain`), so a message that was
/// never posted can never be sent.
///
/// So the driver, which is the only thing that knows what it did, says so.
pub(crate) enum SendFailure {
    /// Nothing was typed and no Send tap went out. The assignment stays retryable.
    BeforeEffect(anyhow::Error),
    /// A Send tap went out and its result could not be confirmed. **Not** retryable —
    /// a retry is how a post ends up with two identical comments on it.
    AfterEffect(anyhow::Error),
    /// This worker no longer owns the assignment. The winning worker owns the row, so the
    /// loser must leave without changing it and without tapping Send.
    OwnershipLost(anyhow::Error),
}

impl SendFailure {
    pub fn effect_may_have_gone_out(&self) -> bool {
        matches!(self, Self::AfterEffect(_))
    }

    pub fn ownership_lost(&self) -> bool {
        matches!(self, Self::OwnershipLost(_))
    }

    pub fn into_error(self) -> anyhow::Error {
        match self {
            Self::BeforeEffect(error) | Self::AfterEffect(error) | Self::OwnershipLost(error) => {
                error
            }
        }
    }

    pub fn detail(&self) -> String {
        match self {
            Self::BeforeEffect(error) | Self::AfterEffect(error) | Self::OwnershipLost(error) => {
                format!("{error:#}")
            }
        }
    }

    pub fn before(error: impl Into<anyhow::Error>) -> Self {
        Self::BeforeEffect(error.into())
    }

    pub fn after(error: impl Into<anyhow::Error>) -> Self {
        Self::AfterEffect(error.into())
    }
}

/// A one-shot permit placed at the last instruction before a non-idempotent Send tap.
///
/// The callback normally performs the assignment's `preparing -> sending` CAS. Keeping it
/// here, instead of in the campaign before entering a driver, leaves drawer setup, parent
/// lookup, typing, and Send arming on the retryable side of the persisted effect boundary.
pub(crate) struct EffectGate<'a> {
    callback: Option<Box<dyn FnOnce() -> anyhow::Result<bool> + Send + 'a>>,
    crossed: bool,
}

impl<'a> EffectGate<'a> {
    pub fn new(callback: impl FnOnce() -> anyhow::Result<bool> + Send + 'a) -> Self {
        Self {
            callback: Some(Box::new(callback)),
            crossed: false,
        }
    }

    pub fn allow() -> Self {
        Self::new(|| Ok(true))
    }

    pub fn cross(&mut self) -> Result<(), SendFailure> {
        let callback = self.callback.take().ok_or_else(|| {
            SendFailure::lost_ownership(anyhow::anyhow!("effect gate was already consumed"))
        })?;
        match callback().map_err(SendFailure::before)? {
            true => {
                self.crossed = true;
                Ok(())
            }
            false => Err(SendFailure::lost_ownership(anyhow::anyhow!(
                "assignment ownership was lost before Send"
            ))),
        }
    }

    pub fn crossed(&self) -> bool {
        self.crossed
    }
}

impl SendFailure {
    pub fn lost_ownership(error: impl Into<anyhow::Error>) -> Self {
        Self::OwnershipLost(error.into())
    }
}

/// The three device-specific steps of an Interaction assignment.
#[async_trait::async_trait]
pub(crate) trait TargetDriver: Send + Sync {
    /// A short name for logs and refusal messages.
    fn kind(&self) -> &'static str;

    /// Remove a composer left armed by a process that died before its effect gate.
    ///
    /// A successful cleanup returns to the feed. The campaign therefore calls this before
    /// `open_target`, which recreates the target and reply-parent proof from fresh state.
    async fn clear_stale_comment_ui(
        &self,
        session: &dyn UiSession,
        stop: &AtomicBool,
    ) -> Result<(), SendFailure>;

    /// Open the target post and prove the device landed on *that* post.
    ///
    /// Must not type or tap anything: an unproved open followed by a send posts the
    /// campaign's comment to whatever the phone happened to be showing.
    async fn open_target(
        &self,
        session: &dyn UiSession,
        target: &ResolvedTikTokTarget,
    ) -> anyhow::Result<TargetProof>;

    /// Send a root comment and read it back.
    ///
    /// **Leaves the comment drawer open.** The caller publishes an evidence frame after
    /// this returns and that frame has to show the comment in the list.
    async fn send_root(
        &self,
        session: &dyn UiSession,
        prepared: &PreparedThreadMessage,
        stop: &AtomicBool,
        effect_gate: &mut EffectGate<'_>,
    ) -> Result<SendOutcome, SendFailure>;

    /// Like the post that is already open, if this backend can.
    ///
    /// Defaults to a refusal rather than to doing nothing, because "the operator asked for a
    /// like and none happened" has to be visible. The hierarchy driver overrides it; the
    /// pixel driver does not, since a like on a post page there would need calibrated
    /// coordinates nobody has measured — and inventing them is what `screen.rs` refuses to
    /// do for uncalibrated screens.
    async fn like_target(
        &self,
        _session: &dyn UiSession,
        _effect_gate: &mut ActionEffectGate<'_>,
    ) -> Result<crate::tiktok_like::LikeVerdict, crate::ActionFailure> {
        Err(crate::ActionFailure::before(anyhow::anyhow!(
            "{} không thả tim được: đường nhận dạng ảnh chưa đo toạ độ nút tim trên trang bài.              Bỏ chọn Thả tim, hoặc dùng máy Android",
            self.kind()
        )))
    }

    async fn save_target(
        &self,
        _session: &dyn UiSession,
        effect_gate: &mut ActionEffectGate<'_>,
    ) -> crate::SaveEvidence {
        struct UnmeasuredPixelSave;
        #[async_trait::async_trait]
        impl crate::SaveAdapter for UnmeasuredPixelSave {
            async fn observe(&mut self) -> anyhow::Result<crate::SaveObservation> {
                Ok(crate::SaveObservation {
                    identity: None,
                    sequence: 1,
                    state: crate::BookmarkState::Unreadable,
                    tap_point: None,
                })
            }
            async fn tap(&mut self, _point: crate::TapPoint) -> anyhow::Result<()> {
                anyhow::bail!("unmeasured pixel Save cannot tap")
            }
        }
        crate::tiktok_save(&mut UnmeasuredPixelSave, |_| {
            effect_gate
                .cross()
                .map_err(|error| anyhow::anyhow!(error.to_string()))
        })
        .await
    }

    /// Send a reply under `parent`.
    ///
    /// Implementations must classify their failures honestly — see [`SendFailure`]. The
    /// steps that locate the parent all happen before anything is typed, and reporting
    /// them as `AfterEffect` blocks retry for a message that was never posted.
    async fn send_reply(
        &self,
        session: &dyn UiSession,
        parent: &CommentLocatorIdentity,
        prepared: &PreparedThreadMessage,
        stop: &AtomicBool,
        effect_gate: &mut EffectGate<'_>,
    ) -> Result<SendOutcome, SendFailure>;
}
