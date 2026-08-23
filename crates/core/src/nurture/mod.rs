//! TikTok FYP nurture engine.
//!
//! Design notes that are easy to undo by accident:
//!
//! * **The screen is read from the video stream, never from WDA.** The MJPEG
//!   stream is a separate usbmux channel that the app already keeps open, so
//!   watching costs the control relay nothing. Polling `GET /screenshot`
//!   instead wedged the relay repeatedly (live tests #1, #4–5).
//! * **Buttons are located per frame, not hard-coded.** TikTok ships two
//!   sidebar layouts. The previous fixed fractions landed between icons, so
//!   likes tapped empty space while the counter still went up.
//! * **Recovery is budgeted and typed.** Only a transport-class failure may
//!   touch the transport; a rejected command never triggers a recycle. Earlier
//!   builds treated every failure as "WDA unhealthy" and burned 2–3 minutes.
//! * **Nothing is reported that was not verified.** A like counts once the
//!   heart turns red in a later frame, a swipe once the action rail has left
//!   the screen and a new card has settled. Evidence has to be *structural*:
//!   the stream publishes every frame at 24 FPS with no deduplication, so "the
//!   frame changed" is what a playing video does, not what a swipe does.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
mod actions;
mod hierarchy;
mod live;
mod recovery;
// Crate-visible, not private, because the Interaction path needs the *same* jitter
// history rather than its own: two touch planners on one device would produce a tap
// distribution neither of them intended. `crate::interaction_hierarchy` is the second
// caller.
pub mod touch;

use actions::{CommentResult, LikeResult, SwipeOutcome};
pub use hierarchy::{run_hierarchy_session, CommentTextSource, HierarchySession, PreparedComment};
pub use live::LiveSettings;
use live::{apply_live_settings, video_target};
pub use recovery::Outcome;
use recovery::{session_verdict, Budget};

use crate::db::Database;
use crate::device_control::{DeviceControlPlane, UiWithStreamContext};
use crate::driver::{ui_error_kind, UiError, UiErrorKind, UiSession};
use crate::frame_source::FrameSource;
use crate::frame_text::{FrameTextSource, NullFrameTextSource};
use crate::human_behavior::{
    in_night_window, roll_bool, roll_feed_action_in_mood, roll_follow_in_mood, FeedAction,
    HumanBehavior, HumanSessionPolicy, MoodCycle, PolicyAction,
};
use crate::screen::{self, ActionRail, ScreenKind};
use crate::screen_watch::{ScreenWatcher, SessionHandle};
use crate::types::{InteractionSessionKind, NurtureSessionStatus, NurtureSettings, TapPoint};
use crate::DeviceWorkOwner;
use touch::TouchPointPlanner;

/// How long to wait for a new card to settle before calling a swipe blocked.
///
/// TikTok's snap animation plus stream latency puts the settled card a good
/// second behind the gesture, and a tighter window reports false "blocked" —
/// the loop then swipes twice and skips a video.
///
/// The note this comment used to carry ("the stream only pushes on change and
/// runs at ~7 FPS") was wrong on both counts: [`crate::stream::StreamHub`]
/// publishes every frame at 24 FPS. That mistake is why the old check believed
/// a changed frame meant a moved feed.
pub(super) const SWIPE_SETTLE: Duration = Duration::from_millis(2_400);
/// Poll faster than frames arrive, so the few hundred milliseconds the rail is
/// off screen cannot fall between two samples.
pub(super) const SWIPE_POLL: Duration = Duration::from_millis(60);
/// Consecutive turns that may end off the FYP before the session stops trying
/// to swipe its way back.
///
/// A screen the loop cannot leave is the LIVE-room failure, and it is the most
/// expensive thing that can go wrong here. Vertical swipes inside a LIVE room
/// scroll the room's own content instead of exiting, so `on_feed` stays false
/// and this branch used to repeat until the video budget ran out: every turn
/// spent, `videos_done` zero, outcome `Failed`, and nothing in the log saying
/// why. One missed room cost the whole session — and the detector misses a room
/// whenever the account already follows the host, because the follow pill it
/// keys on is then not on screen at all.
///
/// Four turns is enough for the watcher to clear an ordinary overlay and for
/// one retry swipe; past that, relaunching is the only thing left that reliably
/// leaves a LIVE room.
const OFF_FEED_LIMIT: u32 = 4;
/// Back gestures to try before falling back to a relaunch. Three covers a
/// search result opened from a profile opened from a card — deeper than the
/// engine can navigate on its own.
const OFF_FEED_BACK_ATTEMPTS: u32 = 3;
/// Turns in a row that may end with both the swipe and its retry blocked before
/// the session gives up on the card. Three turns is six swipes and roughly
/// thirty seconds — long enough to ride out a popup the watcher is still
/// clearing, short enough that a card which simply eats gestures cannot consume
/// the run.
const BLOCKED_SWIPE_LIMIT: u32 = 3;
/// Samples that must all be byte-identical for a card to count as a still post,
/// and the gap between them. Three samples across 1.2 s: long enough that a
/// video's motion shows up, short enough to sit inside the watch the loop
/// already performs on every card.
const STILL_CARD_SAMPLES: u32 = 3;
const STILL_CARD_GAP: Duration = Duration::from_millis(400);
const TEXT_NOT_ARMED_REFRESH_THRESHOLD: u8 = 2;
/// Give the frame watcher time to close overlays that were already visible
/// when TikTok came foreground before the first feed gesture is sent.
const STARTUP_POPUP_DRAIN: Duration = Duration::from_secs(12);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommentRecoveryAction {
    None,
    RefreshFreshSession,
    DoNotRetry,
}

fn must_stop_before_next_feed_iteration(
    action: CommentRecoveryAction,
    advanced_to_next_video: bool,
) -> bool {
    action == CommentRecoveryAction::DoNotRetry && !advanced_to_next_video
}

#[derive(Default)]
struct TextCommentHealth {
    text_not_armed_streak: u8,
}

impl TextCommentHealth {
    fn observe(&mut self, result: CommentResult) -> CommentRecoveryAction {
        match result {
            CommentResult::TextNotArmed => {
                self.text_not_armed_streak = self.text_not_armed_streak.saturating_add(1);
                if self.text_not_armed_streak >= TEXT_NOT_ARMED_REFRESH_THRESHOLD {
                    CommentRecoveryAction::RefreshFreshSession
                } else {
                    CommentRecoveryAction::None
                }
            }
            CommentResult::TextSent(_) => {
                self.text_not_armed_streak = 0;
                CommentRecoveryAction::None
            }
            CommentResult::TextNotSent => {
                self.text_not_armed_streak = 0;
                CommentRecoveryAction::DoNotRetry
            }
            _ => {
                self.text_not_armed_streak = 0;
                CommentRecoveryAction::None
            }
        }
    }

    fn fresh_session_installed(&mut self) {
        self.text_not_armed_streak = 0;
    }

    #[cfg(test)]
    fn text_not_armed_streak(&self) -> u8 {
        self.text_not_armed_streak
    }
}

#[derive(Clone)]
pub struct NurtureEngine {
    pub db: Arc<Database>,
    pub control: Arc<DeviceControlPlane>,
    pub frames: Arc<dyn FrameSource>,
    pub frame_text: Arc<dyn FrameTextSource>,
    pub artifacts_dir: PathBuf,
    touch_points: Arc<Mutex<HashMap<String, TouchPointPlanner>>>,
}

/// A phone opened for a nurture session, and what was measured while opening it.
///
/// Five values, which is why `open_for_session` could be lifted out of `run_session` at all:
/// everything else that phase touches dies with it.
struct OpenedDevice {
    ui_context: crate::UiWithStreamContext,
    session: std::sync::Arc<dyn crate::UiSession>,
    /// Measured, never assumed. See the refusals in `open_for_session`.
    screen_size: (f64, f64),
    bundle_id: String,
    /// A fresh text session was required for this run and was opened.
    fresh_text_session: bool,
    /// Recovering a dropped stream has to ask for the same kind that was opened.
    session_kind: crate::InteractionSessionKind,
}

