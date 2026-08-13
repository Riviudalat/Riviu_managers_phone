//! Which backend composes a post, and which one can take it down again.
//!
//! Mirrors [`crate::interaction_target`] deliberately: same trait-per-backend shape, same
//! rule that a capability nobody measured **refuses by default** rather than doing nothing
//! quietly. The reason to copy that shape rather than invent one is the same reason
//! `tiktok_like` was extracted — two implementations of "what counts as proof" drift, and
//! here the thing that drifts is whether a public post can be removed.
//!
//! # The asymmetry that shapes this file
//!
//! Composing is recoverable: a composer that stops halfway leaves an unpublished draft, and
//! the worst case is an operator tidying up by hand. Deleting is not. So:
//!
//! * [`PublishDriver::prove_own_post`] and [`PublishDriver::delete_proved_post`] **default
//!   to a refusal**. A backend gets them by implementing them, never by inheriting them.
//!
//! Composing itself is **not on this trait yet**, and deliberately so. The existing pixel
//! composer lives in `publish_commands` and works; moving it here before the hierarchy one
//! exists would be a refactor with no second implementation to justify it, and the plan's
//! own sequence is labels and refusals first. This file is the half that has to exist
//! before any post goes out, because it is the half that decides whether one can come back.
//!
//! And the refusal has to be checked *before* anything is published. A run that posts and
//! then discovers it cannot delete has already broken the promise it was given; see
//! [`PublishDriver::can_remove_its_own_post`].

// Nothing calls this yet, and that is the intended order rather than an oversight: the
// refusals and their tests have to exist before a composer can be allowed to publish
// anything, because the question they answer — "can this backend take the post down
// again?" — must be settled before the first post, not after.
//
// It is now settled, and the answer is no. Four surfaces on trill 46.3.3 were measured and
// none carries a labelled delete control: the own-post page, the sheet behind its `...`, a
// long-press in the profile grid, and the privacy sheet (AGENTS.md 9.43). The caption half of
// the proof chain does work — a caption reads back verbatim well past the threshold the design
// needed (9.40) — so what is missing is only the last tap, and there is no honest way to
// synthesise it. Asked with that measurement in hand, the operator chose to publish nothing
// from Android rather than post and remove by hand, so `publish_commands` refuses an Android
// target outright instead of gating on this trait.
//
// Kept rather than deleted because the decision is about *this* build of TikTok. If a
// labelled delete path appears — a different app version, a different locale, or the
// only-me visibility control in the privacy sheet if the operator ever counts that as
// removal — the shape it has to fit is already here, along with the tests that say what
// counts as proof.
//
// Comes off the moment `publish_commands` gates a post-then-delete campaign on
// `can_remove_its_own_post`. Scoped to the module so real dead code elsewhere still fails
// the build.
#![allow(dead_code)]

use std::fmt;

/// How a post-then-remove attempt ended, named for what was actually proved.
///
/// Copied from `SendFailure`'s two-variant shape, **and the reasoning inverts** — which is
/// the whole point of writing it out rather than reusing that type:
///
/// * For a comment, `AfterEffect` blocks a retry so the comment is not posted twice.
/// * For a delete, `AfterEffect` blocks a retry because the *second* attempt would land on
///   whatever is now the newest post. The first attempt may have removed the right one; a
///   retry would remove a different one.
///
/// Same variant, same "do not retry", opposite argument. That inversion is the strongest
/// reason the delete path insists on proving which post is open before it taps anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteFailure {
    /// Nothing destructive was sent. Safe to try again.
    BeforeEffect(String),
    /// A confirm tap went out and its result was not observed. **Never retried.**
    AfterEffect(String),
}

impl DeleteFailure {
    /// Whether a destructive tap may already have landed.
    pub fn effect_may_have_gone_out(&self) -> bool {
        matches!(self, Self::AfterEffect(_))
    }

    pub fn message(&self) -> &str {
        match self {
            Self::BeforeEffect(message) | Self::AfterEffect(message) => message,
        }
    }
}

impl fmt::Display for DeleteFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BeforeEffect(message) => write!(formatter, "{message}"),
            // Said out loud in the operator's language, because this is the state that
            // needs a human to open the phone and look.
            Self::AfterEffect(message) => write!(
                formatter,
                "{message} — đã gửi cú tap xoá nhưng không xác nhận được; \
                 KHÔNG thử lại tự động, vì lần thử thứ hai sẽ rơi vào bài mới nhất hiện tại"
            ),
        }
    }
}

