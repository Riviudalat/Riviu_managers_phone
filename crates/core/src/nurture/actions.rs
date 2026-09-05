//! The individual feed gestures, each confirmed against the frame stream.
//!
//! Nothing here reports success it has not seen: a like counts once the heart
//! turns red in a later frame, a follow once the badge disappears, a swipe once
//! the action rail has left the screen and a new card has settled, a comment
//! once the Send button was observed armed.

use std::sync::atomic::AtomicBool;
use std::time::Duration;

use anyhow::anyhow;
use chrono::Utc;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use uuid::Uuid;

use crate::driver::UiSession;
use crate::human_behavior::pick_direction_seeded;
use crate::interaction::{CommentOcrObservation, PreparedThreadMessage, ThreadSendEvidence};
use crate::interaction_target::SendFailure;
#[cfg(test)]
use crate::openai_client::pick_from_pool;
use crate::openai_client::{
    host_of, ocr_caption, prepare_caption_comment, prepare_grounded_comment,
    provider_supports_vision, EvidenceKind,
};
use crate::screen::{self, ActionRail, CommentDrawer};
use crate::types::{NurtureCommentAttempt, NurtureSettings, SwipeGesture, TapPoint};

use super::{frame_digest, sleep_interruptible, NurtureEngine, SWIPE_SETTLE};

/// Which route produced (or would have produced) this comment's context. Used
/// as the `source` on every attempt row, including skips, so the audit shows
/// which path gave up rather than attributing every skip to vision.
fn context_source(settings: &NurtureSettings) -> &'static str {
    if provider_supports_vision(settings) {
        "grounded-vision"
    } else {
        "ocr-caption"
    }
}

/// Where a feed swipe starts and ends, as fractions of screen height.
///
/// The start used to be 0.75, and on a photo post that is the row of page dots
/// and the top of the caption block — TikTok reads a drag beginning there as an
/// interaction with the card, not a scroll, and swallows it whole. Measured on
/// one such card, four swipes from 0.75 moved nothing at all (the frame did not
/// change by a single byte), while three from 0.62 advanced the feed twice and
/// left the card the third time. A live run sat on one photo post for 280
/// seconds sending swipes that could never work.
///
/// 0.62 clears the caption on every card seen so far and still gives a drag of
/// ~0.42 of the screen, close to the 0.50 it replaces.
const SWIPE_FROM_Y: f64 = 0.62;
const SWIPE_TO_Y: f64 = 0.20;

/// Time for the comment drawer to finish opening and load its first screenful.
///
/// The drawer animates up and *then* fetches, so the window is not the animation — it is the
/// network round trip behind it on a fleet of budget phones. Reading earlier finds an empty
/// list and concludes the post has no comments.
const COMMENT_DRAWER_SETTLE: Duration = Duration::from_millis(3_500);

/// Time for the keyboard to come up and the input field to take focus after the tap.
///
/// Shorter than the drawer because nothing is fetched — it is one IME animation. Typing into
/// a field that has not focused yet sends the text nowhere, silently.
const COMMENT_INPUT_SETTLE: Duration = Duration::from_millis(1_200);

/// What a swipe could be *proven* to have done.
///
/// The three variants exist because they authorise different next moves, and
/// collapsing them to a bool loses exactly the distinction that matters: a
/// gesture the feed swallowed is safe to repeat, one that moved the feed is
/// not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SwipeOutcome {
    /// The rail left the screen and a settled feed card came back. The only
    /// outcome counted as a video watched.
    Advanced,
    /// The rail left but nothing settled inside the window — still animating,
    /// covered by an overlay, or landed on a card that has no rail at all
    /// (LIVE preview). The gesture *was* taken, so repeating it would skip a
    /// card; the caller re-reads the screen instead.
    Moved,
    /// The rail never left. Frames kept arriving — a playing video guarantees
    /// that — so this is a gesture the feed swallowed, and repeating it is
    /// both safe and the right move.
    Blocked,
}

/// What happened to a like attempt. "Already liked" is a normal outcome and
/// must not be reported as a failure — conflating the two is what made the old
/// logs unreadable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum LikeResult {
    Liked,
    AlreadyLiked,
    /// Nothing was tapped: the current frame is not an actionable feed card
    /// with a locatable rail, so there is no heart to aim at.
    NotOnFeed,
    /// Tapped, and the heart never reddened. Carries the redness before the tap
    /// and the highest seen after it — without those two numbers a miss is
    /// indistinguishable from a heart that reddened too slowly to catch.
    NotConfirmed {
        before: f64,
        best: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FollowResult {
    /// Pixel OCR cannot prove an exact canonical author-profile hierarchy identity.
    SourceUnavailable,
}

/// Where a text-comment attempt stopped. A transport ACK from `/wda/keys` is not
/// success: TikTok may accept the request without putting anything in the field,
/// so arming and disarming Send remain separate, frame-confirmed outcomes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum CommentResult {
    /// Text was posted, with what the API actually reported spending on it.
    ///
    /// Tokens, not a price. The USD this used to carry was two hand-typed numbers multiplied
    /// by these very counts — three different pairs of them existed in the codebase at once,
    /// none matching the configured model, and no UI could edit them. A number the app
    /// cannot know is worse than no number.
    TextSent {
        prompt_tokens: u32,
        completion_tokens: u32,
    },
    /// The active session cannot inject trusted text into TikTok.
    TextChannelUnavailable,
    /// Contextual preparation was rejected before any drawer gesture.
    ContextSkipped,
    /// The write-ahead audit row could not be persisted, so no UI was touched.
    AuditUnavailable,
    /// The card used to prepare the text was no longer under the gesture.
    CardChanged,
    /// The comment icon did not open the drawer.
    NoDrawer,
    /// The drawer already contained text before this attempt started.
    ExistingDraft,
    /// Typing returned OK but the Send button never armed.
    TextNotArmed,
    /// Send was tapped but never returned to its unarmed state.
    TextNotSent,
}