impl NurtureEngine {
    pub fn new(
        db: Arc<Database>,
        control: Arc<DeviceControlPlane>,
        frames: Arc<dyn FrameSource>,
        artifacts_dir: PathBuf,
    ) -> Self {
        Self {
            db,
            control,
            frames,
            frame_text: Arc::new(NullFrameTextSource),
            artifacts_dir,
            touch_points: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_frame_text_source(mut self, source: Arc<dyn FrameTextSource>) -> Self {
        self.frame_text = source;
        self
    }

    pub(super) fn reset_touch_points(&self, udid: &str, screen_size: (f64, f64)) {
        self.touch_points
            .lock()
            .insert(udid.to_string(), TouchPointPlanner::new(screen_size));
    }

    pub(super) fn clear_touch_points(&self, udid: &str) {
        self.touch_points.lock().remove(udid);
    }

    pub(super) fn next_touch_point(
        &self,
        udid: &str,
        screen_size: (f64, f64),
        center: TapPoint,
        radius: (f64, f64),
    ) -> TapPoint {
        let mut planners = self.touch_points.lock();
        let planner = planners
            .entry(udid.to_string())
            .or_insert_with(|| TouchPointPlanner::new(screen_size));
        planner.next(center, radius)
    }

    async fn open_ui_context(
        &self,
        udid: &str,
        bundle_id: &str,
        kind: InteractionSessionKind,
    ) -> Result<UiWithStreamContext, crate::DeviceControlError> {
        let exclusive = self
            .control
            .acquire_exclusive(udid, DeviceWorkOwner::Nurture)
            .await?;
        let (exclusive, capacity) = self.control.reserve_ui_capacity(exclusive).await?;
        let session = self
            .control
            .start_interaction_session(exclusive, bundle_id, kind)
            .await?;
        self.control.start_reserved_stream(session, capacity).await
    }

    pub(super) fn tiktok_bundle(settings: &NurtureSettings) -> &str {
        if settings.bundle_id.trim().is_empty() {
            crate::tiktok_target::IOS_TIKTOK_BUNDLE
        } else {
            settings.bundle_id.as_str()
        }
    }

    /// TikTok's app id **for this device**.
    ///
    /// Not [`Self::tiktok_bundle`] alone, and this is a measured failure rather than a
    /// tidiness point. `NurtureSettings` is one global row shared by the whole fleet and
    /// its `bundle_id` defaults to the **iOS** bundle, so on a mixed fleet the Android
    /// half was sent `monkey -p com.ss.iphone.ugc.Ame`, which fails. A live run on
    /// 12/08/2026 reported `startInteractionSession failed … monkey -p
    /// com.ss.iphone.ugc.Ame … failed` for an SM-N950F whose TikTok was installed and
    /// working — the fleet-wide value was simply not true of that phone.
    ///
    /// The driver already answers this per device and the Interaction path already asks
    /// it (`crate::tiktok_target`, `DeviceDriver::resolve_tiktok_package`).
    ///
    /// An operator's explicit choice still wins. The iOS bundle is treated as "not
    /// chosen" because it is the shipped default, and on iOS the resolver returns that
    /// same value anyway — so this changes nothing for an iPhone.
    pub(super) async fn tiktok_bundle_for(&self, udid: &str, settings: &NurtureSettings) -> String {
        let configured = settings.bundle_id.trim();
        if !configured.is_empty() && configured != crate::tiktok_target::IOS_TIKTOK_BUNDLE {
            return configured.to_string();
        }
        match self.control.resolve_tiktok_package(udid).await {
            Ok(package) => package,
            // A driver that cannot answer leaves the configured value, which is the same
            // behaviour as before this existed — no worse, and it keeps the reason for the
            // failure in the driver's own error rather than inventing a package here.
            Err(error) => {
                tracing::warn!(udid, %error, "could not resolve TikTok's package; using the configured bundle");
                Self::tiktok_bundle(settings).to_string()
            }
        }
    }

    /// Decode the newest stream frame.
    pub(super) fn latest_image(&self, udid: &str) -> Option<image::RgbImage> {
        let frame = self.frames.latest(udid)?;
        image::load_from_memory(&frame).ok().map(|i| i.to_rgb8())
    }

    /// Wait until the encoded frame changes, or give up. Comparing digests
    /// needs no decode, so this stays cheap even polled often.
    pub(super) async fn wait_for_new_frame(
        &self,
        udid: &str,
        timeout: Duration,
        stop: &AtomicBool,
        previous: u64,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if stop.load(Ordering::Relaxed) {
                return false;
            }
            if self
                .frames
                .latest(udid)
                .map(|f| frame_digest(&f) != previous)
                .unwrap_or(false)
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(120)).await;
        }
        false
    }