/// What a backend can do with a post of our own.
///
/// Not object-safe by accident: it is used as `&dyn PublishDriver`, so every method takes
/// `&self` and returns owned data.
pub trait PublishDriver: Send + Sync {
    /// Which reader this is, for evidence rows. `"pixel"` or `"hierarchy"`.
    fn kind(&self) -> &'static str;

    /// Whether this backend can take down a post it just made.
    ///
    /// Checked **at driver selection, before anything is composed**. A backend that answers
    /// `false` is refused up front rather than allowed to post and then fail at the delete
    /// step, because by then the promise is already broken and the only remedy is a human
    /// opening the phone.
    ///
    /// Defaults to `false`. A backend earns `true` by overriding the two delete methods,
    /// and overriding this without them is the mistake `a_driver_that_cannot_delete_is_not_
    /// offered_for_a_post_then_delete_campaign` exists to catch.
    fn can_remove_its_own_post(&self) -> bool {
        false
    }

    /// Find our own post and prove the one on screen is it.
    ///
    /// Defaults to a refusal. The proof is the campaign's own caption read back off the
    /// post page — a string this code wrote — and a backend that cannot read text off the
    /// screen cannot produce it. Guessing from grid position is exactly the ordering
    /// assumption that `importId` exists to eliminate, and being wrong here deletes
    /// somebody else's post.
    fn prove_own_post(&self, caption: &str) -> Result<PostProof, DeleteFailure> {
        let _ = caption;
        Err(DeleteFailure::BeforeEffect(format!(
            "{} không chứng minh được bài trên màn là bài của campaign: \
             đường này không đọc được caption trên trang bài. Không xoá gì cả.",
            self.kind()
        )))
    }

    /// Remove the post that `proof` identifies.
    ///
    /// Defaults to a refusal, and takes a [`PostProof`] rather than a caption so the ordering
    /// is visible in the signature: a caller must hold a proof, and a proof can only come from
    /// [`PostProof::new`], which refuses unless the observations line up. Note the limit —
    /// `new` is `pub`, so this is not proof that a driver produced it; see [`PostProof`].
    fn delete_proved_post(&self, proof: &PostProof) -> Result<(), DeleteFailure> {
        let _ = proof;
        Err(DeleteFailure::BeforeEffect(format!(
            "{} không xoá được bài: nhãn menu/xoá/xác nhận chưa được đo trên build này. \
             Gỡ bài bằng tay.",
            self.kind()
        )))
    }
}

/// Evidence that the post on screen is the one this campaign published.
///
/// Cannot be **forged by struct literal** from outside this module — the fields are private
/// and [`Self::new`] takes the observations rather than a bare "yes", so every proof has run
/// the refusals below.
///
/// Stated precisely, because an earlier version of this comment claimed more than the code
/// gives: `new` is `pub`, so any code in this crate *can* call it without going through a
/// [`PublishDriver::prove_own_post`] implementation. What enforces "prove, then delete" is
/// therefore the checks inside `new`, not the visibility — binding it to the trait would need
/// a sealed token, which is not there. Do not read the type as proof that a driver produced
/// it; read it as proof that the observations passed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostProof {
    caption: String,
    /// How strongly the caption matched: `"exact"`, or `"prefix"` when the page truncated
    /// it. Recorded rather than collapsed, because the two are different confidences and a
    /// stored campaign should say which one it had.
    caption_proof: &'static str,
    /// Whether the on-screen image count matched the bundle. `"match"`, or `"unread"` when
    /// the counter could not be read — which is not a failure, since AGENTS.md 9.20
    /// measured that counter as a transient overlay.
    count_proof: &'static str,
}

impl PostProof {
    /// The only way to make one, and it refuses the cases that must not proceed.
    ///
    /// `follow_control_present` is a hard refusal rather than a downgrade: a `Follow ` label
    /// on the rail means the post belongs to somebody else, and no amount of caption
    /// matching outweighs that.
    pub fn new(
        caption: &str,
        caption_matched_exactly: bool,
        caption_matched_prefix: bool,
        count_matched: Option<bool>,
        follow_control_present: bool,
    ) -> Result<Self, DeleteFailure> {
        if follow_control_present {
            return Err(DeleteFailure::BeforeEffect(
                "trang bài có nút Follow — đây là bài của người khác, không phải của mình. \
                 Không xoá."
                    .to_string(),
            ));
        }
        let caption_proof = if caption_matched_exactly {
            "exact"
        } else if caption_matched_prefix {
            "prefix"
        } else {
            return Err(DeleteFailure::BeforeEffect(format!(
                "caption trên màn không khớp caption của campaign ({} ký tự) — không xoá",
                caption.chars().count()
            )));
        };
        Ok(Self {
            caption: caption.to_string(),
            caption_proof,
            // `None` downgrades, it does not fail: the image counter measured as a
            // transient overlay (AGENTS.md 9.20), so demanding it would refuse most
            // genuine posts.
            count_proof: match count_matched {
                Some(true) => "match",
                Some(false) => {
                    return Err(DeleteFailure::BeforeEffect(
                        "số ảnh trên bài không khớp bundle — không xoá".to_string(),
                    ))
                }
                None => "unread",
            },
        })
    }

