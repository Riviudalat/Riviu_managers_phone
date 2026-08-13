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
use crate::interaction::{PreparedThreadMessage, ThreadSendEvidence};
#[cfg(test)]
use crate::openai_client::pick_from_pool;
use crate::openai_client::{
    host_of, ocr_caption, prepare_caption_comment, prepare_grounded_comment,
    provider_supports_vision,
};
use crate::screen::{self, ActionRail, CommentDrawer};
use crate::types::{
    NurtureCommentAttempt, NurtureCommentCost, NurtureSettings, SwipeGesture, TapPoint,
};

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

const COMMENT_DRAWER_SETTLE: Duration = Duration::from_millis(3_500);
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

/// Where a text-comment attempt stopped. A transport ACK from `/wda/keys` is not
/// success: TikTok may accept the request without putting anything in the field,
/// so arming and disarming Send remain separate, frame-confirmed outcomes.
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)] // Reserved for an explicitly selected emoji-reaction flow.
pub(super) enum CommentResult {
    /// Text was posted, with its vision-generation cost in USD.
    TextSent(f64),
    /// The active session cannot inject trusted text into TikTok.
    TextChannelUnavailable,
    /// Contextual preparation was rejected before any drawer gesture.
    ContextSkipped,
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
            CommentResult::TextSent(_) => "đã gửi bình luận chữ",
            CommentResult::TextChannelUnavailable => {
                "Riviu Agent chưa sẵn sàng cho bình luận chữ — chạy Agent Repair"
            }
            CommentResult::ContextSkipped => "bỏ qua: AI không xác nhận được comment bám nội dung",
            CommentResult::NoDrawer => "không mở được khay bình luận",
            CommentResult::ExistingDraft => "khay bình luận đang có bản nháp cũ",
            CommentResult::TextNotArmed => "đã gõ bình luận chữ nhưng nút gửi không sáng",
            CommentResult::TextNotSent => "đã bấm gửi bình luận chữ nhưng nút không tắt",
        }
    }
}

struct PreparedTextComment {
    text: String,
    model: String,
    base_url_host: String,
    prompt_tokens: u32,
    completion_tokens: u32,
    usd: f64,
    source: &'static str,
    frame_sha256: Option<String>,
    caption_preview: Option<String>,
    context_confidence: Option<u8>,
    relevance: Option<u8>,
    evidence_support: Option<u8>,
    attempt_id: Option<String>,
}

