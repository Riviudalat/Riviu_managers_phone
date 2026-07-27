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
use crate::openai_client::choose_emoji_reaction;
use crate::screen::{self, ActionRail, CommentDrawer, ScreenKind};
use crate::types::{NurtureCommentCost, NurtureSettings, SwipeGesture, TapPoint};

use super::{frame_digest, sleep_interruptible, NurtureEngine, SWIPE_SETTLE};

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
    NotConfirmed { before: f64, best: f64 },
}

/// Where a comment attempt stopped. The flow has five places it can give up and
/// they need different fixes, so one shared "skipped" line is useless: a live
/// run on `05101fdb` abandoned 4 of 5 attempts and the log could not say which
/// step failed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum CommentResult {
    /// Posted, with what the reaction cost in USD.
    Sent(f64),
    /// The comment icon did not open the drawer.
    NoDrawer,
    /// The drawer opened but the emoji icon did not raise the composer.
    NoComposer,
    /// The composer is up but no emoji grid was found in it.
    NoGrid,
    /// The chosen cell was tapped and the send arrow never armed.
    NotArmed,
    /// Send was tapped and the arrow never disarmed — nothing was posted.
    NotSent,
}

impl CommentResult {
    pub(super) fn reason(&self) -> &'static str {
        match self {
            CommentResult::Sent(_) => "đã gửi",
            CommentResult::NoDrawer => "không mở được khay bình luận",
            CommentResult::NoComposer => "khay mở nhưng không lên được composer",
            CommentResult::NoGrid => "composer lên nhưng không thấy lưới emoji",
            CommentResult::NotArmed => "đã chọn emoji nhưng nút gửi không sáng",
            CommentResult::NotSent => "đã bấm gửi nhưng nút không tắt",
        }
    }
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

    /// Post a reaction on the current video.
    ///
    /// **Why emoji and not text.** TikTok's comment box cannot be driven by a
    /// stock WebDriverAgent on this device: the field never takes keyboard
    /// focus, exposes no accessible element, and swallows every synthesized
    /// keystroke — `/wda/keys`, raw HID and `element/value` all leave it
    /// unchanged, while the same calls type fine in other apps. The full
    /// evidence is in `AGENTS.md`. The emoji panel *is* reachable, so the model
    /// picks a reaction that fits the video and the engine taps that cell.
    ///
    /// The text path is written and tested (`generate_vision_comment`); it
    /// switches back on the day the agent can focus the field.
    ///
    /// Every step is confirmed from the stream, and every tap re-checks the
    /// screen first: the composer's controls sit where the feed's navigation
    /// bar is, so acting on a stale reading changes tabs instead of missing.
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
        let frame = self
            .frames
            .latest(udid)
            .ok_or_else(|| anyhow!("không có frame để chọn cảm xúc"))?;
        let (reaction, usd) = choose_emoji_reaction(settings, &frame).await;

        let tap = |x: f64, y: f64| TapPoint {
            x: screen_size.0 * x,
            y: screen_size.1 * y,
        };

        // 1. open the comment drawer
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

        // 2. the emoji icon opens the real composer — the "Thêm bình luận…"
        //    pill does not respond to a synthetic tap at all.
        {
            let _guard = gestures.lock().await;
            session
                .tap(tap(screen::DRAWER_EMOJI_ICON.0, screen::DRAWER_EMOJI_ICON.1))
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

        // 3. select the emoji tab. The panel reopens on whatever was used last,
        //    and the neighbouring tabs are sticker packs whose yellow artwork
        //    the grid detector cannot tell from emoji — but tapping a sticker
        //    inserts nothing, so Send never arms.
        {
            let _guard = gestures.lock().await;
            session
                .tap(tap(screen::COMPOSER_EMOJI_TAB.0, screen::COMPOSER_EMOJI_TAB.1))
                .await?;
        }
        sleep_interruptible(Duration::from_millis(900), stop).await;
        let panel_img = self.latest_image(udid).unwrap_or(composer_img);

        // 4. tap the chosen cell. The grid is located per frame because a
        //    "recently used" row appears after the first reaction and shifts it.
        let grid = screen::find_emoji_grid(&panel_img);
        if grid.is_empty() {
            tracing::warn!("[nurture {udid}] không thấy lưới emoji trong composer");
            self.close_comment_ui(udid, session, gestures, screen_size, stop)
                .await;
            return Ok(CommentResult::NoGrid);
        }
        // Clamp rather than give up. How many rows are visible depends on the
        // panel's scroll position and on whether a "recently used" section is
        // present, so an exact cell is not always on screen — and posting a
        // near-enough reaction beats posting nothing.
        let row_idx = reaction.row.min(grid.len() - 1);
        let row = &grid[row_idx];
        let col_idx = reaction.col.min(row.len() - 1);
        if row_idx != reaction.row || col_idx != reaction.col {
            tracing::info!(
                "[nurture {udid}] ô ({},{}) ngoài lưới {}x{} — dùng ({row_idx},{col_idx})",
                reaction.row,
                reaction.col,
                grid.len(),
                row.len()
            );
        }

        // 5. tap a cell and wait for the send arrow to turn solid red — the only
        //    proof the reaction reached the field.
        //
        //    A cell can miss: the panel is still settling, or the detected blob
        //    was a section header rather than a live cell. Neighbours are just
        //    as good a reaction, so a miss moves along the row instead of
        //    abandoning the comment. Measured 2 of 7 attempts recovered by this.
        let mut candidates = vec![(row_idx, col_idx)];
        for (dr, dc) in [(0usize, 1usize), (1, 0)] {
            let r = (row_idx + dr).min(grid.len() - 1);
            let c = (col_idx + dc).min(grid[r].len() - 1);
            if !candidates.contains(&(r, c)) {
                candidates.push((r, c));
            }
        }

        let mut armed = false;
        for (attempt, (r, c)) in candidates.iter().enumerate() {
            let cell = grid[*r][*c];
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
                tracing::info!("[nurture {udid}] ô ({r},{c}) không chèn được — thử ô kế");
            }
        }
        if !armed {
            self.close_comment_ui(udid, session, gestures, screen_size, stop)
                .await;
            return Ok(CommentResult::NotArmed);
        }

        {
            let _guard = gestures.lock().await;
            session
                .tap(tap(screen::COMPOSER_SEND.0, screen::COMPOSER_SEND.1))
                .await?;
        }
        // Sent when the arrow disarms — the field emptied.
        let sent = self
            .wait_for_frame(udid, Duration::from_secs(5), stop, |img| {
                screen::composer_send_redness(img) <= empty_redness + 40.0
            })
            .await
            .is_some();

        self.close_comment_ui(udid, session, gestures, screen_size, stop)
            .await;

        if !sent {
            return Ok(CommentResult::NotSent);
        }
        tracing::info!("[nurture {udid}] đã gửi cảm xúc {}", reaction.label);

        let cost = NurtureCommentCost {
            id: Uuid::new_v4().to_string(),
            udid: udid.to_string(),
            model: settings.model.clone(),
            base_url_host: crate::openai_client::host_of(&settings.base_url),
            prompt_tokens: 0,
            completion_tokens: 0,
            usd,
            preview: reaction.label.to_string(),
            created_at: Utc::now().to_rfc3339(),
        };
        let _ = self.db.add_nurture_comment_cost(&cost);
        Ok(CommentResult::Sent(usd))
    }
}