    pub fn caption(&self) -> &str {
        &self.caption
    }

    pub fn caption_proof(&self) -> &'static str {
        self.caption_proof
    }

    pub fn count_proof(&self) -> &'static str {
        self.count_proof
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A backend that composes and nothing else — the pixel path's shape.
    struct ComposeOnly;

    impl PublishDriver for ComposeOnly {
        fn kind(&self) -> &'static str {
            "pixel"
        }
    }

    #[test]
    fn a_driver_that_cannot_delete_is_not_offered_for_a_post_then_delete_campaign() {
        // The check that has to happen before anything is published. Posting first and
        // discovering the refusal afterwards leaves a live post on the account with no
        // automated way to remove it, which is the one outcome the operator's rule forbids.
        let driver = ComposeOnly;
        assert!(!driver.can_remove_its_own_post());
    }

    #[test]
    fn the_default_delete_path_refuses_and_says_so_in_the_operator_s_language() {
        let driver = ComposeOnly;
        let refusal = driver
            .prove_own_post("caption của campaign")
            .expect_err("a backend that cannot read the page must not prove anything");
        assert!(!refusal.effect_may_have_gone_out());
        assert!(refusal.message().contains("pixel"));
        assert!(!refusal.message().is_ascii(), "must read as Vietnamese");
    }

    #[test]
    fn a_follow_control_on_the_rail_outweighs_any_caption_match() {
        // Decisive, not a downgrade: the post belongs to somebody else. An exact caption
        // match alongside it means two accounts wrote the same words, which is exactly when
        // a caption-only rule would delete the wrong post.
        let refusal = PostProof::new("caption", true, true, Some(true), true)
            .expect_err("somebody else's post must be refused");
        assert!(!refusal.effect_may_have_gone_out());
        assert!(refusal.message().contains("Follow"));
    }

    #[test]
    fn a_caption_that_does_not_match_refuses_rather_than_downgrading() {
        let refusal = PostProof::new("caption", false, false, Some(true), false)
            .expect_err("no caption match means no proof");
        assert!(refusal.message().contains("không khớp"));
    }

    #[test]
    fn a_truncated_caption_still_proves_but_records_that_it_was_a_prefix() {
        // The page may cut the caption. That is weaker evidence, not absent evidence — but
        // the stored campaign has to say which it was, or a later reader cannot tell how
        // much the delete rested on.
        let proof = PostProof::new("caption dài", false, true, Some(true), false)
            .expect("a prefix match is still a match");
        assert_eq!(proof.caption_proof(), "prefix");
        assert_eq!(proof.count_proof(), "match");
    }

    #[test]
    fn an_unreadable_image_counter_downgrades_but_a_wrong_one_refuses() {
        // Asymmetric on purpose. AGENTS.md 9.20 measured the counter as a transient
        // overlay, so "could not read it" is normal and must not block. "Read it and it
        // disagreed" is a different fact: the post on screen is not the bundle.
        let unread = PostProof::new("caption", true, false, None, false).expect("unread is fine");
        assert_eq!(unread.count_proof(), "unread");

        let mismatched = PostProof::new("caption", true, false, Some(false), false)
            .expect_err("a counter that disagrees is a different post");
        assert!(mismatched.message().contains("số ảnh"));
    }

    #[test]
    fn an_unconfirmed_delete_is_never_retried_and_says_why() {
        // The inversion that shapes this file: for a comment, not retrying avoids a double
        // post; for a delete, not retrying avoids removing whatever is newest *now*.
        let failure = DeleteFailure::AfterEffect("tap xác nhận đã gửi".to_string());
        assert!(failure.effect_may_have_gone_out());
        let rendered = failure.to_string();
        assert!(rendered.contains("KHÔNG thử lại"));
        assert!(rendered.contains("bài mới nhất hiện tại"));
    }

    #[test]
    fn a_refusal_before_any_tap_stays_retryable() {
        let failure = DeleteFailure::BeforeEffect("thiếu nhãn".to_string());
        assert!(!failure.effect_may_have_gone_out());
    }
}