impl NurtureEngine {
    /// Send one already-prepared campaign message. The caller must persist the
    /// prepared text/hash before invoking this method. This deliberately keeps
    /// the same frame-confirmed drawer contract as nurture comments, but does
    /// not call an AI provider while the composer is open.
    pub async fn send_prepared_thread_comment(
        &self,
        udid: &str,
        session: &dyn UiSession,
        gestures: &tokio::sync::Mutex<()>,
        prepared: &PreparedThreadMessage,
        stop: &AtomicBool,
    ) -> anyhow::Result<ThreadSendEvidence> {
        if !session.supports_text_input() {
            return Err(anyhow!("text_channel_unavailable"));
        }
        let Some(open_bytes) = self.frames.latest(udid) else {
            return Err(anyhow!("frame_unavailable"));
        };
        let open_image = image::load_from_memory(&open_bytes)
            .map_err(|_| anyhow!("frame_decode_failed"))?
            .to_rgb8();
        let screen_size = session.window_size().await.unwrap_or((375.0, 667.0));
        // Locate the rail per frame (handles already-followed cards where the
        // red badge is hidden). Fail the attempt rather than tapping the
        // layout-2 fallback constants blind — on a layout-1 card that lands on
        // the Save icon and silently bookmarks the video.
        let rail = screen::locate_action_rail(&open_image)
            .ok_or_else(|| anyhow!("action_rail_not_located"))?;
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
                .map_err(|e| anyhow!("open_comment_drawer: {e}"))?;
        }
        let drawer = self
            .wait_for_frame(udid, Duration::from_secs(6), stop, |img| {
                !matches!(
                    screen::comment_drawer_state(img).0,
                    CommentDrawer::Closed | CommentDrawer::Unknown
                )
            })
            .await
            .ok_or_else(|| anyhow!("comment_drawer_not_confirmed"))?;
        if screen::comment_drawer_state(&drawer).0 != CommentDrawer::Open {
            return Err(anyhow!("comment_drawer_has_existing_draft"));
        }
        sleep_interruptible(COMMENT_DRAWER_SETTLE, stop).await;
        let before_typing = self
            .frames
            .latest(udid)
            .ok_or_else(|| anyhow!("frame_unavailable_before_typing"))?;
        {
            let _guard = gestures.lock().await;
            session
                .tap(point(screen::COMMENT_INPUT.0, screen::COMMENT_INPUT.1))
                .await
                .map_err(|e| anyhow!("focus_comment_input: {e}"))?;
            sleep_interruptible(COMMENT_INPUT_SETTLE, stop).await;
            session
                .type_text(&prepared.text)
                .await
                .map_err(|e| anyhow!("type_comment: {e}"))?;
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
            .ok_or_else(|| anyhow!("armed_frame_missing"))?;
        if armed_frame.is_none() {
            return Err(anyhow!("send_not_armed"));
        }
        let before_send = frame_digest(&armed_bytes);
        {
            let _guard = gestures.lock().await;
            session
                .tap(point(screen::SEND_BUTTON.0, screen::SEND_BUTTON.1))
                .await
                .map_err(|e| anyhow!("tap_send: {e}"))?;
        }
        // "Ready to type" and "sent" are the same classification — an open,
        // unarmed drawer — separated only by when they were observed. So the
        // frames already seen are excluded, `before_typing` above all, because
        // it satisfied this very predicate and a stream running behind would
        // otherwise replay it as proof the field emptied. That matters more
        // here than in the nurture path: this frame's digest is persisted as
        // `cleared_frame_sha256`, so an unexcluded replay writes the
        // *pre-typing* screen into the record as evidence the comment posted.
        let _cleared = self
            .wait_for_frame_after(
                udid,
                Duration::from_secs(6),
                stop,
                &[before_send, frame_digest(&before_typing)],
                |img| screen::comment_drawer_state(img).0 == CommentDrawer::Open,
            )
            .await
            .ok_or_else(|| anyhow!("send_clear_not_confirmed"))?;
        let cleared_bytes = self
            .frames
            .latest(udid)
            .ok_or_else(|| anyhow!("cleared_frame_missing"))?;
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
    pub async fn send_prepared_thread_reply(
        &self,
        udid: &str,
        session: &dyn UiSession,
        gestures: &tokio::sync::Mutex<()>,
        reply_point: TapPoint,
        prepared: &PreparedThreadMessage,
        stop: &AtomicBool,
    ) -> anyhow::Result<ThreadSendEvidence> {
        if !session.supports_text_input() {
            return Err(anyhow!("text_channel_unavailable"));
        }
        let screen_size = session.window_size().await.unwrap_or((375.0, 667.0));
        let reply_point = self.next_touch_point(udid, screen_size, reply_point, (8.0, 8.0));
        {
            let _guard = gestures.lock().await;
            session
                .tap(reply_point)
                .await
                .map_err(|e| anyhow!("tap_reply: {e}"))?;
        }
        self.wait_for_frame(udid, Duration::from_secs(5), stop, |img| {
            !matches!(
                screen::comment_drawer_state(img).0,
                CommentDrawer::Closed | CommentDrawer::Unknown
            )
        })
        .await
        .ok_or_else(|| anyhow!("reply_composer_not_confirmed"))?;
        sleep_interruptible(COMMENT_INPUT_SETTLE, stop).await;
        let before_typing = self
            .frames
            .latest(udid)
            .ok_or_else(|| anyhow!("frame_unavailable_before_reply"))?;
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
                .map_err(|e| anyhow!("focus_reply_input: {e}"))?;
            sleep_interruptible(COMMENT_INPUT_SETTLE, stop).await;
            session
                .type_text(&prepared.text)
                .await
                .map_err(|e| anyhow!("type_reply: {e}"))?;
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
            .ok_or_else(|| anyhow!("reply_armed_frame_missing"))?;
        if !armed {
            return Err(anyhow!("reply_send_not_armed"));
        }
        let before_send = frame_digest(&armed_bytes);
        {
            let _guard = gestures.lock().await;
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
                .map_err(|e| anyhow!("tap_reply_send: {e}"))?;
        }
        // Same rule as the top-level comment: a frame already seen cannot be
        // the proof, and this one is persisted as evidence.
        self.wait_for_frame_after(
            udid,
            Duration::from_secs(6),
            stop,
            &[before_send, frame_digest(&before_typing)],
            |img| screen::comment_drawer_state(img).0 == CommentDrawer::Open,
        )
        .await
        .ok_or_else(|| anyhow!("reply_clear_not_confirmed"))?;
        let cleared_bytes = self
            .frames
            .latest(udid)
            .ok_or_else(|| anyhow!("reply_cleared_frame_missing"))?;
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
        udid: &str,
        session: &dyn UiSession,
        gestures: &tokio::sync::Mutex<()>,
        rail: &ActionRail,
        screen_size: (f64, f64),
        stop: &AtomicBool,
    ) -> anyhow::Result<bool> {
        let Some(img) = self.latest_image(udid) else {
            return Ok(false);
        };
        // No badge means this author is already followed.
        if !screen::follow_badge_present(&img, rail) {
            return Ok(false);
        }

        let mut rng = StdRng::from_entropy();
        let point = self.next_touch_point(
            udid,
            screen_size,
            TapPoint {
                x: screen_size.0 * rail.x,
                y: screen_size.1 * rail.follow_y,
            },
            (10.0, 10.0),
        );
        sleep_interruptible(Duration::from_millis(rng.gen_range(300..700)), stop).await;
        {
            let _guard = gestures.lock().await;
            session.tap(point).await?;
        }
        // Require the confirming frame to still be an actionable feed: a system
        // alert dims the whole screen, which reads as "badge gone" at the rail
        // and would otherwise count a follow the alert actually swallowed.
        let gone = self
            .wait_for_frame(udid, Duration::from_millis(2_500), stop, |img| {
                !screen::follow_badge_present(img, rail)
                    && screen::feed_ready(img, Some(screen_size.0))
            })
            .await
            .is_some();
        Ok(gone)
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
        rail: &ActionRail,
        screen_size: (f64, f64),
        settings: &NurtureSettings,
        _pool: &[String],
        stop: &AtomicBool,
    ) -> anyhow::Result<CommentResult> {
        if !session.supports_text_input() {
            return Ok(CommentResult::TextChannelUnavailable);
        }
        let prepared = if settings.api_key.trim().is_empty() {
            // Unit fixtures still exercise the proven drawer sender with an
            // explicit pool entry. Production never passes a pool, so an empty
            // API key is a contextual skip rather than a generic post.
            #[cfg(test)]
            {
                pick_from_pool(_pool).map(|text| PreparedTextComment {
                    text,
                    model: settings.model.clone(),
                    base_url_host: host_of(&settings.base_url),
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    usd: 0.0,
                    source: "test-fixture",
                    frame_sha256: None,
                    caption_preview: None,
                    context_confidence: None,
                    relevance: None,
                    evidence_support: None,
                    attempt_id: None,
                })
            }
            #[cfg(not(test))]
            {
                None
            }
        } else {
            let Some(frames) = self.collect_comment_frames(udid, screen_size, stop).await else {
                self.record_context_skip_attempt(
                    udid,
                    settings,
                    context_source(settings),
                    "evidence_unavailable",
                );
                return Ok(CommentResult::ContextSkipped);
            };
            let direction = pick_direction_seeded(
                &settings.ai_directions,
                frames
                    .first()
                    .map(|frame| frame_digest(frame))
                    .unwrap_or_default(),
            );
            let prepared_result = if provider_supports_vision(settings) {
                prepare_grounded_comment(settings, &frames, direction.as_deref()).await
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
                    text: comment.text,
                    model: comment.model,
                    base_url_host: comment.base_url_host,
                    prompt_tokens: comment.prompt_tokens,
                    completion_tokens: comment.completion_tokens,
                    usd: comment.usd,
                    source: context_source(settings),
                    frame_sha256: Some(comment.frame_sha256),
                    caption_preview: comment
                        .caption
                        .as_deref()
                        .map(|caption| caption.chars().take(160).collect()),
                    context_confidence: Some(comment.context_confidence),
                    relevance: Some(comment.relevance),
                    evidence_support: Some(comment.evidence_support),
                    attempt_id: None,
                }),
                Err(error) => {
                    tracing::info!("[nurture {udid}] bỏ qua comment semantic: {error}");
                    if std::env::var_os("RIVIU_LIVE_NURTURE_VERBOSE").is_some() {
                        eprintln!("[nurture {udid}] comment semantic skip: {error}");
                    }
                    None
                }
            }
        };
        let Some(mut prepared) = prepared else {
            self.record_context_skip_attempt(
                udid,
                settings,
                context_source(settings),
                "context_skipped",
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
            usd: prepared.usd,
            preview: prepared.text.chars().take(160).collect(),
            caption_preview: prepared.caption_preview.clone().unwrap_or_default(),
            frame_sha256: prepared.frame_sha256.clone().unwrap_or_default(),
            context_confidence: prepared.context_confidence,
            relevance: prepared.relevance,
            evidence_support: prepared.evidence_support,
            created_at: Utc::now().to_rfc3339(),
        };
        if let Err(error) = self.db.add_nurture_comment_attempt(&attempt) {
            tracing::warn!("[nurture {udid}] không ghi được comment attempt: {error}");
        }
        prepared.attempt_id = Some(attempt.id);

        // AI/OCR preparation can take several seconds; reacquire the rail from
        // the newest frame before opening the drawer so we never tap a stale
        // card's comment coordinate.
        let active_rail = self
            .latest_image(udid)
            .and_then(|img| screen::locate_action_rail(&img))
            .unwrap_or(*rail);
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
        let open_result = async {
            let _guard = gestures.lock().await;
            session.tap(tap(active_rail.x, active_rail.comment_y)).await
        }
        .await;
        if let Err(error) = open_result {
            self.update_comment_attempt(&prepared, "open_error");
            self.close_comment_ui(udid, session, gestures, screen_size, stop)
                .await;
            return Err(error);
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

        let cost = NurtureCommentCost {
            id: Uuid::new_v4().to_string(),
            udid: udid.to_string(),
            model: prepared.model,
            base_url_host: prepared.base_url_host,
            prompt_tokens: prepared.prompt_tokens,
            completion_tokens: prepared.completion_tokens,
            usd: prepared.usd,
            preview: prepared.text,
            created_at: Utc::now().to_rfc3339(),
        };
        if let Err(error) = self.db.add_nurture_comment_cost(&cost) {
            tracing::warn!("[nurture {udid}] không ghi được cost bình luận: {error}");
        }
        Ok(CommentResult::TextSent(cost.usd))
    }

    /// Capture a short, same-post evidence window from the existing MJPEG
    /// source. Any popup or transition inside the window invalidates the set;
    /// posting a sentence from mixed screens is worse than skipping a turn.
    async fn collect_comment_frames(
        &self,
        udid: &str,
        screen_size: (f64, f64),
        stop: &AtomicBool,
    ) -> Option<Vec<Vec<u8>>> {
        let mut frames = Vec::with_capacity(3);
        for sample in 0..3 {
            if stop.load(std::sync::atomic::Ordering::Relaxed) {
                return None;
            }
            let frame = self.frames.latest(udid)?;
            let image = image::load_from_memory(&frame).ok()?.to_rgb8();
            if !screen::feed_ready(&image, Some(screen_size.0)) {
                return None;
            }
            frames.push((*frame).clone());
            if sample < 2 {
                sleep_interruptible(Duration::from_millis(600), stop).await;
            }
        }
        Some(frames)
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
        stop: &AtomicBool,
    ) -> Option<super::hierarchy::PreparedComment> {
        if settings.api_key.trim().is_empty() {
            return None;
        }
        let frames = match self.collect_grounding_frames(udid, stop).await {
            Some(frames) => frames,
            None => {
                self.record_context_skip_attempt(
                    udid,
                    settings,
                    context_source(settings),
                    "evidence_unavailable",
                );
                return None;
            }
        };
        let direction = pick_direction_seeded(
            &settings.ai_directions,
            frames.first().map(|frame| frame_digest(frame)).unwrap_or(0),
        );
        let prepared = if provider_supports_vision(settings) {
            prepare_grounded_comment(settings, &frames, direction.as_deref()).await
        } else {
            let Some(caption) = self.read_caption(frames.last()).await else {
                self.record_context_skip_attempt(
                    udid,
                    settings,
                    "ocr-caption",
                    "caption_ocr_empty",
                );
                return None;
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
                tracing::warn!("[nurture {udid}] không soạn được bình luận: {error}");
                self.record_context_skip_attempt(
                    udid,
                    settings,
                    context_source(settings),
                    "context_skipped",
                );
                return None;
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
            usd: comment.usd,
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
            created_at: Utc::now().to_rfc3339(),
        };
        let attempt_id = attempt.id.clone();
        if let Err(error) = self.db.add_nurture_comment_attempt(&attempt) {
            tracing::warn!("[nurture {udid}] không ghi được comment attempt: {error}");
        }
        Some(super::hierarchy::PreparedComment {
            text: comment.text,
            usd: comment.usd,
            attempt_id: Some(attempt_id),
        })
    }

    /// Close out a hierarchy comment's audit row.
    pub(super) fn finish_hierarchy_comment(&self, attempt_id: Option<&str>, outcome: &str) {
        let Some(id) = attempt_id else { return };
        if let Err(error) = self.db.update_nurture_comment_attempt_outcome(id, outcome) {
            tracing::warn!("không cập nhật outcome comment attempt {id}: {error}");
        }
    }

    fn update_comment_attempt(&self, prepared: &PreparedTextComment, outcome: &str) {
        let Some(id) = prepared.attempt_id.as_deref() else {
            return;
        };
        if let Err(error) = self.db.update_nurture_comment_attempt_outcome(id, outcome) {
            tracing::warn!("không cập nhật outcome comment attempt {id}: {error}");
        }
    }

    /// Read the caption off a frame, or `None` if OCR has nothing usable.
    async fn read_caption(&self, frame: Option<&Vec<u8>>) -> Option<String> {
        let observations = self.frame_text.recognize(frame?).await.ok()?;
        ocr_caption(&observations)
    }

    fn record_context_skip_attempt(
        &self,
        udid: &str,
        settings: &NurtureSettings,
        source: &str,
        outcome: &str,
    ) {
        let attempt = NurtureCommentAttempt {
            id: Uuid::new_v4().to_string(),
            udid: udid.to_string(),
            outcome: outcome.to_string(),
            source: source.to_string(),
            model: settings.model.clone(),
            base_url_host: host_of(&settings.base_url),
            prompt_tokens: 0,
            completion_tokens: 0,
            usd: 0.0,
            preview: String::new(),
            caption_preview: String::new(),
            frame_sha256: String::new(),
            context_confidence: None,
            relevance: None,
            evidence_support: None,
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

    struct RecordingSession {
        frames: Arc<TestFrames>,
        stop: Arc<AtomicBool>,
        arm_on_type: bool,
        stop_on_type: bool,
        supports_text_input: bool,
        open_with_existing_draft: bool,
        type_error: bool,
        send_error: bool,
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

        fn failing_type(mut self) -> Self {
            self.type_error = true;
            self
        }

        fn failing_send(mut self) -> Self {
            self.send_error = true;
            self
        }

        fn near(point: &TapPoint, expected: (f64, f64)) -> bool {
            // Production taps are deliberately sampled inside the control's
            // safe hitbox, so the fixture must model a hit area rather than
            // requiring the old single-pixel center.
            (point.x - 375.0 * expected.0).abs() < 20.0
                && (point.y - 667.0 * expected.1).abs() < 20.0
        }
    }

    #[async_trait]
    impl UiSession for RecordingSession {
        async fn tap(&self, point: TapPoint) -> anyhow::Result<()> {
            if !self.supports_text_input && Self::near(&point, screen::DRAWER_EMOJI_ICON) {
                anyhow::bail!("emoji fallback reached");
            }
            if self.send_error && Self::near(&point, screen::SEND_BUTTON) {
                anyhow::bail!("send failed after touch");
            }
            if Self::near(&point, screen::DRAWER_DISMISS) {
                self.frames.show_feed();
            } else if Self::near(&point, screen::SEND_BUTTON) {
                self.send_taps.fetch_add(1, Ordering::Relaxed);
                self.frames.show_posted();
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
    async fn a_follow_is_not_confirmed_on_a_non_feed_frame() {
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

        assert!(
            !confirmed,
            "a badge that vanished on a non-feed (dimmed) frame must not count as a follow"
        );
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

        let costs = engine
            .db
            .list_nurture_comment_costs(10)
            .expect("comment costs");
        assert_eq!(costs.len(), 1);
        assert_eq!(costs[0].preview, COMMENT);
        assert_eq!(costs[0].usd, 0.0, "pool text has no per-comment AI cost");
        let attempts = engine
            .db
            .list_nurture_comment_attempts(10)
            .expect("comment attempts");
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].outcome, "sent");
        assert_eq!(attempts[0].preview, COMMENT);
        assert_eq!(attempts[0].source, "test-fixture");

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
        assert!(
            engine
                .db
                .list_nurture_comment_costs(10)
                .expect("comment costs")
                .is_empty(),
            "an unconfirmed send must not be billed as a posted comment"
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
        anyhow::Result<crate::interaction::ThreadSendEvidence>,
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

        let evidence = result.expect("thread comment");
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
            error.to_string().contains("send_clear_not_confirmed"),
            "unexpected error: {error}"
        );
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
        assert!(engine
            .db
            .list_nurture_comment_costs(10)
            .expect("comment costs")
            .is_empty());

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
        assert!(engine
            .db
            .list_nurture_comment_costs(10)
            .expect("comment costs")
            .is_empty());
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