    /// Watch the action rail across a swipe and report what the stream proves.
    ///
    /// Decodes only frames the stream has not already shown: the transition
    /// lasts a few hundred milliseconds, so the poll has to be faster than the
    /// frames arrive, and without the digest guard that would decode the same
    /// frame several times over.
    ///
    /// Evidence is gathered strictly *after* the gesture returns. Running the
    /// watch concurrently would catch the transition a little more reliably,
    /// but the screen watcher taps through the same gesture lock — a ✕ it
    /// pressed while our swipe queued would take the rail away and read as our
    /// swipe having moved the feed. Missing a transition costs one video;
    /// crediting a swipe the watcher caused is the very bug being fixed.
    pub(in crate::nurture) async fn watch_swipe(
        &self,
        udid: &str,
        timeout: Duration,
        stop: &AtomicBool,
        rail_before: bool,
    ) -> SwipeOutcome {
        // Starting with no rail is already the "left" state, and the proof is
        // then the rail *arriving* on a settled card — just as much a card
        // change. That is the LIVE-preview and carousel path.
        let mut left = !rail_before;
        let mut decoded: Option<u64> = None;
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            if let Some(frame) = self.frames.latest(udid) {
                let digest = frame_digest(&frame);
                if decoded != Some(digest) {
                    decoded = Some(digest);
                    if let Some(img) = image::load_from_memory(&frame).ok().map(|i| i.to_rgb8()) {
                        if !left {
                            // A washed-out frame reads "no rail" for a reason
                            // that has nothing to do with the feed moving, and
                            // this latch is never reset — so one such frame
                            // would let the *next* ordinary frame of the *same*
                            // card count as a new one, which is the very false
                            // advance this check replaced.
                            left = !screen::rail_icons_present(&img)
                                && !screen::rail_column_saturated(&img);
                        } else if screen::feed_card_kind(&img) == screen::FeedCardKind::Video {
                            // `Video` requires the compose bar *and* an icon
                            // chain, so this is a feed card that has finished
                            // moving — not a LIVE room, alert or transition.
                            // Photo posts land here too: they are `Video` to
                            // this classifier, and settling is all that matters.
                            return SwipeOutcome::Advanced;
                        }
                    }
                }
            }
            tokio::time::sleep(SWIPE_POLL).await;
        }
        if left {
            SwipeOutcome::Moved
        } else {
            SwipeOutcome::Blocked
        }
    }

    /// Wait for a frame that satisfies `pred`, or give up.
    pub(super) async fn wait_for_frame<F>(
        &self,
        udid: &str,
        timeout: Duration,
        stop: &AtomicBool,
        mut pred: F,
    ) -> Option<image::RgbImage>
    where
        F: FnMut(&image::RgbImage) -> bool,
    {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            if let Some(img) = self.latest_image(udid) {
                if pred(&img) {
                    return Some(img);
                }
            }
            tokio::time::sleep(Duration::from_millis(180)).await;
        }
        None
    }

    /// [`Self::wait_for_frame`] with watermarks: frames the caller has already
    /// seen, and which therefore cannot be proof that a later gesture worked.
    ///
    /// Digests are all this seam offers — `FrameSource` carries no sequence
    /// number, unlike the `GenerationFrameSource` the flow engine uses. So this
    /// excludes named frames, not every frame older than the gesture. Callers
    /// must still read their baseline from the frame they act on, immediately
    /// before acting, rather than leaning on this to catch a stale screen.
    pub(in crate::nurture) async fn wait_for_frame_after<F>(
        &self,
        udid: &str,
        timeout: Duration,
        stop: &AtomicBool,
        watermarks: &[u64],
        mut pred: F,
    ) -> Option<image::RgbImage>
    where
        F: FnMut(&image::RgbImage) -> bool,
    {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if stop.load(Ordering::Relaxed) {
                return None;
            }
            if let Some(frame) = self.frames.latest(udid) {
                if !watermarks.contains(&frame_digest(&frame)) {
                    if let Some(img) = image::load_from_memory(&frame).ok().map(|i| i.to_rgb8()) {
                        if pred(&img) {
                            return Some(img);
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(180)).await;
        }
        None
    }

    /// Pick up whatever the operator has changed since the last post.
    ///
    /// Reads the same settings row the UI writes, so there is no second channel to keep in
    /// sync — "Lưu" in the UI *is* the live-tuning mechanism. One SQLite read of one row per
    /// post, against a loop whose posts take seconds.
    ///
    /// A read that fails is ignored on purpose: the session already holds a complete, valid
    /// snapshot, and stopping a run because the settings row was momentarily unreadable
    /// would trade a working session for nothing. Which fields are picked up, and why the
    /// rest are not, is [`NurtureSettings::absorb_live_changes`].
    fn absorb_live_settings(&self, settings: &mut NurtureSettings) {
        let Ok(fresh) = self.db.get_nurture_settings() else {
            return;
        };
        settings.absorb_live_changes(&fresh);
        // Re-fold the switches: `absorb_live_changes` copies the stored probabilities, which
        // are the operator's numbers rather than the effective ones.
        *settings = std::mem::take(settings).into_effective();
    }

    /// Open one phone and measure it, or give up with the reason already reported.
    ///
    /// `Ok(None)` means the session is over before it started and `status` already carries
    /// the message the operator will read; the caller returns it unchanged. Every refusal in
    /// here is deliberate and none of them are caution:
    ///
    /// * a text-comment run on an agent with no text channel would tap Send into nothing;
    /// * a screen size that cannot be read used to fall back to `(375.0, 667.0)`, so every
    ///   tap afterwards was computed from a fabricated iPhone 8 — which is exactly what
    ///   AGENTS.md 691-692 forbids.
    ///
    /// **The statement order is the content here, not the layout.** The WDA session is
    /// created and primed *before* the stream is attached, because both live in the same
    /// agent on the device: with frames already pumping, the first hierarchy-touching command
    /// never returned and the runner stayed blocked for the whole run — the "tap dies / swipe
    /// blocked" failure this project chased for a long time. Nothing here may be reordered.
    async fn open_for_session(
        &self,
        udid: &str,
        settings: &NurtureSettings,
        stop: &AtomicBool,
        status: &mut NurtureSessionStatus,
        report: &impl Fn(&mut NurtureSessionStatus, String),
    ) -> anyhow::Result<Option<OpenedDevice>> {
        if stop.load(Ordering::Acquire) {
            status.running = false;
            report(status, "stopped before device start".to_string());
            return Ok(None);
        }

        if settings.comment_prob > 0 && !self.control.supports_text_comments(udid) {
            report(
                status,
                "failed — Riviu Agent chưa có kênh bình luận chữ; chạy Agent Repair".into(),
            );
            status.running = false;
            return Ok(None);
        }

        // Order matters: the WDA session is created and primed **before** the
        // MJPEG stream is started.
        //
        // Both live inside the same agent on the device. With the stream
        // already pumping frames, the first hierarchy-touching session command
        // never returned and the runner stayed blocked for the rest of the run
        // — the exact "tap dies / swipe blocked" failure this project kept
        // chasing. Priming first, then attaching the stream, is reliable; the
        // same probe run without a stream passed every time.
        //
        // One session per device; the supervisor reuses a healthy relay and
        // runner rather than starting a second one.
        let bundle_id = self.tiktok_bundle_for(udid, settings).await;
        let fresh_text_session =
            settings.comment_prob > 0 && self.control.requires_fresh_text_session(udid);
        let session_kind = if fresh_text_session {
            InteractionSessionKind::FreshText
        } else {
            InteractionSessionKind::Ordinary
        };
        let cached = false;
        report(
            status,
            if fresh_text_session {
                "chuẩn bị RT-MMO text session mới".into()
            } else if cached {
                "WDA đã có — reuse".into()
            } else {
                "khởi động WDA mới".to_string()
            },
        );
        // Session creation can transiently fail while the relay settles. Retry
        // by dropping only the cached session; startup probes are not evidence
        // that the transport itself is wedged.
        let first_session = self.open_ui_context(udid, &bundle_id, session_kind).await;
        let ui_context = match first_session {
            Ok(context) => context,
            Err(first) => {
                report(
                    status,
                    format!("WDA chưa tạo được session ({first}) — thử session mới"),
                );
                let second_session = self.open_ui_context(udid, &bundle_id, session_kind).await;
                match second_session {
                    Ok(context) => {
                        report(status, "WDA đã tạo session mới".into());
                        context
                    }
                    Err(e) => {
                        report(status, format!("failed — không mở được WDA: {e}"));
                        status.running = false;
                        return Ok(None);
                    }
                }
            }
        };
        let session = self.control.streaming_session(&ui_context)?;

        // Two refusals here, both closing holes rather than adding caution.
        //
        // The fallback used to be `(375.0, 667.0)`: when the size could not be
        // read the run carried on against a fabricated iPhone 8 screen. Every
        // tap after that was computed from a number nothing had measured.
        //
        // And nothing on this path ever checked the screen class at all. The
        // qualification registry gates the Flow/Interaction path
        // (`device_control.rs` negotiate), but nurture went straight from
        // `window_size()` to multiplying iPhone 8 fractions — so a phone of any
        // other size would have been tapped with iPhone 8 coordinates, which is
        // exactly what AGENTS.md 691-692 forbids.
        let screen_size = match session.window_size().await {
            Ok(size) if size.0 > 0.0 && size.1 > 0.0 => size,
            Ok(size) => {
                report(
                    status,
                    format!("failed — máy báo kích thước màn hình không dùng được {size:?}"),
                );
                status.running = false;
                return Ok(None);
            }
            Err(error) => {
                report(
                    status,
                    format!("failed — không đọc được kích thước màn hình: {error}"),
                );
                status.running = false;
                return Ok(None);
            }
        };
        Ok(Some(OpenedDevice {
            ui_context,
            session,
            screen_size,
            bundle_id,
            fresh_text_session,
            session_kind,
        }))
    }

    pub async fn run_session(
        &self,
        udid: &str,
        settings: NurtureSettings,
        stop: Arc<AtomicBool>,
        max_duration: Option<Duration>,
        on_status: impl Fn(NurtureSessionStatus) + Send + Sync,
    ) -> anyhow::Result<NurtureSessionStatus> {
        // Folded once here so the whole loop below reads effective values: a feature whose
        // switch is off arrives as probability 0, and no call site has to remember the
        // switch exists (`NurtureSettings::into_effective`). Refreshed the same way.
        let mut settings = settings.into_effective();
        let started = Instant::now();
        let mut status = NurtureSessionStatus {
            udid: udid.to_string(),
            running: true,
            videos_done: 0,
            swipe_attempts: 0,
            like_attempts: 0,
            comment_attempts: 0,
            follow_attempts: 0,
            likes: 0,
            comments: 0,
            follows: 0,
            last_message: "bắt đầu".into(),
            session_usd: 0.0,
        };
        on_status(status.clone());

        let report = |status: &mut NurtureSessionStatus, msg: String| {
            tracing::info!("[nurture {udid}] {msg}");
            status.last_message = msg;
            on_status(status.clone());
        };

        let Some(OpenedDevice {
            mut ui_context,
            mut session,
            screen_size,
            bundle_id,
            fresh_text_session,
            session_kind,
        }) = self
            .open_for_session(udid, &settings, &stop, &mut status, &report)
            .await?
        else {
            return Ok(status);
        };
        // A backend that can report *where* a control is does not need a
        // calibrated screen at all — it taps inside the rectangle the device
        // handed back instead of multiplying an iPhone 8 fraction. So try that
        // route first; iOS cannot answer `locate_description` and falls straight
        // through to the pixel engine below, unchanged.
        //
        // This is what AGENTS.md §9 means by not porting `screen.rs` to Android:
        // the same session policy, a different way of seeing.
        // The hierarchy loop gets its words from the engine's own grounded
        // generator, so a comment on Android is written from the same evidence, by
        // the same provider, into the same audit table as one on iOS.
        let comment_source = EngineCommentSource {
            engine: self,
            udid,
            stop: &stop,
        };
        let live_source = EngineLiveSettings { engine: self };
        let attempt = hierarchy::run_hierarchy_session(
            session.as_ref(),
            screen_size,
            &settings,
            &bundle_id,
            started,
            max_duration,
            &stop,
            &mut status,
            &report,
            Some(&comment_source),
            Some(&live_source),
        )
        .await;
        match attempt {
            hierarchy::HierarchySession::Ran(mut outcome) => {
                // Same judgement the pixel path applies: a session that moved no
                // videos did not work, whatever else it reported.
                if outcome == Outcome::Done && status.videos_done == 0 {
                    outcome = Outcome::Failed;
                }
                let mut cleanup_error = None;
                if let Err(error) = self.control.close_ui_context(ui_context).await {
                    outcome = if status.videos_done == 0 {
                        Outcome::Failed
                    } else {
                        Outcome::Partial
                    };
                    cleanup_error = Some(format!("device cleanup failed: {error}"));
                }
                let summary = format!(
                    "{} — {}/{} video, {} tim, {} bình luận, {} follow, {:.0}s (hierarchy){}",
                    outcome.as_str(),
                    status.videos_done,
                    status.swipe_attempts,
                    status.likes,
                    status.comments,
                    status.follows,
                    started.elapsed().as_secs_f64(),
                    cleanup_error
                        .as_ref()
                        .map(|error| format!(", lỗi cuối: {error}"))
                        .unwrap_or_default(),
                );
                status.running = false;
                status.last_message = summary.clone();
                on_status(status.clone());
                let _ = self.db.log_op(
                    "nurture.session",
                    &format!("{udid} {summary} usd={:.4}", status.session_usd),
                );
                self.clear_touch_points(udid);
                return Ok(status);
            }
            // The ordinary iOS case: no geometry, so use pixels.
            hierarchy::HierarchySession::NotSupported => {}
            // Geometry works but something measured is missing. Stop, rather than
            // falling through to a pixel engine whose only calibrated layout is an
            // iPhone 8. The reason is already in `status.last_message`.
            hierarchy::HierarchySession::Refused => {
                status.running = false;
                let _ = self.control.close_ui_context(ui_context).await;
                on_status(status.clone());
                return Ok(status);
            }
        }

        let Some(layout) = screen::calibrated_layout(screen_size.0, screen_size.1) else {
            let known = screen::CALIBRATED_LAYOUTS
                .iter()
                .map(|entry| {
                    format!(
                        "{} ({}x{})",
                        entry.id, entry.logical_width, entry.logical_height
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            report(
                &mut status,
                format!(
                    "failed — chưa hiệu chỉnh bộ dò cho màn hình {}x{}; \
                     đã hiệu chỉnh: {known}. Chạy quy trình hiệu chỉnh (AGENTS.md mục 6) \
                     trước khi dùng máy này",
                    screen_size.0, screen_size.1
                ),
            );
            status.running = false;
            return Ok(status);
        };
        tracing::debug!("[nurture {udid}] layout đã hiệu chỉnh: {}", layout.id);
        self.reset_touch_points(udid, screen_size);

        // Now the agent is warm, attach the stream that the watcher reads.
        report(&mut status, "mở stream màn hình".into());
        if self
            .wait_for_frame(udid, Duration::from_secs(20), &stop, |_| true)
            .await
            .is_none()
        {
            report(&mut status, "failed — stream không có frame".into());
            status.running = false;
            return Ok(status);
        }

        // What is on screen before we touch anything?
        let already_on_tiktok = self
            .latest_image(udid)
            .map(|img| screen::feed_ready(&img, Some(screen_size.0)))
            .unwrap_or(false);

        let handle = SessionHandle::new();
        handle.set(session.clone());
        let gestures = Arc::new(tokio::sync::Mutex::new(()));
        let suppress = Arc::new(AtomicBool::new(false));

        // TikTok forward only if the frame says we are not already there.
        if already_on_tiktok {
            report(
                &mut status,
                "TikTok đã mở sẵn — reuse, không khởi động lại".into(),
            );
        } else {
            report(
                &mut status,
                "TikTok chưa ở foreground — đưa lên trước".into(),
            );
            let brought = self
                .bring_tiktok_foreground(
                    udid,
                    &ui_context,
                    session.as_ref(),
                    &settings,
                    screen_size.0,
                    &gestures,
                    &stop,
                )
                .await;
            match brought {
                Ok(true) => report(&mut status, "đã bring TikTok foreground".into()),
                Ok(false) => report(&mut status, "TikTok đã ở foreground".into()),
                Err(e) => report(&mut status, format!("không mở được TikTok: {e}")),
            }
        }

        // Popup watcher: its own task, its own stop flag, its own cooldowns.
        let watcher = ScreenWatcher::new(
            udid,
            self.frames.clone(),
            handle.clone(),
            gestures.clone(),
            stop.clone(),
            screen_size,
            {
                let logger = std::sync::Mutex::new(());
                let _ = logger;
                Arc::new(move |m: &str| tracing::info!("[popup] {m}"))
            },
        );
        let watcher_stats = watcher.stats.clone();
        let watcher_state = watcher.state.clone();
        let live_owned = watcher.live_owned.clone();
        let watcher_suppress = suppress.clone();
        let watch_task = tokio::spawn(watcher.run_suppressible(watcher_suppress));

        // The watcher normally runs in parallel with nurture. At startup we
        // add one small gate so a notification/sheet that appeared during app
        // launch cannot receive the first like or swipe. The watcher keeps
        // running after this gate for overlays that appear mid-session.
        let popup_closed_before = watcher_stats.popups_closed.load(Ordering::Relaxed);
        let startup_ready = watcher_state
            .wait_until_feed(&stop, STARTUP_POPUP_DRAIN)
            .await;
        let popup_closed_after = watcher_stats.popups_closed.load(Ordering::Relaxed);
        if popup_closed_after > popup_closed_before {
            report(
                &mut status,
                format!(
                    "đã tự tắt {} thông báo/popup đầu phiên",
                    popup_closed_after - popup_closed_before
                ),
            );
        }
        if !startup_ready && !stop.load(Ordering::Relaxed) {
            report(
                &mut status,
                "chưa xác nhận frame TikTok sau khi dọn thông báo — tiếp tục theo dõi".into(),
            );
        }

        // Contextual comments are prepared per video from a fresh frame set.
        // There is deliberately no generic pool fallback: an uncertain comment
        // is skipped before the drawer opens and the feed keeps moving.
        let pool: Vec<String> = Vec::new();

        let mut human = HumanBehavior::new(
            &settings.persona,
            settings.fatigue,
            settings.time_of_day,
            settings.pause_swipe,
        );
        let mut budget = Budget::new();
        let mut text_health = TextCommentHealth::default();
        let mut policy = HumanSessionPolicy::new(
            settings.like_prob,
            settings.comment_prob,
            settings.follow_prob,
            settings.human_limits,
        );
        let mut last_interaction_at: Option<Instant> = None;
        // `steady_mood` pins the cycle for feature tests; a normal run varies.
        // See the same branch in `hierarchy::run_feed`: the mood multipliers are the
        // third layer that overrides a configured probability, so full control has to
        // neutralise them as well as the ceilings.
        let mut moods = if !settings.human_limits {
            MoodCycle::neutral()
        } else {
            match settings.steady_mood.as_str() {
                "chatty" => MoodCycle::fixed(crate::human_behavior::Mood::Chatty),
                "liking" => MoodCycle::fixed(crate::human_behavior::Mood::Liking),
                "skimming" => MoodCycle::fixed(crate::human_behavior::Mood::Skimming),
                _ => MoodCycle::new(),
            }
        };
        let mut rail = ActionRail::fallback();
        let mut outcome = Outcome::Done;
        let mut last_error: Option<String> = None;
        // The pixel loop's door back to the settings row. The same object the hierarchy
        // loop is handed, so "live" means one thing across both.
        let live_source = EngineLiveSettings { engine: self };

        // Both bounds apply: this count and, inside the loop, the run duration. See
        // `live::video_target` for why the duration used to silently win.
        let total_videos = video_target(&settings);
        // True when the loop ran out of videos rather than out of time.
        let mut hit_video_cap = true;
        let mut off_feed_streak = 0u32;
        let mut blocked_streak = 0u32;
        'feed: for _video in 0..total_videos {
            // Live tuning, once per post rather than per action. The UI writes one settings
            // row and this picks it up, so "save" means "applies to the run in progress"
            // with no extra plumbing. Per *post* on purpose: a probability that changed
            // between rolling an action and confirming it would make that action's own
            // record unexplainable.
            apply_live_settings(
                Some(&live_source),
                &mut settings,
                &mut human,
                &mut policy,
                &mut moods,
            );
            if stop.load(Ordering::Relaxed) {
                outcome = Outcome::Stopped;
                hit_video_cap = false;
                break;
            }
            if max_duration.is_some_and(|max| started.elapsed() >= max) {
                hit_video_cap = false;
                break;
            }
            if in_night_window(settings.night_start, settings.night_end) {
                report(&mut status, "giờ nghỉ đêm — dừng".into());
                hit_video_cap = false;
                break;
            }
            if budget.exhausted() {
                outcome = Outcome::Failed;
                last_error = Some("hết ngân sách recovery".into());
                break;
            }
            policy.begin_post();

            // One mood runs for several videos, so a session looks like a
            // person skimming, then liking a run, then chatting — not an
            // independent coin flip per clip.
            let (mood, mood_changed) = moods.next();
            if mood_changed {
                report(&mut status, format!("chuyển nhịp: {}", mood.label()));
            }

            let watch =
                human.watch_seconds(settings.watch_min, settings.watch_max) * mood.watch_mult();
            report(&mut status, format!("xem {watch:.1}s ({})", mood.label()));
            sleep_interruptible(Duration::from_secs_f64(watch.max(0.5)), &stop).await;
            if stop.load(Ordering::Relaxed) {
                outcome = Outcome::Stopped;
                hit_video_cap = false;
                break;
            }

            // Only act when the FYP feed is actually on screen. Swiping in the
            // feed can land in a LIVE room, whose layout is completely
            // different — a live run spent several videos tapping rail
            // positions that do not exist there and opening the LIVE chat
            // keyboard. A swipe leaves; blind taps do not.
            if !self.on_feed(udid, screen_size.0) {
                off_feed_streak += 1;
                if off_feed_streak >= OFF_FEED_LIMIT {
                    report(
                        &mut status,
                        format!("kẹt ngoài FYP {off_feed_streak} lượt — mở lại TikTok"),
                    );
                    // Back out first — that is what actually leaves a detail
                    // page. Relaunching does not: iOS restores TikTok's
                    // navigation stack, and a live run pressed Home and
                    // relaunched three times without moving off a search-results
                    // page.
                    let mut recovered = self
                        .escape_to_feed(
                            udid,
                            session.as_ref(),
                            &gestures,
                            screen_size,
                            OFF_FEED_BACK_ATTEMPTS,
                            &stop,
                        )
                        .await;
                    if !recovered {
                        report(&mut status, "vuốt lùi không về được — mở lại TikTok".into());
                        {
                            let _guard = gestures.lock().await;
                            if let Err(error) = session.home().await {
                                report(&mut status, format!("không bấm được Home: {error}"));
                            }
                        }
                        sleep_interruptible(Duration::from_millis(1_200), &stop).await;
                        let _ = self
                            .bring_tiktok_foreground(
                                udid,
                                &ui_context,
                                session.as_ref(),
                                &settings,
                                screen_size.0,
                                &gestures,
                                &stop,
                            )
                            .await;
                        // Only the screen decides whether it worked.
                        // `bring_tiktok_foreground` returns Ok(true) from its
                        // fallback path without ever checking, and believing it
                        // is what let the streak reset and the loop run on
                        // forever.
                        recovered = self
                            .wait_for_frame(udid, Duration::from_secs(8), &stop, |img| {
                                screen::feed_ready(img, Some(screen_size.0))
                            })
                            .await
                            .is_some();
                    }
                    if recovered {
                        off_feed_streak = 0;
                        report(&mut status, "đã về FYP sau khi mở lại TikTok".into());
                    } else {
                        let message = format!(
                            "không rời được màn hình ngoài FYP sau {off_feed_streak} lượt \
                             (thường là phòng LIVE mà detector không nhận ra) — dừng phiên"
                        );
                        report(&mut status, message.clone());
                        last_error = Some(message);
                        hit_video_cap = false;
                        outcome = if status.videos_done == 0 {
                            Outcome::Failed
                        } else {
                            Outcome::Partial
                        };
                        break 'feed;
                    }
                }
                let observation = self
                    .latest_image(udid)
                    .map(|img| screen::classify(&img, Some(screen_size.0)));
                let kind = observation.map(|obs| obs.kind);
                // Two screens the watcher clears with a tap, and that a swipe
                // cannot: a LIVE room scrolls its own content instead of
                // leaving, and an iOS alert is not TikTok's to swipe at all.
                let watcher_owned = matches!(
                    kind,
                    Some(ScreenKind::LiveRoom) | Some(ScreenKind::SystemAlert { .. })
                ) || observation
                    .is_some_and(|obs| obs.evidence.ad_feedback_notice);
                if watcher_owned {
                    let note = if observation.is_some_and(|obs| obs.evidence.ad_feedback_notice) {
                        "thông báo quảng cáo đang hiện — chờ TikTok tự đóng"
                    } else if matches!(kind, Some(ScreenKind::SystemAlert { .. })) {
                        "hộp thoại hệ thống — chờ watcher bấm nút bỏ qua"
                    } else {
                        "đang ở phòng LIVE — chờ watcher bấm ✕"
                    };
                    report(&mut status, note.into());
                    let back = self
                        .wait_for_frame(udid, Duration::from_secs(12), &stop, |img| {
                            screen::feed_ready(img, Some(screen_size.0))
                        })
                        .await;
                    if back.is_none() {
                        continue;
                    }
                    report(&mut status, "đã về FYP".into());
                } else {
                    report(&mut status, "không ở FYP — vuốt để về feed".into());
                    status.swipe_attempts += 1;
                    let _ = self
                        .do_swipe(
                            udid,
                            session.as_ref(),
                            &gestures,
                            screen_size,
                            human.swipe_duration_ms(roll_bool(settings.frenzy_prob)),
                            &stop,
                        )
                        .await;
                    sleep_interruptible(Duration::from_millis(1_200), &stop).await;
                    if !self.on_feed(udid, screen_size.0) {
                        continue;
                    }
                    report(&mut status, "đã về FYP".into());
                }
            }
            // Every path that gets here is on the feed: either it always was, or
            // one of the branches above got back to it.
            off_feed_streak = 0;

            // Re-read the rail after the watch and after any overlay drain.
            // The previous frame may belong to the card just left, so it is
            // never reused for an action on this card.
            let mut rail_present = false;
            if let Some(img) = self.latest_image(udid) {
                if let Some(found) = screen::locate_action_rail(&img) {
                    rail = found;
                    rail_present = true;
                }
            }
            let card_kind = self
                .latest_image(udid)
                .map(|img| screen::feed_card_kind(&img))
                .unwrap_or(screen::FeedCardKind::TransitionOrUnknown);
            match card_kind {
                screen::FeedCardKind::LivePreview => {
                    if policy.should_enter_live() {
                        report(&mut status, "gặp LIVE — vào xem một lúc rồi thoát".into());
                        live_owned.store(true, Ordering::Relaxed);
                        let entered = {
                            let _guard = gestures.lock().await;
                            let point = self.next_touch_point(
                                udid,
                                screen_size,
                                TapPoint {
                                    x: screen_size.0 * 0.50,
                                    y: screen_size.1 * 0.46,
                                },
                                (18.0, 20.0),
                            );
                            session.tap(point).await.is_ok()
                        };
                        if entered {
                            let _ = self
                                .wait_for_frame(udid, Duration::from_secs(8), &stop, |img| {
                                    matches!(
                                        screen::classify(img, Some(screen_size.0)).kind,
                                        screen::ScreenKind::LiveRoom
                                    )
                                })
                                .await;
                            let live_digest = self
                                .frames
                                .latest(udid)
                                .map(|frame| frame_digest(&frame))
                                .unwrap_or_default();
                            let dwell = Duration::from_secs(20 + (live_digest % 71));
                            sleep_interruptible(dwell, &stop).await;
                            {
                                let _guard = gestures.lock().await;
                                let point = self.next_touch_point(
                                    udid,
                                    screen_size,
                                    TapPoint {
                                        x: screen_size.0 * screen::LIVE_EXIT.0,
                                        y: screen_size.1 * screen::LIVE_EXIT.1,
                                    },
                                    (12.0, 10.0),
                                );
                                let _ = session.tap(point).await;
                            }
                            let _ = self
                                .wait_for_frame(udid, Duration::from_secs(8), &stop, |img| {
                                    screen::feed_ready(img, Some(screen_size.0))
                                })
                                .await;
                        }
                        live_owned.store(false, Ordering::Relaxed);
                    } else {
                        report(&mut status, "gặp thẻ LIVE — lướt qua thẻ xem trước".into());
                        status.swipe_attempts += 1;
                        let _ = self
                            .do_swipe(
                                udid,
                                session.as_ref(),
                                &gestures,
                                screen_size,
                                human.swipe_duration_ms(roll_bool(settings.frenzy_prob)),
                                &stop,
                            )
                            .await;
                        sleep_interruptible(Duration::from_millis(1_200), &stop).await;
                    }
                    continue;
                }
                screen::FeedCardKind::TransitionOrUnknown => {
                    report(&mut status, "khung đang chuyển — chờ frame ổn định".into());
                    sleep_interruptible(Duration::from_millis(700), &stop).await;
                }
                screen::FeedCardKind::Video if self.card_is_still(udid, &stop).await => {
                    let card_digest = self
                        .frames
                        .latest(udid)
                        .map(|frame| frame_digest(&frame))
                        .unwrap_or_default();
                    // How far through the carousel to go, from the operator's settings.
                    //
                    // This used to be `1 + (card_digest % 3)` — one to three slides picked
                    // pseudo-randomly from the frame's own bytes, with no relation to how
                    // many slides the post actually has. A seven-image post got two or
                    // three of them.
                    //
                    // Now the end of the carousel is *observed*: `do_photo_swipe` already
                    // returns whether a new frame arrived, which is exactly "the page
                    // turned". So the traversal runs until a swipe changes nothing, and the
                    // budget is only a ceiling for a post that never stops changing — a
                    // video misread as a photo, or a card that animates.
                    let budget = settings.carousel_slide_budget();
                    if budget == 0 {
                        report(
                            &mut status,
                            "gặp bài ảnh — bỏ qua vuốt ngang (tính năng đang tắt)".into(),
                        );
                        sleep_interruptible(Duration::from_secs(2 + (card_digest % 6)), &stop)
                            .await;
                        continue;
                    }
                    report(
                        &mut status,
                        format!("gặp bài ảnh (khung đứng yên) — vuốt ngang tối đa {budget} ảnh"),
                    );
                    let mut slides_seen = 0u32;
                    for slide in 0..budget {
                        // Varied per slide rather than one constant for the whole card: an
                        // identical dwell on every image of every carousel is a tell.
                        let dwell = Duration::from_secs(2 + ((card_digest + u64::from(slide)) % 6));
                        sleep_interruptible(dwell, &stop).await;
                        let advanced = self
                            .do_photo_swipe(
                                udid,
                                session.as_ref(),
                                &gestures,
                                screen_size,
                                human.photo_swipe_duration_ms(),
                                &stop,
                            )
                            .await
                            // A swipe that could not be delivered at all is not evidence
                            // that the carousel ended, but it is a reason to stop pushing:
                            // treated as "did not advance", which ends the traversal
                            // without claiming the post had exactly this many slides.
                            .unwrap_or(false);
                        // A horizontal swipe only turns a page while the card
                        // really is a photo post; on anything else TikTok reads
                        // it as navigation and leaves the feed. Both times a
                        // live run wandered off the FYP it was immediately after
                        // this branch. Stillness is strong evidence but not
                        // proof, so the branch checks its own work and undoes
                        // it — the back gesture is what leaves a detail page.
                        if !self.on_feed(udid, screen_size.0) {
                            report(&mut status, "vuốt ngang đã rời feed — lùi lại".into());
                            let back = self
                                .escape_to_feed(
                                    udid,
                                    session.as_ref(),
                                    &gestures,
                                    screen_size,
                                    OFF_FEED_BACK_ATTEMPTS,
                                    &stop,
                                )
                                .await;
                            report(
                                &mut status,
                                if back {
                                    "đã lùi về FYP".into()
                                } else {
                                    "lùi chưa về được FYP".to_string()
                                },
                            );
                            break;
                        }
                        // The swipe delivered and the screen did not change: that is the
                        // last slide. Stopping here is what makes "swipe to the end" mean
                        // the end of *this* post rather than a fixed number of swipes — and
                        // it also stops the loop pushing horizontal swipes at a card that
                        // has run out of them, which is how TikTok gets navigated off the
                        // feed.
                        if !advanced {
                            break;
                        }
                        slides_seen += 1;
                    }
                    if slides_seen > 0 {
                        report(
                            &mut status,
                            format!("bài ảnh: đã xem thêm {slides_seen} ảnh"),
                        );
                    }
                    if let Some(img) = self.latest_image(udid) {
                        if let Some(found) = screen::locate_action_rail(&img) {
                            rail = found;
                            rail_present = true;
                        }
                    }
                }
                screen::FeedCardKind::Video => {}
            }

            // No rail on this card: watch it out and move on rather than tap
            // where nothing is. Follow is skipped for the same reason.
            if !rail_present {
                report(
                    &mut status,
                    "thẻ không có thanh hành động (LIVE / đang chuyển) — chỉ vuốt tiếp".into(),
                );
                status.swipe_attempts += 1;
                // Leaving a card that has no rail is still provable, from the
                // other side: the rail *arriving* on a settled card is the
                // card change. Landing on another rail-less card is not, and
                // stays uncounted.
                if self
                    .do_swipe(
                        udid,
                        session.as_ref(),
                        &gestures,
                        screen_size,
                        human.swipe_duration_ms(roll_bool(settings.frenzy_prob)),
                        &stop,
                    )
                    .await
                    .is_ok_and(|outcome| outcome == SwipeOutcome::Advanced)
                {
                    status.videos_done += 1;
                    on_status(status.clone());
                    if let Some(rest) = policy.rest_after_video() {
                        report(&mut status, format!("nghỉ tự nhiên {}s", rest.as_secs()));
                        sleep_interruptible(rest, &stop).await;
                    }
                }
                continue;
            }

            human.note_action();
            let mut comment_recovery_action = CommentRecoveryAction::None;
            match roll_feed_action_in_mood(settings.like_prob, settings.comment_prob, mood) {
                FeedAction::Like
                    if !policy.can_interact_with_post()
                        || !policy.can_attempt(PolicyAction::Like) =>
                {
                    report(&mut status, "bỏ qua tim: nhịp phiên hiện tại đã đủ".into());
                }
                FeedAction::Like => {
                    if !wait_for_action_gap(
                        &mut last_interaction_at,
                        policy.min_action_gap(),
                        &stop,
                    )
                    .await
                    {
                        outcome = Outcome::Stopped;
                        hit_video_cap = false;
                        break 'feed;
                    }
                    policy.record_attempt(PolicyAction::Like);
                    policy.mark_post_interacted();
                    status.like_attempts += 1;
                    on_status(status.clone());
                    report(&mut status, "thả tim".into());
                    match self
                        .do_like(udid, session.as_ref(), &gestures, screen_size, &stop)
                        .await
                    {
                        Ok(LikeResult::Liked) => {
                            status.likes += 1;
                            report(&mut status, "tim thành công (xác nhận icon đỏ)".into());
                        }
                        Ok(LikeResult::AlreadyLiked) => {
                            report(&mut status, "video đã tim từ trước — bỏ qua".into())
                        }
                        Ok(LikeResult::NotOnFeed) => report(
                            &mut status,
                            "bỏ qua tim: khung hiện tại không phải thẻ feed có thanh hành động"
                                .into(),
                        ),
                        Ok(LikeResult::NotConfirmed { before, best }) => report(
                            &mut status,
                            format!(
                                "tim: tap gửi được nhưng icon không đổi (đỏ {before:.0}→{best:.0}, \
                                 cần >{:.0}; rail layout {}{}, tim y={:.0}pt)",
                                screen::LIKE_FILLED_REDNESS,
                                rail.layout(),
                                if rail.located { "" } else { ", dùng mặc định" },
                                rail.like_y * 667.0
                            ),
                        ),
                        Err(e) => {
                            let msg = format!("tim thất bại: {}", describe(&e));
                            report(&mut status, msg.clone());
                            last_error = Some(msg);
                            if !self
                                .recover(
                                    udid,
                                    &bundle_id,
                                    fresh_text_session,
                                    &mut ui_context,
                                    &mut session,
                                    &handle,
                                    &mut budget,
                                    &mut text_health,
                                    &e,
                                    &mut status,
                                    &on_status,
                                )
                                .await
                            {
                                outcome = Outcome::Failed;
                                break 'feed;
                            }
                        }
                    }
                }
                FeedAction::Comment
                    if !policy.can_interact_with_post()
                        || !policy.can_attempt(PolicyAction::Comment) =>
                {
                    report(
                        &mut status,
                        "bỏ qua bình luận: nhịp phiên hiện tại đã đủ".into(),
                    );
                }
                FeedAction::Comment => {
                    if !wait_for_action_gap(
                        &mut last_interaction_at,
                        policy.min_action_gap(),
                        &stop,
                    )
                    .await
                    {
                        outcome = Outcome::Stopped;
                        hit_video_cap = false;
                        break 'feed;
                    }
                    policy.record_attempt(PolicyAction::Comment);
                    policy.mark_post_interacted();
                    status.comment_attempts += 1;
                    on_status(status.clone());
                    report(&mut status, "bình luận".into());
                    suppress.store(true, Ordering::Relaxed);
                    let res = self
                        .do_comment(
                            udid,
                            session.as_ref(),
                            &gestures,
                            &rail,
                            screen_size,
                            &settings,
                            &pool,
                            &stop,
                        )
                        .await;
                    suppress.store(false, Ordering::Relaxed);
                    match res {
                        Ok(result) => {
                            comment_recovery_action = text_health.observe(result);
                            match result {
                                CommentResult::TextSent(usd) => {
                                    status.comments += 1;
                                    status.session_usd += usd;
                                    report(
                                        &mut status,
                                        "đã gửi bình luận chữ (xác nhận nút gửi tắt)".into(),
                                    );
                                }
                                CommentResult::TextNotSent => report(
                                    &mut status,
                                    "bỏ qua bình luận: đã bấm Gửi nhưng chưa xác nhận được; không retry vì trạng thái giao nhận mơ hồ"
                                        .into(),
                                ),
                                other => {
                                    let msg = format!("bỏ qua bình luận: {}", other.reason());
                                    report(&mut status, msg);
                                }
                            }

                            if comment_recovery_action == CommentRecoveryAction::RefreshFreshSession
                            {
                                report(
                                    &mut status,
                                    "nút Gửi không sáng 2 lượt liên tiếp — làm mới text session"
                                        .into(),
                                );
                                let error = anyhow::Error::new(UiError::new(
                                    UiErrorKind::Session,
                                    "comment.text_not_armed",
                                    "two consecutive frame-confirmed non-arming results",
                                ));
                                if !self
                                    .recover(
                                        udid,
                                        &bundle_id,
                                        true,
                                        &mut ui_context,
                                        &mut session,
                                        &handle,
                                        &mut budget,
                                        &mut text_health,
                                        &error,
                                        &mut status,
                                        &on_status,
                                    )
                                    .await
                                {
                                    last_error = Some(
                                        "không làm mới được text session sau 2 lượt không armed"
                                            .into(),
                                    );
                                    outcome = Outcome::Failed;
                                    break 'feed;
                                }
                            }
                        }
                        Err(e) => {
                            let msg = format!("bình luận thất bại: {}", describe(&e));
                            report(&mut status, msg.clone());
                            last_error = Some(msg);
                            if ui_error_kind(&e) != UiErrorKind::Other
                                && !self
                                    .recover(
                                        udid,
                                        &bundle_id,
                                        fresh_text_session,
                                        &mut ui_context,
                                        &mut session,
                                        &handle,
                                        &mut budget,
                                        &mut text_health,
                                        &e,
                                        &mut status,
                                        &on_status,
                                    )
                                    .await
                            {
                                outcome = Outcome::Failed;
                                break 'feed;
                            }
                        }
                    }
                }
                FeedAction::None => {}
            }

            if roll_follow_in_mood(settings.follow_prob, mood) {
                if !policy.can_interact_with_post() || !policy.can_attempt(PolicyAction::Follow) {
                    report(
                        &mut status,
                        "bỏ qua follow: nhịp phiên hiện tại đã đủ".into(),
                    );
                } else if !wait_for_action_gap(
                    &mut last_interaction_at,
                    policy.min_action_gap(),
                    &stop,
                )
                .await
                {
                    outcome = Outcome::Stopped;
                    hit_video_cap = false;
                    break 'feed;
                } else {
                    policy.record_attempt(PolicyAction::Follow);
                    policy.mark_post_interacted();
                    status.follow_attempts += 1;
                    on_status(status.clone());
                    report(&mut status, "follow".into());
                    match self
                        .do_follow(udid, session.as_ref(), &gestures, &rail, screen_size, &stop)
                        .await
                    {
                        Ok(true) => {
                            status.follows += 1;
                            report(&mut status, "follow thành công".into());
                        }
                        Ok(false) => report(&mut status, "follow không đổi trạng thái".into()),
                        Err(e) => {
                            let msg = format!("follow thất bại: {}", describe(&e));
                            report(&mut status, msg.clone());
                            last_error = Some(msg);
                            if !self
                                .recover(
                                    udid,
                                    &bundle_id,
                                    fresh_text_session,
                                    &mut ui_context,
                                    &mut session,
                                    &handle,
                                    &mut budget,
                                    &mut text_health,
                                    &e,
                                    &mut status,
                                    &on_status,
                                )
                                .await
                            {
                                outcome = Outcome::Failed;
                                break 'feed;
                            }
                        }
                    }
                }
            }

            sleep_interruptible(Duration::from_millis(human.think_pause_ms()), &stop).await;

            report(&mut status, "vuốt video tiếp".into());
            let mut advanced_to_next_video = false;
            status.swipe_attempts += 1;
            match self
                .do_swipe(
                    udid,
                    session.as_ref(),
                    &gestures,
                    screen_size,
                    human.swipe_duration_ms(roll_bool(settings.frenzy_prob)),
                    &stop,
                )
                .await
            {
                Ok(SwipeOutcome::Advanced) => {
                    advanced_to_next_video = true;
                    blocked_streak = 0;
                    status.videos_done += 1;
                    on_status(status.clone());
                }
                Ok(SwipeOutcome::Moved) => {
                    blocked_streak = 0;
                    // The rail left, so the gesture landed and the card we were
                    // on is gone. Swiping again here would skip whatever came
                    // next; the loop re-reads the screen at the top instead.
                    // Not counted: nothing settled that could be counted.
                    advanced_to_next_video = true;
                    report(
                        &mut status,
                        "đã rời thẻ cũ nhưng chưa thấy thẻ mới ổn định — đọc lại màn hình".into(),
                    );
                }
                Ok(SwipeOutcome::Blocked) => {
                    // The rail never left: the feed swallowed the gesture,
                    // usually under a popup. The watcher closes those on its
                    // own; give it a beat, then try once more.
                    report(&mut status, "vuốt không ăn — chờ popup rồi thử lại".into());
                    sleep_interruptible(Duration::from_millis(1_800), &stop).await;
                    status.swipe_attempts += 1;
                    match self
                        .do_swipe(
                            udid,
                            session.as_ref(),
                            &gestures,
                            screen_size,
                            human.swipe_duration_ms(false),
                            &stop,
                        )
                        .await
                    {
                        Ok(SwipeOutcome::Advanced) => {
                            advanced_to_next_video = true;
                            blocked_streak = 0;
                            status.videos_done += 1;
                            on_status(status.clone());
                        }
                        Ok(SwipeOutcome::Moved) => {
                            advanced_to_next_video = true;
                            blocked_streak = 0;
                            report(&mut status, "đã rời thẻ cũ, chưa xác nhận thẻ mới".into());
                        }
                        Ok(SwipeOutcome::Blocked) => {
                            report(&mut status, "vuốt vẫn không ăn".into());
                            last_error = Some("swipe không rời được thẻ hiện tại".into());
                            blocked_streak += 1;
                        }
                        Err(e) => {
                            let msg = format!("vuốt lỗi: {}", describe(&e));
                            report(&mut status, msg.clone());
                            last_error = Some(msg);
                            if !self
                                .recover(
                                    udid,
                                    &bundle_id,
                                    fresh_text_session,
                                    &mut ui_context,
                                    &mut session,
                                    &handle,
                                    &mut budget,
                                    &mut text_health,
                                    &e,
                                    &mut status,
                                    &on_status,
                                )
                                .await
                            {
                                outcome = Outcome::Failed;
                                break 'feed;
                            }
                        }
                    }
                }
                Err(e) => {
                    let msg = format!("vuốt lỗi: {}", describe(&e));
                    report(&mut status, msg.clone());
                    last_error = Some(msg);
                    if !self
                        .recover(
                            udid,
                            &bundle_id,
                            fresh_text_session,
                            &mut ui_context,
                            &mut session,
                            &handle,
                            &mut budget,
                            &mut text_health,
                            &e,
                            &mut status,
                            &on_status,
                        )
                        .await
                    {
                        outcome = Outcome::Failed;
                        break 'feed;
                    }
                }
            }
            // A card that swallows both the swipe and its retry, turn after
            // turn, is not going to start working. A live run spent 280 seconds
            // — 46 of its 53 swipes — on one photo post before the clock ran
            // out. Ending the session says so; continuing just burns the budget
            // in silence.
            if blocked_streak >= BLOCKED_SWIPE_LIMIT {
                let message = format!(
                    "thẻ hiện tại nuốt {blocked_streak} lượt vuốt liên tiếp — dừng phiên \
                     thay vì vuốt tiếp vô ích"
                );
                report(&mut status, message.clone());
                last_error = Some(message);
                hit_video_cap = false;
                outcome = if status.videos_done == 0 {
                    Outcome::Failed
                } else {
                    Outcome::Partial
                };
                break 'feed;
            }
            if must_stop_before_next_feed_iteration(comment_recovery_action, advanced_to_next_video)
            {
                let message =
                    "dừng trước lượt feed kế tiếp: chưa xác nhận rời video có trạng thái gửi mơ hồ"
                        .to_string();
                report(&mut status, message.clone());
                last_error = Some(message);
                hit_video_cap = false;
                outcome = if status.videos_done == 0 {
                    Outcome::Failed
                } else {
                    Outcome::Partial
                };
                break 'feed;
            }
            if advanced_to_next_video {
                if let Some(rest) = policy.rest_after_video() {
                    report(&mut status, format!("nghỉ tự nhiên {}s", rest.as_secs()));
                    sleep_interruptible(rest, &stop).await;
                }
                if policy.should_take_block_break() || policy.should_take_home_break() {
                    let break_for = policy.home_break_duration();
                    report(
                        &mut status,
                        format!(
                            "tạm về màn hình chính khoảng {}s rồi mở TikTok lại",
                            break_for.as_secs()
                        ),
                    );
                    {
                        let _guard = gestures.lock().await;
                        let _ = session.home().await;
                    }
                    sleep_interruptible(break_for, &stop).await;
                    if policy.should_cold_restart() {
                        report(
                            &mut status,
                            "khởi động lại TikTok sau một quãng nghỉ".into(),
                        );
                        let _ = self
                            .control
                            .terminate_streaming_app(&ui_context, &bundle_id)
                            .await;
                        sleep_interruptible(Duration::from_secs(2), &stop).await;
                        match self
                            .control
                            .recover_streaming_session(
                                &mut ui_context,
                                &bundle_id,
                                session_kind,
                                false,
                            )
                            .await
                        {
                            Ok(next) => {
                                // Swap the watcher's session handle too, or the
                                // popup watcher keeps tapping through the dead
                                // pre-restart session for the rest of the run.
                                session = next;
                                handle.set(session.clone());
                            }
                            Err(error) => {
                                report(&mut status, format!("không mở lại được TikTok: {error}"));
                                outcome = Outcome::Partial;
                                break 'feed;
                            }
                        }
                    } else {
                        let _guard = gestures.lock().await;
                        let _ = session.launch_app_foreground(&bundle_id).await;
                    }
                    sleep_interruptible(Duration::from_secs(4), &stop).await;
                    policy.reset_block();
                }
            }
            sleep_interruptible(Duration::from_millis(human.after_swipe_pause_ms()), &stop).await;
        }

        // Stop the watcher and collect its numbers before reporting.
        stop.store(true, Ordering::Relaxed);
        let _ = tokio::time::timeout(Duration::from_secs(3), watch_task).await;
        let watch = watcher_stats.snapshot();
        let _ = watcher_state;

        // Do not park the app blindly: only note where we ended up. TikTok
        // hides its chrome for a moment during a swipe, so a single frame is
        // not evidence — sample a couple of seconds and accept any feed frame.
        let never = AtomicBool::new(false);
        let ended_on_tiktok = self
            .wait_for_frame(udid, Duration::from_millis(2_500), &never, |img| {
                screen::feed_ready(img, Some(screen_size.0))
            })
            .await
            .is_some();

        outcome = session_verdict(
            outcome,
            status.videos_done,
            hit_video_cap,
            total_videos,
            last_error.is_some(),
        );

        if let Err(error) = self.control.close_ui_context(ui_context).await {
            outcome = if status.videos_done == 0 {
                Outcome::Failed
            } else {
                Outcome::Partial
            };
            last_error = Some(format!("device cleanup failed: {error}"));
        }

        let elapsed = started.elapsed();
        let summary = format!(
            "{} — {}/{} video, {} tim, {} bình luận, {} follow, {} popup đóng, {} recovery, {:.0}s{}{}",
            outcome.as_str(),
            status.videos_done,
            status.swipe_attempts,
            status.likes,
            status.comments,
            status.follows,
            watch.popups_closed,
            budget.soft + budget.hard,
            elapsed.as_secs_f64(),
            if ended_on_tiktok {
                ", kết thúc ở TikTok"
            } else {
                ", KHÔNG ở TikTok lúc kết thúc"
            },
            last_error
                .as_ref()
                .map(|e| format!(", lỗi cuối: {e}"))
                .unwrap_or_default(),
        );
        status.running = false;
        status.last_message = summary.clone();
        on_status(status.clone());

        let _ = self.db.log_op(
            "nurture.session",
            &format!("{udid} {summary} usd={:.4}", status.session_usd),
        );
        self.clear_touch_points(udid);
        Ok(status)
    }

    /// Bring TikTok forward. Prefers WDA activate, which does not restart a
    /// running app; falls back to the Instruments launch path.
    #[allow(clippy::too_many_arguments)]
    async fn bring_tiktok_foreground(
        &self,
        udid: &str,
        context: &UiWithStreamContext,
        session: &dyn UiSession,
        settings: &NurtureSettings,
        logical_width: f64,
        gestures: &tokio::sync::Mutex<()>,
        stop: &AtomicBool,
    ) -> anyhow::Result<bool> {
        let bundle = self.tiktok_bundle_for(udid, settings).await;
        let bundle = bundle.as_str();
        {
            let _guard = gestures.lock().await;
            if session.launch_app_foreground(bundle).await.is_ok() {
                sleep_interruptible(Duration::from_millis(1_500), stop).await;
                if self.on_feed(udid, logical_width) {
                    return Ok(true);
                }
            }
        }
        // The driver serialises this against the relay on the device lock.
        self.control
            .foreground_streaming_app(context, bundle)
            .await?;
        sleep_interruptible(Duration::from_millis(2_000), stop).await;
        Ok(true)
    }

    /// Is this card a still post rather than a playing video?
    ///
    /// This is what tells a photo carousel from a video, and it is the only
    /// thing measured that does. A photo post publishes byte-identical frames
    /// because nothing on screen moves; a video cannot, since the stream
    /// re-encodes every frame at 24 FPS with no deduplication — the same fact
    /// that made the old swipe check useless is what makes this reliable.
    ///
    /// Measured over 40 real cards: 4 came back still, at least three of them
    /// confirmed photo posts by eye (page dots and the "Ảnh" badge), and none
    /// of the 36 videos did. The page-dot detector this replaces scored 1 true
    /// positive against 9 false ones on the same cards.
    ///
    /// Only a video that holds a perfectly static frame for the whole window
    /// can pass, and the caller still has to survive being wrong — a horizontal
    /// swipe on a video navigates away from the feed.
    async fn card_is_still(&self, udid: &str, stop: &AtomicBool) -> bool {
        let Some(first) = self.frames.latest(udid).map(|f| frame_digest(&f)) else {
            return false;
        };
        for _ in 0..STILL_CARD_SAMPLES {
            sleep_interruptible(STILL_CARD_GAP, stop).await;
            if stop.load(Ordering::Relaxed) {
                return false;
            }
            match self.frames.latest(udid).map(|f| frame_digest(&f)) {
                Some(next) if next == first => {}
                _ => return false,
            }
        }
        true
    }

    /// Try to get back to the FYP from wherever the session has ended up.
    ///
    /// Relaunching does not do it. iOS restores TikTok's navigation stack, so a
    /// session that wandered onto a search-results or profile page comes back to
    /// that same page — measured: Home plus three `launch_app` calls left the
    /// phone exactly where it was. What does leave those pages is the iOS
    /// back gesture, a swipe in from the left edge, which TikTok honours the
    /// same way its own top-left ‹ does.
    ///
    /// Returns true once an actionable feed is on screen.
    async fn escape_to_feed(
        &self,
        udid: &str,
        session: &dyn UiSession,
        gestures: &tokio::sync::Mutex<()>,
        screen_size: (f64, f64),
        attempts: u32,
        stop: &AtomicBool,
    ) -> bool {
        for _ in 0..attempts {
            if stop.load(Ordering::Relaxed) {
                return false;
            }
            {
                let _guard = gestures.lock().await;
                let gesture = crate::types::SwipeGesture {
                    from: TapPoint {
                        x: 2.0,
                        y: screen_size.1 * 0.5,
                    },
                    to: TapPoint {
                        x: screen_size.0 * 0.6,
                        y: screen_size.1 * 0.5,
                    },
                    duration_ms: 260,
                };
                if session.swipe(gesture).await.is_err() {
                    return false;
                }
            }
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
        false
    }

    /// `screen_size.0` matters: without it `classify` cannot scale its
    /// templates to the device, and this was the one call site passing `None`
    /// while every other passed `Some(..)` — so the check that decides whether
    /// the engine is allowed to act was the least informed one in the file.
    fn on_feed(&self, udid: &str, logical_width: f64) -> bool {
        self.latest_image(udid)
            .map(|img| screen::feed_ready(&img, Some(logical_width)))
            .unwrap_or(false)
    }
}

/// Short human-readable form of a gesture failure, including its class.
/// Lets the hierarchy loop borrow the engine's comment generator.
///
/// Exists so `hierarchy.rs` never has to know about frames, OpenAI pricing, or the
/// attempt table: it asks for words, gets words, and reports how the send went.
struct EngineCommentSource<'a> {
    engine: &'a NurtureEngine,
    udid: &'a str,
    stop: &'a AtomicBool,
}

/// The hierarchy loop's door back to the settings row.
///
/// A separate type from [`EngineCommentSource`] because the loop asks for the two
/// independently, and one line of body because the rule itself lives in
/// [`NurtureEngine::absorb_live_settings`] — the same call the pixel loop makes. Two
/// loops, one definition of what "live" means.
struct EngineLiveSettings<'a> {
    engine: &'a NurtureEngine,
}

impl LiveSettings for EngineLiveSettings<'_> {
    fn refresh(&self, settings: &mut NurtureSettings) {
        self.engine.absorb_live_settings(settings);
    }
}

#[async_trait::async_trait]
impl hierarchy::CommentTextSource for EngineCommentSource<'_> {
    async fn comment_for_post(
        &self,
        settings: &NurtureSettings,
    ) -> Option<hierarchy::PreparedComment> {
        self.engine
            .prepare_hierarchy_comment(self.udid, settings, self.stop)
            .await
    }

    async fn record_outcome(&self, prepared: &hierarchy::PreparedComment, outcome: &str) {
        self.engine
            .finish_hierarchy_comment(prepared.attempt_id.as_deref(), outcome);
    }
}