impl CommentResult {
    pub(super) fn reason(&self) -> &'static str {
        match self {
            CommentResult::TextSent { .. } => "đã gửi bình luận chữ",
            CommentResult::TextChannelUnavailable => {
                "Riviu Agent chưa sẵn sàng cho bình luận chữ — chạy Agent Repair"
            }
            CommentResult::ContextSkipped => "bỏ qua: AI không xác nhận được comment bám nội dung",
            CommentResult::AuditUnavailable => "không ghi được audit trước hành động",
            CommentResult::CardChanged => "thẻ đã đổi giữa lúc soạn và lúc gõ",
            CommentResult::NoDrawer => "không mở được khay bình luận",
            CommentResult::ExistingDraft => "khay bình luận đang có bản nháp cũ",
            CommentResult::TextNotArmed => "đã gõ bình luận chữ nhưng nút gửi không sáng",
            CommentResult::TextNotSent => "đã bấm gửi bình luận chữ nhưng nút không tắt",
        }
    }

    pub(super) fn did_act(self) -> bool {
        matches!(
            self,
            Self::TextSent { .. }
                | Self::NoDrawer
                | Self::ExistingDraft
                | Self::TextNotArmed
                | Self::TextNotSent
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PixelCardIdentity {
    author: String,
    caption: Option<String>,
}

impl PixelCardIdentity {
    fn same_card(&self, other: &Self) -> bool {
        // `None` is an observed OCR miss, not a wildcard. Treating it as one lets a first
        // sample with no caption bless later, conflicting captions from adjacent cards by the
        // same author. Exact `Option` equality is conservative: when the evidence gained or
        // lost a caption we cannot prove it stayed on one card, so no public action follows.
        self.author == other.author && self.caption == other.caption
    }
}

struct CommentEvidence {
    frames: Vec<Vec<u8>>,
    identity: PixelCardIdentity,
}

fn normalize_card_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Find the lower-left metadata cluster and use its first line as the author.
///
/// Canonical captures put the author and caption near the bottom of the card, but videos can
/// also paint large headline text immediately above them. A fixed author row therefore either
/// misses real metadata or mistakes the headline for the account. Grouping vertically adjacent,
/// left-aligned OCR lines keeps the account/caption block together and leaves the inset overlay
/// in its own cluster. Without a credible author line callers fail closed instead of substituting
/// a whole-frame digest.
fn pixel_card_identity_from_observations(
    observations: &[CommentOcrObservation],
) -> Option<PixelCardIdentity> {
    #[derive(Clone)]
    struct MetadataLine {
        y: f64,
        x: f64,
        width: f64,
        text: String,
    }

    let mut lines = observations
        .iter()
        .filter(|observation| {
            observation.confidence >= 0.6
                && (0.68..=0.91).contains(&observation.y)
                && observation.x <= 0.70
                && observation.width >= 0.04
        })
        .filter_map(|observation| {
            let text = normalize_card_text(&observation.text);
            let sensible = (2..=64).contains(&text.chars().count())
                && text.chars().any(char::is_alphabetic)
                && !matches!(
                    text.as_str(),
                    "follow"
                        | "đã follow"
                        | "like"
                        | "comments"
                        | "share"
                        | "trang chủ"
                        | "cửa hàng"
                        | "hộp thư"
                        | "hồ sơ"
                );
            sensible.then_some(MetadataLine {
                y: observation.y,
                x: observation.x,
                width: observation.width,
                text,
            })
        })
        .collect::<Vec<_>>();
    lines.sort_by(|left, right| {
        left.y
            .total_cmp(&right.y)
            .then_with(|| left.x.total_cmp(&right.x))
    });

    let mut clusters: Vec<Vec<MetadataLine>> = Vec::new();
    for line in lines {
        let joins_last = clusters.last().is_some_and(|cluster| {
            let first = &cluster[0];
            let previous = cluster.last().expect("metadata cluster is nonempty");
            line.y - previous.y <= 0.065 && (line.x - first.x).abs() <= 0.065
        });
        if joins_last {
            clusters
                .last_mut()
                .expect("metadata cluster is nonempty")
                .push(line);
        } else {
            clusters.push(vec![line]);
        }
    }

    // An isolated line below the author row is caption/overlay evidence, not an account
    // identity. Require the canonical author-plus-caption block; otherwise a shared caption
    // such as "#fyp" can impersonate an author on two adjacent cards. Within the author range,
    // the bottommost multi-line cluster wins: headline overlays are above it.
    let cluster = clusters
        .into_iter()
        .filter(|cluster| {
            let first = &cluster[0];
            if cluster.len() < 2 || !(0.70..=0.85).contains(&first.y) || first.x > 0.30 {
                return false;
            }

            let author_chars = first.text.chars().count();
            let handle = first.text.strip_prefix('@').is_some_and(|handle| {
                (2..=32).contains(&handle.chars().count())
                    && handle
                        .chars()
                        .all(|ch| ch.is_alphanumeric() || matches!(ch, '.' | '_' | '-'))
            });
            let widest_caption = cluster
                .iter()
                .skip(1)
                .map(|line| line.width)
                .fold(0.0_f64, f64::max);
            let display_name = (2..=32).contains(&author_chars)
                && first.text.split_whitespace().count() <= 4
                && !first
                    .text
                    .chars()
                    .last()
                    .is_some_and(|ch| matches!(ch, '.' | ',' | '!' | '?' | ':' | ';'))
                && first.width <= widest_caption * 0.85;

            handle || display_name
        })
        .max_by(|left, right| left[0].y.total_cmp(&right[0].y))?;
    let author = cluster[0].text.clone();
    let mut caption_lines = Vec::new();
    for line in cluster.into_iter().skip(1) {
        if caption_lines.iter().all(|known| known != &line.text) {
            caption_lines.push(line.text);
        }
    }
    let caption = (!caption_lines.is_empty()).then(|| caption_lines.join(" "));
    Some(PixelCardIdentity { author, caption })
}

struct PreparedTextComment {
    text: String,
    model: String,
    base_url_host: String,
    prompt_tokens: u32,
    completion_tokens: u32,
    /// What the gateway said this cost. `None` when it did not say, never a locally made guess.
    cost_usd: Option<f64>,
    source: &'static str,
    frame_sha256: Option<String>,
    caption_preview: Option<String>,
    context_confidence: Option<u8>,
    relevance: Option<u8>,
    evidence_support: Option<u8>,
    distinct_frames: Option<u8>,
    grounded_on: Option<PixelCardIdentity>,
    attempt_id: String,
}

fn classify_pixel_gate_cleanup(failure: SendFailure, cleaned: bool, surface: &str) -> SendFailure {
    if cleaned || failure.ownership_lost() {
        failure
    } else {
        SendFailure::after(anyhow!(
            "effect gate failed after typing and {surface} UI cleanup was not verified: {}",
            failure.into_error()
        ))
    }
}

impl NurtureEngine {
    /// Clear a draft left armed by a process that died before its effect gate.
    ///
    /// This runs before the target is opened again. Returning to a verified feed is
    /// intentional: any locator captured before cleanup is invalid, so the campaign must
    /// reopen the post and re-prove a reply parent before it can type.
    pub(crate) async fn clear_stale_pixel_comment_ui(
        &self,
        udid: &str,
        session: &dyn UiSession,
        gestures: &tokio::sync::Mutex<()>,
        stop: &AtomicBool,
    ) -> Result<(), SendFailure> {
        let Some(frame) = self.frames.latest(udid) else {
            return Ok(());
        };
        let image = image::load_from_memory(&frame)
            .map_err(|_| SendFailure::before(anyhow!("stale_comment_frame_decode_failed")))?
            .to_rgb8();
        if screen::comment_drawer_state(&image).0 != CommentDrawer::SendArmed {
            return Ok(());
        }

        let screen_size = crate::screen::measured_screen_size(session)
            .await
            .map_err(SendFailure::before)?;
        let cleaned = self
            .close_comment_ui(udid, session, gestures, screen_size, stop)
            .await;
        let verified = cleaned
            && self
                .frames
                .latest(udid)
                .and_then(|frame| image::load_from_memory(&frame).ok())
                .map(|image| {
                    let image = image.to_rgb8();
                    screen::feed_ready(&image, Some(screen_size.0))
                        && screen::comment_drawer_state(&image).0 != CommentDrawer::SendArmed
                })
                .unwrap_or(false);
        if verified {
            Ok(())
        } else {
            Err(SendFailure::after(anyhow!(
                "crash-stale pixel comment composer cleanup was not verified"
            )))
        }
    }

    /// Send one already-prepared campaign message. The caller must persist the
    /// prepared text/hash before invoking this method. This deliberately keeps
    /// the same frame-confirmed drawer contract as nurture comments, but does
    /// not call an AI provider while the composer is open.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn send_prepared_thread_comment(
        &self,
        udid: &str,
        session: &dyn UiSession,
        gestures: &tokio::sync::Mutex<()>,
        prepared: &PreparedThreadMessage,
        stop: &AtomicBool,
    ) -> Result<ThreadSendEvidence, SendFailure> {
        let mut effect_gate = crate::interaction_target::EffectGate::allow();
        self.send_prepared_thread_comment_with_gate(
            udid,
            session,
            gestures,
            prepared,
            stop,
            &mut effect_gate,
        )
        .await
    }

    pub(crate) async fn send_prepared_thread_comment_with_gate(
        &self,
        udid: &str,
        session: &dyn UiSession,
        gestures: &tokio::sync::Mutex<()>,
        prepared: &PreparedThreadMessage,
        stop: &AtomicBool,
        effect_gate: &mut crate::interaction_target::EffectGate<'_>,
    ) -> Result<ThreadSendEvidence, SendFailure> {
        if !session.supports_text_input() {
            return Err(SendFailure::before(anyhow!("text_channel_unavailable")));
        }
        let Some(open_bytes) = self.frames.latest(udid) else {
            return Err(SendFailure::before(anyhow!("frame_unavailable")));
        };
        let open_image = image::load_from_memory(&open_bytes)
            .map_err(|_| SendFailure::before(anyhow!("frame_decode_failed")))?
            .to_rgb8();
        // Refuse rather than fabricate: this path taps composer and Send, so a wrong
        // multiplier publishes a comment somewhere nobody chose.
        let screen_size = crate::screen::measured_screen_size(session)
            .await
            .map_err(SendFailure::before)?;
        // Locate the rail per frame (handles already-followed cards where the
        // red badge is hidden). Fail the attempt rather than tapping the
        // layout-2 fallback constants blind — on a layout-1 card that lands on
        // the Save icon and silently bookmarks the video.
        let rail = screen::locate_action_rail(&open_image)
            .ok_or_else(|| SendFailure::before(anyhow!("action_rail_not_located")))?;
        let point = |x: f64, y: f64| {
            self.next_touch_point(
                udid,
                screen_size,
                TapPoint {
                    x: screen_size.0 * x,
                    y: screen_size.1 * y,
                },
                (8.0, 8.0),
            )
        };

        {
            let _guard = gestures.lock().await;
            session
                .tap(point(rail.x, rail.comment_y))
                .await
                .map_err(|e| SendFailure::before(anyhow!("open_comment_drawer: {e}")))?;
        }
        let drawer = self
            .wait_for_frame(udid, Duration::from_secs(6), stop, |img| {
                !matches!(
                    screen::comment_drawer_state(img).0,
                    CommentDrawer::Closed | CommentDrawer::Unknown
                )
            })
            .await
            .ok_or_else(|| SendFailure::before(anyhow!("comment_drawer_not_confirmed")))?;
        if screen::comment_drawer_state(&drawer).0 != CommentDrawer::Open {
            return Err(SendFailure::before(anyhow!(
                "comment_drawer_has_existing_draft"
            )));
        }
        sleep_interruptible(COMMENT_DRAWER_SETTLE, stop).await;
        let before_typing = self
            .frames
            .latest(udid)
            .ok_or_else(|| SendFailure::before(anyhow!("frame_unavailable_before_typing")))?;
        {
            let _guard = gestures.lock().await;
            session
                .tap(point(screen::COMMENT_INPUT.0, screen::COMMENT_INPUT.1))
                .await
                .map_err(|e| SendFailure::before(anyhow!("focus_comment_input: {e}")))?;
            sleep_interruptible(COMMENT_INPUT_SETTLE, stop).await;
            session
                // The literal prefix, because this path cannot reach TikTok's suggestion list
                // and so cannot make a real mention — see `append_mentions_by_picker`. Empty
                // unless the campaign asked for tags, which keeps every other comment
                // byte-identical to what it was.
                .type_text(&format!(
                    "{}{}",
                    prepared.literal_mention_prefix(),
                    prepared.text
                ))
                .await
                .map_err(|e| SendFailure::before(anyhow!("type_comment: {e}")))?;
        }
        let changed = self
            .wait_for_new_frame(
                udid,
                Duration::from_secs(6),
                stop,
                frame_digest(&before_typing),
            )
            .await;
        let armed_frame = if changed {
            self.wait_for_frame(udid, Duration::from_secs(6), stop, |img| {
                screen::comment_drawer_state(img).0 == CommentDrawer::SendArmed
            })
            .await
        } else {
            None
        };
        let armed_bytes = self
            .frames
            .latest(udid)
            .ok_or_else(|| SendFailure::before(anyhow!("armed_frame_missing")))?;
        if armed_frame.is_none() {
            return Err(SendFailure::before(anyhow!("send_not_armed")));
        }
        let before_send = frame_digest(&armed_bytes);
        let gate_failure = {
            let _guard = gestures.lock().await;
            match effect_gate.cross() {
                Ok(()) => {
                    session
                        .tap(point(screen::SEND_BUTTON.0, screen::SEND_BUTTON.1))
                        .await
                        .map_err(|e| SendFailure::after(anyhow!("tap_send: {e}")))?;
                    None
                }
                Err(failure) => Some(failure),
            }
        };
        if let Some(failure) = gate_failure {
            let cleaned = self
                .close_comment_ui(udid, session, gestures, screen_size, stop)
                .await;
            return Err(classify_pixel_gate_cleanup(failure, cleaned, "comment"));
        }
        // "Ready to type" and "sent" are the same classification — an open,
        // unarmed drawer — separated only by when they were observed. So the
        // frames already seen are excluded, `before_typing` above all, because
        // it satisfied this very predicate and a stream running behind would
        // otherwise replay it as proof the field emptied. That matters more
        // here than in the nurture path: this frame's digest is persisted as
        // `cleared_frame_sha256`, so an unexcluded replay writes the
        // *pre-typing* screen into the record as evidence the comment posted.
        // The bytes of the frame that satisfied the predicate, not a fresh read. A second
        // `frames.latest()` here used to hash whatever had arrived by then, so the verdict and
        // the stored evidence could describe different screens.
        let (cleared_bytes, _cleared) = self
            .wait_for_frame_after(
                udid,
                Duration::from_secs(6),
                stop,
                &[before_send, frame_digest(&before_typing)],
                |img| screen::comment_drawer_state(img).0 == CommentDrawer::Open,
            )
            .await
            .ok_or_else(|| SendFailure::after(anyhow!("send_clear_not_confirmed")))?;
        self.close_comment_ui(udid, session, gestures, screen_size, stop)
            .await;
        Ok(ThreadSendEvidence {
            text_sha256: prepared.text_sha256.clone(),
            armed_frame_sha256: crate::interaction::frame_sha256(&armed_bytes),
            cleared_frame_sha256: crate::interaction::frame_sha256(&cleared_bytes),
        })
    }

    /// Continue a comment drawer after the locator has identified the exact
    /// parent and its Reply control. The caller owns the identity proof; this
    /// method only performs the same armed/cleared composer checks.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn send_prepared_thread_reply(
        &self,
        udid: &str,
        session: &dyn UiSession,
        gestures: &tokio::sync::Mutex<()>,
        reply_point: TapPoint,
        prepared: &PreparedThreadMessage,
        stop: &AtomicBool,
    ) -> Result<ThreadSendEvidence, SendFailure> {
        let mut effect_gate = crate::interaction_target::EffectGate::allow();
        self.send_prepared_thread_reply_with_gate(
            udid,
            session,
            gestures,
            reply_point,
            prepared,
            stop,
            &mut effect_gate,
        )
        .await
    }

    // The public-effect boundary deliberately exposes each independently established proof
    // (session, gesture lock, reply point, prepared text, stop token and durable gate).
    #[allow(
        clippy::too_many_arguments,
        reason = "effect boundary keeps independently proved reply inputs explicit"
    )]
    pub(crate) async fn send_prepared_thread_reply_with_gate(
        &self,
        udid: &str,
        session: &dyn UiSession,
        gestures: &tokio::sync::Mutex<()>,
        reply_point: TapPoint,
        prepared: &PreparedThreadMessage,
        stop: &AtomicBool,
        effect_gate: &mut crate::interaction_target::EffectGate<'_>,
    ) -> Result<ThreadSendEvidence, SendFailure> {
        if !session.supports_text_input() {
            return Err(SendFailure::before(anyhow!("text_channel_unavailable")));
        }
        // Refuse rather than fabricate — the reply point, composer and Send are all
        // derived from this.
        let screen_size = crate::screen::measured_screen_size(session)
            .await
            .map_err(SendFailure::before)?;
        let reply_point = self.next_touch_point(udid, screen_size, reply_point, (8.0, 8.0));
        {
            let _guard = gestures.lock().await;
            session
                .tap(reply_point)
                .await
                .map_err(|e| SendFailure::before(anyhow!("tap_reply: {e}")))?;
        }
        self.wait_for_frame(udid, Duration::from_secs(5), stop, |img| {
            !matches!(
                screen::comment_drawer_state(img).0,
                CommentDrawer::Closed | CommentDrawer::Unknown
            )
        })
        .await
        .ok_or_else(|| SendFailure::before(anyhow!("reply_composer_not_confirmed")))?;
        sleep_interruptible(COMMENT_INPUT_SETTLE, stop).await;
        let before_typing = self
            .frames
            .latest(udid)
            .ok_or_else(|| SendFailure::before(anyhow!("frame_unavailable_before_reply")))?;
        {
            let _guard = gestures.lock().await;
            session
                .tap(self.next_touch_point(
                    udid,
                    screen_size,
                    TapPoint {
                        x: screen_size.0 * screen::COMMENT_INPUT.0,
                        y: screen_size.1 * screen::COMMENT_INPUT.1,
                    },
                    (24.0, 10.0),
                ))
                .await
                .map_err(|e| SendFailure::before(anyhow!("focus_reply_input: {e}")))?;
            sleep_interruptible(COMMENT_INPUT_SETTLE, stop).await;
            session
                // The literal prefix, because this path cannot reach TikTok's suggestion list
                // and so cannot make a real mention — see `append_mentions_by_picker`. Empty
                // unless the campaign asked for tags, which keeps every other comment
                // byte-identical to what it was.
                .type_text(&format!(
                    "{}{}",
                    prepared.literal_mention_prefix(),
                    prepared.text
                ))
                .await
                .map_err(|e| SendFailure::before(anyhow!("type_reply: {e}")))?;
        }
        let changed = self
            .wait_for_new_frame(
                udid,
                Duration::from_secs(6),
                stop,
                frame_digest(&before_typing),
            )
            .await;
        let armed = changed
            && self
                .wait_for_frame(udid, Duration::from_secs(6), stop, |img| {
                    screen::comment_drawer_state(img).0 == CommentDrawer::SendArmed
                })
                .await
                .is_some();
        let armed_bytes = self
            .frames
            .latest(udid)
            .ok_or_else(|| SendFailure::before(anyhow!("reply_armed_frame_missing")))?;
        if !armed {
            return Err(SendFailure::before(anyhow!("reply_send_not_armed")));
        }
        let before_send = frame_digest(&armed_bytes);
        let gate_failure = {
            let _guard = gestures.lock().await;
            match effect_gate.cross() {
                Ok(()) => {
                    session
                        .tap(self.next_touch_point(
                            udid,
                            screen_size,
                            TapPoint {
                                x: screen_size.0 * screen::SEND_BUTTON.0,
                                y: screen_size.1 * screen::SEND_BUTTON.1,
                            },
                            (8.0, 8.0),
                        ))
                        .await
                        .map_err(|e| SendFailure::after(anyhow!("tap_reply_send: {e}")))?;
                    None
                }
                Err(failure) => Some(failure),
            }
        };
        if let Some(failure) = gate_failure {
            let cleaned = self
                .close_comment_ui(udid, session, gestures, screen_size, stop)
                .await;
            return Err(classify_pixel_gate_cleanup(failure, cleaned, "reply"));
        }
        // Same rule as the top-level comment: a frame already seen cannot be
        // the proof, and this one is persisted as evidence.
        // Same rule, same reason as the root path above.
        let (cleared_bytes, _cleared) = self
            .wait_for_frame_after(
                udid,
                Duration::from_secs(6),
                stop,
                &[before_send, frame_digest(&before_typing)],
                |img| screen::comment_drawer_state(img).0 == CommentDrawer::Open,
            )
            .await
            .ok_or_else(|| SendFailure::after(anyhow!("reply_clear_not_confirmed")))?;
        {
            let _guard = gestures.lock().await;
            let _ = session
                .tap(self.next_touch_point(
                    udid,
                    screen_size,
                    TapPoint {
                        x: screen_size.0 * screen::DRAWER_DISMISS.0,
                        y: screen_size.1 * screen::DRAWER_DISMISS.1,
                    },
                    (12.0, 10.0),
                ))
                .await;
        }
        Ok(ThreadSendEvidence {
            text_sha256: prepared.text_sha256.clone(),
            armed_frame_sha256: crate::interaction::frame_sha256(&armed_bytes),
            cleared_frame_sha256: crate::interaction::frame_sha256(&cleared_bytes),
        })
    }

    /// Tap the heart, and take the rail, the baseline and the confirmation from
    /// frames that are all qualified.
    ///
    /// The caller's rail is deliberately not used. It was located before
    /// `wait_for_action_gap`, which sleeps 12–35 s, so by the time a like runs
    /// it can belong to a card that is long gone — and tapping coordinates from
    /// a previous card is the "tapped 14 in a row for 0 likes" failure. The
    /// comment path already re-locates for the same reason. If no rail can be
    /// found on the *current* frame, nothing is tapped at all.
    pub(super) async fn do_like(
        &self,
        udid: &str,
        session: &dyn UiSession,
        gestures: &tokio::sync::Mutex<()>,
        screen_size: (f64, f64),
        stop: &AtomicBool,
    ) -> anyhow::Result<LikeResult> {
        // The human pause comes first, so everything below is read from the
        // frame the tap is actually aimed at. Reading the rail and the baseline
        // *before* a 400–900 ms pause leaves a window in which the feed can
        // move on, and a card that arrives in that window is already liked
        // often enough to confirm a like nobody performed.
        let mut rng = StdRng::from_entropy();
        sleep_interruptible(Duration::from_millis(rng.gen_range(400..900)), stop).await;

        let Some(frame) = self.frames.latest(udid) else {
            return Ok(LikeResult::NotOnFeed);
        };
        let Some(img) = image::load_from_memory(&frame).ok().map(|i| i.to_rgb8()) else {
            return Ok(LikeResult::NotOnFeed);
        };
        if !screen::feed_ready(&img, Some(screen_size.0)) {
            return Ok(LikeResult::NotOnFeed);
        }
        let Some(rail) = screen::locate_action_rail(&img) else {
            return Ok(LikeResult::NotOnFeed);
        };

        let before = screen::like_redness_at(&img, &rail);
        // Already liked — tapping again would un-like it.
        if before > screen::LIKE_FILLED_REDNESS {
            return Ok(LikeResult::AlreadyLiked);
        }
        let watermark = frame_digest(&frame);

        let point = self.next_touch_point(
            udid,
            screen_size,
            TapPoint {
                x: screen_size.0 * rail.x,
                y: screen_size.1 * rail.like_y,
            },
            (10.0, 12.0),
        );
        {
            let _guard = gestures.lock().await;
            session.tap(point).await?;
        }

        // Absolute, not relative: the heart is either filled or it is not, and
        // the fill level does not depend on the video behind it. A relative
        // test does, which is how a red-heavy clip made real likes read as
        // misses and outlines read as already-liked.
        //
        // The confirming frame must postdate the tap and must still be an
        // actionable feed — a system alert dims the screen, and a dimmed frame
        // is not one the heart can be read from. `do_follow` has carried that
        // second guard for a while; this path did not.
        let mut best = before;
        let confirmed = self
            .wait_for_frame_after(
                udid,
                Duration::from_millis(2_500),
                stop,
                &[watermark],
                |img| {
                    if !screen::feed_ready(img, Some(screen_size.0)) {
                        return false;
                    }
                    let now = screen::like_redness_at(img, &rail);
                    best = best.max(now);
                    now > screen::LIKE_FILLED_REDNESS
                },
            )
            .await
            .is_some();
        Ok(if confirmed {
            LikeResult::Liked
        } else {
            LikeResult::NotConfirmed { before, best }
        })
    }

    /// Tap follow, then confirm the red badge is gone.
    pub(super) async fn do_follow(
        &self,
        _udid: &str,
        _session: &dyn UiSession,
        _gestures: &tokio::sync::Mutex<()>,
        _rail: &ActionRail,
        _screen_size: (f64, f64),
        _stop: &AtomicBool,
    ) -> anyhow::Result<FollowResult> {
        // Keep the signature tied to the production call site while explicitly declining every
        // pixel attempt. A frame can prove a badge and an OCR name, but it has no resource id,
        // ancestry, canonical @handle or hierarchy generation. Those fields are required to
        // locate the exact Following-list row during cleanup, so tapping here would create an
        // effect which Riviu could not later own. The hierarchy path is the only Follow producer.
        Ok(FollowResult::SourceUnavailable)
    }

    /// Swipe to the next video, proving from the stream that the feed actually
    /// moved.
    ///
    /// "Did any byte of the frame change?" cannot answer this.
    /// [`crate::stream::StreamHub::publish`] does no deduplication and the
    /// stream runs at 24 FPS, so a video playing on the card you are *already*
    /// on produces a different frame every ~42 ms — roughly 57 chances inside
    /// the settle window to call a swipe that went nowhere a success. Fixtures
    /// `feed-same-card-{1,2,3}.jpg` are three frames of one card, taken seconds
    /// apart with identical author, caption and counts, and their digests all
    /// differ.
    ///
    /// The action rail answers it instead: it is on every video card and it is
    /// gone while the feed is between cards (`feed-mid-swipe.jpg`), so
    /// `rail → no rail → rail` is the feed moving and a playing video cannot
    /// fake it. See [`SwipeOutcome`].
    pub(super) async fn do_swipe(
        &self,
        udid: &str,
        session: &dyn UiSession,
        gestures: &tokio::sync::Mutex<()>,
        screen_size: (f64, f64),
        duration_ms: u64,
        stop: &AtomicBool,
    ) -> anyhow::Result<SwipeOutcome> {
        let mut rng = StdRng::from_entropy();
        let x0 = screen_size.0 * 0.5 + rng.gen_range(-20.0..20.0);
        let gesture = SwipeGesture {
            from: TapPoint {
                x: x0,
                y: screen_size.1 * (SWIPE_FROM_Y + rng.gen_range(-0.03..0.03)),
            },
            to: TapPoint {
                x: x0 + rng.gen_range(-8.0..8.0),
                y: (screen_size.1 * SWIPE_TO_Y + rng.gen_range(-40.0..40.0)).max(40.0),
            },
            duration_ms,
        };
        // Hold the gesture lock across the swipe *and* the watch, and run them
        // together.
        //
        // The first version of this collected evidence only after
        // `session.swipe()` returned, arguing that watching concurrently would
        // let a ✕ the screen watcher pressed read as our swipe moving the feed.
        // Both halves of that were wrong. Measured on the device, six swipes out
        // of six: the gesture call returns at 535–550 ms, while the rail is off
        // screen from 222–402 ms and only for 80–120 ms. The window this check
        // keys on had always closed before the check started looking — a live
        // run scored 1 confirmed advance out of 16 swipes. And the watcher was
        // never excluded anyway: it is suppressed only around the comment
        // drawer, so it was free to tap through the whole settle window.
        //
        // Holding the lock across both is what actually excludes it — the
        // watcher taps through this same lock — and running them together is
        // the only way to be watching while the transition happens.
        let _guard = gestures.lock().await;
        // Read the baseline under the lock as well, so nothing can change the
        // screen between measuring it and swiping.
        let rail_before = self
            .latest_image(udid)
            .map(|img| screen::rail_icons_present(&img));
        let Some(rail_before) = rail_before else {
            // No pre-swipe baseline (e.g. right after a stream clear). The
            // gesture still goes out, so repeating it could skip a card, but
            // nothing here proves it landed anywhere.
            session.swipe(gesture).await?;
            return Ok(SwipeOutcome::Moved);
        };
        // The window now starts with the gesture, so it has to cover the
        // gesture's own duration as well as the settle.
        let window = Duration::from_millis(duration_ms) + SWIPE_SETTLE;
        let (sent, outcome) = tokio::join!(
            session.swipe(gesture),
            self.watch_swipe(udid, window, stop, rail_before)
        );
        sent?;
        Ok(outcome)
    }

    /// Advance one slide in a TikTok photo carousel. Photo posts use a
    /// horizontal gesture and still require a newer frame as proof; the
    /// caller decides how many slides to sample.
    pub(super) async fn do_photo_swipe(
        &self,
        udid: &str,
        session: &dyn UiSession,
        gestures: &tokio::sync::Mutex<()>,
        screen_size: (f64, f64),
        duration_ms: u64,
        stop: &AtomicBool,
    ) -> anyhow::Result<bool> {
        let before = self.frames.latest(udid).map(|f| frame_digest(&f));
        let mut rng = StdRng::from_entropy();
        let y = screen_size.1 * rng.gen_range(0.38..0.62);
        let gesture = SwipeGesture {
            from: TapPoint {
                x: screen_size.0 * rng.gen_range(0.74..0.84),
                y,
            },
            to: TapPoint {
                x: screen_size.0 * rng.gen_range(0.16..0.27),
                y: y + rng.gen_range(-4.0..4.0),
            },
            duration_ms,
        };
        {
            let _guard = gestures.lock().await;
            session.swipe(gesture).await?;
        }
        let Some(before) = before else {
            // No pre-swipe baseline: cannot confirm the carousel advanced.
            return Ok(false);
        };
        Ok(self
            .wait_for_new_frame(udid, SWIPE_SETTLE, stop, before)
            .await)
    }

    /// Close the comment drawer. Safe to call from any state: the dismiss point
    /// is near the top of the screen, well away from anything destructive.
    pub(super) async fn dismiss_drawer(
        &self,
        udid: &str,
        session: &dyn UiSession,
        gestures: &tokio::sync::Mutex<()>,
        screen_size: (f64, f64),
        stop: &AtomicBool,
    ) {
        let _guard = gestures.lock().await;
        let _ = session
            .tap(self.next_touch_point(
                udid,
                screen_size,
                TapPoint {
                    x: screen_size.0 * screen::DRAWER_DISMISS.0,
                    y: screen_size.1 * screen::DRAWER_DISMISS.1,
                },
                (12.0, 10.0),
            ))
            .await;
        sleep_interruptible(Duration::from_millis(700), stop).await;
    }

    /// Dismiss until the feed is back, or give up after a few tries.
    ///
    /// One dismiss tap closes the drawer but not the composer stacked above it,
    /// and an abandoned attempt used to return with the composer still up — so
    /// the *next* attempt tapped the comment icon into the composer's own
    /// controls and failed too. A live run alternated failure reasons in
    /// lockstep because of this: NotArmed, NoComposer, NotArmed, NoComposer.
    pub(super) async fn close_comment_ui(
        &self,
        udid: &str,
        session: &dyn UiSession,
        gestures: &tokio::sync::Mutex<()>,
        screen_size: (f64, f64),
        stop: &AtomicBool,
    ) -> bool {
        for _ in 0..3 {
            self.dismiss_drawer(udid, session, gestures, screen_size, stop)
                .await;
            if self
                .wait_for_frame(udid, Duration::from_secs(3), stop, |img| {
                    screen::feed_ready(img, Some(screen_size.0))
                })
                .await
                .is_some()
            {
                return true;
            }
        }
        tracing::warn!("[nurture {udid}] không đóng được giao diện bình luận về feed");
        false
    }

    /// Re-prove the evidence card and rail under the gesture lock, then open its drawer.
    /// `Some(CardChanged)` means no tap occurred; `None` means the open tap was delivered.
    async fn open_grounded_comment_drawer(
        &self,
        udid: &str,
        session: &dyn UiSession,
        gestures: &tokio::sync::Mutex<()>,
        screen_size: (f64, f64),
        prepared: &PreparedTextComment,
    ) -> anyhow::Result<Option<CommentResult>> {
        let _guard = gestures.lock().await;
        let Some(active_frame) = self.frames.latest(udid) else {
            self.update_comment_attempt(prepared, "skipped: card_changed");
            return Ok(Some(CommentResult::CardChanged));
        };
        let Some(active_image) = image::load_from_memory(&active_frame)
            .ok()
            .map(|image| image.to_rgb8())
        else {
            self.update_comment_attempt(prepared, "skipped: card_changed");
            return Ok(Some(CommentResult::CardChanged));
        };
        let fresh_identity = if prepared.grounded_on.is_some() {
            self.read_card_identity(&active_frame).await
        } else {
            None
        };
        let identity_matches = match (&prepared.grounded_on, &fresh_identity) {
            (Some(expected), Some(actual)) => expected.same_card(actual),
            (Some(_), None) => false,
            (None, _) => true,
        };
        let Some(active_rail) = screen::locate_action_rail(&active_image) else {
            self.update_comment_attempt(prepared, "skipped: card_changed");
            return Ok(Some(CommentResult::CardChanged));
        };
        if !screen::feed_ready(&active_image, Some(screen_size.0)) || !identity_matches {
            self.update_comment_attempt(prepared, "skipped: card_changed");
            return Ok(Some(CommentResult::CardChanged));
        }
        let point = self.next_touch_point(
            udid,
            screen_size,
            TapPoint {
                x: screen_size.0 * active_rail.x,
                y: screen_size.1 * active_rail.comment_y,
            },
            (8.0, 8.0),
        );
        session.tap(point).await?;
        Ok(None)
    }

    /// Post a text comment on the current video.
    ///
    /// Context collection and the two AI passes happen before opening the drawer
    /// so a slow provider cannot leave UI behind. Typing is only an attempted
    /// input: the stream must show Send arm before it is tapped, then disarm
    /// afterwards.
    /// A failed text attempt is closed and classified without trying an emoji in
    /// the same field, which avoids accidentally posting mixed content.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn do_comment(
        &self,
        udid: &str,
        session: &dyn UiSession,
        gestures: &tokio::sync::Mutex<()>,
        _rail: &ActionRail,
        screen_size: (f64, f64),
        settings: &NurtureSettings,
        _pool: &[String],
        stop: &AtomicBool,
    ) -> anyhow::Result<CommentResult> {
        if !session.supports_text_input() {
            return Ok(CommentResult::TextChannelUnavailable);
        }
        // What a failed preparation cost and why it failed. Filled by the error arm below
        // and read by the skip row, so a billed draft the verifier refused is money the
        // audit still sees — and a row an operator can read, the same two courtesies the
        // hierarchy path's skip row has carried since its own review round.
        let mut skip_spend: Option<crate::openai_client::CommentSpend> = None;
        let mut skip_reason: Option<String> = None;
        let prepared = if settings.api_key.trim().is_empty() {
            // Unit fixtures still exercise the proven drawer sender with an
            // explicit pool entry. Production never passes a pool, so an empty
            // API key is a contextual skip rather than a generic post.
            #[cfg(test)]
            {
                pick_from_pool(_pool).map(|text| PreparedTextComment {
                    cost_usd: None,
                    text,
                    model: settings.model.clone(),
                    base_url_host: host_of(&settings.base_url),
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    source: "test-fixture",
                    frame_sha256: None,
                    caption_preview: None,
                    context_confidence: None,
                    relevance: None,
                    evidence_support: None,
                    distinct_frames: None,
                    grounded_on: None,
                    attempt_id: String::new(),
                })
            }
            #[cfg(not(test))]
            {
                None
            }
        } else {
            let Some(evidence) = self.collect_comment_frames(udid, screen_size, stop).await else {
                self.record_context_skip_attempt(
                    udid,
                    settings,
                    context_source(settings),
                    "evidence_unavailable",
                    0,
                    None,
                );
                return Ok(CommentResult::ContextSkipped);
            };
            let frames = &evidence.frames;
            let direction = pick_direction_seeded(
                &settings.ai_directions,
                frames
                    .first()
                    .map(|frame| frame_digest(frame))
                    .unwrap_or_default(),
            );
            let prepared_result = if provider_supports_vision(settings) {
                prepare_grounded_comment(
                    settings,
                    frames,
                    EvidenceKind::Moments,
                    direction.as_deref(),
                    // Empty on purpose: this loop meets a post by scrolling onto it, so there
                    // is no link to look the caption up by. See `openai_client::PostBrief`.
                    Default::default(),
                )
                .await
            } else {
                // An OCR miss is a turn with nothing to say, not a device
                // failure. Propagating it with `?` made `do_comment` return
                // `Err`, which the caller reads as a gesture failure and spends
                // a recovery budget on — and it left no attempt row at all, so
                // the skip was invisible in the audit. Same handling as any
                // other unusable context.
                let Some(caption) = self.read_caption(frames.last()).await else {
                    self.record_context_skip_attempt(
                        udid,
                        settings,
                        "ocr-caption",
                        "caption_ocr_empty",
                        0,
                        None,
                    );
                    return Ok(CommentResult::ContextSkipped);
                };
                let fingerprint = format!(
                    "{:016x}",
                    frames.last().map(|f| frame_digest(f)).unwrap_or_default()
                );
                prepare_caption_comment(settings, &caption, &fingerprint, direction.as_deref())
                    .await
            };
            match prepared_result {
                Ok(comment) => Some(PreparedTextComment {
                    cost_usd: comment.cost_usd,
                    text: comment.text,
                    model: comment.model,
                    base_url_host: comment.base_url_host,
                    prompt_tokens: comment.prompt_tokens,
                    completion_tokens: comment.completion_tokens,
                    source: context_source(settings),
                    frame_sha256: Some(comment.frame_sha256),
                    caption_preview: comment
                        .caption
                        .as_deref()
                        .map(|caption| caption.chars().take(160).collect()),
                    context_confidence: Some(comment.context_confidence),
                    relevance: Some(comment.relevance),
                    evidence_support: Some(comment.evidence_support),
                    distinct_frames: Some(comment.distinct_frames),
                    grounded_on: Some(evidence.identity.clone()),
                    attempt_id: String::new(),
                }),
                Err(error) => {
                    tracing::info!("[nurture {udid}] bỏ qua comment semantic: {error}");
                    if std::env::var_os("RIVIU_LIVE_NURTURE_VERBOSE").is_some() {
                        eprintln!("[nurture {udid}] comment semantic skip: {error}");
                    }
                    // The failure may still carry a bill — a billed draft whose verifier
                    // refused it, a malformed body after a charged call. Keep the price and
                    // the (bounded) reason for the skip row below; dropping the error here
                    // used to drop both.
                    skip_spend = crate::openai_client::spend_of_failure(&error);
                    skip_reason = Some(error.to_string().chars().take(160).collect());
                    None
                }
            }
        };
        let Some(mut prepared) = prepared else {
            let outcome = match skip_reason {
                Some(reason) => format!("context_skipped: {reason}"),
                None => "context_skipped".to_string(),
            };
            self.record_context_skip_attempt(
                udid,
                settings,
                context_source(settings),
                &outcome,
                0,
                skip_spend,
            );
            return Ok(CommentResult::ContextSkipped);
        };

        let attempt = NurtureCommentAttempt {
            id: Uuid::new_v4().to_string(),
            udid: udid.to_string(),
            outcome: "prepared".into(),
            source: prepared.source.into(),
            model: prepared.model.clone(),
            base_url_host: prepared.base_url_host.clone(),
            prompt_tokens: prepared.prompt_tokens,
            completion_tokens: prepared.completion_tokens,
            cost_usd: prepared.cost_usd,
            preview: prepared.text.chars().take(160).collect(),
            caption_preview: prepared.caption_preview.clone().unwrap_or_default(),
            frame_sha256: prepared.frame_sha256.clone().unwrap_or_default(),
            context_confidence: prepared.context_confidence,
            relevance: prepared.relevance,
            evidence_support: prepared.evidence_support,
            distinct_frames: prepared.distinct_frames,
            // The pixel engine does not page carousels, so this path never has slides.
            carousel_slides: None,
            created_at: Utc::now().to_rfc3339(),
        };
        // The audit row is a write-ahead gate: a public comment without its prepared text,
        // evidence and outcome cannot be reconstructed afterwards. Do not touch the drawer
        // unless the row exists.
        prepared.attempt_id = match self.db.add_nurture_comment_attempt(&attempt) {
            Ok(()) => attempt.id,
            Err(error) => {
                tracing::warn!(
                    "[nurture {udid}] KHÔNG ghi được comment attempt ({error}) — bỏ qua trước \
                     khi mở khay bình luận"
                );
                return Ok(CommentResult::AuditUnavailable);
            }
        };

        let tap = |x: f64, y: f64| {
            self.next_touch_point(
                udid,
                screen_size,
                TapPoint {
                    x: screen_size.0 * x,
                    y: screen_size.1 * y,
                },
                (8.0, 8.0),
            )
        };

        // 1. open the comment drawer
        match self
            .open_grounded_comment_drawer(udid, session, gestures, screen_size, &prepared)
            .await
        {
            Ok(Some(result)) => return Ok(result),
            Ok(None) => {}
            Err(error) => {
                self.update_comment_attempt(&prepared, "open_error");
                self.close_comment_ui(udid, session, gestures, screen_size, stop)
                    .await;
                return Err(error);
            }
        }
        let drawer = self
            .wait_for_frame(udid, Duration::from_secs(5), stop, |img| {
                !matches!(
                    screen::comment_drawer_state(img).0,
                    CommentDrawer::Closed | CommentDrawer::Unknown
                )
            })
            .await;
        if drawer.is_none() {
            self.update_comment_attempt(&prepared, "no_drawer");
            self.close_comment_ui(udid, session, gestures, screen_size, stop)
                .await;
            return Ok(CommentResult::NoDrawer);
        }

        // TikTok keeps the composer inert while its comment list is loading.
        // The reference RT-MMO flow waits 3-5 seconds before focusing it; live
        // frames showed the loading indicator still present after 2.8 seconds.
        sleep_interruptible(COMMENT_DRAWER_SETTLE, stop).await;

        // Read state and digest from the same newest encoded frame. A drawer
        // that is armed before this attempt owns an older draft; typing or
        // sending in that state would misattribute its contents to `prepared`.
        let Some(open_frame) = self.frames.latest(udid) else {
            self.update_comment_attempt(&prepared, "no_drawer");
            self.close_comment_ui(udid, session, gestures, screen_size, stop)
                .await;
            return Ok(CommentResult::NoDrawer);
        };
        let before_typing = frame_digest(&open_frame);
        let drawer_state = image::load_from_memory(&open_frame)
            .ok()
            .map(|img| screen::comment_drawer_state(&img.to_rgb8()).0)
            .unwrap_or(CommentDrawer::Unknown);
        if drawer_state != CommentDrawer::Open {
            self.update_comment_attempt(
                &prepared,
                if drawer_state == CommentDrawer::SendArmed {
                    "existing_draft"
                } else {
                    "no_drawer"
                },
            );
            self.close_comment_ui(udid, session, gestures, screen_size, stop)
                .await;
            return Ok(if drawer_state == CommentDrawer::SendArmed {
                CommentResult::ExistingDraft
            } else {
                CommentResult::NoDrawer
            });
        }

        // Focus with RT-MMO's stable sessionless swipe-tap, then type through
        // its text channel. The request returning OK is not evidence of insertion.
        let input_result = async {
            let _guard = gestures.lock().await;
            session
                .tap(tap(screen::COMMENT_INPUT.0, screen::COMMENT_INPUT.1))
                .await?;
            sleep_interruptible(COMMENT_INPUT_SETTLE, stop).await;
            session.type_text(&prepared.text).await
        }
        .await;
        if let Err(error) = input_result {
            self.update_comment_attempt(&prepared, "type_error");
            self.close_comment_ui(udid, session, gestures, screen_size, stop)
                .await;
            return Err(error);
        }
        let armed = self
            .wait_for_new_frame(udid, Duration::from_secs(5), stop, before_typing)
            .await
            && self
                .wait_for_frame(udid, Duration::from_secs(5), stop, |img| {
                    screen::comment_drawer_state(img).0 == CommentDrawer::SendArmed
                })
                .await
                .is_some();
        if !armed {
            self.update_comment_attempt(&prepared, "text_not_armed");
            self.close_comment_ui(udid, session, gestures, screen_size, stop)
                .await;
            return Ok(CommentResult::TextNotArmed);
        }

        let before_send = self
            .frames
            .latest(udid)
            .map(|frame| frame_digest(&frame))
            .unwrap_or_default();
        let send_result = async {
            let _guard = gestures.lock().await;
            session
                .tap(tap(screen::SEND_BUTTON.0, screen::SEND_BUTTON.1))
                .await
        }
        .await;
        if let Err(error) = send_result {
            self.update_comment_attempt(&prepared, "send_error");
            self.close_comment_ui(udid, session, gestures, screen_size, stop)
                .await;
            return Err(error);
        }
        // The field must empty back to the open, unarmed drawer. Unknown frames
        // are not accepted as proof because the screen may have moved elsewhere.
        //
        // Both frames the caller has already seen are excluded. `before_send`
        // is the obvious one; `before_typing` matters because the drawer was
        // *also* `Open` then — it is the same predicate this check uses — so a
        // stream running a second behind could otherwise replay the pre-typing
        // screen as proof that the send emptied the field.
        let sent = self
            .wait_for_frame_after(
                udid,
                Duration::from_secs(5),
                stop,
                &[before_send, before_typing],
                |img| screen::comment_drawer_state(img).0 == CommentDrawer::Open,
            )
            .await
            .is_some();

        self.close_comment_ui(udid, session, gestures, screen_size, stop)
            .await;

        if !sent {
            self.update_comment_attempt(&prepared, "text_uncertain");
            return Ok(CommentResult::TextNotSent);
        }
        self.update_comment_attempt(&prepared, "sent");
        tracing::info!(
            "[nurture {udid}] đã gửi bình luận chữ source={} frame_sha256={:?} context={:?} relevance={:?} evidence={:?}",
            prepared.source,
            prepared.frame_sha256,
            prepared.context_confidence,
            prepared.relevance,
            prepared.evidence_support
        );

        // The `nurture_comment_costs` write that used to sit here is gone with the table
        // (migration 16). It was a strict subset of the row `update_comment_attempt` has
        // already written above -- same model, same host, same tokens, same preview -- into a
        // table whose only reader was a command the frontend never called. One DB write per
        // comment, for data nobody could open.
        Ok(CommentResult::TextSent {
            prompt_tokens: prepared.prompt_tokens,
            completion_tokens: prepared.completion_tokens,
        })
    }

    /// Capture a short, same-post evidence window from the existing MJPEG
    /// source. Any popup or transition inside the window invalidates the set;
    /// posting a sentence from mixed screens is worse than skipping a turn.
    async fn collect_comment_frames(
        &self,
        udid: &str,
        screen_size: (f64, f64),
        stop: &AtomicBool,
    ) -> Option<CommentEvidence> {
        let mut frames = Vec::with_capacity(3);
        let mut grounded_on: Option<PixelCardIdentity> = None;
        for sample in 0..3 {
            if stop.load(std::sync::atomic::Ordering::Relaxed) {
                return None;
            }
            let frame = self.frames.latest(udid)?;
            let image = image::load_from_memory(&frame).ok()?.to_rgb8();
            if !screen::feed_ready(&image, Some(screen_size.0)) {
                return None;
            }
            let identity = self.read_card_identity(&frame).await?;
            match &grounded_on {
                Some(expected) if !expected.same_card(&identity) => return None,
                None => grounded_on = Some(identity),
                Some(_) => {}
            }
            frames.push((*frame).clone());
            if sample < 2 {
                sleep_interruptible(Duration::from_millis(600), stop).await;
            }
        }
        Some(CommentEvidence {
            frames,
            identity: grounded_on?,
        })
    }

    /// Frames for grounding a comment, without the iPhone pixel gate.
    ///
    /// [`Self::collect_comment_frames`] rejects anything `screen::feed_ready`
    /// dislikes, and that detector is calibrated for one iPhone 8 layout — on an
    /// Android frame it would reject every sample and the comment would always come
    /// out as "context unavailable". The hierarchy loop has already established
    /// that the feed tab and the action rail are on screen, which is *stronger*
    /// evidence than a pixel heuristic, so the gate is not merely skipped here, it
    /// is replaced.
    pub(super) async fn collect_grounding_frames(
        &self,
        udid: &str,
        stop: &AtomicBool,
    ) -> Option<Vec<Vec<u8>>> {
        let mut frames = Vec::with_capacity(3);
        for sample in 0..3 {
            if stop.load(std::sync::atomic::Ordering::Relaxed) {
                return None;
            }
            let frame = self.frames.latest(udid)?;
            frames.push((*frame).clone());
            if sample < 2 {
                sleep_interruptible(Duration::from_millis(600), stop).await;
            }
        }
        (!frames.is_empty()).then_some(frames)
    }

    /// Generate one grounded comment and record its attempt row.
    ///
    /// Deliberately the *same* provider path the pixel engine uses — vision when
    /// the provider has it, OCR caption otherwise — so the two backends do not
    /// develop separate voices or separate audit trails. Only the frame gate
    /// differs, for the reason on [`Self::collect_grounding_frames`].
    pub(super) async fn prepare_hierarchy_comment(
        &self,
        udid: &str,
        settings: &NurtureSettings,
        // Frames the carousel traversal already offered, oldest first. Empty on a post that
        // was never paged — a video, or a build that cannot page — and then the frames are
        // sampled here exactly as before. `slides_offered` counts the slides behind them,
        // duplicates included, and is 0 when nothing was paged.
        slides: Vec<Vec<u8>>,
        slides_offered: u32,
        stop: &AtomicBool,
    ) -> Result<Option<super::hierarchy::PreparedComment>, super::hierarchy::CommentSourceError>
    {
        if settings.api_key.trim().is_empty() {
            // **The one skip that used to leave no trace at all.** Every other reason a post
            // goes uncommented writes a row, so the operator can read `nurture_comment_attempts`
            // and see what happened; this one returned before the first of them. On screen it
            // produced the identical line to "the frame had nothing worth saying", so a farm
            // configured with no key looked like a farm that kept deciding to stay quiet.
            self.record_context_skip_attempt(
                udid,
                settings,
                context_source(settings),
                "no_api_key",
                slides_offered,
                None,
            );
            return Ok(None);
        }
        // **Slides first, and they cost nothing extra.** The traversal was already paying for
        // every flick, its 900 ms settle and a hierarchy dump per slide, and the comment never
        // saw any of it: it was written before the first sideways gesture, from three samples
        // taken 600 ms apart of image one. On a still card those three samples are one
        // picture, so a six-image post was commented on from one sixth of itself.
        //
        // Falling back to sampling here is not a lesser path — it is what every video post
        // still does, unchanged.
        // Read before `slides` is moved: what the pictures *are* decides what the model may
        // say about them, and once the two sources have been folded into one `frames` they are
        // indistinguishable. Sampling produces moments of one card; the traversal produces
        // pages of a post, and calling the second the first invites narrated motion.
        let kind = if slides.is_empty() {
            EvidenceKind::Moments
        } else {
            EvidenceKind::CarouselSlides
        };
        let frames = if slides.is_empty() {
            match self.collect_grounding_frames(udid, stop).await {
                Some(frames) => frames,
                None => {
                    self.record_context_skip_attempt(
                        udid,
                        settings,
                        context_source(settings),
                        "evidence_unavailable",
                        slides_offered,
                        None,
                    );
                    return Ok(None);
                }
            }
        } else {
            slides
        };
        let direction = pick_direction_seeded(
            &settings.ai_directions,
            frames.first().map(|frame| frame_digest(frame)).unwrap_or(0),
        );
        let prepared = if provider_supports_vision(settings) {
            prepare_grounded_comment(
                settings,
                &frames,
                kind,
                direction.as_deref(),
                Default::default(),
            )
            .await
        } else {
            let Some(caption) = self.read_caption(frames.last()).await else {
                self.record_context_skip_attempt(
                    udid,
                    settings,
                    "ocr-caption",
                    "caption_ocr_empty",
                    slides_offered,
                    None,
                );
                return Ok(None);
            };
            let fingerprint = format!(
                "{:016x}",
                frames.last().map(|f| frame_digest(f)).unwrap_or_default()
            );
            prepare_caption_comment(settings, &caption, &fingerprint, direction.as_deref()).await
        };
        let comment = match prepared {
            Ok(comment) => comment,
            Err(error) => {
                // **The reason has to survive the failure.** `context_skipped` on its own
                // says a comment did not happen and refuses to say why, and the only copy of
                // the why went to `tracing::warn!` — which no operator-facing surface reads,
                // and which the command-line harness does not even install a subscriber for.
                // A rejected API key, a model id that does not exist, a verifier that keeps
                // scoring the draft as generic, and a network that is down all produced the
                // same row, so the first real run of this feature could only say "it skipped".
                //
                // Bounded, because a transport error can carry a whole response body and this
                // is a column an operator reads, not a log.
                tracing::warn!("[nurture {udid}] không soạn được bình luận: {error}");
                let reason: String = error.to_string().chars().take(160).collect();
                self.record_context_skip_attempt(
                    udid,
                    settings,
                    context_source(settings),
                    &format!("context_skipped: {reason}"),
                    slides_offered,
                    crate::openai_client::spend_of_failure(&error),
                );
                return Ok(None);
            }
        };
        let attempt = NurtureCommentAttempt {
            id: Uuid::new_v4().to_string(),
            udid: udid.to_string(),
            outcome: "prepared".into(),
            source: context_source(settings).into(),
            model: comment.model.clone(),
            base_url_host: comment.base_url_host.clone(),
            prompt_tokens: comment.prompt_tokens,
            completion_tokens: comment.completion_tokens,
            cost_usd: comment.cost_usd,
            preview: comment.text.chars().take(160).collect(),
            caption_preview: comment
                .caption
                .as_deref()
                .map(|caption| caption.chars().take(160).collect())
                .unwrap_or_default(),
            frame_sha256: comment.frame_sha256.clone(),
            context_confidence: Some(comment.context_confidence),
            relevance: Some(comment.relevance),
            evidence_support: Some(comment.evidence_support),
            distinct_frames: Some(comment.distinct_frames),
            carousel_slides: Some(slides_offered),
            created_at: Utc::now().to_rfc3339(),
        };
        self.persist_hierarchy_comment(
            attempt,
            comment.text,
            comment.prompt_tokens,
            comment.completion_tokens,
        )
        .map(Some)
    }

    fn persist_hierarchy_comment(
        &self,
        attempt: NurtureCommentAttempt,
        text: String,
        prompt_tokens: u32,
        completion_tokens: u32,
    ) -> Result<super::hierarchy::PreparedComment, super::hierarchy::CommentSourceError> {
        // Same write-ahead rule as the pixel path: without a durable row, return no text to
        // the hierarchy driver, so it cannot possibly reach `run.comment`.
        let audit_attempt = match super::hierarchy::CommentAuditToken::persist_for_text(
            &self.db, &attempt, &text,
        ) {
            Ok(token) => token,
            Err(error) => {
                tracing::warn!(
                    "[nurture {}] KHÔNG ghi được comment attempt ({error}) — bỏ qua trước \
                         khi mở khay bình luận",
                    attempt.udid
                );
                return Err(super::hierarchy::CommentSourceError::AuditUnavailable);
            }
        };
        Ok(super::hierarchy::PreparedComment {
            text,
            prompt_tokens,
            completion_tokens,
            audit_attempt,
        })
    }

    /// Close out a hierarchy comment's audit row.
    pub(super) fn finish_hierarchy_comment(&self, attempt_id: &str, outcome: &str) {
        if let Err(error) = self
            .db
            .update_nurture_comment_attempt_outcome(attempt_id, outcome)
        {
            tracing::warn!("không cập nhật outcome comment attempt {attempt_id}: {error}");
        }
    }

    fn update_comment_attempt(&self, prepared: &PreparedTextComment, outcome: &str) {
        if let Err(error) = self
            .db
            .update_nurture_comment_attempt_outcome(&prepared.attempt_id, outcome)
        {
            tracing::warn!(
                "không cập nhật outcome comment attempt {}: {error}",
                prepared.attempt_id
            );
        }
    }

    /// Read the caption off a frame, or `None` if OCR has nothing usable.
    async fn read_caption(&self, frame: Option<&Vec<u8>>) -> Option<String> {
        let observations = self.frame_text.recognize(frame?).await.ok()?;
        ocr_caption(&observations)
    }

    async fn read_card_identity(&self, frame: &[u8]) -> Option<PixelCardIdentity> {
        let observations = self.frame_text.recognize(frame).await.ok()?;
        pixel_card_identity_from_observations(&observations)
    }

    /// One row for a comment the post's budget was charged for that never reached the drawer.
    ///
    /// The deferred path spends the action gap and `record_attempt` *before* the slides are
    /// paged, so without this a card that changed underneath the traversal would leave a spent
    /// attempt and no row at all — the exact hole `no_api_key` used to leave.
    pub(super) fn record_deferred_skip(
        &self,
        udid: &str,
        settings: &NurtureSettings,
        reason: &str,
        slides_offered: u32,
    ) {
        self.record_context_skip_attempt(
            udid,
            settings,
            context_source(settings),
            reason,
            slides_offered,
            None,
        );
    }

    /// One row for a comment that never got written, so a quiet post is legible.
    ///
    /// `slides_offered` is what the carousel traversal paged, duplicates included, or 0 on a
    /// path that never pages.
    /// `spend` is what the attempt cost before it failed, and `None` means it cost nothing.
    ///
    /// **The distinction is the whole point.** Every one of these used to write
    /// `prompt_tokens: 0`, so an attempt the verification gate rejected — a draft, a
    /// verification, and a retry of both — was filed as free. Measured on the operator's own
    /// database on 25/08/2026: thirteen of thirty-three attempts were stored as costing
    /// nothing, and those thirteen were the expensive ones. A skip that really did happen
    /// before the first call still passes `None`, because a zero written for a call that was
    /// never made is as wrong as a zero written for one that was.
    fn record_context_skip_attempt(
        &self,
        udid: &str,
        settings: &NurtureSettings,
        source: &str,
        outcome: &str,
        slides_offered: u32,
        spend: Option<crate::openai_client::CommentSpend>,
    ) {
        let attempt = NurtureCommentAttempt {
            id: Uuid::new_v4().to_string(),
            udid: udid.to_string(),
            outcome: outcome.to_string(),
            source: source.to_string(),
            model: settings.model.clone(),
            base_url_host: host_of(&settings.base_url),
            prompt_tokens: spend.map(|s| s.prompt_tokens).unwrap_or(0),
            completion_tokens: spend.map(|s| s.completion_tokens).unwrap_or(0),
            cost_usd: spend.and_then(|s| s.cost_usd),
            preview: String::new(),
            caption_preview: String::new(),
            frame_sha256: String::new(),
            context_confidence: None,
            relevance: None,
            evidence_support: None,
            distinct_frames: None,
            carousel_slides: Some(slides_offered),
            created_at: Utc::now().to_rfc3339(),
        };
        if let Err(error) = self.db.add_nurture_comment_attempt(&attempt) {
            tracing::warn!("không ghi được context skip attempt: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;
    use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
    use parking_lot::Mutex;

    use super::*;
    use crate::db::Database;
    use crate::driver::DeviceDriver;
    use crate::frame_source::{Frame, FrameSource, FrameStream};
    use crate::types::DeviceInfo;

    const UDID: &str = "comment-test-device";
    const COMMENT: &str = "dep qua ban oi";

    struct FixedCardText;

    #[async_trait]
    impl crate::FrameTextSource for FixedCardText {
        async fn recognize(&self, _frame: &[u8]) -> anyhow::Result<Vec<CommentOcrObservation>> {
            Ok(vec![
                CommentOcrObservation {
                    text: "creator_a".into(),
                    confidence: 0.98,
                    x: 0.04,
                    y: 0.81,
                    width: 0.22,
                    height: 0.03,
                },
                CommentOcrObservation {
                    text: "caption chung".into(),
                    confidence: 0.98,
                    x: 0.04,
                    y: 0.86,
                    width: 0.30,
                    height: 0.03,
                },
            ])
        }
    }

    struct SequenceCardText {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl crate::FrameTextSource for SequenceCardText {
        async fn recognize(&self, _frame: &[u8]) -> anyhow::Result<Vec<CommentOcrObservation>> {
            let author = if self.calls.fetch_add(1, Ordering::Relaxed) == 0 {
                "creator_a"
            } else {
                "creator_b"
            };
            Ok(vec![
                CommentOcrObservation {
                    text: author.into(),
                    confidence: 0.98,
                    x: 0.04,
                    y: 0.81,
                    width: 0.22,
                    height: 0.03,
                },
                CommentOcrObservation {
                    text: "caption chung".into(),
                    confidence: 0.98,
                    x: 0.04,
                    y: 0.86,
                    width: 0.30,
                    height: 0.03,
                },
            ])
        }
    }

    struct CaptionSequenceCardText {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl crate::FrameTextSource for CaptionSequenceCardText {
        async fn recognize(&self, _frame: &[u8]) -> anyhow::Result<Vec<CommentOcrObservation>> {
            let call = self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(vec![
                CommentOcrObservation {
                    text: "creator_a".into(),
                    confidence: 0.98,
                    x: 0.04,
                    y: 0.81,
                    width: 0.22,
                    height: 0.03,
                },
                CommentOcrObservation {
                    text: if call < 2 { "caption a" } else { "caption b" }.into(),
                    confidence: 0.98,
                    x: 0.04,
                    y: 0.86,
                    width: 0.30,
                    height: 0.03,
                },
            ])
        }
    }

    struct CaptionedCardText;

    #[async_trait]
    impl crate::FrameTextSource for CaptionedCardText {
        async fn recognize(&self, _frame: &[u8]) -> anyhow::Result<Vec<CommentOcrObservation>> {
            Ok(vec![
                CommentOcrObservation {
                    text: "creator_a".into(),
                    confidence: 0.98,
                    x: 0.04,
                    y: 0.81,
                    width: 0.22,
                    height: 0.03,
                },
                CommentOcrObservation {
                    text: "caption b".into(),
                    confidence: 0.98,
                    x: 0.04,
                    y: 0.86,
                    width: 0.30,
                    height: 0.03,
                },
            ])
        }
    }

    #[test]
    fn pixel_card_identity_requires_author_and_normalizes_ocr_text() {
        let observations = vec![
            CommentOcrObservation {
                text: "  Creator_A  ".into(),
                confidence: 0.98,
                x: 0.04,
                y: 0.81,
                width: 0.22,
                height: 0.03,
            },
            CommentOcrObservation {
                text: "  Một   caption  ".into(),
                confidence: 0.90,
                x: 0.04,
                y: 0.86,
                width: 0.40,
                height: 0.03,
            },
        ];
        let identity = pixel_card_identity_from_observations(&observations).expect("author");
        assert_eq!(identity.author, "creator_a");
        assert_eq!(identity.caption.as_deref(), Some("một caption"));
        assert!(pixel_card_identity_from_observations(&observations[1..]).is_none());
        assert!(!identity.same_card(&PixelCardIdentity {
            author: "creator_b".into(),
            caption: identity.caption.clone(),
        }));
        assert!(!PixelCardIdentity {
            author: "creator_a".into(),
            caption: None,
        }
        .same_card(&PixelCardIdentity {
            author: "creator_a".into(),
            caption: Some("another card".into()),
        }));
    }

    #[test]
    fn pixel_card_identity_rejects_a_single_caption_or_overlay_as_author() {
        let caption_only = [CommentOcrObservation {
            text: "một caption đứng một mình".into(),
            confidence: 0.98,
            x: 0.04,
            y: 0.81,
            width: 0.46,
            height: 0.03,
        }];

        assert!(
            pixel_card_identity_from_observations(&caption_only).is_none(),
            "one lower-left text line is not proof that OCR found the author"
        );
    }

    #[test]
    fn pixel_card_identity_rejects_a_two_line_caption_without_author_metadata() {
        let caption_only = [
            CommentOcrObservation {
                text: "một caption đứng thành hai dòng".into(),
                confidence: 0.98,
                x: 0.04,
                y: 0.81,
                width: 0.52,
                height: 0.03,
            },
            CommentOcrObservation {
                text: "nhưng không có dòng tác giả".into(),
                confidence: 0.97,
                x: 0.04,
                y: 0.86,
                width: 0.48,
                height: 0.03,
            },
        ];

        assert!(
            pixel_card_identity_from_observations(&caption_only).is_none(),
            "two left-aligned prose lines are caption wrapping, not author metadata"
        );
    }

    #[test]
    fn pixel_card_identity_uses_metadata_cluster_below_overlay_headline() {
        let observations = vec![
            CommentOcrObservation {
                text: "VIỆT NAM THẮNG LỚN".into(),
                confidence: 0.99,
                x: 0.12,
                y: 0.69,
                width: 0.68,
                height: 0.05,
            },
            CommentOcrObservation {
                text: "TRONG TRẬN CHUNG KẾT".into(),
                confidence: 0.99,
                x: 0.12,
                y: 0.75,
                width: 0.72,
                height: 0.05,
            },
            CommentOcrObservation {
                text: "Thể Thao 247".into(),
                confidence: 0.98,
                x: 0.03,
                y: 0.82,
                width: 0.25,
                height: 0.03,
            },
            CommentOcrObservation {
                text: "Một caption của bài đăng".into(),
                confidence: 0.96,
                x: 0.03,
                y: 0.86,
                width: 0.52,
                height: 0.03,
            },
        ];

        let identity = pixel_card_identity_from_observations(&observations).expect("metadata");
        assert_eq!(identity.author, "thể thao 247");
        assert_eq!(
            identity.caption.as_deref(),
            Some("một caption của bài đăng")
        );
    }

    #[tokio::test]
    async fn comment_evidence_rejects_caption_a_to_b_across_canonical_samples() {
        let frames = Arc::new(TestFrames::new());
        let (engine, db_path) = test_engine(frames);
        let engine = engine.with_frame_text_source(Arc::new(CaptionSequenceCardText {
            calls: AtomicUsize::new(0),
        }));

        assert!(
            engine
                .collect_comment_frames(UDID, (375.0, 667.0), &AtomicBool::new(false))
                .await
                .is_none(),
            "a missing first caption must not turn all later captions into a wildcard"
        );
        let _ = std::fs::remove_file(db_path);
    }

    struct EmptyStream;

    #[async_trait]
    impl FrameStream for EmptyStream {
        async fn next(&mut self) -> Option<Frame> {
            None
        }
    }

    /// A frame source with nothing cached — models the moment right after a
    /// stream clear, when `latest()` briefly returns None.
    struct NoFrames;

    impl FrameSource for NoFrames {
        fn subscribe(&self, _udid: &str) -> Box<dyn FrameStream> {
            Box::new(EmptyStream)
        }

        fn latest(&self, _udid: &str) -> Option<Frame> {
            None
        }
    }

    struct TestFrames {
        current: Mutex<Frame>,
        feed: Frame,
        followed: Frame,
        open: Frame,
        armed: Frame,
        posted: Frame,
        /// When set, sending replays the pre-typing drawer byte for byte —
        /// what a stream running a second behind hands back.
        replay_stale_drawer: bool,
    }

    impl TestFrames {
        fn new() -> Self {
            Self::with_stale_replay(false)
        }

        fn with_stale_replay(replay_stale_drawer: bool) -> Self {
            let feed = Arc::new(include_bytes!("../../tests/fixtures/feed-iphone8.jpg").to_vec());
            let mut followed_image = image::load_from_memory(&feed)
                .expect("decode feed fixture")
                .to_rgb8();
            let rail = screen::locate_action_rail(&followed_image).expect("fixture rail");
            let (width, height) = followed_image.dimensions();
            let x0 = ((rail.x - 0.045) * f64::from(width)).max(0.0) as u32;
            let x1 = ((rail.x + 0.045) * f64::from(width)).min(f64::from(width - 1)) as u32;
            let y0 = ((rail.follow_y - 0.028) * f64::from(height)).max(0.0) as u32;
            let y1 =
                ((rail.follow_y + 0.028) * f64::from(height)).min(f64::from(height - 1)) as u32;
            for y in y0..=y1 {
                for x in x0..=x1 {
                    followed_image.put_pixel(x, y, Rgb([24, 24, 24]));
                }
            }
            let followed = encode_frame(followed_image);
            let open = encode_frame(drawer_frame(false, false));
            let armed = encode_frame(drawer_frame(true, false));
            // The drawer after a send is not the drawer before typing: the
            // comment that was just posted is now in the list. Reusing one
            // frame for both is what let the happy-path test pass without the
            // send proving anything.
            let posted = encode_frame(drawer_frame(false, true));
            Self {
                current: Mutex::new(feed.clone()),
                feed,
                followed,
                open,
                armed,
                posted,
                replay_stale_drawer,
            }
        }

        fn show_feed(&self) {
            *self.current.lock() = self.feed.clone();
        }

        fn show_open(&self) {
            *self.current.lock() = self.open.clone();
        }

        fn show_followed(&self) {
            *self.current.lock() = self.followed.clone();
        }

        fn show_armed(&self) {
            *self.current.lock() = self.armed.clone();
        }

        fn show_posted(&self) {
            let frame = if self.replay_stale_drawer {
                self.open.clone()
            } else {
                self.posted.clone()
            };
            *self.current.lock() = frame;
        }

        fn is_feed(&self) -> bool {
            Arc::ptr_eq(&self.current.lock(), &self.feed)
        }
    }

    impl FrameSource for TestFrames {
        fn subscribe(&self, _udid: &str) -> Box<dyn FrameStream> {
            Box::new(EmptyStream)
        }

        fn latest(&self, _udid: &str) -> Option<Frame> {
            Some(self.current.lock().clone())
        }
    }

    /// Replays a scripted sequence of real captures, one per `latest()` call,
    /// then holds on the last. `watch_swipe` decodes only frames whose digest
    /// it has not already seen, so the script is consumed at the rate the
    /// detector genuinely samples the stream.
    struct ScriptedFrames {
        script: Mutex<std::collections::VecDeque<Frame>>,
        last: Mutex<Frame>,
    }

    impl ScriptedFrames {
        /// Frames the detector has not reached yet. Tests assert on this
        /// rather than on elapsed time: a debug build decodes a 1 MP JPEG far
        /// slower than [`SWIPE_POLL`], so a wall-clock window that is generous
        /// in release can expire after one frame here and pass for the wrong
        /// reason.
        fn unread(&self) -> usize {
            self.script.lock().len()
        }
    }

    impl ScriptedFrames {
        fn new(script: &[&'static [u8]]) -> Self {
            let queue: std::collections::VecDeque<Frame> = script
                .iter()
                .map(|bytes| Arc::new(bytes.to_vec()))
                .collect();
            let first = queue
                .front()
                .expect("scripted frames must not be empty")
                .clone();
            Self {
                // Only a placeholder: the first `latest()` pops the same frame
                // and overwrites it. Seeding from the front rather than popping
                // is what keeps script[0] from being swallowed.
                last: Mutex::new(first),
                script: Mutex::new(queue),
            }
        }
    }

    impl ScriptedFrames {
        /// The same script from frames a test *built*, rather than from fixtures on disk.
        ///
        /// Needed because [`Self::new`] takes `&'static [u8]`, which an `include_bytes!`
        /// fixture satisfies and a freshly encoded image cannot.
        fn from_frames(script: Vec<Vec<u8>>) -> Self {
            let queue: std::collections::VecDeque<Frame> =
                script.into_iter().map(Arc::new).collect();
            let first = queue
                .front()
                .expect("scripted frames must not be empty")
                .clone();
            Self {
                last: Mutex::new(first),
                script: Mutex::new(queue),
            }
        }
    }

    impl FrameSource for ScriptedFrames {
        fn subscribe(&self, _udid: &str) -> Box<dyn FrameStream> {
            Box::new(EmptyStream)
        }

        fn latest(&self, _udid: &str) -> Option<Frame> {
            let mut last = self.last.lock();
            if let Some(next) = self.script.lock().pop_front() {
                *last = next;
            }
            Some(last.clone())
        }
    }

    /// Three frames of one sponsored card, captured seconds apart with the
    /// same author, caption and counts. Their bytes — and so their digests —
    /// all differ, because the video kept playing.
    const SAME_CARD: [&[u8]; 3] = [
        include_bytes!("../../tests/fixtures/feed-same-card-1.jpg"),
        include_bytes!("../../tests/fixtures/feed-same-card-2.jpg"),
        include_bytes!("../../tests/fixtures/feed-same-card-3.jpg"),
    ];
    /// Records taps and nothing else, so a test can assert both where the tap
    /// landed and that no tap went out at all.
    #[derive(Default)]
    struct TapRecorder {
        taps: Mutex<Vec<TapPoint>>,
    }

    #[async_trait]
    impl UiSession for TapRecorder {
        async fn tap(&self, point: TapPoint) -> anyhow::Result<()> {
            self.taps.lock().push(point);
            Ok(())
        }

        async fn swipe(&self, _gesture: SwipeGesture) -> anyhow::Result<()> {
            Ok(())
        }

        async fn type_text(&self, _text: &str) -> anyhow::Result<()> {
            anyhow::bail!("not used")
        }

        async fn home(&self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn find_and_tap(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            anyhow::bail!("not used")
        }

        async fn assert_visible(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            anyhow::bail!("not used")
        }

        fn stream_url(&self) -> Option<String> {
            None
        }
    }

    const FEED: &[u8] = include_bytes!("../../tests/fixtures/feed-iphone8.jpg");
    const LIKED: &[u8] = include_bytes!("../../tests/fixtures/feed-heart-liked.jpg");
    const MID_SWIPE: &[u8] = include_bytes!("../../tests/fixtures/feed-mid-swipe.jpg");
    const LIVE_ROOM: &[u8] = include_bytes!("../../tests/fixtures/live-room-1.jpg");

    /// `posted` paints an extra neutral block in the comment list, standing in
    /// for the comment that was just published. It keeps the frame classifying
    /// as an open, unarmed drawer while making it a *different* frame from the
    /// one before typing — which is what the real screen does.
    fn drawer_frame(armed: bool, posted: bool) -> RgbImage {
        let mut img = RgbImage::from_pixel(750, 1334, Rgb([225, 225, 225]));
        if armed {
            paint_icon(&mut img, screen::SEND_BUTTON, Rgb([255, 35, 65]));
        }
        if posted {
            paint_icon(&mut img, (0.30, 0.70), Rgb([205, 205, 205]));
        }
        img
    }

    fn paint_icon(img: &mut RgbImage, point: (f64, f64), color: Rgb<u8>) {
        let cx = (point.0 * img.width() as f64) as i32;
        let cy = (point.1 * img.height() as f64) as i32;
        for y in (cy - 32).max(0)..(cy + 32).min(img.height() as i32) {
            for x in (cx - 34).max(0)..(cx + 34).min(img.width() as i32) {
                img.put_pixel(x as u32, y as u32, color);
            }
        }
    }

    fn encode_frame(img: RgbImage) -> Frame {
        let mut out = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(img)
            .write_to(&mut out, ImageFormat::Png)
            .expect("encode test frame");
        Arc::new(out.into_inner())
    }

    /// The screen class this fixture is built for.
    ///
    /// iPhone 8, because that is the only entry in `screen::CALIBRATED_LAYOUTS` and every
    /// geometry constant these tests hit-test against was measured on it. **Declared here
    /// rather than left to a fallback**: the mock did not implement `window_size` at all, so
    /// the production code's `unwrap_or((375.0, 667.0))` was silently supplying it. Replacing
    /// that fallback with a refusal turned both thread-comment tests red, which is how anyone
    /// learned they had been running against fabricated geometry.
    ///
    /// A mock that states its screen is a mock whose taps mean something. Changing this to an
    /// Android size would need every constant in `screen.rs` re-measured for that class first
    /// -- which is exactly what `calibrated_layout` returning `None` is there to insist on.
    const MOCK_SCREEN: (f64, f64) = (375.0, 667.0);

    struct RecordingSession {
        frames: Arc<TestFrames>,
        stop: Arc<AtomicBool>,
        arm_on_type: bool,
        stop_on_type: bool,
        supports_text_input: bool,
        open_with_existing_draft: bool,
        type_error: bool,
        send_error: bool,
        dismiss_succeeds: bool,
        follow_success: bool,
        ordinary_taps: AtomicUsize,
        send_taps: AtomicUsize,
        native_taps: Mutex<Vec<TapPoint>>,
        typed: Mutex<Vec<String>>,
    }

    impl RecordingSession {
        fn new(
            frames: Arc<TestFrames>,
            stop: Arc<AtomicBool>,
            arm_on_type: bool,
            stop_on_type: bool,
        ) -> Self {
            Self {
                frames,
                stop,
                arm_on_type,
                stop_on_type,
                supports_text_input: true,
                open_with_existing_draft: false,
                type_error: false,
                send_error: false,
                dismiss_succeeds: true,
                follow_success: false,
                ordinary_taps: AtomicUsize::new(0),
                send_taps: AtomicUsize::new(0),
                native_taps: Mutex::new(Vec::new()),
                typed: Mutex::new(Vec::new()),
            }
        }

        fn without_text_input(mut self) -> Self {
            self.supports_text_input = false;
            self
        }

        fn with_existing_draft(mut self) -> Self {
            self.open_with_existing_draft = true;
            self
        }

        fn following_succeeds(mut self) -> Self {
            self.follow_success = true;
            self
        }

        fn failing_type(mut self) -> Self {
            self.type_error = true;
            self
        }

        fn failing_send(mut self) -> Self {
            self.send_error = true;
            self
        }

        fn with_unverified_dismiss(mut self) -> Self {
            self.dismiss_succeeds = false;
            self
        }

        fn near(point: &TapPoint, expected: (f64, f64)) -> bool {
            // Production taps are deliberately sampled inside the control's
            // safe hitbox, so the fixture must model a hit area rather than
            // requiring the old single-pixel center.
            //
            // Anchored to `MOCK_SCREEN` rather than to bare literals, so the fixture and the
            // size the mock reports cannot drift apart. They used to: the literals were here
            // and `window_size()` was not implemented at all, which is why these tests were
            // silently exercising the production fallback.
            (point.x - MOCK_SCREEN.0 * expected.0).abs() < 20.0
                && (point.y - MOCK_SCREEN.1 * expected.1).abs() < 20.0
        }
    }

    #[async_trait]
    impl UiSession for RecordingSession {
        /// Report a measured size, because the subject of these tests is evidence, not geometry.
        ///
        /// **These two tests used to pass because of a bug.** `UiSession::window_size` has a
        /// default impl that bails, this mock did not override it, and the send paths wrote
        /// `unwrap_or((375.0, 667.0))` -- so every assertion below ran against fabricated
        /// iPhone 8 geometry. When the fabrication was replaced with a refusal (27/08/2026)
        /// both tests went red, which is the only reason anyone found out they had been
        /// exercising the fallback rather than the path a real phone takes.
        ///
        /// The value is `MOCK_SCREEN` -- the same constant the hit tests use, so the fixture
        /// cannot disagree with itself about what screen it is modelling.
        async fn window_size(&self) -> anyhow::Result<(f64, f64)> {
            Ok(MOCK_SCREEN)
        }

        async fn tap(&self, point: TapPoint) -> anyhow::Result<()> {
            if !self.supports_text_input && Self::near(&point, screen::DRAWER_EMOJI_ICON) {
                anyhow::bail!("emoji fallback reached");
            }
            if self.send_error && Self::near(&point, screen::SEND_BUTTON) {
                self.send_taps.fetch_add(1, Ordering::Relaxed);
                anyhow::bail!("send failed after touch");
            }
            if Self::near(&point, screen::DRAWER_DISMISS) {
                if self.dismiss_succeeds {
                    self.frames.show_feed();
                }
            } else if Self::near(&point, screen::SEND_BUTTON) {
                self.send_taps.fetch_add(1, Ordering::Relaxed);
                self.frames.show_posted();
            } else if self.follow_success {
                self.ordinary_taps.fetch_add(1, Ordering::Relaxed);
                self.frames.show_followed();
            } else if self.ordinary_taps.fetch_add(1, Ordering::Relaxed) == 0 {
                if self.open_with_existing_draft {
                    self.frames.show_armed();
                } else {
                    self.frames.show_open();
                }
            }
            Ok(())
        }

        async fn tap_native(&self, point: TapPoint) -> anyhow::Result<()> {
            self.native_taps.lock().push(point);
            Ok(())
        }

        async fn swipe(&self, _gesture: crate::types::SwipeGesture) -> anyhow::Result<()> {
            Ok(())
        }

        async fn type_text(&self, text: &str) -> anyhow::Result<()> {
            self.typed.lock().push(text.to_string());
            if self.type_error {
                anyhow::bail!("keys failed after request");
            }
            let input_was_focused = !self.native_taps.lock().is_empty()
                || self.ordinary_taps.load(Ordering::Relaxed) >= 2;
            if self.arm_on_type && input_was_focused {
                self.frames.show_armed();
            }
            if self.stop_on_type {
                self.stop.store(true, Ordering::Relaxed);
            }
            Ok(())
        }

        fn supports_text_input(&self) -> bool {
            self.supports_text_input
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
    }

    struct UnusedDriver;

    #[async_trait]
    impl DeviceDriver for UnusedDriver {
        async fn list_devices(&self) -> anyhow::Result<Vec<DeviceInfo>> {
            unreachable!("driver is not used by do_comment")
        }

        async fn refresh_device(&self, _udid: &str) -> anyhow::Result<DeviceInfo> {
            unreachable!("driver is not used by do_comment")
        }

        async fn install_app(&self, _udid: &str, _path: &Path) -> anyhow::Result<()> {
            unreachable!("driver is not used by do_comment")
        }

        async fn uninstall_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
            unreachable!("driver is not used by do_comment")
        }

        async fn screenshot(&self, _udid: &str, _dest: &Path) -> anyhow::Result<PathBuf> {
            unreachable!("driver is not used by do_comment")
        }

        async fn syslog_tail(&self, _udid: &str, _lines: usize) -> anyhow::Result<String> {
            unreachable!("driver is not used by do_comment")
        }

        async fn launch_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
            unreachable!("driver is not used by do_comment")
        }

        async fn terminate_app(
            &self,
            _udid: &str,
            _bundle_id: &str,
        ) -> anyhow::Result<crate::ProcessAbsenceProof> {
            unreachable!("driver is not used by do_comment")
        }

        async fn reboot(&self, _udid: &str) -> anyhow::Result<()> {
            unreachable!("driver is not used by do_comment")
        }

        async fn start_ui_session(&self, _udid: &str) -> anyhow::Result<Box<dyn UiSession>> {
            unreachable!("driver is not used by do_comment")
        }

        async fn ensure_stream(&self, _udid: &str) -> anyhow::Result<String> {
            unreachable!("driver is not used by do_comment")
        }

        async fn prepare_device(&self, _udid: &str) -> anyhow::Result<()> {
            unreachable!("driver is not used by do_comment")
        }
    }

    fn test_engine(frames: Arc<TestFrames>) -> (NurtureEngine, PathBuf) {
        test_engine_from(frames)
    }

    fn test_engine_from(frames: Arc<dyn FrameSource>) -> (NurtureEngine, PathBuf) {
        let db_path =
            std::env::temp_dir().join(format!("riviu-core-comment-test-{}.db", Uuid::new_v4()));
        let control = Arc::new(crate::DeviceControlPlane::new(
            Arc::new(UnusedDriver),
            Arc::new(crate::DeviceWorkCoordinator::new()),
            Arc::new(crate::StreamBudgetManager::default()),
        ));
        let engine = NurtureEngine::new(
            Arc::new(Database::open(&db_path).expect("test db")),
            control,
            frames,
            std::env::temp_dir(),
        );
        (engine, db_path)
    }

    #[tokio::test]
    async fn a_swipe_without_a_before_frame_is_not_counted_as_an_advance() {
        // Right after a stream clear the frame cache is empty. A swipe with no
        // before-frame has no evidence the feed moved and must not be counted.
        let stop = Arc::new(AtomicBool::new(false));
        let session =
            RecordingSession::new(Arc::new(TestFrames::new()), stop.clone(), false, false);
        let (engine, db_path) = test_engine_from(Arc::new(NoFrames));

        let outcome = engine
            .do_swipe(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                (375.0, 667.0),
                200,
                stop.as_ref(),
            )
            .await
            .expect("swipe");

        assert_eq!(
            outcome,
            SwipeOutcome::Moved,
            "a swipe with no before-frame must not count as a confirmed advance, \
             and must not be repeated either — the gesture already went out"
        );
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn a_playing_video_on_the_same_card_is_not_an_advance() {
        // The regression this whole change exists for. All three frames are the
        // same sponsored card; the old check compared whole-frame digests, and
        // because a playing video changes every frame it reported a confirmed
        // advance for a feed that never moved.
        let stop = Arc::new(AtomicBool::new(false));
        let session =
            RecordingSession::new(Arc::new(TestFrames::new()), stop.clone(), false, false);
        // The first frame is consumed as the pre-swipe baseline, the rest by
        // the watch.
        let frames = Arc::new(ScriptedFrames::new(&[
            SAME_CARD[0],
            SAME_CARD[0],
            SAME_CARD[1],
            SAME_CARD[2],
        ]));
        let digests: Vec<u64> = SAME_CARD.iter().map(|f| frame_digest(f)).collect();
        assert!(
            digests[0] != digests[1] && digests[1] != digests[2],
            "fixtures must have differing digests or they cannot show the defect: {digests:?}"
        );
        let (engine, db_path) = test_engine_from(frames.clone());

        let outcome = engine
            .do_swipe(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                (375.0, 667.0),
                200,
                stop.as_ref(),
            )
            .await
            .expect("swipe");

        assert_eq!(
            outcome,
            SwipeOutcome::Blocked,
            "the rail never left, so the feed never moved — a changed frame is \
             the video playing, not a swipe landing"
        );
        assert_eq!(
            frames.unread(),
            0,
            "all three frames of the card must have been examined"
        );
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn the_rail_leaving_and_a_card_settling_is_an_advance() {
        // The structural signal: rail → no rail (mid-swipe) → rail on a settled
        // feed card. A playing video cannot produce the middle frame.
        let stop = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(ScriptedFrames::new(&[FEED, MID_SWIPE, SAME_CARD[0]]));
        let (engine, db_path) = test_engine_from(frames.clone());

        let outcome = engine
            .watch_swipe(UDID, SWIPE_SETTLE, stop.as_ref(), true)
            .await;

        assert_eq!(outcome, SwipeOutcome::Advanced);
        assert_eq!(
            frames.unread(),
            0,
            "the settled card must have been reached"
        );
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn a_rail_that_leaves_without_settling_is_not_counted_but_is_not_repeated() {
        // Mid-swipe and nothing after it: the gesture landed, so swiping again
        // would skip whatever is arriving, but no card settled to count.
        let stop = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(ScriptedFrames::new(&[MID_SWIPE]));
        let (engine, db_path) = test_engine_from(frames.clone());

        let outcome = engine
            .watch_swipe(UDID, Duration::from_millis(800), stop.as_ref(), true)
            .await;

        assert_eq!(outcome, SwipeOutcome::Moved);
        assert_eq!(
            frames.unread(),
            0,
            "the mid-swipe frame must have been read"
        );
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn landing_in_a_live_room_does_not_settle_as_a_feed_card() {
        // A LIVE room has no compose bar and no icon chain, so it must not pass
        // for a new feed card however long it is on screen. Guards the settle
        // check against being weakened to "the rail came back".
        let stop = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(ScriptedFrames::new(&[MID_SWIPE, LIVE_ROOM]));
        let (engine, db_path) = test_engine_from(frames.clone());

        let outcome = engine
            .watch_swipe(UDID, SWIPE_SETTLE, stop.as_ref(), true)
            .await;

        assert_eq!(outcome, SwipeOutcome::Moved);
        assert_eq!(
            frames.unread(),
            0,
            "the LIVE frame must have been read and rejected, not merely unread"
        );
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn leaving_a_card_that_has_no_rail_is_proven_by_the_rail_arriving() {
        // LIVE previews and carousels have no rail, so "the rail left" is not
        // available. Going from no rail to a settled card that has one is the
        // same card change read from the other side.
        let stop = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(ScriptedFrames::new(&[LIVE_ROOM, FEED]));
        let (engine, db_path) = test_engine_from(frames.clone());

        let outcome = engine
            .watch_swipe(UDID, SWIPE_SETTLE, stop.as_ref(), false)
            .await;

        assert_eq!(outcome, SwipeOutcome::Advanced);
        assert_eq!(frames.unread(), 0);
        let _ = std::fs::remove_file(db_path);
    }

    /// `do_like` had no test at all before this. These four cover the guards it
    /// gained, and each asserts on the taps that went out — a like that reports
    /// the right verdict while still touching the screen is not a pass.
    async fn like_on(script: &[&'static [u8]]) -> (LikeResult, Vec<TapPoint>) {
        let stop = Arc::new(AtomicBool::new(false));
        let session = TapRecorder::default();
        let (engine, db_path) = test_engine_from(Arc::new(ScriptedFrames::new(script)));
        let result = engine
            .do_like(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                (375.0, 667.0),
                stop.as_ref(),
            )
            .await
            .expect("like");
        let taps = session.taps.lock().clone();
        let _ = std::fs::remove_file(db_path);
        (result, taps)
    }

    #[tokio::test]
    async fn an_already_liked_card_is_left_alone() {
        // Tapping a filled heart un-likes it, so this must not reach the screen.
        let (result, taps) = like_on(&[LIKED]).await;

        assert_eq!(result, LikeResult::AlreadyLiked);
        assert!(
            taps.is_empty(),
            "an already-liked heart was tapped: {taps:?}"
        );
    }

    #[tokio::test]
    async fn a_live_room_is_never_tapped_for_a_like() {
        // The heart coordinates land on LIVE room chrome. `do_follow` has always
        // required an actionable feed here; the like path did not.
        let (result, taps) = like_on(&[LIVE_ROOM]).await;

        assert_eq!(result, LikeResult::NotOnFeed);
        assert!(taps.is_empty(), "tapped inside a LIVE room: {taps:?}");
    }

    #[tokio::test]
    async fn a_frame_with_no_locatable_rail_is_never_tapped() {
        // Mid-swipe still classifies as a feed — the compose bar is visible —
        // but the rail is half faded out and cannot be located. Tapping the
        // last known coordinates here is the "14 in a row for 0 likes" failure.
        let (result, taps) = like_on(&[MID_SWIPE]).await;

        assert_eq!(result, LikeResult::NotOnFeed);
        assert!(taps.is_empty(), "tapped a frame with no rail: {taps:?}");
    }

    #[tokio::test]
    async fn a_like_is_confirmed_from_the_heart_the_tap_aimed_at() {
        // Unliked card, then the same card with the heart filled.
        let (result, taps) = like_on(&[FEED, FEED, LIKED]).await;

        assert_eq!(result, LikeResult::Liked);
        assert_eq!(taps.len(), 1, "exactly one tap: {taps:?}");
        // 629 px of a 1334 px frame is 314.5 pt, and the planner stays inside
        // the heart's (10, 12) radius.
        let tap = &taps[0];
        assert!(
            (tap.y - 314.5).abs() <= 12.0,
            "tap landed at y={:.1} pt, not on the located heart",
            tap.y
        );
        assert!(
            (tap.x - 375.0 * screen::RAIL_X).abs() <= 10.0,
            "tap landed at x={:.1} pt, not in the rail column",
            tap.x
        );
    }

    #[tokio::test]
    async fn a_heart_that_never_fills_is_reported_with_both_readings() {
        // The stream keeps serving the same unliked card. A miss has to carry
        // the numbers, or it is indistinguishable from a slow heart.
        let (result, _taps) = like_on(&[FEED]).await;

        let LikeResult::NotConfirmed { before, best } = result else {
            panic!("expected NotConfirmed, got {result:?}");
        };
        assert!(
            before < screen::LIKE_FILLED_REDNESS && best < screen::LIKE_FILLED_REDNESS,
            "before={before:.1} best={best:.1}"
        );
    }

    /// The photo-post test the page dots could not carry.
    ///
    /// A photo post publishes byte-identical frames because nothing moves. A
    /// video cannot: the stream re-encodes every frame at 24 FPS and does not
    /// deduplicate, which is the same fact that made the old swipe check
    /// worthless and makes this one work.
    #[tokio::test]
    async fn a_card_whose_frames_never_change_reads_as_a_still_post() {
        let stop = Arc::new(AtomicBool::new(false));
        // The same frame handed back for the whole window.
        let frames = Arc::new(ScriptedFrames::new(&[FEED]));
        let (engine, db_path) = test_engine_from(frames);

        assert!(engine.card_is_still(UDID, stop.as_ref()).await);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn a_card_whose_frames_change_is_not_a_still_post() {
        // Three frames of one *playing* card. They differ, which is exactly why
        // they cannot be a photo post — and why they were useless as proof of a
        // swipe.
        let stop = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(ScriptedFrames::new(&[
            SAME_CARD[0],
            SAME_CARD[1],
            SAME_CARD[2],
        ]));
        let (engine, db_path) = test_engine_from(frames);

        assert!(!engine.card_is_still(UDID, stop.as_ref()).await);
        let _ = std::fs::remove_file(db_path);
    }

    /// **The bug that made this check useless on a real phone.**
    ///
    /// It used to hash the whole encoded frame, and the phone's own status bar is part of that.
    /// Measured 23/08/2026 on ce0717171c2a64d50d: three samples of a genuinely still photo post
    /// (`Hynxy ở Nha Trang · Photo`) differed by 185, 267 and 82 sampled pixels, and every one
    /// of them sat inside y 16..49 — the animated network icon. Below that line the frames were
    /// pixel-identical. Pushed through minicap's own pipeline (half of each edge, JPEG `-Q 70`)
    /// the difference survived: 83,113 / 83,201 / 83,212 bytes, and `frame_digest` differed on
    /// all three pairs. So on that phone no photo post could ever be recognised.
    ///
    /// The block below sits at y 8..32 of a 1334-tall frame, i.e. inside the top 4% the digest
    /// ignores, and it moves between samples exactly as the icon does.
    #[tokio::test]
    async fn a_ticking_status_bar_does_not_make_a_still_card_move() {
        let stop = Arc::new(AtomicBool::new(false));
        let corner = |y: u32| {
            let mut image = image::load_from_memory(FEED).expect("fixture").to_rgb8();
            for dy in 0..24 {
                for dx in 0..120 {
                    image.put_pixel(500 + dx, y + dy, Rgb([255, 255, 255]));
                }
            }
            let mut out = Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(image)
                .write_to(&mut out, ImageFormat::Jpeg)
                .expect("encode");
            out.into_inner()
        };
        // 8, 16, 20 — all three blocks end at y=43 at the latest, inside the JPEG block row
        // 40..47. **This alignment is the test, not decoration:** JPEG quantises in 8x8 blocks,
        // so a block painted down to y=51 sits in the row 48..55 and its quantisation error
        // lands on y=53..55 — below the 4% line — which made an earlier version of this test
        // fail for a reason that had nothing to do with the status bar.
        let (a, b, c) = (corner(8), corner(16), corner(20));
        let frames = Arc::new(ScriptedFrames::from_frames(vec![a.clone(), b, c, a]));
        let (engine, db_path) = test_engine_from(frames);

        assert!(
            engine.card_is_still(UDID, stop.as_ref()).await,
            "only the status bar changed, so the card did not move"
        );
        let _ = std::fs::remove_file(db_path);
    }

    /// And the other half: a change in the picture is still a change.
    ///
    /// Without this, "ignore the top 4%" could quietly become "ignore everything" and the
    /// carousel traversal would start running on videos, where a horizontal swipe navigates
    /// off the feed.
    #[tokio::test]
    async fn a_change_below_the_status_bar_is_still_a_moving_card() {
        let stop = Arc::new(AtomicBool::new(false));
        let mid = |y: u32| {
            let mut image = image::load_from_memory(FEED).expect("fixture").to_rgb8();
            for dy in 0..24 {
                for dx in 0..120 {
                    image.put_pixel(500 + dx, y + dy, Rgb([255, 255, 255]));
                }
            }
            let mut out = Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(image)
                .write_to(&mut out, ImageFormat::Jpeg)
                .expect("encode");
            out.into_inner()
        };
        let (a, b) = (mid(600), mid(900));
        let frames = Arc::new(ScriptedFrames::from_frames(vec![
            a.clone(),
            b.clone(),
            a,
            b,
        ]));
        let (engine, db_path) = test_engine_from(frames);

        assert!(!engine.card_is_still(UDID, stop.as_ref()).await);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn a_stopped_session_never_reports_a_still_card() {
        let stop = Arc::new(AtomicBool::new(true));
        let frames = Arc::new(ScriptedFrames::new(&[FEED]));
        let (engine, db_path) = test_engine_from(frames);

        assert!(
            !engine.card_is_still(UDID, stop.as_ref()).await,
            "a stop must not be reported as a still card, or the session would \
             start a horizontal swipe on its way out"
        );
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn pixel_follow_refuses_without_author_profile_hierarchy_proof() {
        let frames = Arc::new(TestFrames::new());
        let stop = Arc::new(AtomicBool::new(false));
        let session = RecordingSession::new(frames.clone(), stop.clone(), false, false);
        let (engine, db_path) = test_engine(frames);
        let engine = engine.with_frame_text_source(Arc::new(SequenceCardText {
            calls: AtomicUsize::new(0),
        }));

        let verdict = engine
            .do_follow(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                &ActionRail::fallback(),
                (375.0, 667.0),
                stop.as_ref(),
            )
            .await
            .expect("classified follow");

        assert_eq!(verdict, FollowResult::SourceUnavailable);
        assert_eq!(session.ordinary_taps.load(Ordering::Relaxed), 0);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn pixel_follow_does_not_tap_even_when_ocr_author_stays_the_same() {
        let frames = Arc::new(TestFrames::new());
        let stop = Arc::new(AtomicBool::new(false));
        let session =
            RecordingSession::new(frames.clone(), stop.clone(), false, false).following_succeeds();
        let (engine, db_path) = test_engine(frames);
        let engine = engine.with_frame_text_source(Arc::new(FixedCardText));

        let verdict = engine
            .do_follow(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                &ActionRail::fallback(),
                (375.0, 667.0),
                stop.as_ref(),
            )
            .await
            .expect("follow");

        assert_eq!(verdict, FollowResult::SourceUnavailable);
        assert_eq!(session.ordinary_taps.load(Ordering::Relaxed), 0);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn pixel_follow_refuses_before_a_non_feed_confirmation_can_mislead_it() {
        // The real feed fixture shows a red follow badge; tapping follow flips
        // the mock to a grey drawer frame — badge gone but NOT an actionable
        // feed, exactly like the SIM-less activation alert that dims the screen.
        // "Badge absent" alone must no longer count as a follow.
        let frames = Arc::new(TestFrames::new());
        let feed_img =
            image::load_from_memory(&FrameSource::latest(frames.as_ref(), UDID).expect("feed"))
                .expect("decode feed")
                .to_rgb8();
        assert!(
            screen::follow_badge_present(&feed_img, &ActionRail::fallback()),
            "fixture precondition: the feed frame must show a red follow badge"
        );

        let stop = Arc::new(AtomicBool::new(false));
        let session = RecordingSession::new(frames.clone(), stop.clone(), false, false);
        let (engine, db_path) = test_engine(frames);
        let engine = engine.with_frame_text_source(Arc::new(FixedCardText));

        let confirmed = engine
            .do_follow(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                &ActionRail::fallback(),
                (375.0, 667.0),
                stop.as_ref(),
            )
            .await
            .expect("follow");

        assert_eq!(
            confirmed,
            FollowResult::SourceUnavailable,
            "pixel evidence cannot own a reversible Follow"
        );
        assert_eq!(session.ordinary_taps.load(Ordering::Relaxed), 0);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn audit_insert_failure_stops_before_opening_comment_ui() {
        let frames = Arc::new(TestFrames::new());
        let stop = Arc::new(AtomicBool::new(false));
        let session = RecordingSession::new(frames.clone(), stop.clone(), true, false);
        let (engine, db_path) = test_engine(frames);
        rusqlite::Connection::open(&db_path)
            .expect("open audit database")
            .execute_batch(
                "CREATE TRIGGER fail_nurture_comment_attempt_insert \
                 BEFORE INSERT ON nurture_comment_attempts \
                 BEGIN SELECT RAISE(FAIL, 'forced'); END;",
            )
            .expect("install failing audit trigger");

        let result = engine
            .do_comment(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                &ActionRail::fallback(),
                (375.0, 667.0),
                &NurtureSettings::default(),
                &[COMMENT.to_string()],
                stop.as_ref(),
            )
            .await
            .expect("audit failure is a classified no-op");

        assert_eq!(result, CommentResult::AuditUnavailable);
        assert!(!result.did_act());
        assert_eq!(session.ordinary_taps.load(Ordering::Relaxed), 0);
        assert_eq!(session.send_taps.load(Ordering::Relaxed), 0);
        assert!(session.typed.lock().is_empty());
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn hierarchy_source_returns_no_text_when_write_ahead_audit_fails() {
        let frames = Arc::new(TestFrames::new());
        let (engine, db_path) = test_engine(frames);
        rusqlite::Connection::open(&db_path)
            .expect("open audit database")
            .execute_batch(
                "CREATE TRIGGER fail_nurture_comment_attempt_insert \
                 BEFORE INSERT ON nurture_comment_attempts \
                 BEGIN SELECT RAISE(FAIL, 'forced'); END;",
            )
            .expect("install failing audit trigger");
        let attempt = NurtureCommentAttempt {
            id: "hierarchy-audit-test".into(),
            udid: UDID.into(),
            outcome: "prepared".into(),
            source: "test-fixture".into(),
            model: "test".into(),
            base_url_host: "localhost".into(),
            prompt_tokens: 1,
            completion_tokens: 1,
            cost_usd: None,
            preview: COMMENT.into(),
            caption_preview: String::new(),
            frame_sha256: String::new(),
            context_confidence: None,
            relevance: None,
            evidence_support: None,
            distinct_frames: None,
            carousel_slides: Some(0),
            created_at: Utc::now().to_rfc3339(),
        };

        assert!(
            engine
                .persist_hierarchy_comment(attempt, COMMENT.into(), 1, 1)
                .is_err(),
            "the hierarchy source must return no PreparedComment unless the audit row exists"
        );
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn grounded_comment_refuses_card_b_after_preparing_on_card_a() {
        let frames = Arc::new(TestFrames::new());
        let stop = Arc::new(AtomicBool::new(false));
        let session = RecordingSession::new(frames.clone(), stop, true, false);
        let (engine, db_path) = test_engine(frames);
        let engine = engine.with_frame_text_source(Arc::new(SequenceCardText {
            // Any nonzero initial value makes the fresh read identify creator_b.
            calls: AtomicUsize::new(1),
        }));
        let attempt_id = "grounded-card-test".to_string();
        engine
            .db
            .add_nurture_comment_attempt(&NurtureCommentAttempt {
                id: attempt_id.clone(),
                udid: UDID.into(),
                outcome: "prepared".into(),
                source: "test-fixture".into(),
                model: "test".into(),
                base_url_host: "localhost".into(),
                prompt_tokens: 0,
                completion_tokens: 0,
                cost_usd: None,
                preview: COMMENT.into(),
                caption_preview: String::new(),
                frame_sha256: String::new(),
                context_confidence: None,
                relevance: None,
                evidence_support: None,
                distinct_frames: None,
                carousel_slides: None,
                created_at: Utc::now().to_rfc3339(),
            })
            .expect("write prepared audit row");
        let prepared = PreparedTextComment {
            text: COMMENT.into(),
            model: "test".into(),
            base_url_host: "localhost".into(),
            prompt_tokens: 0,
            completion_tokens: 0,
            cost_usd: None,
            source: "test-fixture",
            frame_sha256: None,
            caption_preview: None,
            context_confidence: None,
            relevance: None,
            evidence_support: None,
            distinct_frames: None,
            grounded_on: Some(PixelCardIdentity {
                author: "creator_a".into(),
                caption: Some("caption chung".into()),
            }),
            attempt_id,
        };

        let result = engine
            .open_grounded_comment_drawer(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                (375.0, 667.0),
                &prepared,
            )
            .await
            .expect("classified card change");

        assert_eq!(result, Some(CommentResult::CardChanged));
        assert_eq!(session.ordinary_taps.load(Ordering::Relaxed), 0);
        assert_eq!(session.send_taps.load(Ordering::Relaxed), 0);
        assert!(session.typed.lock().is_empty());
        let attempts = engine
            .db
            .list_nurture_comment_attempts(10)
            .expect("read audit");
        assert_eq!(attempts[0].outcome, "skipped: card_changed");
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn grounded_comment_opens_card_a_after_reproving_card_a() {
        let frames = Arc::new(TestFrames::new());
        let stop = Arc::new(AtomicBool::new(false));
        let session = RecordingSession::new(frames.clone(), stop, true, false);
        let (engine, db_path) = test_engine(frames);
        let engine = engine.with_frame_text_source(Arc::new(FixedCardText));
        let prepared = PreparedTextComment {
            text: COMMENT.into(),
            model: "test".into(),
            base_url_host: "localhost".into(),
            prompt_tokens: 0,
            completion_tokens: 0,
            cost_usd: None,
            source: "test-fixture",
            frame_sha256: None,
            caption_preview: None,
            context_confidence: None,
            relevance: None,
            evidence_support: None,
            distinct_frames: None,
            grounded_on: Some(PixelCardIdentity {
                author: "creator_a".into(),
                caption: Some("caption chung".into()),
            }),
            attempt_id: "same-card-test".into(),
        };

        let result = engine
            .open_grounded_comment_drawer(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                (375.0, 667.0),
                &prepared,
            )
            .await
            .expect("re-prove same card");

        assert_eq!(result, None);
        assert_eq!(session.ordinary_taps.load(Ordering::Relaxed), 1);
        assert_eq!(session.send_taps.load(Ordering::Relaxed), 0);
        assert!(session.typed.lock().is_empty());
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn grounded_comment_refuses_a_new_caption_after_the_baseline_missed_it() {
        let frames = Arc::new(TestFrames::new());
        let stop = Arc::new(AtomicBool::new(false));
        let session = RecordingSession::new(frames.clone(), stop, true, false);
        let (engine, db_path) = test_engine(frames);
        let engine = engine.with_frame_text_source(Arc::new(CaptionedCardText));
        let attempt_id = "grounded-caption-test".to_string();
        engine
            .db
            .add_nurture_comment_attempt(&NurtureCommentAttempt {
                id: attempt_id.clone(),
                udid: UDID.into(),
                outcome: "prepared".into(),
                source: "test-fixture".into(),
                model: "test".into(),
                base_url_host: "localhost".into(),
                prompt_tokens: 0,
                completion_tokens: 0,
                cost_usd: None,
                preview: COMMENT.into(),
                caption_preview: String::new(),
                frame_sha256: String::new(),
                context_confidence: None,
                relevance: None,
                evidence_support: None,
                distinct_frames: None,
                carousel_slides: None,
                created_at: Utc::now().to_rfc3339(),
            })
            .expect("write prepared audit row");
        let prepared = PreparedTextComment {
            text: COMMENT.into(),
            model: "test".into(),
            base_url_host: "localhost".into(),
            prompt_tokens: 0,
            completion_tokens: 0,
            cost_usd: None,
            source: "test-fixture",
            frame_sha256: None,
            caption_preview: None,
            context_confidence: None,
            relevance: None,
            evidence_support: None,
            distinct_frames: None,
            grounded_on: Some(PixelCardIdentity {
                author: "creator_a".into(),
                caption: None,
            }),
            attempt_id,
        };

        let result = engine
            .open_grounded_comment_drawer(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                (375.0, 667.0),
                &prepared,
            )
            .await
            .expect("classified changed identity");

        assert_eq!(result, Some(CommentResult::CardChanged));
        assert_eq!(session.ordinary_taps.load(Ordering::Relaxed), 0);
        let attempts = engine
            .db
            .list_nurture_comment_attempts(10)
            .expect("read audit");
        assert_eq!(attempts[0].outcome, "skipped: card_changed");
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn pool_comment_is_typed_at_the_input_and_persisted_after_confirmed_send() {
        let frames = Arc::new(TestFrames::new());
        let stop = Arc::new(AtomicBool::new(false));
        let session = RecordingSession::new(frames.clone(), stop.clone(), true, false);
        let (engine, db_path) = test_engine(frames);

        let result = engine
            .do_comment(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                &ActionRail::fallback(),
                (375.0, 667.0),
                &NurtureSettings::default(),
                &[COMMENT.to_string()],
                stop.as_ref(),
            )
            .await
            .expect("comment flow");

        assert_eq!(session.typed.lock().as_slice(), &[COMMENT.to_string()]);
        assert!(
            session.native_taps.lock().is_empty(),
            "RT-MMO comment focus must use its stable sessionless swipe tap"
        );
        assert_eq!(
            session.ordinary_taps.load(Ordering::Relaxed),
            2,
            "one tap opens the drawer and one focuses its input"
        );
        assert_eq!(session.send_taps.load(Ordering::Relaxed), 1);
        assert_eq!(result.reason(), "đã gửi bình luận chữ");

        // Asserted on `nurture_comment_attempts` rather than on the costs table migration 16
        // dropped. Nothing is lost: the attempt row carried every field the cost row did.
        let attempts = engine
            .db
            .list_nurture_comment_attempts(10)
            .expect("comment attempts");
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].outcome, "sent");
        assert_eq!(attempts[0].preview, COMMENT);
        assert_eq!(attempts[0].source, "test-fixture");
        assert_eq!(
            (attempts[0].prompt_tokens, attempts[0].completion_tokens),
            (0, 0),
            "pool text spends no AI tokens per comment"
        );

        drop(engine);
        let _ = std::fs::remove_file(db_path);
    }

    /// The negative half of the test above, and the reason it can now fail at
    /// all.
    ///
    /// "Sent" and "ready to type" are the same classification — an open,
    /// unarmed drawer — separated only by when they were observed. So a stream
    /// running behind, replaying the pre-typing drawer after the send, satisfies
    /// the post-send check exactly. The old mock did precisely that: tapping
    /// Send reinstated the pre-typing frame, so the happy-path assertion held
    /// no matter what the send did.
    ///
    /// Excluding the frames already seen is what separates the two. It does not
    /// make the signal *distinguishing* — that still needs a captured
    /// comment-posted screen — but it does stop an already-seen frame proving a
    /// later effect.
    #[tokio::test]
    async fn a_replayed_pre_typing_drawer_is_not_proof_the_comment_posted() {
        let frames = Arc::new(TestFrames::with_stale_replay(true));
        let stop = Arc::new(AtomicBool::new(false));
        let session = RecordingSession::new(frames.clone(), stop.clone(), true, false);
        let (engine, db_path) = test_engine(frames);

        let result = engine
            .do_comment(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                &ActionRail::fallback(),
                (375.0, 667.0),
                &NurtureSettings::default(),
                &[COMMENT.to_string()],
                stop.as_ref(),
            )
            .await
            .expect("comment flow");

        assert_eq!(session.send_taps.load(Ordering::Relaxed), 1);
        assert_eq!(result, CommentResult::TextNotSent);
        // And it must stay non-retryable: the Send tap is a side effect that
        // has already gone out, so retrying posts twice.
        assert_eq!(
            super::super::TextCommentHealth::default().observe(result),
            super::super::CommentRecoveryAction::DoNotRetry
        );
        let attempts = engine
            .db
            .list_nurture_comment_attempts(10)
            .expect("comment attempts");
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].outcome, "text_uncertain");
        // The costs table this used to read was written on the `"sent"` path only, so
        // "nothing was billed" is exactly "no attempt reports `sent`" -- which the assertion
        // above already states more precisely.
        assert_ne!(
            attempts[0].outcome, "sent",
            "an unconfirmed send must not be recorded as a posted comment"
        );

        drop(engine);
        let _ = std::fs::remove_file(db_path);
    }

    /// The link-driven Interaction surface posts through
    /// `send_prepared_thread_comment`, which had no test at all and carried the
    /// same defect the nurture comment path did: "ready to type" and "sent" are
    /// both an open, unarmed drawer, so a stream running behind could replay the
    /// pre-typing screen as proof the field emptied.
    ///
    /// It matters more here. This path *persists* the confirming frame's digest
    /// as `cleared_frame_sha256`, so an unexcluded replay writes the pre-typing
    /// screen into the campaign record as evidence the comment posted.
    async fn thread_comment_on(
        stale_replay: bool,
    ) -> (
        Result<crate::interaction::ThreadSendEvidence, SendFailure>,
        Arc<TestFrames>,
    ) {
        let frames = Arc::new(TestFrames::with_stale_replay(stale_replay));
        let stop = Arc::new(AtomicBool::new(false));
        let session = RecordingSession::new(frames.clone(), stop.clone(), true, false);
        let (engine, db_path) = test_engine(frames.clone());
        let prepared = PreparedThreadMessage {
            ordinal: 1,
            actor_udid: UDID.to_string(),
            text: COMMENT.to_string(),
            text_sha256: "deadbeef".into(),
            parent_ordinal: None,
            mentions: Vec::new(),
        };

        let result = engine
            .send_prepared_thread_comment(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                &prepared,
                stop.as_ref(),
            )
            .await;
        drop(engine);
        let _ = std::fs::remove_file(db_path);
        (result, frames)
    }

    #[tokio::test]
    async fn a_thread_comment_is_confirmed_from_a_frame_that_postdates_the_send() {
        let (result, _) = thread_comment_on(false).await;

        let evidence = match result {
            Ok(evidence) => evidence,
            Err(failure) => panic!("thread comment failed: {}", failure.into_error()),
        };
        assert_eq!(evidence.text_sha256, "deadbeef");
        assert_ne!(
            evidence.cleared_frame_sha256, evidence.armed_frame_sha256,
            "the cleared frame must not be the armed one"
        );
    }

    #[tokio::test]
    async fn a_replayed_pre_typing_drawer_is_not_proof_a_thread_comment_posted() {
        let (result, _) = thread_comment_on(true).await;

        let error = result.expect_err("a replayed frame must not confirm the send");
        assert!(
            error.effect_may_have_gone_out(),
            "the replay is after the Send tap and must stay unretryable"
        );
        let error = error.into_error();
        assert!(
            error.to_string().contains("send_clear_not_confirmed"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn a_thread_transport_error_before_send_stays_retryable() {
        let frames = Arc::new(TestFrames::new());
        let stop = Arc::new(AtomicBool::new(false));
        let session =
            RecordingSession::new(frames.clone(), stop.clone(), true, false).failing_type();
        let (engine, db_path) = test_engine(frames);
        let prepared = PreparedThreadMessage {
            ordinal: 1,
            actor_udid: UDID.to_string(),
            text: COMMENT.to_string(),
            text_sha256: "before".into(),
            parent_ordinal: None,
            mentions: Vec::new(),
        };

        let failure = engine
            .send_prepared_thread_comment(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                &prepared,
                stop.as_ref(),
            )
            .await
            .expect_err("type transport error");
        assert!(!failure.effect_may_have_gone_out());
        assert_eq!(session.send_taps.load(Ordering::Relaxed), 0);
        drop(engine);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn a_thread_transport_error_at_send_stays_unretryable() {
        let frames = Arc::new(TestFrames::new());
        let stop = Arc::new(AtomicBool::new(false));
        let session =
            RecordingSession::new(frames.clone(), stop.clone(), true, false).failing_send();
        let (engine, db_path) = test_engine(frames);
        let prepared = PreparedThreadMessage {
            ordinal: 1,
            actor_udid: UDID.to_string(),
            text: COMMENT.to_string(),
            text_sha256: "after".into(),
            parent_ordinal: None,
            mentions: Vec::new(),
        };

        let failure = engine
            .send_prepared_thread_comment(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                &prepared,
                stop.as_ref(),
            )
            .await
            .expect_err("Send transport error");
        assert!(failure.effect_may_have_gone_out());
        assert_eq!(session.send_taps.load(Ordering::Relaxed), 1);
        drop(engine);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test(start_paused = true)]
    async fn a_denied_pixel_root_gate_cleans_the_typed_composer_without_sending() {
        let frames = Arc::new(TestFrames::new());
        let stop = Arc::new(AtomicBool::new(false));
        let session = RecordingSession::new(frames.clone(), stop.clone(), true, false);
        let (engine, db_path) = test_engine(frames.clone());
        let prepared = PreparedThreadMessage {
            ordinal: 1,
            actor_udid: UDID.to_string(),
            text: COMMENT.to_string(),
            text_sha256: "denied-root".into(),
            parent_ordinal: None,
            mentions: Vec::new(),
        };
        let mut gate = crate::interaction_target::EffectGate::new(|| Ok(false));

        let failure = engine
            .send_prepared_thread_comment_with_gate(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                &prepared,
                stop.as_ref(),
                &mut gate,
            )
            .await
            .expect_err("lost ownership aborts before Send");

        assert!(failure.ownership_lost());
        assert_eq!(session.send_taps.load(Ordering::Relaxed), 0);
        assert_eq!(session.typed.lock().as_slice(), &[COMMENT.to_string()]);
        let frame = frames.latest(UDID).expect("cleanup frame");
        let image = image::load_from_memory(&frame)
            .expect("decode cleanup frame")
            .to_rgb8();
        assert!(screen::feed_ready(&image, Some(MOCK_SCREEN.0)));
        drop(engine);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test(start_paused = true)]
    async fn an_unverified_pixel_gate_cleanup_is_after_effect() {
        let frames = Arc::new(TestFrames::new());
        let stop = Arc::new(AtomicBool::new(false));
        let session = RecordingSession::new(frames.clone(), stop.clone(), true, false)
            .with_unverified_dismiss();
        let (engine, db_path) = test_engine(frames);
        let prepared = PreparedThreadMessage {
            ordinal: 1,
            actor_udid: UDID.to_string(),
            text: COMMENT.to_string(),
            text_sha256: "dirty-root".into(),
            parent_ordinal: None,
            mentions: Vec::new(),
        };
        let mut gate = crate::interaction_target::EffectGate::new(|| {
            Err(anyhow!("effect gate database failure"))
        });

        let failure = engine
            .send_prepared_thread_comment_with_gate(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                &prepared,
                stop.as_ref(),
                &mut gate,
            )
            .await
            .expect_err("unverified cleanup cannot stay retryable");

        assert!(failure.effect_may_have_gone_out());
        assert_eq!(session.send_taps.load(Ordering::Relaxed), 0);
        drop(engine);
        let _ = std::fs::remove_file(db_path);
    }

    fn prepared_thread_reply() -> PreparedThreadMessage {
        PreparedThreadMessage {
            ordinal: 2,
            actor_udid: UDID.to_string(),
            text: COMMENT.to_string(),
            text_sha256: "reply-sha".into(),
            parent_ordinal: Some(1),
            mentions: Vec::new(),
        }
    }

    fn prepared_thread_root() -> PreparedThreadMessage {
        PreparedThreadMessage {
            ordinal: 1,
            actor_udid: UDID.to_string(),
            text: COMMENT.to_string(),
            text_sha256: "root-sha".into(),
            parent_ordinal: None,
            mentions: Vec::new(),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_crash_stale_pixel_root_draft_is_discarded_before_the_fresh_retry() {
        let frames = Arc::new(TestFrames::new());
        frames.show_armed();
        let stop = Arc::new(AtomicBool::new(false));
        let session = RecordingSession::new(frames.clone(), stop.clone(), true, false);
        let (engine, db_path) = test_engine(frames.clone());
        let gestures = tokio::sync::Mutex::new(());

        if let Err(failure) = engine
            .clear_stale_pixel_comment_ui(UDID, &session, &gestures, stop.as_ref())
            .await
        {
            panic!(
                "verified cleanup should let the assignment reopen and retry: {}",
                failure.into_error()
            );
        }
        let evidence = match engine
            .send_prepared_thread_comment(
                UDID,
                &session,
                &gestures,
                &prepared_thread_root(),
                stop.as_ref(),
            )
            .await
        {
            Ok(evidence) => evidence,
            Err(failure) => panic!("fresh root retry failed: {}", failure.into_error()),
        };

        assert_eq!(evidence.text_sha256, "root-sha");
        assert_eq!(session.typed.lock().as_slice(), &[COMMENT.to_string()]);
        assert_eq!(session.send_taps.load(Ordering::Relaxed), 1);
        drop(engine);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test(start_paused = true)]
    async fn an_unverified_crash_stale_pixel_root_cleanup_is_after_effect() {
        let frames = Arc::new(TestFrames::new());
        frames.show_armed();
        let stop = Arc::new(AtomicBool::new(false));
        let session = RecordingSession::new(frames.clone(), stop.clone(), true, false)
            .with_unverified_dismiss();
        let (engine, db_path) = test_engine(frames);

        let failure = engine
            .clear_stale_pixel_comment_ui(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                stop.as_ref(),
            )
            .await
            .expect_err("an armed draft that cannot be cleared is ambiguous");

        assert!(failure.effect_may_have_gone_out());
        assert!(session.typed.lock().is_empty());
        assert_eq!(session.send_taps.load(Ordering::Relaxed), 0);
        drop(engine);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test(start_paused = true)]
    async fn a_crash_stale_pixel_reply_draft_is_not_appended_to_on_retry() {
        let frames = Arc::new(TestFrames::new());
        frames.show_armed();
        let stop = Arc::new(AtomicBool::new(false));
        let session = RecordingSession::new(frames.clone(), stop.clone(), true, false);
        let (engine, db_path) = test_engine(frames);
        let gestures = tokio::sync::Mutex::new(());

        if let Err(failure) = engine
            .clear_stale_pixel_comment_ui(UDID, &session, &gestures, stop.as_ref())
            .await
        {
            panic!(
                "verified cleanup should let the caller re-prove the parent: {}",
                failure.into_error()
            );
        }
        match engine
            .send_prepared_thread_reply(
                UDID,
                &session,
                &gestures,
                TapPoint { x: 120.0, y: 300.0 },
                &prepared_thread_reply(),
                stop.as_ref(),
            )
            .await
        {
            Ok(_) => {}
            Err(failure) => panic!("fresh reply retry failed: {}", failure.into_error()),
        }

        assert_eq!(
            session.typed.lock().as_slice(),
            &[COMMENT.to_string()],
            "the fresh payload is the only text request; stale text is never appended"
        );
        assert_eq!(session.send_taps.load(Ordering::Relaxed), 1);
        drop(engine);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test(start_paused = true)]
    async fn an_unverified_crash_stale_pixel_reply_cleanup_is_after_effect() {
        let frames = Arc::new(TestFrames::new());
        frames.show_armed();
        let stop = Arc::new(AtomicBool::new(false));
        let session = RecordingSession::new(frames.clone(), stop.clone(), true, false)
            .with_unverified_dismiss();
        let (engine, db_path) = test_engine(frames);

        let failure = engine
            .clear_stale_pixel_comment_ui(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                stop.as_ref(),
            )
            .await
            .expect_err("the reply path must not enter a dirty composer");

        assert!(failure.effect_may_have_gone_out());
        assert!(session.typed.lock().is_empty());
        assert_eq!(session.send_taps.load(Ordering::Relaxed), 0);
        drop(engine);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test(start_paused = true)]
    async fn a_denied_pixel_reply_gate_cleans_the_typed_composer_without_sending() {
        let frames = Arc::new(TestFrames::new());
        let stop = Arc::new(AtomicBool::new(false));
        let session = RecordingSession::new(frames.clone(), stop.clone(), true, false);
        let (engine, db_path) = test_engine(frames.clone());
        let mut gate = crate::interaction_target::EffectGate::new(|| Ok(false));

        let failure = engine
            .send_prepared_thread_reply_with_gate(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                TapPoint { x: 120.0, y: 300.0 },
                &prepared_thread_reply(),
                stop.as_ref(),
                &mut gate,
            )
            .await
            .expect_err("lost ownership aborts before reply Send");

        assert!(failure.ownership_lost());
        assert_eq!(session.send_taps.load(Ordering::Relaxed), 0);
        assert_eq!(session.typed.lock().as_slice(), &[COMMENT.to_string()]);
        let frame = frames.latest(UDID).expect("cleanup frame");
        let image = image::load_from_memory(&frame)
            .expect("decode cleanup frame")
            .to_rgb8();
        assert!(screen::feed_ready(&image, Some(MOCK_SCREEN.0)));
        drop(engine);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn a_thread_reply_transport_error_before_send_stays_retryable() {
        let frames = Arc::new(TestFrames::new());
        let stop = Arc::new(AtomicBool::new(false));
        let session =
            RecordingSession::new(frames.clone(), stop.clone(), true, false).failing_type();
        let (engine, db_path) = test_engine(frames);

        let failure = engine
            .send_prepared_thread_reply(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                TapPoint { x: 120.0, y: 300.0 },
                &prepared_thread_reply(),
                stop.as_ref(),
            )
            .await
            .expect_err("reply type transport error");

        assert!(!failure.effect_may_have_gone_out());
        assert_eq!(session.send_taps.load(Ordering::Relaxed), 0);
        drop(engine);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn a_thread_reply_transport_error_at_send_stays_unretryable() {
        let frames = Arc::new(TestFrames::new());
        let stop = Arc::new(AtomicBool::new(false));
        let session =
            RecordingSession::new(frames.clone(), stop.clone(), true, false).failing_send();
        let (engine, db_path) = test_engine(frames);

        let failure = engine
            .send_prepared_thread_reply(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                TapPoint { x: 120.0, y: 300.0 },
                &prepared_thread_reply(),
                stop.as_ref(),
            )
            .await
            .expect_err("reply Send transport error");

        assert!(failure.effect_may_have_gone_out());
        assert_eq!(session.send_taps.load(Ordering::Relaxed), 1);
        drop(engine);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn a_thread_reply_clear_failure_after_send_stays_unretryable() {
        let frames = Arc::new(TestFrames::with_stale_replay(true));
        let stop = Arc::new(AtomicBool::new(false));
        let session = RecordingSession::new(frames.clone(), stop.clone(), true, false);
        let (engine, db_path) = test_engine(frames);

        let failure = engine
            .send_prepared_thread_reply(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                TapPoint { x: 120.0, y: 300.0 },
                &prepared_thread_reply(),
                stop.as_ref(),
            )
            .await
            .expect_err("a replayed pre-typing frame must not confirm the reply");

        assert!(failure.effect_may_have_gone_out());
        assert_eq!(session.send_taps.load(Ordering::Relaxed), 1);
        let error = failure.into_error();
        assert!(
            error.to_string().contains("reply_clear_not_confirmed"),
            "unexpected error: {error}"
        );
        drop(engine);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn missing_context_is_skipped_and_audited_before_opening_the_drawer() {
        let frames = Arc::new(TestFrames::new());
        let stop = Arc::new(AtomicBool::new(false));
        let session = RecordingSession::new(frames.clone(), stop.clone(), true, false);
        let (engine, db_path) = test_engine(frames);

        let result = engine
            .do_comment(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                &ActionRail::fallback(),
                (375.0, 667.0),
                &NurtureSettings::default(),
                &[],
                stop.as_ref(),
            )
            .await
            .expect("context skip");

        assert_eq!(result, CommentResult::ContextSkipped);
        assert_eq!(session.ordinary_taps.load(Ordering::Relaxed), 0);
        let attempts = engine
            .db
            .list_nurture_comment_attempts(10)
            .expect("comment attempts");
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].outcome, "context_skipped");

        drop(engine);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn keys_ack_without_an_armed_send_button_is_not_a_sent_comment() {
        let frames = Arc::new(TestFrames::new());
        let stop = Arc::new(AtomicBool::new(false));
        let session = RecordingSession::new(frames.clone(), stop.clone(), false, true);
        let (engine, db_path) = test_engine(frames);

        let result = engine
            .do_comment(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                &ActionRail::fallback(),
                (375.0, 667.0),
                &NurtureSettings::default(),
                &[COMMENT.to_string()],
                stop.as_ref(),
            )
            .await
            .expect("unarmed typing is a classified outcome");

        assert_eq!(session.typed.lock().as_slice(), &[COMMENT.to_string()]);
        assert_eq!(session.send_taps.load(Ordering::Relaxed), 0);
        assert_eq!(
            result.reason(),
            "đã gõ bình luận chữ nhưng nút gửi không sáng"
        );
        assert!(
            !engine
                .db
                .list_nurture_comment_attempts(10)
                .expect("comment attempts")
                .iter()
                .any(|attempt| attempt.outcome == "sent"),
            "a draft that never armed Send must not be recorded as sent"
        );

        drop(engine);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn an_existing_armed_draft_is_never_typed_over_or_sent() {
        let frames = Arc::new(TestFrames::new());
        let stop = Arc::new(AtomicBool::new(false));
        let session =
            RecordingSession::new(frames.clone(), stop.clone(), true, false).with_existing_draft();
        let (engine, db_path) = test_engine(frames.clone());

        let result = engine
            .do_comment(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                &ActionRail::fallback(),
                (375.0, 667.0),
                &NurtureSettings::default(),
                &[COMMENT.to_string()],
                stop.as_ref(),
            )
            .await
            .expect("an existing draft is a classified skip");

        assert!(
            session.typed.lock().is_empty(),
            "must not append to old text"
        );
        assert_eq!(session.send_taps.load(Ordering::Relaxed), 0);
        assert_eq!(result, CommentResult::ExistingDraft);
        assert!(
            !engine
                .db
                .list_nurture_comment_attempts(10)
                .expect("comment attempts")
                .iter()
                .any(|attempt| attempt.outcome == "sent"),
            "finding an existing draft must not be recorded as sending one"
        );
        assert!(frames.is_feed(), "the existing draft must be closed");

        drop(engine);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn text_channel_unavailable_does_not_fall_back_to_emoji() {
        let frames = Arc::new(TestFrames::new());
        let stop = Arc::new(AtomicBool::new(false));
        let session =
            RecordingSession::new(frames.clone(), stop.clone(), false, false).without_text_input();
        let (engine, db_path) = test_engine(frames);

        let result = engine
            .do_comment(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                &ActionRail::fallback(),
                (375.0, 667.0),
                &NurtureSettings::default(),
                &[COMMENT.to_string()],
                stop.as_ref(),
            )
            .await
            .expect("missing text capability is a classified comment outcome");

        assert_eq!(result, CommentResult::TextChannelUnavailable);
        assert!(
            session.typed.lock().is_empty(),
            "stock must not send /wda/keys"
        );

        drop(engine);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn a_keys_error_closes_the_comment_ui_before_returning() {
        let frames = Arc::new(TestFrames::new());
        let stop = Arc::new(AtomicBool::new(false));
        let session =
            RecordingSession::new(frames.clone(), stop.clone(), false, false).failing_type();
        let (engine, db_path) = test_engine(frames.clone());

        let error = engine
            .do_comment(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                &ActionRail::fallback(),
                (375.0, 667.0),
                &NurtureSettings::default(),
                &[COMMENT.to_string()],
                stop.as_ref(),
            )
            .await
            .expect_err("typing fixture fails");

        assert!(error.to_string().contains("keys failed"));
        assert!(
            frames.is_feed(),
            "typing errors must not leave the drawer open"
        );

        drop(engine);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn a_send_error_closes_the_comment_ui_before_returning() {
        let frames = Arc::new(TestFrames::new());
        let stop = Arc::new(AtomicBool::new(false));
        let session =
            RecordingSession::new(frames.clone(), stop.clone(), true, false).failing_send();
        let (engine, db_path) = test_engine(frames.clone());

        let error = engine
            .do_comment(
                UDID,
                &session,
                &tokio::sync::Mutex::new(()),
                &ActionRail::fallback(),
                (375.0, 667.0),
                &NurtureSettings::default(),
                &[COMMENT.to_string()],
                stop.as_ref(),
            )
            .await
            .expect_err("send fixture fails");

        assert!(error.to_string().contains("send failed"));
        assert!(
            frames.is_feed(),
            "send errors must not leave the drawer open"
        );

        drop(engine);
        let _ = std::fs::remove_file(db_path);
    }
}
