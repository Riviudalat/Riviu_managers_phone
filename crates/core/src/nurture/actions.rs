//! The individual feed gestures, each confirmed against the frame stream.
//!
//! Nothing here reports success it has not seen: a like counts once the heart
//! turns red in a later frame, a follow once the badge disappears, a swipe once
//! the frame changes, a comment once the Send button was observed armed.

use std::sync::atomic::AtomicBool;
use std::time::Duration;

use anyhow::anyhow;
use chrono::Utc;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use uuid::Uuid;

use crate::driver::UiSession;
use crate::human_behavior::pick_direction;
use crate::openai_client::{
    choose_emoji_reaction, generate_vision_comment, host_of, pick_from_pool,
};
use crate::screen::{self, ActionRail, CommentDrawer, ScreenKind};
use crate::types::{NurtureCommentCost, NurtureSettings, SwipeGesture, TapPoint};

use super::{frame_digest, sleep_interruptible, NurtureEngine, SWIPE_SETTLE};

const COMMENT_DRAWER_SETTLE: Duration = Duration::from_millis(3_500);
const COMMENT_INPUT_SETTLE: Duration = Duration::from_millis(1_200);

/// What happened to a like attempt. "Already liked" is a normal outcome and
/// must not be reported as a failure — conflating the two is what made the old
/// logs unreadable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum LikeResult {
    Liked,
    AlreadyLiked,
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
    /// Emoji fallback was posted by a session whose text channel is not trusted.
    EmojiSent(f64),
    /// Neither vision generation nor the pre-generated pool yielded text.
    NoText,
    /// The comment icon did not open the drawer.
    NoDrawer,
    /// The drawer already contained text before this attempt started.
    ExistingDraft,
    /// The emoji fallback could not open the composer.
    NoComposer,
    /// The emoji fallback could not locate a usable grid.
    NoGrid,
    /// Typing returned OK but the Send button never armed.
    TextNotArmed,
    /// Send was tapped but never returned to its unarmed state.
    TextNotSent,
    /// The emoji fallback did not arm its send button.
    EmojiNotArmed,
    /// The emoji fallback send button did not disarm.
    EmojiNotSent,
}

impl CommentResult {
    pub(super) fn reason(&self) -> &'static str {
        match self {
            CommentResult::TextSent(_) => "đã gửi bình luận chữ",
            CommentResult::TextChannelUnavailable => {
                "Riviu Agent chưa sẵn sàng cho bình luận chữ — chạy Agent Repair"
            }
            CommentResult::EmojiSent(_) => "đã gửi bình luận emoji",
            CommentResult::NoText => "không có nội dung bình luận chữ dùng được",
            CommentResult::NoDrawer => "không mở được khay bình luận",
            CommentResult::ExistingDraft => "khay bình luận đang có bản nháp cũ",
            CommentResult::NoComposer => "không mở được composer emoji",
            CommentResult::NoGrid => "không thấy lưới emoji",
            CommentResult::TextNotArmed => "đã gõ bình luận chữ nhưng nút gửi không sáng",
            CommentResult::TextNotSent => "đã bấm gửi bình luận chữ nhưng nút không tắt",
            CommentResult::EmojiNotArmed => "emoji không làm nút gửi sáng",
            CommentResult::EmojiNotSent => "đã bấm gửi emoji nhưng nút không tắt",
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
}

impl NurtureEngine {
    /// Tap the heart, then confirm from a later frame that it turned red.
    pub(super) async fn do_like(
        &self,
        udid: &str,
        session: &dyn UiSession,
        gestures: &tokio::sync::Mutex<()>,
        rail: &ActionRail,
        screen_size: (f64, f64),
        stop: &AtomicBool,
    ) -> anyhow::Result<LikeResult> {
        let before = self
            .latest_image(udid)
            .map(|img| screen::like_redness_at(&img, rail))
            .unwrap_or(0.0);
        // Already liked — tapping again would un-like it.
        if before > screen::LIKE_FILLED_REDNESS {
            return Ok(LikeResult::AlreadyLiked);
        }

        let mut rng = StdRng::from_entropy();
        let point = TapPoint {
            x: screen_size.0 * rail.x + rng.gen_range(-3.0..3.0),
            y: screen_size.1 * rail.like_y + rng.gen_range(-4.0..4.0),
        };
        sleep_interruptible(Duration::from_millis(rng.gen_range(400..900)), stop).await;
        {
            let _guard = gestures.lock().await;
            session.tap(point).await?;
        }

        // Absolute, not relative: the heart is either filled or it is not, and
        // the fill level does not depend on the video behind it. A relative
        // test does, which is how a red-heavy clip made real likes read as
        // misses and outlines read as already-liked.
        let mut best = before;
        let confirmed = self
            .wait_for_frame(udid, Duration::from_millis(2_500), stop, |img| {
                let now = screen::like_redness_at(img, rail);
                best = best.max(now);
                now > screen::LIKE_FILLED_REDNESS
            })
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
        let point = TapPoint {
            x: screen_size.0 * rail.x,
            y: screen_size.1 * rail.follow_y + rng.gen_range(-3.0..3.0),
        };
        sleep_interruptible(Duration::from_millis(rng.gen_range(300..700)), stop).await;
        {
            let _guard = gestures.lock().await;
            session.tap(point).await?;
        }
        let gone = self
            .wait_for_frame(udid, Duration::from_millis(2_500), stop, |img| {
                !screen::follow_badge_present(img, rail)
            })
            .await
            .is_some();
        Ok(gone)
    }