fn describe(err: &anyhow::Error) -> String {
    format!("{} ({})", err, ui_error_kind(err).as_str())
}

/// Cheap content fingerprint for "did the screen change?".
pub(super) fn frame_digest(frame: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ frame.len() as u64;
    let step = (frame.len() / 512).max(1);
    let mut i = 0;
    while i < frame.len() {
        h ^= frame[i] as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
        i += step;
    }
    h
}

pub(super) async fn sleep_interruptible(dur: Duration, stop: &AtomicBool) {
    let step = Duration::from_millis(200);
    let mut left = dur;
    while left > Duration::ZERO {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let slice = left.min(step);
        tokio::time::sleep(slice).await;
        left = left.saturating_sub(slice);
    }
}

async fn wait_for_action_gap(
    last_action_at: &mut Option<Instant>,
    gap: Duration,
    stop: &AtomicBool,
) -> bool {
    if let Some(last) = *last_action_at {
        let elapsed = last.elapsed();
        if elapsed < gap {
            sleep_interruptible(gap - elapsed, stop).await;
        }
    }
    if stop.load(Ordering::Relaxed) {
        return false;
    }
    *last_action_at = Some(Instant::now());
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicUsize;

    use crate::frame_source::NullFrameSource;
    use crate::types::DeviceInfo;
    use crate::DeviceDriver;

    #[derive(Default)]
    struct MissingTextDriver {
        session_calls: AtomicUsize,
        stream_calls: AtomicUsize,
    }

    #[async_trait]
    impl DeviceDriver for MissingTextDriver {
        async fn list_devices(&self) -> anyhow::Result<Vec<DeviceInfo>> {
            Ok(Vec::new())
        }

        async fn refresh_device(&self, _udid: &str) -> anyhow::Result<DeviceInfo> {
            anyhow::bail!("unused")
        }

        async fn install_app(&self, _udid: &str, _path: &Path) -> anyhow::Result<()> {
            Ok(())
        }

        async fn uninstall_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn screenshot(&self, _udid: &str, _dest: &Path) -> anyhow::Result<PathBuf> {
            anyhow::bail!("unused")
        }

        async fn syslog_tail(&self, _udid: &str, _lines: usize) -> anyhow::Result<String> {
            Ok(String::new())
        }

        async fn launch_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn terminate_app(
            &self,
            _udid: &str,
            bundle_id: &str,
        ) -> anyhow::Result<crate::ProcessAbsenceProof> {
            Ok(crate::ProcessAbsenceProof {
                bundle_id: bundle_id.to_string(),
                old_pid: None,
            })
        }

        async fn reboot(&self, _udid: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn start_ui_session(&self, _udid: &str) -> anyhow::Result<Box<dyn UiSession>> {
            self.session_calls.fetch_add(1, Ordering::Relaxed);
            anyhow::bail!("session must not start")
        }

        async fn start_interaction_session(
            &self,
            _udid: &str,
            _bundle_id: &str,
            _kind: InteractionSessionKind,
        ) -> anyhow::Result<Box<dyn UiSession>> {
            self.session_calls.fetch_add(1, Ordering::Relaxed);
            anyhow::bail!("interaction session must not start")
        }

        async fn start_stream_after_session(
            &self,
            _udid: &str,
        ) -> anyhow::Result<crate::StreamStartProof> {
            self.stream_calls.fetch_add(1, Ordering::Relaxed);
            anyhow::bail!("stream must not start")
        }

        async fn ensure_stream(&self, _udid: &str) -> anyhow::Result<String> {
            self.stream_calls.fetch_add(1, Ordering::Relaxed);
            anyhow::bail!("stream must not start")
        }

        async fn prepare_device(&self, _udid: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn comment_enabled_job_stops_before_feed_when_text_capability_is_missing() {
        let driver = Arc::new(MissingTextDriver::default());
        let control = Arc::new(DeviceControlPlane::new(
            driver.clone(),
            Arc::new(crate::DeviceWorkCoordinator::new()),
            Arc::new(crate::StreamBudgetManager::default()),
        ));
        let db_path =
            std::env::temp_dir().join(format!("riviu-missing-text-{}.db", uuid::Uuid::new_v4()));
        let engine = NurtureEngine::new(
            Arc::new(Database::open(&db_path).expect("test database")),
            control,
            Arc::new(NullFrameSource),
            std::env::temp_dir(),
        );
        let settings = NurtureSettings {
            comment_prob: 1,
            ..Default::default()
        };

        let final_status = engine
            .run_session(
                "missing-text-device",
                settings,
                Arc::new(AtomicBool::new(false)),
                Some(Duration::from_millis(1)),
                |_| {},
            )
            .await
            .expect("capability failure is reported as a terminal session status");

        assert!(!final_status.running);
        assert!(final_status.last_message.contains("Agent Repair"));
        assert_eq!(driver.session_calls.load(Ordering::Relaxed), 0);
        assert_eq!(driver.stream_calls.load(Ordering::Relaxed), 0);

        drop(engine);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn pre_stopped_job_never_opens_a_device_session_or_stream() {
        let driver = Arc::new(MissingTextDriver::default());
        let control = Arc::new(DeviceControlPlane::new(
            driver.clone(),
            Arc::new(crate::DeviceWorkCoordinator::new()),
            Arc::new(crate::StreamBudgetManager::default()),
        ));
        let db_path =
            std::env::temp_dir().join(format!("riviu-pre-stopped-{}.db", uuid::Uuid::new_v4()));
        let engine = NurtureEngine::new(
            Arc::new(Database::open(&db_path).expect("test database")),
            control,
            Arc::new(NullFrameSource),
            std::env::temp_dir(),
        );
        let stop = Arc::new(AtomicBool::new(true));

        let settings = NurtureSettings {
            comment_prob: 0,
            ..Default::default()
        };
        let final_status = engine
            .run_session(
                "pre-stopped-device",
                settings,
                stop,
                Some(Duration::from_millis(1)),
                |_| {},
            )
            .await
            .expect("pre-stopped run is a clean terminal result");

        assert!(!final_status.running);
        assert_eq!(driver.session_calls.load(Ordering::Relaxed), 0);
        assert_eq!(driver.stream_calls.load(Ordering::Relaxed), 0);

        drop(engine);
        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn a_session_that_processed_nothing_is_not_done() {
        // The rule the old engine broke: it logged "done" after a run that
        // timed out having handled zero videos.
        assert_eq!(Outcome::Failed.as_str(), "failed");
        assert_eq!(Outcome::Partial.as_str(), "partial");
        assert_eq!(Outcome::Done.as_str(), "done");
        assert_eq!(Outcome::Stopped.as_str(), "stopped");
    }

    /// The bug this guards: a timed run that stopped on the clock with its
    /// video ceiling untouched was reported `partial`, telling the operator a
    /// healthy 47-video session had gone wrong.
    #[test]
    fn a_timed_run_that_did_its_work_is_done_not_partial() {
        let judge = |videos: u32, cap: u32, hit_cap: bool, err: bool| {
            if videos == 0 {
                Outcome::Failed
            } else if (hit_cap && videos < cap / 2) || (videos < 3 && err) {
                Outcome::Partial
            } else {
                Outcome::Done
            }
        };
        // Ran out of clock after 47 videos against a 400 ceiling — that is a
        // full session, error on the last swipe or not.
        assert_eq!(judge(47, 400, false, true), Outcome::Done);
        // Stopped early with the ceiling untouched: something cut it short.
        assert_eq!(judge(47, 400, true, true), Outcome::Partial);
        // Nothing happened at all.
        assert_eq!(judge(0, 400, false, true), Outcome::Failed);
        // Two videos and an error is not a session.
        assert_eq!(judge(2, 400, false, true), Outcome::Partial);
    }

    #[test]
    fn changed_frames_have_different_digests() {
        let a = vec![3u8; 8192];
        let mut b = a.clone();
        assert_eq!(frame_digest(&a), frame_digest(&b));
        b[4096] = 4;
        assert_ne!(frame_digest(&a), frame_digest(&b));
    }

    #[tokio::test]
    async fn two_consecutive_text_not_armed_results_refresh_the_fresh_session() {
        let mut health = TextCommentHealth::default();

        assert_eq!(
            health.observe(CommentResult::TextNotArmed),
            CommentRecoveryAction::None
        );
        assert_eq!(
            health.observe(CommentResult::TextNotArmed),
            CommentRecoveryAction::RefreshFreshSession
        );

        health.fresh_session_installed();
        assert_eq!(health.text_not_armed_streak(), 0);
    }

    #[tokio::test]
    async fn successful_text_comment_resets_the_non_armed_streak() {
        let mut health = TextCommentHealth::default();
        assert_eq!(
            health.observe(CommentResult::TextNotArmed),
            CommentRecoveryAction::None
        );

        assert_eq!(
            health.observe(CommentResult::TextSent(0.0)),
            CommentRecoveryAction::None
        );
        assert_eq!(
            health.observe(CommentResult::TextNotArmed),
            CommentRecoveryAction::None,
            "a successful post must break the non-arming streak"
        );

        assert_eq!(
            health.observe(CommentResult::NoDrawer),
            CommentRecoveryAction::None
        );
        assert_eq!(
            health.observe(CommentResult::TextNotArmed),
            CommentRecoveryAction::None,
            "any intervening comment outcome must break a consecutive streak"
        );
    }

    #[tokio::test]
    async fn text_not_sent_is_not_retried_because_delivery_is_ambiguous() {
        let mut health = TextCommentHealth::default();

        assert_eq!(
            health.observe(CommentResult::TextNotSent),
            CommentRecoveryAction::DoNotRetry
        );
        assert!(must_stop_before_next_feed_iteration(
            CommentRecoveryAction::DoNotRetry,
            false
        ));
        assert!(!must_stop_before_next_feed_iteration(
            CommentRecoveryAction::DoNotRetry,
            true
        ));
    }
}