    /// Swipe to the next video, confirming from the stream that the frame
    /// actually changed. Returns `Ok(false)` when the gesture was accepted but
    /// the feed did not move — usually a popup on top.
    pub(super) async fn do_swipe(
        &self,
        udid: &str,
        session: &dyn UiSession,
        gestures: &tokio::sync::Mutex<()>,
        screen_size: (f64, f64),
        frenzy: bool,
        stop: &AtomicBool,
    ) -> anyhow::Result<bool> {
        let before = self.frames.latest(udid).map(|f| frame_digest(&f));
        let mut rng = StdRng::from_entropy();
        let x0 = screen_size.0 * 0.5 + rng.gen_range(-20.0..20.0);
        let gesture = SwipeGesture {
            from: TapPoint {
                x: x0,
                y: screen_size.1 * 0.75,
            },
            to: TapPoint {
                x: x0 + rng.gen_range(-8.0..8.0),
                y: (screen_size.1 * 0.25 + rng.gen_range(-50.0..50.0)).max(40.0),
            },
            duration_ms: if frenzy {
                rng.gen_range(180..280)
            } else {
                rng.gen_range(280..450)
            },
        };
        {
            let _guard = gestures.lock().await;
            session.swipe(gesture).await?;
        }

        let Some(before) = before else {
            return Ok(true);
        };
        let changed = self
            .wait_for_new_frame(udid, SWIPE_SETTLE, stop, before)
            .await;
        Ok(changed)
    }

    /// Close the comment drawer. Safe to call from any state: the dismiss point
    /// is near the top of the screen, well away from anything destructive.
    pub(super) async fn dismiss_drawer(
        &self,
        session: &dyn UiSession,
        gestures: &tokio::sync::Mutex<()>,
        screen_size: (f64, f64),
        stop: &AtomicBool,
    ) {
        let _guard = gestures.lock().await;
        let _ = session
            .tap(TapPoint {
                x: screen_size.0 * screen::DRAWER_DISMISS.0,
                y: screen_size.1 * screen::DRAWER_DISMISS.1,
            })
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
            self.dismiss_drawer(session, gestures, screen_size, stop)
                .await;
            if self
                .wait_for_frame(udid, Duration::from_secs(3), stop, |img| {
                    screen::classify(img, Some(screen_size.0)).kind == ScreenKind::Feed
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

    #[allow(dead_code, clippy::too_many_arguments)]
    async fn do_emoji_comment(
        &self,
        udid: &str,
        session: &dyn UiSession,
        gestures: &tokio::sync::Mutex<()>,
        rail: &ActionRail,
        screen_size: (f64, f64),
        settings: &NurtureSettings,
        stop: &AtomicBool,
    ) -> anyhow::Result<CommentResult> {
        let frame = self
            .frames
            .latest(udid)
            .ok_or_else(|| anyhow!("không có frame để chọn cảm xúc"))?;
        let (reaction, usd) = choose_emoji_reaction(settings, &frame).await;
        let tap = |x: f64, y: f64| TapPoint {
            x: screen_size.0 * x,
            y: screen_size.1 * y,
        };

        {
            let _guard = gestures.lock().await;
            session.tap(tap(rail.x, rail.comment_y)).await?;
        }
        if self
            .wait_for_frame(udid, Duration::from_secs(5), stop, |img| {
                !matches!(
                    screen::comment_drawer_state(img).0,
                    CommentDrawer::Closed | CommentDrawer::Unknown
                )
            })
            .await
            .is_none()
        {
            self.close_comment_ui(udid, session, gestures, screen_size, stop)
                .await;
            return Ok(CommentResult::NoDrawer);
        }

        {
            let _guard = gestures.lock().await;
            session
                .tap(tap(
                    screen::DRAWER_EMOJI_ICON.0,
                    screen::DRAWER_EMOJI_ICON.1,
                ))
                .await?;
        }
        let composer = self
            .wait_for_frame(udid, Duration::from_secs(5), stop, |img| {
                screen::composer_send_redness(img) > 25.0
            })
            .await;
        let Some(composer_img) = composer else {
            self.close_comment_ui(udid, session, gestures, screen_size, stop)
                .await;
            return Ok(CommentResult::NoComposer);
        };
        let empty_redness = screen::composer_send_redness(&composer_img);

        {
            let _guard = gestures.lock().await;
            session
                .tap(tap(
                    screen::COMPOSER_EMOJI_TAB.0,
                    screen::COMPOSER_EMOJI_TAB.1,
                ))
                .await?;
        }
        sleep_interruptible(Duration::from_millis(900), stop).await;
        let panel_img = self.latest_image(udid).unwrap_or(composer_img);
        let grid = screen::find_emoji_grid(&panel_img);
        if grid.is_empty() {
            self.close_comment_ui(udid, session, gestures, screen_size, stop)
                .await;
            return Ok(CommentResult::NoGrid);
        }

        let row_idx = reaction.row.min(grid.len() - 1);
        let row = &grid[row_idx];
        let col_idx = reaction.col.min(row.len() - 1);
        let mut candidates = vec![(row_idx, col_idx)];
        for (dr, dc) in [(0usize, 1usize), (1, 0)] {
            let r = (row_idx + dr).min(grid.len() - 1);
            let c = (col_idx + dc).min(grid[r].len() - 1);
            if !candidates.contains(&(r, c)) {
                candidates.push((r, c));
            }
        }

        let mut armed = false;
        for (attempt, (row, col)) in candidates.iter().enumerate() {
            let cell = grid[*row][*col];
            {
                let _guard = gestures.lock().await;
                session.tap(tap(cell.0, cell.1)).await?;
            }
            if self
                .wait_for_frame(udid, Duration::from_secs(4), stop, |img| {
                    screen::composer_send_redness(img) > empty_redness + 40.0
                })
                .await
                .is_some()
            {
                armed = true;
                break;
            }
            if attempt + 1 < candidates.len() {
                tracing::info!("[nurture {udid}] emoji không chèn được — thử ô kế");
            }
        }
        if !armed {
            self.close_comment_ui(udid, session, gestures, screen_size, stop)
                .await;
            return Ok(CommentResult::EmojiNotArmed);
        }

        {
            let _guard = gestures.lock().await;
            session
                .tap(tap(screen::COMPOSER_SEND.0, screen::COMPOSER_SEND.1))
                .await?;
        }
        let sent = self
            .wait_for_frame(udid, Duration::from_secs(5), stop, |img| {
                screen::composer_send_redness(img) <= empty_redness + 40.0
            })
            .await
            .is_some();
        self.close_comment_ui(udid, session, gestures, screen_size, stop)
            .await;
        if !sent {
            return Ok(CommentResult::EmojiNotSent);
        }

        let cost = NurtureCommentCost {
            id: Uuid::new_v4().to_string(),
            udid: udid.to_string(),
            model: settings.model.clone(),
            base_url_host: host_of(&settings.base_url),
            prompt_tokens: 0,
            completion_tokens: 0,
            usd,
            preview: reaction.label.to_string(),
            created_at: Utc::now().to_rfc3339(),
        };
        if let Err(error) = self.db.add_nurture_comment_cost(&cost) {
            tracing::warn!("[nurture {udid}] không ghi được cost emoji: {error}");
        }
        Ok(CommentResult::EmojiSent(usd))
    }

    /// Post a text comment on the current video.
    ///
    /// Vision generation happens before opening the drawer so a slow API cannot
    /// leave UI behind. A generated sentence is preferred; the pre-generated
    /// pool is the offline fallback. Typing is only an attempted input: the
    /// stream must show Send arm before it is tapped, then disarm afterwards.
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
        pool: &[String],
        stop: &AtomicBool,
    ) -> anyhow::Result<CommentResult> {
        if !session.supports_text_input() {
            return Ok(CommentResult::TextChannelUnavailable);
        }
        let frame = self
            .frames
            .latest(udid)
            .ok_or_else(|| anyhow!("không có frame để sinh bình luận"))?;
        let direction = pick_direction(&settings.ai_directions);
        let generated = if settings.api_key.trim().is_empty() {
            None
        } else {
            match generate_vision_comment(settings, &frame, direction.as_deref()).await {
                Ok(comment) => Some(comment),
                Err(error) => {
                    tracing::warn!(
                        "[nurture {udid}] sinh bình luận vision lỗi: {error} — dùng pool"
                    );
                    None
                }
            }
        };
        let prepared = if let Some(comment) = generated {
            Some(PreparedTextComment {
                text: comment.text,
                model: comment.model,
                base_url_host: comment.base_url_host,
                prompt_tokens: comment.prompt_tokens,
                completion_tokens: comment.completion_tokens,
                usd: comment.usd,
                source: "vision",
            })
        } else {
            pick_from_pool(pool).map(|text| PreparedTextComment {
                text,
                model: settings.model.clone(),
                base_url_host: host_of(&settings.base_url),
                prompt_tokens: 0,
                completion_tokens: 0,
                usd: 0.0,
                source: "pool",
            })
        };
        let Some(prepared) = prepared else {
            return Ok(CommentResult::NoText);
        };

        let tap = |x: f64, y: f64| TapPoint {
            x: screen_size.0 * x,
            y: screen_size.1 * y,
        };

        // 1. open the comment drawer
        let open_result = async {
            let _guard = gestures.lock().await;
            session.tap(tap(rail.x, rail.comment_y)).await
        }
        .await;
        if let Err(error) = open_result {
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
            self.close_comment_ui(udid, session, gestures, screen_size, stop)
                .await;
            return Ok(CommentResult::TextNotArmed);
        }

        let send_result = async {
            let _guard = gestures.lock().await;
            session
                .tap(tap(screen::SEND_BUTTON.0, screen::SEND_BUTTON.1))
                .await
        }
        .await;
        if let Err(error) = send_result {
            self.close_comment_ui(udid, session, gestures, screen_size, stop)
                .await;
            return Err(error);
        }
        // The field must empty back to the open, unarmed drawer. Unknown frames
        // are not accepted as proof because the screen may have moved elsewhere.
        let sent = self
            .wait_for_frame(udid, Duration::from_secs(5), stop, |img| {
                screen::comment_drawer_state(img).0 == CommentDrawer::Open
            })
            .await
            .is_some();

        self.close_comment_ui(udid, session, gestures, screen_size, stop)
            .await;

        if !sent {
            return Ok(CommentResult::TextNotSent);
        }
        tracing::info!(
            "[nurture {udid}] đã gửi bình luận chữ ({}) {:?}",
            prepared.source,
            prepared.text
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

    struct TestFrames {
        current: Mutex<Frame>,
        feed: Frame,
        open: Frame,
        armed: Frame,
    }

    impl TestFrames {
        fn new() -> Self {
            let feed = Arc::new(include_bytes!("../../tests/fixtures/feed-iphone8.jpg").to_vec());
            let open = encode_frame(drawer_frame(false));
            let armed = encode_frame(drawer_frame(true));
            Self {
                current: Mutex::new(feed.clone()),
                feed,
                open,
                armed,
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

    fn drawer_frame(armed: bool) -> RgbImage {
        let mut img = RgbImage::from_pixel(750, 1334, Rgb([225, 225, 225]));
        if armed {
            paint_icon(&mut img, screen::SEND_BUTTON, Rgb([255, 35, 65]));
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
            (point.x - 375.0 * expected.0).abs() < 1.0 && (point.y - 667.0 * expected.1).abs() < 1.0
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
                self.frames.show_open();
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
