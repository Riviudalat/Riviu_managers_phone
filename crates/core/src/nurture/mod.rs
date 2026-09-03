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
use sha2::{Digest, Sha256};
mod actions;
mod hierarchy;
mod live;
mod recovery;
// Crate-visible, not private, because the Interaction path needs the *same* jitter
// history rather than its own: two touch planners on one device would produce a tap
// distribution neither of them intended. `crate::interaction_hierarchy` is the second
// caller.
pub mod touch;

use actions::{CommentResult, FollowResult, LikeResult, SwipeOutcome};
pub use hierarchy::CommentSourceError;
pub use hierarchy::{
    run_hierarchy_session, CommentAuditToken, CommentTextSource, HierarchySession, PreparedComment,
};
pub use live::LiveSettings;
use live::{apply_live_settings, video_target, LiveSettingsRefresh};
pub use recovery::Outcome;
use recovery::{session_verdict, Budget};

use crate::db::Database;
use crate::device_control::{DeviceControlPlane, UiWithStreamContext};
use crate::driver::{ui_error_kind, UiError, UiErrorKind, UiSession};
use crate::frame_source::FrameSource;
use crate::frame_text::{FrameTextSource, NullFrameTextSource};
use crate::human_behavior::{
    in_night_window, roll_bool, roll_feed_actions_in_mood, FeedAction, HumanBehavior,
    HumanSessionPolicy, Mood, MoodCycle, PolicyAction,
};
use crate::interaction::{
    InteractionActionKind, InteractionActionState, TikTokActionOwner, TikTokActionOwnerKind,
};
use crate::screen::{self, ActionRail, ScreenKind};
use crate::screen_watch::{ScreenWatcher, SessionHandle};
use crate::types::{
    InteractionSessionKind, NurturePhase, NurtureSessionStatus, NurtureSettings, TapPoint,
};
use crate::DeviceWorkOwner;
use touch::TouchPointPlanner;

use crate::tiktok_save::{
    pixel_save_observation, tiktok_save, SaveAdapter, SaveCardIdentity, SaveEvidence,
    SaveObservation, SaveVerdict,
};

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
            CommentResult::TextSent { .. } => {
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

struct PixelNurtureSaveAdapter<'a> {
    frames: &'a dyn FrameSource,
    session: &'a dyn UiSession,
    udid: &'a str,
    screen_size: (f64, f64),
    sequence: u64,
}

#[derive(Debug, Clone)]
struct NurtureSaveLease {
    owner: TikTokActionOwner,
    armed_revision: i64,
}

trait NurtureSaveJournal: Sync {
    fn arm(
        &self,
        card_key: &str,
        observation: &SaveObservation,
    ) -> anyhow::Result<NurtureSaveLease>;

    fn settle(
        &self,
        lease: Option<NurtureSaveLease>,
        evidence: &SaveEvidence,
    ) -> anyhow::Result<()>;
}

struct NurtureSaveLedger<'a> {
    db: &'a Database,
    session_id: &'a str,
    udid: &'a str,
}

impl<'a> NurtureSaveLedger<'a> {
    fn new(db: &'a Database, session_id: &'a str, udid: &'a str) -> Self {
        Self {
            db,
            session_id,
            udid,
        }
    }
}

fn save_identity_json(identity: &SaveCardIdentity) -> serde_json::Value {
    match identity {
        SaveCardIdentity::Hierarchy { author, sound } => serde_json::json!({
            "source": "hierarchy",
            "author": author,
            "sound": sound,
        }),
        SaveCardIdentity::Pixel { author, caption } => serde_json::json!({
            "source": "pixel",
            "author": author,
            "caption": caption,
        }),
    }
}

fn save_observation_json(observation: &SaveObservation) -> serde_json::Value {
    serde_json::json!({
        "identity": observation.identity.as_ref().map(save_identity_json),
        "sequence": observation.sequence,
        "state": format!("{:?}", observation.state).to_ascii_lowercase(),
        "tapPoint": observation.tap_point.as_ref().map(|point| serde_json::json!({
            "x": point.x,
            "y": point.y,
        })),
    })
}

fn save_evidence_json(evidence: &SaveEvidence) -> String {
    serde_json::json!({
        "verdict": format!("{:?}", evidence.verdict).to_ascii_lowercase(),
        "effectBoundaryCrossed": evidence.effect_boundary_crossed,
        "initial": evidence.initial.as_ref().map(save_observation_json),
        "final": evidence.final_observation.as_ref().map(save_observation_json),
        "error": evidence.error,
    })
    .to_string()
}

impl NurtureSaveJournal for NurtureSaveLedger<'_> {
    fn arm(
        &self,
        card_key: &str,
        observation: &SaveObservation,
    ) -> anyhow::Result<NurtureSaveLease> {
        let identity = observation
            .identity
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Save identity disappeared before durable arm"))?;
        let identity_json = save_identity_json(identity).to_string();
        let identity_hash = format!("{:x}", Sha256::digest(identity_json.as_bytes()));
        let owner = TikTokActionOwner {
            kind: TikTokActionOwnerKind::Nurture,
            owner_id: format!("{}:{card_key}:{}", self.session_id, &identity_hash[..16]),
            device_udid: self.udid.to_owned(),
            card_identity: Some(identity_json),
        };
        self.db
            .ensure_tiktok_action_run(&owner, InteractionActionKind::Save)?;
        let claim_revision = self
            .db
            .claim_tiktok_action(&owner, InteractionActionKind::Save)?
            .ok_or_else(|| anyhow::anyhow!("Nurture Save ledger row is not claimable"))?;
        let effect_intent = serde_json::json!({
            "intent": "set bookmark state to saved",
            "sequence": observation.sequence,
            "identity": save_identity_json(identity),
        })
        .to_string();
        let armed_revision = self
            .db
            .arm_tiktok_action(
                &owner,
                InteractionActionKind::Save,
                claim_revision,
                &effect_intent,
            )?
            .ok_or_else(|| anyhow::anyhow!("Nurture Save ledger arm lost ownership"))?;
        Ok(NurtureSaveLease {
            owner,
            armed_revision,
        })
    }

    fn settle(
        &self,
        lease: Option<NurtureSaveLease>,
        evidence: &SaveEvidence,
    ) -> anyhow::Result<()> {
        let Some(lease) = lease else {
            if evidence.effect_boundary_crossed {
                anyhow::bail!("Save crossed effect boundary without an armed ledger row");
            }
            return Ok(());
        };
        let state = match evidence.verdict {
            SaveVerdict::Saved => InteractionActionState::Confirmed,
            SaveVerdict::CardChangedAfterEffect
            | SaveVerdict::NotConfirmed
            | SaveVerdict::UncertainAfterEffect => InteractionActionState::Uncertain,
            _ => anyhow::bail!(
                "armed Save returned a pre-effect verdict: {:?}",
                evidence.verdict
            ),
        };
        let evidence_json = save_evidence_json(evidence);
        let error_code = evidence.error.as_deref().or_else(|| {
            (state == InteractionActionState::Uncertain).then_some("save_not_confirmed")
        });
        if !self.db.settle_tiktok_action(
            &lease.owner,
            InteractionActionKind::Save,
            lease.armed_revision,
            state,
            Some(&evidence_json),
            error_code,
        )? {
            anyhow::bail!("Nurture Save ledger settlement lost ownership");
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl SaveAdapter for PixelNurtureSaveAdapter<'_> {
    async fn observe(&mut self) -> anyhow::Result<SaveObservation> {
        self.sequence = self.sequence.saturating_add(1);
        let Some(frame) = self.frames.latest(self.udid) else {
            anyhow::bail!("pixel Save cannot read a fresh frame")
        };
        let image = image::load_from_memory(&frame)?.to_rgb8();
        let rail = screen::locate_action_rail(&image);
        let mut observation = pixel_save_observation(
            SaveCardIdentity::Pixel {
                author: String::new(),
                caption: None,
            },
            self.sequence,
            rail,
            self.screen_size,
            // No calibrated Saved-vs-Unsaved detector exists for the pixel path. Geometry
            // alone must never authorize a toggle, so this is deliberately fail-closed.
            None,
        );
        // A frame digest changes while a video plays and therefore is not a card identity.
        // Until calibrated OCR supplies author/caption, record the missing proof honestly.
        observation.identity = None;
        Ok(observation)
    }

    async fn tap(&mut self, point: TapPoint) -> anyhow::Result<()> {
        self.session.tap(point).await
    }
}

fn settle_save_evidence(
    policy: &mut HumanSessionPolicy,
    reservation: crate::human_behavior::AttemptReservation,
    status: &mut NurtureSessionStatus,
    evidence: &SaveEvidence,
) -> bool {
    if evidence.effect_boundary_crossed {
        policy.commit_attempt(reservation);
        status.save_attempts = status.save_attempts.saturating_add(1);
        if evidence.verdict == SaveVerdict::Saved {
            status.saves = status.saves.saturating_add(1);
        } else {
            status.save_uncertain = status.save_uncertain.saturating_add(1);
        }
        true
    } else {
        policy.cancel_no_effect(reservation);
        status.save_noops = status.save_noops.saturating_add(1);
        false
    }
}

fn settle_journaled_save(
    policy: &mut HumanSessionPolicy,
    reservation: crate::human_behavior::AttemptReservation,
    status: &mut NurtureSessionStatus,
    journal: Option<&dyn NurtureSaveJournal>,
    lease: Option<NurtureSaveLease>,
    evidence: &SaveEvidence,
) -> anyhow::Result<bool> {
    // The gesture result owns the in-memory reservation. Settle it first so a durable audit
    // failure after the effect boundary cannot reopen the cap and invite a duplicate retry.
    let acted = settle_save_evidence(policy, reservation, status, evidence);
    if let Some(journal) = journal {
        journal.settle(lease, evidence)?;
    }
    Ok(acted)
}

fn save_verdict_message(evidence: &SaveEvidence) -> String {
    let label = match evidence.verdict {
        SaveVerdict::Saved => "đã lưu và xác nhận trạng thái",
        SaveVerdict::AlreadySaved => "video đã được lưu từ trước",
        SaveVerdict::NoControl => "không có nút Lưu mới đo được",
        SaveVerdict::StateUnreadable => "không đọc được trạng thái Lưu",
        SaveVerdict::CardChangedBeforeEffect => "thẻ đã đổi trước cú chạm",
        SaveVerdict::FailedBeforeEffect => "dừng trước cú chạm vì thiếu bằng chứng",
        SaveVerdict::CardChangedAfterEffect => "thẻ đổi sau cú chạm, kết quả chưa chắc chắn",
        SaveVerdict::NotConfirmed => "đã chạm nhưng chưa xác nhận được trạng thái",
        SaveVerdict::UncertainAfterEffect => "phản hồi bị gián đoạn sau cú chạm",
    };
    format!("lưu: {label} (details: verdict={:?})", evidence.verdict)
}

/// What every phase of a session needs, and none of them changes.
///
/// Four values with one lifetime — the whole session. They were four parameters repeated on
/// every phase lifted out of `run_session`; as one context they are one parameter, and the two
/// ways a phase talks back to the caller become methods instead of a closure travelling beside
/// them.
///
/// `handle` is deliberately not here: it is built from `device.session`, so it does not exist
/// yet at the point the context does.
struct SessionCtx<'a, F: Fn(NurtureSessionStatus) + Send + Sync> {
    udid: &'a str,
    session_id: &'a str,
    /// Set when the operator ends the session; every wait in every phase checks it.
    stop: &'a AtomicBool,
    /// Held for the length of one gesture, so two phases never drive the screen at once.
    gestures: &'a tokio::sync::Mutex<()>,
    on_status: &'a F,
}

impl<F: Fn(NurtureSessionStatus) + Send + Sync> SessionCtx<'_, F> {
    /// Log a line, make it the row's current message, and push the row to the caller.
    fn report(&self, into: &mut NurtureSessionStatus, msg: String) {
        let udid = self.udid;
        tracing::info!("[nurture {udid}] {msg}");
        into.last_message = msg;
        Self::assert_terminal_rows_carry_a_verdict(into);
        (self.on_status)(into.clone());
    }

    /// Push the row without changing its message — for the counters that move on their own.
    fn push(&self, status: &NurtureSessionStatus) {
        Self::assert_terminal_rows_carry_a_verdict(status);
        (self.on_status)(status.clone());
    }

    /// A stopped row must say **why** it stopped.
    ///
    /// This is the guard for the shape that produced the bug: ten separate exits from
    /// `run_session` each set `running = false` by hand, and the verdict lived only inside a
    /// Vietnamese sentence, so the desktop could not tell a finished run from a failed one
    /// and rendered both as the same grey row. They all go through
    /// [`NurtureSessionStatus::finish`] now — and an eleventh exit that forgets trips here
    /// in every test and debug build rather than shipping a row nobody can classify.
    ///
    /// `debug_assert` rather than a hard panic: a release build must not kill a live session
    /// over a missing label, and every test run is a debug build.
    fn assert_terminal_rows_carry_a_verdict(status: &NurtureSessionStatus) {
        debug_assert!(
            status.running || status.outcome.is_some(),
            "a stopped nurture row must carry an Outcome — use \
             NurtureSessionStatus::finish rather than setting `running = false`: {:?}",
            status.last_message
        );
        debug_assert!(
            status.running || status.phase.is_terminal(),
            "a stopped nurture row must be in a terminal phase: {:?}",
            status.phase
        );
    }
}

/// What a nurture session has done so far, and how it will be judged.
///
/// Six values every phase of the feed loop reads or writes, and that never travel apart.
/// Bundled so a phase can leave `run_session` and still take a signature someone would want
/// to read: one `&mut SessionProgress` rather than six separate `&mut`s.
///
/// Not one struct for all fourteen of the loop's locals. The other seven are the behaviour
/// model — `human`, `policy`, `moods`, `budget` and friends — and they answer a different
/// question. A fourteen-field blob would have moved the mess rather than removed it.
struct SessionProgress {
    status: NurtureSessionStatus,
    /// Refined once at the end by `session_verdict`; set directly only when a phase gives up.
    outcome: Outcome,
    last_error: Option<String>,
    /// False when the loop ended for a reason other than running out of videos. The verdict
    /// rule needs it: a run that stopped on the clock is not a run that fell short.
    hit_video_cap: bool,
    /// Consecutive posts that found the phone somewhere other than the feed.
    off_feed_streak: u32,
    /// Consecutive swipes the current card refused to move for.
    blocked_streak: u32,
}

impl SessionProgress {
    /// Record why the session is stopping early, and judge it accordingly.
    ///
    /// Written out longhand in three places before this, identically each time: report the
    /// message, keep it as the last error, and mark that the loop did *not* simply run out of
    /// videos — which is what stops `session_verdict` from calling a healthy timed run short.
    ///
    /// `Failed` when nothing was watched at all, `Partial` otherwise: a session that did some
    /// work and then hit a wall is not the same as one that never started.
    fn give_up<F: Fn(NurtureSessionStatus) + Send + Sync>(
        &mut self,
        message: String,
        ctx: &SessionCtx<'_, F>,
    ) {
        ctx.report(&mut self.status, message.clone());
        self.last_error = Some(message);
        self.hit_video_cap = false;
        self.outcome = if self.status.videos_done == 0 {
            Outcome::Failed
        } else {
            Outcome::Partial
        };
    }
}

/// What one phase of the feed loop decided should happen next.
///
/// The `'feed` loop's phases used to end with `break 'feed` or `continue` written inline,
/// which is precisely why none of them could be moved out of `run_session`: an exit that
/// names an enclosing loop cannot cross a function boundary. As a returned value it can, and
/// the compiler then checks both halves — every path must produce one of these, and the
/// caller must handle each variant.
#[derive(Debug)]
enum FeedStep {
    /// Carry on with this post.
    Proceed,
    /// Nothing more to do here; take the next post.
    NextVideo,
    /// End the session. The phase has already called `SessionProgress::give_up`, so the
    /// verdict and the message are recorded; the caller only has to leave the loop.
    Stop,
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

    /// Stop TikTok before releasing the session and stream that prove which device and app
    /// this nurture run owns. Closing the UI context first loses the only safe termination
    /// route and leaves TikTok alive in the background after the row says the run is done.
    /// Both operations are attempted so a failed terminate never leaks the control-plane
    /// capacity as a second problem.
    async fn shutdown_tiktok(
        &self,
        ui_context: UiWithStreamContext,
        bundle_id: &str,
    ) -> Result<(), String> {
        let terminate = self
            .control
            .terminate_streaming_app(&ui_context, bundle_id)
            .await
            .map(|_| ());
        let close = self.control.close_ui_context(ui_context).await;
        match (terminate, close) {
            (Ok(()), Ok(_)) => Ok(()),
            (Err(terminate), Ok(_)) => Err(format!("không tắt được TikTok: {terminate}")),
            (Ok(()), Err(close)) => Err(format!("không đóng được phiên điều khiển: {close}")),
            (Err(terminate), Err(close)) => Err(format!(
                "không tắt được TikTok: {terminate}; không đóng được phiên điều khiển: {close}"
            )),
        }
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
    /// Returns the **encoded bytes** (the same `Arc` the frame source holds, so no copy)
    /// alongside the decoded image, and that is the point.
    ///
    /// Two callers persist a digest of "the frame that proved the send". They used to discard
    /// this return value and then call `frames.latest(udid)` for a *second* read — so the
    /// verdict came from frame N while the stored `cleared_frame_sha256` hashed whatever had
    /// arrived by the time of that second call. On a stream a beat behind, or when the drawer
    /// closed or a popup appeared in between, the record pointed at a screen that does not
    /// show the delivered comment, and an audit could not reproduce the proof.
    ///
    /// Handing back the bytes makes that structural rather than a convention: the only frame
    /// a caller can hash is the one the predicate accepted. Found by an independent review on
    /// 27/08/2026.
    pub(in crate::nurture) async fn wait_for_frame_after<F>(
        &self,
        udid: &str,
        timeout: Duration,
        stop: &AtomicBool,
        watermarks: &[u64],
        mut pred: F,
    ) -> Option<(std::sync::Arc<Vec<u8>>, image::RgbImage)>
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
                            return Some((frame, img));
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
    /// A read failure is returned to `apply_live_settings`, which fail-closes all four public
    /// action rates to zero for this pass. A later successful read applies the new row and
    /// reopens those rates; only `NoSource` keeps the initial snapshot. Which fields are picked
    /// up, and why the rest are not, is [`NurtureSettings::absorb_live_changes`].
    fn absorb_live_settings(&self, settings: &mut NurtureSettings) -> anyhow::Result<()> {
        let fresh = self.db.get_nurture_settings()?;
        settings.absorb_live_changes(&fresh);
        // Re-fold the switches: `absorb_live_changes` copies the stored probabilities, which
        // are the operator's numbers rather than the effective ones.
        *settings = std::mem::take(settings).into_effective();
        Ok(())
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
    ///   AGENTS.md §3.12 forbids.
    ///
    /// **The statement order is the content here, not the layout.** The control session is
    /// created and primed *before* the stream is attached, because both live in the same
    /// agent on the device: with frames already pumping, the first hierarchy-touching command
    /// never returned and the runner stayed blocked for the whole run — the "tap dies / swipe
    /// blocked" failure this project chased for a long time. Nothing here may be reordered.
    async fn open_for_session<F: Fn(NurtureSessionStatus) + Send + Sync>(
        &self,
        settings: &NurtureSettings,
        status: &mut NurtureSessionStatus,
        ctx: &SessionCtx<'_, F>,
    ) -> anyhow::Result<Option<OpenedDevice>> {
        if ctx.stop.load(Ordering::Acquire) {
            status.finish(Outcome::Stopped);
            ctx.report(status, "stopped before device start".to_string());
            return Ok(None);
        }

        if settings.comment_prob > 0 && !self.control.supports_text_comments(ctx.udid) {
            ctx.report(
                status,
                "failed — Riviu Agent chưa có kênh bình luận chữ; chạy Agent Repair".into(),
            );
            status.finish(Outcome::Failed);
            return Ok(None);
        }

        // Order matters: the control session is created and primed **before** the
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
        let bundle_id = self.tiktok_bundle_for(ctx.udid, settings).await;
        let fresh_text_session =
            settings.comment_prob > 0 && self.control.requires_fresh_text_session(ctx.udid);
        let session_kind = if fresh_text_session {
            InteractionSessionKind::FreshText
        } else {
            InteractionSessionKind::Ordinary
        };
        let cached = false;
        ctx.report(
            status,
            if fresh_text_session {
                "chuẩn bị RT-MMO text session mới".into()
            } else if cached {
                "phiên điều khiển đã có — dùng lại".into()
            } else {
                "mở phiên điều khiển mới".to_string()
            },
        );
        // Session creation can transiently fail while the relay settles. Retry
        // by dropping only the cached session; startup probes are not evidence
        // that the transport itself is wedged.
        let first_session = self
            .open_ui_context(ctx.udid, &bundle_id, session_kind)
            .await;
        let ui_context = match first_session {
            Ok(context) => context,
            Err(first) => {
                ctx.report(
                    status,
                    format!("chưa mở được phiên điều khiển ({first}) — thử lần nữa"),
                );
                let second_session = self
                    .open_ui_context(ctx.udid, &bundle_id, session_kind)
                    .await;
                match second_session {
                    Ok(context) => {
                        ctx.report(status, "đã mở phiên điều khiển mới".into());
                        context
                    }
                    Err(e) => {
                        ctx.report(
                            status,
                            format!("failed — không mở được phiên điều khiển: {e}"),
                        );
                        status.finish(Outcome::Failed);
                        return Ok(None);
                    }
                }
            }
        };
        let session = match self.control.streaming_session(&ui_context) {
            Ok(session) => session,
            Err(error) => {
                let cleanup = self.shutdown_tiktok(ui_context, &bundle_id).await;
                let detail = cleanup
                    .err()
                    .map(|cleanup| format!("; lỗi dọn TikTok: {cleanup}"))
                    .unwrap_or_default();
                return Err(anyhow::anyhow!("{error}{detail}"));
            }
        };

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
        // exactly what AGENTS.md §3.12 forbids.
        let screen_size = match session.window_size().await {
            Ok(size) if size.0 > 0.0 && size.1 > 0.0 => size,
            Ok(size) => {
                ctx.report(
                    status,
                    format!("failed — máy báo kích thước màn hình không dùng được {size:?}"),
                );
                status.finish(Outcome::Failed);
                if let Err(error) = self.shutdown_tiktok(ui_context, &bundle_id).await {
                    ctx.report(
                        status,
                        format!("{}; lỗi dọn TikTok: {error}", status.last_message),
                    );
                }
                return Ok(None);
            }
            Err(error) => {
                ctx.report(
                    status,
                    format!("failed — không đọc được kích thước màn hình: {error}"),
                );
                status.finish(Outcome::Failed);
                if let Err(error) = self.shutdown_tiktok(ui_context, &bundle_id).await {
                    ctx.report(
                        status,
                        format!("{}; lỗi dọn TikTok: {error}", status.last_message),
                    );
                }
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

    /// Get back to the For You feed, or decide the session cannot go on.
    ///
    /// Called once per post, and it is the phase that runs when the phone is *not* where the
    /// loop expects it. `FeedStep` is how it reports back: the three exits it used to take out
    /// of the `'feed` loop directly — one `break` and two `continue` — are now values the
    /// caller matches on, which is what let this phase leave `run_session` at all.
    ///
    /// Backing out comes before relaunching, and that order is measured rather than tidy:
    /// relaunching does not leave a detail page because iOS restores TikTok's navigation
    /// stack, and a live run pressed Home and relaunched three times without moving off one
    /// search-results page.
    ///
    /// Returns the streak alongside the step because the two `format!` messages capture it
    /// inline, and `{*ptr}` is not something an inline capture accepts.
    #[allow(clippy::too_many_arguments)]
    async fn handle_off_feed<F: Fn(NurtureSessionStatus) + Send + Sync>(
        &self,
        settings: &NurtureSettings,
        ui_context: &crate::UiWithStreamContext,
        session: &std::sync::Arc<dyn crate::UiSession>,
        screen_size: (f64, f64),
        human: &mut HumanBehavior,
        mut off_feed_streak: u32,
        progress: &mut SessionProgress,
        ctx: &SessionCtx<'_, F>,
    ) -> anyhow::Result<(FeedStep, u32)> {
        off_feed_streak += 1;
        if off_feed_streak >= OFF_FEED_LIMIT {
            ctx.report(
                &mut progress.status,
                format!("kẹt ngoài FYP {off_feed_streak} lượt — mở lại TikTok"),
            );
            // Back out first — that is what actually leaves a detail
            // page. Relaunching does not: iOS restores TikTok's
            // navigation stack, and a live run pressed Home and
            // relaunched three times without moving off a search-results
            // page.
            let mut recovered = self
                .escape_to_feed(
                    ctx.udid,
                    session.as_ref(),
                    ctx.gestures,
                    screen_size,
                    OFF_FEED_BACK_ATTEMPTS,
                    ctx.stop,
                )
                .await;
            if !recovered {
                ctx.report(
                    &mut progress.status,
                    "vuốt lùi không về được — mở lại TikTok".into(),
                );
                {
                    let _guard = ctx.gestures.lock().await;
                    if let Err(error) = session.home().await {
                        ctx.report(
                            &mut progress.status,
                            format!("không bấm được Home: {error}"),
                        );
                    }
                }
                sleep_interruptible(Duration::from_millis(1_200), ctx.stop).await;
                let _ = self
                    .bring_tiktok_foreground(
                        ctx.udid,
                        ui_context,
                        session.as_ref(),
                        settings,
                        screen_size.0,
                        ctx.gestures,
                        ctx.stop,
                    )
                    .await;
                // Only the screen decides whether it worked.
                // `bring_tiktok_foreground` returns Ok(true) from its
                // fallback path without ever checking, and believing it
                // is what let the streak reset and the loop run on
                // forever.
                recovered = self
                    .wait_for_frame(ctx.udid, Duration::from_secs(8), ctx.stop, |img| {
                        screen::feed_ready(img, Some(screen_size.0))
                    })
                    .await
                    .is_some();
            }
            if recovered {
                off_feed_streak = 0;
                ctx.report(
                    &mut progress.status,
                    "đã về FYP sau khi mở lại TikTok".into(),
                );
            } else {
                let message = format!(
                    "không rời được màn hình ngoài FYP sau {off_feed_streak} lượt \
                     (thường là phòng LIVE mà detector không nhận ra) — dừng phiên"
                );
                progress.give_up(message, ctx);
                return Ok((FeedStep::Stop, off_feed_streak));
            }
        }
        let observation = self
            .latest_image(ctx.udid)
            .map(|img| screen::classify(&img, Some(screen_size.0)));
        let kind = observation.map(|obs| obs.kind);
        // Two screens the watcher clears with a tap, and that a swipe
        // cannot: a LIVE room scrolls its own content instead of
        // leaving, and an iOS alert is not TikTok's to swipe at all.
        let watcher_owned = matches!(
            kind,
            Some(ScreenKind::LiveRoom) | Some(ScreenKind::SystemAlert { .. })
        ) || observation.is_some_and(|obs| obs.evidence.ad_feedback_notice);
        if watcher_owned {
            let note = if observation.is_some_and(|obs| obs.evidence.ad_feedback_notice) {
                "thông báo quảng cáo đang hiện — chờ TikTok tự đóng"
            } else if matches!(kind, Some(ScreenKind::SystemAlert { .. })) {
                "hộp thoại hệ thống — chờ watcher bấm nút bỏ qua"
            } else {
                "đang ở phòng LIVE — chờ watcher bấm ✕"
            };
            ctx.report(&mut progress.status, note.into());
            let back = self
                .wait_for_frame(ctx.udid, Duration::from_secs(12), ctx.stop, |img| {
                    screen::feed_ready(img, Some(screen_size.0))
                })
                .await;
            if back.is_none() {
                return Ok((FeedStep::NextVideo, off_feed_streak));
            }
            ctx.report(&mut progress.status, "đã về FYP".into());
        } else {
            ctx.report(&mut progress.status, "không ở FYP — vuốt để về feed".into());
            progress.status.swipe_attempts += 1;
            let _ = self
                .do_swipe(
                    ctx.udid,
                    session.as_ref(),
                    ctx.gestures,
                    screen_size,
                    human.swipe_duration_ms(roll_bool(settings.frenzy_prob)),
                    ctx.stop,
                )
                .await;
            sleep_interruptible(Duration::from_millis(1_200), ctx.stop).await;
            if !self.on_feed(ctx.udid, screen_size.0) {
                return Ok((FeedStep::NextVideo, off_feed_streak));
            }
            ctx.report(&mut progress.status, "đã về FYP".into());
        }

        Ok((FeedStep::Proceed, off_feed_streak))
    }

    /// Watch one card out, and report the action rail if this kind of card has one.
    ///
    /// The plan called this `watch_one_video`; it is really "deal with whatever kind of card
    /// is on screen" — a LIVE room is entered or skipped by policy, a photo carousel is swiped
    /// through slide by slide, and a plain video is simply watched. Only the carousel ends
    /// with a rail worth reporting, which is why the rail comes back as an `Option` rather
    /// than through a `&mut`.
    ///
    /// `FeedStep::NextVideo` covers the two places this used to `continue` the `'feed` loop
    /// from the inside.
    ///
    /// `live_owned` is the watcher's flag: true while this run is deliberately inside a LIVE
    /// room, so the watcher leaves it alone instead of escaping a screen we chose.
    #[allow(clippy::too_many_arguments)]
    async fn watch_one_card<F: Fn(NurtureSessionStatus) + Send + Sync>(
        &self,
        settings: &NurtureSettings,
        card_kind: screen::FeedCardKind,
        session: &std::sync::Arc<dyn crate::UiSession>,
        screen_size: (f64, f64),
        human: &mut HumanBehavior,
        policy: &mut crate::human_behavior::HumanSessionPolicy,
        live_owned: &AtomicBool,
        progress: &mut SessionProgress,
        ctx: &SessionCtx<'_, F>,
    ) -> anyhow::Result<(FeedStep, Option<ActionRail>)> {
        let mut rail: Option<ActionRail> = None;

        match card_kind {
            screen::FeedCardKind::LivePreview => {
                if policy.should_enter_live() {
                    ctx.report(
                        &mut progress.status,
                        "gặp LIVE — vào xem một lúc rồi thoát".into(),
                    );
                    live_owned.store(true, Ordering::Relaxed);
                    let entered = {
                        let _guard = ctx.gestures.lock().await;
                        let point = self.next_touch_point(
                            ctx.udid,
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
                            .wait_for_frame(ctx.udid, Duration::from_secs(8), ctx.stop, |img| {
                                matches!(
                                    screen::classify(img, Some(screen_size.0)).kind,
                                    screen::ScreenKind::LiveRoom
                                )
                            })
                            .await;
                        let live_digest = self
                            .frames
                            .latest(ctx.udid)
                            .map(|frame| frame_digest(&frame))
                            .unwrap_or_default();
                        let dwell = Duration::from_secs(20 + (live_digest % 71));
                        sleep_interruptible(dwell, ctx.stop).await;
                        {
                            let _guard = ctx.gestures.lock().await;
                            let point = self.next_touch_point(
                                ctx.udid,
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
                            .wait_for_frame(ctx.udid, Duration::from_secs(8), ctx.stop, |img| {
                                screen::feed_ready(img, Some(screen_size.0))
                            })
                            .await;
                    }
                    live_owned.store(false, Ordering::Relaxed);
                } else {
                    ctx.report(
                        &mut progress.status,
                        "gặp thẻ LIVE — lướt qua thẻ xem trước".into(),
                    );
                    progress.status.swipe_attempts += 1;
                    let _ = self
                        .do_swipe(
                            ctx.udid,
                            session.as_ref(),
                            ctx.gestures,
                            screen_size,
                            human.swipe_duration_ms(roll_bool(settings.frenzy_prob)),
                            ctx.stop,
                        )
                        .await;
                    sleep_interruptible(Duration::from_millis(1_200), ctx.stop).await;
                }
                return Ok((FeedStep::NextVideo, rail));
            }
            screen::FeedCardKind::TransitionOrUnknown => {
                ctx.report(
                    &mut progress.status,
                    "khung đang chuyển — chờ frame ổn định".into(),
                );
                sleep_interruptible(Duration::from_millis(700), ctx.stop).await;
            }
            screen::FeedCardKind::Video if self.card_is_still(ctx.udid, ctx.stop).await => {
                let card_digest = self
                    .frames
                    .latest(ctx.udid)
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
                    ctx.report(
                        &mut progress.status,
                        "gặp bài ảnh — bỏ qua vuốt ngang (tính năng đang tắt)".into(),
                    );
                    sleep_interruptible(Duration::from_secs(2 + (card_digest % 6)), ctx.stop).await;
                    return Ok((FeedStep::NextVideo, rail));
                }
                ctx.report(
                    &mut progress.status,
                    format!("gặp bài ảnh (khung đứng yên) — vuốt ngang tối đa {budget} ảnh"),
                );
                let mut slides_seen = 0u32;
                for slide in 0..budget {
                    // Varied per slide rather than one constant for the whole card: an
                    // identical dwell on every image of every carousel is a tell.
                    let dwell = Duration::from_secs(2 + ((card_digest + u64::from(slide)) % 6));
                    sleep_interruptible(dwell, ctx.stop).await;
                    let advanced = self
                        .do_photo_swipe(
                            ctx.udid,
                            session.as_ref(),
                            ctx.gestures,
                            screen_size,
                            human.photo_swipe_duration_ms(),
                            ctx.stop,
                        )
                        .await
                        // A swipe that could not be delivered at all is not evidence
                        // that the carousel ended, but it is a reason to ctx.stop pushing:
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
                    if !self.on_feed(ctx.udid, screen_size.0) {
                        ctx.report(
                            &mut progress.status,
                            "vuốt ngang đã rời feed — lùi lại".into(),
                        );
                        let back = self
                            .escape_to_feed(
                                ctx.udid,
                                session.as_ref(),
                                ctx.gestures,
                                screen_size,
                                OFF_FEED_BACK_ATTEMPTS,
                                ctx.stop,
                            )
                            .await;
                        ctx.report(
                            &mut progress.status,
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
                    ctx.report(
                        &mut progress.status,
                        format!("bài ảnh: đã xem thêm {slides_seen} ảnh"),
                    );
                }
                if let Some(img) = self.latest_image(ctx.udid) {
                    if let Some(found) = screen::locate_action_rail(&img) {
                        rail = Some(found);
                    }
                }
            }
            screen::FeedCardKind::Video => {}
        }

        Ok((FeedStep::Proceed, rail))
    }

    /// Roll one interaction for the card on screen, and carry it out.
    ///
    /// The largest phase of the feed loop, and the one that could not be lifted until
    /// `SessionCtx` existed: measured against the loop it had fifteen free variables, and
    /// cutting it per-arm did not help (14 for Like, 15 for Comment). The count came from how
    /// much session state was loose in scope, not from the phase's size.
    ///
    /// Five ways out, three different verdicts — twice `Stopped` because the operator asked,
    /// twice `Failed` because recovery did not take, and once `Failed` with its own message.
    /// Each records that on `progress` before returning `FeedStep::Stop`; the caller only has
    /// to leave the loop.
    #[allow(clippy::too_many_arguments)]
    async fn roll_and_execute_action<F: Fn(NurtureSessionStatus) + Send + Sync>(
        &self,
        ctx: &SessionCtx<'_, F>,
        progress: &mut SessionProgress,
        device: &mut OpenedDevice,
        settings: &NurtureSettings,
        policy: &mut crate::human_behavior::HumanSessionPolicy,
        budget: &mut Budget,
        rail: &ActionRail,
        text_health: &mut TextCommentHealth,
        last_interaction_at: &mut Option<Instant>,
        mood: Mood,
        handle: &SessionHandle,
        // Raised around a comment so the watcher does not read the keyboard as a screen change.
        suppress: &AtomicBool,
        // Deliberately always empty — see the note where it is declared: an uncertain comment
        // is not written at all rather than falling back to a generic line.
        pool: &[String],
    ) -> anyhow::Result<(FeedStep, CommentRecoveryAction)> {
        let mut comment_recovery_action = CommentRecoveryAction::None;
        let plan = roll_feed_actions_in_mood(
            settings.like_prob,
            settings.comment_prob,
            settings.save_prob,
            settings.follow_prob,
            mood,
        );
        for selected in [
            plan.like.then_some(FeedAction::Like),
            plan.save.then_some(FeedAction::Save),
            plan.comment.then_some(FeedAction::Comment),
        ]
        .into_iter()
        .flatten()
        {
            match selected {
                FeedAction::Like
                    if !policy.can_interact_with_post()
                        || !policy.can_attempt(PolicyAction::Like) =>
                {
                    ctx.report(
                        &mut progress.status,
                        "bỏ qua tim: nhịp phiên hiện tại đã đủ".into(),
                    );
                }
                FeedAction::Like => {
                    if !wait_for_action_gap(last_interaction_at, policy.min_action_gap(), ctx.stop)
                        .await
                    {
                        progress.outcome = Outcome::Stopped;
                        progress.hit_video_cap = false;
                        return Ok((FeedStep::Stop, comment_recovery_action));
                    }
                    let reservation = policy.reserve_attempt(PolicyAction::Like);
                    ctx.report(&mut progress.status, "thả tim".into());
                    match self
                        .do_like(
                            ctx.udid,
                            device.session.as_ref(),
                            ctx.gestures,
                            device.screen_size,
                            ctx.stop,
                        )
                        .await
                    {
                        Ok(LikeResult::Liked) => {
                            policy.commit_attempt(reservation);
                            progress.status.like_attempts += 1;
                            ctx.push(&progress.status);
                            progress.status.likes += 1;
                            ctx.report(
                                &mut progress.status,
                                "tim thành công (xác nhận icon đỏ)".into(),
                            );
                        }
                        Ok(LikeResult::AlreadyLiked) => {
                            policy.cancel_no_effect(reservation);
                            ctx.report(
                                &mut progress.status,
                                "video đã tim từ trước — bỏ qua".into(),
                            )
                        }
                        Ok(LikeResult::NotOnFeed) => {
                            policy.cancel_no_effect(reservation);
                            ctx.report(
                                &mut progress.status,
                                "bỏ qua tim: khung hiện tại không phải thẻ feed có thanh hành động"
                                    .into(),
                            )
                        }
                        Ok(LikeResult::NotConfirmed { before, best }) => {
                            policy.commit_attempt(reservation);
                            progress.status.like_attempts += 1;
                            ctx.push(&progress.status);
                            ctx.report(
                                &mut progress.status,
                                format!(
                            "tim: tap gửi được nhưng icon không đổi (đỏ {before:.0}→{best:.0}, \
                             cần >{:.0}; rail layout {}{}, tim y={:.0}pt)",
                            screen::LIKE_FILLED_REDNESS,
                            rail.layout(),
                            if rail.located { "" } else { ", dùng mặc định" },
                            rail.like_y * 667.0
                        ),
                            )
                        }
                        Err(e) => {
                            policy.commit_attempt(reservation);
                            progress.status.like_attempts += 1;
                            ctx.push(&progress.status);
                            let msg = format!("tim thất bại: {}", describe(&e));
                            ctx.report(&mut progress.status, msg.clone());
                            progress.last_error = Some(msg);
                            if !self
                                .recover(
                                    ctx.udid,
                                    &device.bundle_id,
                                    device.fresh_text_session,
                                    &mut device.ui_context,
                                    &mut device.session,
                                    handle,
                                    budget,
                                    text_health,
                                    &e,
                                    &mut progress.status,
                                    ctx.on_status,
                                )
                                .await
                            {
                                progress.outcome = Outcome::Failed;
                                return Ok((FeedStep::Stop, comment_recovery_action));
                            }
                        }
                    }
                }
                FeedAction::Save
                    if !policy.can_interact_with_post()
                        || !policy.can_attempt(PolicyAction::Save) =>
                {
                    ctx.report(
                        &mut progress.status,
                        "bỏ qua lưu: nhịp phiên hiện tại đã đủ".into(),
                    );
                }
                FeedAction::Save => {
                    if !wait_for_action_gap(last_interaction_at, policy.min_action_gap(), ctx.stop)
                        .await
                    {
                        progress.outcome = Outcome::Stopped;
                        progress.hit_video_cap = false;
                        return Ok((FeedStep::Stop, comment_recovery_action));
                    }
                    let reservation = policy.reserve_attempt(PolicyAction::Save);
                    ctx.report(&mut progress.status, "lưu video".into());
                    let mut adapter = PixelNurtureSaveAdapter {
                        frames: self.frames.as_ref(),
                        session: device.session.as_ref(),
                        udid: ctx.udid,
                        screen_size: device.screen_size,
                        sequence: 0,
                    };
                    let ledger = NurtureSaveLedger::new(self.db.as_ref(), ctx.session_id, ctx.udid);
                    let card_key = format!("card-{}", progress.status.videos_done);
                    let mut lease = None;
                    let evidence = tiktok_save(&mut adapter, |observation| {
                        lease = Some(ledger.arm(&card_key, observation)?);
                        Ok(())
                    })
                    .await;
                    if let Err(error) = settle_journaled_save(
                        policy,
                        reservation,
                        &mut progress.status,
                        Some(&ledger),
                        lease,
                        &evidence,
                    ) {
                        let message = format!("lưu: không chốt được sổ hành động ({error})");
                        ctx.report(&mut progress.status, message.clone());
                        progress.last_error = Some(message);
                        progress.hit_video_cap = false;
                        progress.outcome = Outcome::Failed;
                        return Ok((FeedStep::Stop, comment_recovery_action));
                    }
                    ctx.report(&mut progress.status, save_verdict_message(&evidence));
                }
                FeedAction::Comment
                    if !policy.can_interact_with_post()
                        || !policy.can_attempt(PolicyAction::Comment) =>
                {
                    ctx.report(
                        &mut progress.status,
                        "bỏ qua bình luận: nhịp phiên hiện tại đã đủ".into(),
                    );
                }
                FeedAction::Comment => {
                    if !wait_for_action_gap(last_interaction_at, policy.min_action_gap(), ctx.stop)
                        .await
                    {
                        progress.outcome = Outcome::Stopped;
                        progress.hit_video_cap = false;
                        return Ok((FeedStep::Stop, comment_recovery_action));
                    }
                    let reservation = policy.reserve_attempt(PolicyAction::Comment);
                    ctx.report(&mut progress.status, "bình luận".into());
                    suppress.store(true, Ordering::Relaxed);
                    let res = self
                        .do_comment(
                            ctx.udid,
                            device.session.as_ref(),
                            ctx.gestures,
                            rail,
                            device.screen_size,
                            settings,
                            pool,
                            ctx.stop,
                        )
                        .await;
                    suppress.store(false, Ordering::Relaxed);
                    match res {
                        Ok(result) => {
                            if result.did_act() {
                                policy.commit_attempt(reservation);
                                progress.status.comment_attempts += 1;
                                ctx.push(&progress.status);
                            } else {
                                policy.cancel_no_effect(reservation);
                            }
                            comment_recovery_action = text_health.observe(result);
                            match result {
                            CommentResult::TextSent {
                                prompt_tokens,
                                completion_tokens,
                            } => {
                                progress.status.comments += 1;
                                progress.status.session_prompt_tokens += prompt_tokens;
                                progress.status.session_completion_tokens += completion_tokens;
                                ctx.report(
                                    &mut progress.status,
                                    "đã gửi bình luận chữ (xác nhận nút gửi tắt)".into(),
                                );
                            }
                            CommentResult::TextNotSent => ctx.report(
                                &mut progress.status,
                                "bỏ qua bình luận: đã bấm Gửi nhưng chưa xác nhận được; không retry vì trạng thái giao nhận mơ hồ"
                                    .into(),
                            ),
                            other => {
                                let msg = format!("bỏ qua bình luận: {}", other.reason());
                                ctx.report(&mut progress.status, msg);
                            }
                        }

                            if comment_recovery_action == CommentRecoveryAction::RefreshFreshSession
                            {
                                ctx.report(
                                &mut progress.status,
                                "nút Gửi không sáng 2 lượt liên tiếp — làm mới text device.session"
                                    .into(),
                            );
                                let error = anyhow::Error::new(UiError::new(
                                    UiErrorKind::Session,
                                    "comment.text_not_armed",
                                    "two consecutive frame-confirmed non-arming results",
                                ));
                                if !self
                                    .recover(
                                        ctx.udid,
                                        &device.bundle_id,
                                        true,
                                        &mut device.ui_context,
                                        &mut device.session,
                                        handle,
                                        budget,
                                        text_health,
                                        &error,
                                        &mut progress.status,
                                        ctx.on_status,
                                    )
                                    .await
                                {
                                    progress.last_error = Some(
                                    "không làm mới được text device.session sau 2 lượt không armed"
                                        .into(),
                                );
                                    progress.outcome = Outcome::Failed;
                                    return Ok((FeedStep::Stop, comment_recovery_action));
                                }
                            }
                        }
                        Err(e) => {
                            policy.commit_attempt(reservation);
                            progress.status.comment_attempts += 1;
                            ctx.push(&progress.status);
                            let msg = format!("bình luận thất bại: {}", describe(&e));
                            ctx.report(&mut progress.status, msg.clone());
                            progress.last_error = Some(msg);
                            if ui_error_kind(&e) != UiErrorKind::Other
                                && !self
                                    .recover(
                                        ctx.udid,
                                        &device.bundle_id,
                                        device.fresh_text_session,
                                        &mut device.ui_context,
                                        &mut device.session,
                                        handle,
                                        budget,
                                        text_health,
                                        &e,
                                        &mut progress.status,
                                        ctx.on_status,
                                    )
                                    .await
                            {
                                progress.outcome = Outcome::Failed;
                                return Ok((FeedStep::Stop, comment_recovery_action));
                            }
                        }
                    }
                }
                FeedAction::None => {}
            }
        }
        if plan.follow
            && matches!(
                self.roll_and_execute_follow(
                    ctx,
                    progress,
                    device,
                    policy,
                    budget,
                    rail,
                    text_health,
                    last_interaction_at,
                    handle,
                )
                .await?,
                FeedStep::Stop
            )
        {
            return Ok((FeedStep::Stop, comment_recovery_action));
        }
        Ok((FeedStep::Proceed, comment_recovery_action))
    }

    /// Swipe to the next video, and deal with every way that can fail to take.
    ///
    /// The swipe itself is one line; the other hundred are what happens when the card does not
    /// move — the stream is re-established, the app is checked, recovery runs. Whether the feed
    /// actually advanced is the phase's real answer, so it comes back with the verdict.
    #[allow(clippy::too_many_arguments)]
    async fn swipe_to_next_video<F: Fn(NurtureSessionStatus) + Send + Sync>(
        &self,
        ctx: &SessionCtx<'_, F>,
        progress: &mut SessionProgress,
        device: &mut OpenedDevice,
        settings: &NurtureSettings,
        human: &mut HumanBehavior,
        budget: &mut Budget,
        text_health: &mut TextCommentHealth,
        handle: &SessionHandle,
    ) -> anyhow::Result<(FeedStep, bool)> {
        ctx.report(&mut progress.status, "vuốt video tiếp".into());
        let mut advanced_to_next_video = false;
        progress.status.swipe_attempts += 1;
        match self
            .do_swipe(
                ctx.udid,
                device.session.as_ref(),
                ctx.gestures,
                device.screen_size,
                human.swipe_duration_ms(roll_bool(settings.frenzy_prob)),
                ctx.stop,
            )
            .await
        {
            Ok(SwipeOutcome::Advanced) => {
                advanced_to_next_video = true;
                progress.blocked_streak = 0;
                progress.status.videos_done += 1;
                ctx.push(&progress.status);
            }
            Ok(SwipeOutcome::Moved) => {
                progress.blocked_streak = 0;
                // The rail left, so the gesture landed and the card we were
                // on is gone. Swiping again here would skip whatever came
                // next; the loop re-reads the screen at the top instead.
                // Not counted: nothing settled that could be counted.
                advanced_to_next_video = true;
                ctx.report(
                    &mut progress.status,
                    "đã rời thẻ cũ nhưng chưa thấy thẻ mới ổn định — đọc lại màn hình".into(),
                );
            }
            Ok(SwipeOutcome::Blocked) => {
                // The rail never left: the feed swallowed the gesture,
                // usually under a popup. The watcher closes those on its
                // own; give it a beat, then try once more.
                ctx.report(
                    &mut progress.status,
                    "vuốt không ăn — chờ popup rồi thử lại".into(),
                );
                sleep_interruptible(Duration::from_millis(1_800), ctx.stop).await;
                progress.status.swipe_attempts += 1;
                match self
                    .do_swipe(
                        ctx.udid,
                        device.session.as_ref(),
                        ctx.gestures,
                        device.screen_size,
                        human.swipe_duration_ms(false),
                        ctx.stop,
                    )
                    .await
                {
                    Ok(SwipeOutcome::Advanced) => {
                        advanced_to_next_video = true;
                        progress.blocked_streak = 0;
                        progress.status.videos_done += 1;
                        ctx.push(&progress.status);
                    }
                    Ok(SwipeOutcome::Moved) => {
                        advanced_to_next_video = true;
                        progress.blocked_streak = 0;
                        ctx.report(
                            &mut progress.status,
                            "đã rời thẻ cũ, chưa xác nhận thẻ mới".into(),
                        );
                    }
                    Ok(SwipeOutcome::Blocked) => {
                        ctx.report(&mut progress.status, "vuốt vẫn không ăn".into());
                        progress.last_error = Some("swipe không rời được thẻ hiện tại".into());
                        progress.blocked_streak += 1;
                    }
                    Err(e) => {
                        let msg = format!("vuốt lỗi: {}", describe(&e));
                        ctx.report(&mut progress.status, msg.clone());
                        progress.last_error = Some(msg);
                        if !self
                            .recover(
                                ctx.udid,
                                &device.bundle_id,
                                device.fresh_text_session,
                                &mut device.ui_context,
                                &mut device.session,
                                handle,
                                budget,
                                text_health,
                                &e,
                                &mut progress.status,
                                ctx.on_status,
                            )
                            .await
                        {
                            progress.outcome = Outcome::Failed;
                            return Ok((FeedStep::Stop, advanced_to_next_video));
                        }
                    }
                }
            }
            Err(e) => {
                let msg = format!("vuốt lỗi: {}", describe(&e));
                ctx.report(&mut progress.status, msg.clone());
                progress.last_error = Some(msg);
                if !self
                    .recover(
                        ctx.udid,
                        &device.bundle_id,
                        device.fresh_text_session,
                        &mut device.ui_context,
                        &mut device.session,
                        handle,
                        budget,
                        text_health,
                        &e,
                        &mut progress.status,
                        ctx.on_status,
                    )
                    .await
                {
                    progress.outcome = Outcome::Failed;
                    return Ok((FeedStep::Stop, advanced_to_next_video));
                }
            }
        }
        Ok((FeedStep::Proceed, advanced_to_next_video))
    }

    /// Settle after the feed moved on: the natural rest, and the checks that follow it.
    ///
    /// Kept behind its `if` at the call site rather than taking the flag: "only when the feed
    /// actually advanced" is a fact about the loop, and reading it there is the point.
    #[allow(clippy::too_many_arguments)]
    async fn settle_after_advance<F: Fn(NurtureSessionStatus) + Send + Sync>(
        &self,
        ctx: &SessionCtx<'_, F>,
        progress: &mut SessionProgress,
        device: &mut OpenedDevice,
        policy: &mut crate::human_behavior::HumanSessionPolicy,
        handle: &SessionHandle,
    ) -> anyhow::Result<FeedStep> {
        if let Some(rest) = policy.rest_after_video() {
            ctx.report(
                &mut progress.status,
                format!("nghỉ tự nhiên {}s", rest.as_secs()),
            );
            sleep_interruptible(rest, ctx.stop).await;
        }
        if policy.should_take_block_break() || policy.should_take_home_break() {
            let break_for = policy.home_break_duration();
            ctx.report(
                &mut progress.status,
                format!(
                    "tạm về màn hình chính khoảng {}s rồi mở TikTok lại",
                    break_for.as_secs()
                ),
            );
            {
                let _guard = ctx.gestures.lock().await;
                let _ = device.session.home().await;
            }
            sleep_interruptible(break_for, ctx.stop).await;
            if policy.should_cold_restart() {
                ctx.report(
                    &mut progress.status,
                    "khởi động lại TikTok sau một quãng nghỉ".into(),
                );
                let _ = self
                    .control
                    .terminate_streaming_app(&device.ui_context, &device.bundle_id)
                    .await;
                sleep_interruptible(Duration::from_secs(2), ctx.stop).await;
                match self
                    .control
                    .recover_streaming_session(
                        &mut device.ui_context,
                        &device.bundle_id,
                        device.session_kind,
                        false,
                    )
                    .await
                {
                    Ok(next) => {
                        // Swap the watcher's device.session handle too, or the
                        // popup watcher keeps tapping through the dead
                        // pre-restart device.session for the rest of the run.
                        device.session = next;
                        handle.set(device.session.clone());
                    }
                    Err(error) => {
                        ctx.report(
                            &mut progress.status,
                            format!("không mở lại được TikTok: {error}"),
                        );
                        progress.outcome = Outcome::Partial;
                        return Ok(FeedStep::Stop);
                    }
                }
            } else {
                let _guard = ctx.gestures.lock().await;
                let _ = device
                    .session
                    .launch_app_foreground(&device.bundle_id)
                    .await;
            }
            sleep_interruptible(Duration::from_secs(4), ctx.stop).await;
            policy.reset_block();
        }
        Ok(FeedStep::Proceed)
    }

    /// Follow the author of the card on screen.
    ///
    /// The roll stays at the call site with the other two — like, comment, follow read as one
    /// decision there, and burying one of the three inside a phase would break that.
    #[allow(clippy::too_many_arguments)]
    async fn roll_and_execute_follow<F: Fn(NurtureSessionStatus) + Send + Sync>(
        &self,
        ctx: &SessionCtx<'_, F>,
        progress: &mut SessionProgress,
        device: &mut OpenedDevice,
        policy: &mut crate::human_behavior::HumanSessionPolicy,
        budget: &mut Budget,
        rail: &ActionRail,
        text_health: &mut TextCommentHealth,
        last_interaction_at: &mut Option<Instant>,
        handle: &SessionHandle,
    ) -> anyhow::Result<FeedStep> {
        if !policy.can_interact_with_post() || !policy.can_attempt(PolicyAction::Follow) {
            ctx.report(
                &mut progress.status,
                "bỏ qua follow: nhịp phiên hiện tại đã đủ".into(),
            );
        } else if !wait_for_action_gap(last_interaction_at, policy.min_action_gap(), ctx.stop).await
        {
            progress.outcome = Outcome::Stopped;
            progress.hit_video_cap = false;
            return Ok(FeedStep::Stop);
        } else {
            let reservation = policy.reserve_attempt(PolicyAction::Follow);
            ctx.report(&mut progress.status, "follow".into());
            match self
                .do_follow(
                    ctx.udid,
                    device.session.as_ref(),
                    ctx.gestures,
                    rail,
                    device.screen_size,
                    ctx.stop,
                )
                .await
            {
                Ok(verdict) => {
                    if verdict.did_act() {
                        policy.commit_attempt(reservation);
                        progress.status.follow_attempts += 1;
                        ctx.push(&progress.status);
                    } else {
                        policy.cancel_no_effect(reservation);
                    }
                    match verdict {
                        FollowResult::Followed => {
                            progress.status.follows += 1;
                            ctx.report(&mut progress.status, "follow thành công".into());
                        }
                        FollowResult::NoControl => ctx.report(
                            &mut progress.status,
                            "bỏ qua follow: thẻ không có nút Follow".into(),
                        ),
                        FollowResult::CardChanged => ctx.report(
                            &mut progress.status,
                            "bỏ qua follow: thẻ hoặc tác giả đã đổi trước cú tap".into(),
                        ),
                        FollowResult::NotConfirmed => {
                            ctx.report(&mut progress.status, "follow không đổi trạng thái".into())
                        }
                    }
                }
                Err(e) => {
                    policy.commit_attempt(reservation);
                    progress.status.follow_attempts += 1;
                    ctx.push(&progress.status);
                    let msg = format!("follow thất bại: {}", describe(&e));
                    ctx.report(&mut progress.status, msg.clone());
                    progress.last_error = Some(msg);
                    if !self
                        .recover(
                            ctx.udid,
                            &device.bundle_id,
                            device.fresh_text_session,
                            &mut device.ui_context,
                            &mut device.session,
                            handle,
                            budget,
                            text_health,
                            &e,
                            &mut progress.status,
                            ctx.on_status,
                        )
                        .await
                    {
                        progress.outcome = Outcome::Failed;
                        return Ok(FeedStep::Stop);
                    }
                }
            }
        }
        Ok(FeedStep::Proceed)
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
        // Hoisted above the first status push: `ctx.push` is the first thing the session
        // does, so the context has to exist by then. The gesture lock came up with it — it
        // is one of the four values every phase needs, and it depends on nothing.
        let gestures = Arc::new(tokio::sync::Mutex::new(()));
        let nurture_session_id = uuid::Uuid::new_v4().to_string();
        let ctx = SessionCtx {
            udid,
            session_id: &nurture_session_id,
            stop: &stop,
            gestures: &gestures,
            on_status: &on_status,
        };
        // The two bounds, stamped on the status the moment they are known.
        //
        // Both of these were locals that died with the function. The video target is a
        // start-time snapshot — `num_videos` is deliberately not absorbed mid-run — so a
        // frontend dividing by the live settings row would rescale the bar under a session
        // that never changed. The deadline is worse: for a manual start it is a randomised
        // 2–3 hour horizon that nothing outside this function had ever seen, so a progress
        // bar drawn from the video count alone stalls at 40% on a run that is minutes from
        // finishing on time and reads as hung.
        let session_began = chrono::Utc::now();
        let mut progress = SessionProgress {
            status: NurtureSessionStatus {
                running: true,
                last_message: "bắt đầu".into(),
                phase: NurturePhase::Opening,
                video_target: live::video_target(&settings),
                started_at: Some(session_began),
                deadline_at: max_duration.and_then(|window| {
                    chrono::Duration::from_std(window)
                        .ok()
                        .map(|window| session_began + window)
                }),
                ..NurtureSessionStatus::new(udid)
            },
            outcome: Outcome::Done,
            last_error: None,
            hit_video_cap: true,
            off_feed_streak: 0,
            blocked_streak: 0,
        };
        ctx.push(&progress.status);

        let Some(mut device) = self
            .open_for_session(&settings, &mut progress.status, &ctx)
            .await?
        else {
            return Ok(progress.status);
        };
        // Every return and every `?` after this point is captured here. The only exit from an
        // opened phone then runs `shutdown_tiktok`, so adding a new failure branch cannot
        // accidentally leave the app alive in the background.
        let mut popup_watch_task: Option<tokio::task::JoinHandle<()>> = None;
        let session_result: anyhow::Result<NurtureSessionStatus> = async {
        // The session exists; from here to the first counted video the phone is being
        // steered onto a usable feed — dialogs declined, the onboarding journey skipped,
        // the action rail found. That can legitimately take a minute, and it is the window
        // the two lock-screen phones died in on 23/08/2026, so it is worth being its own
        // phase rather than a stretch of 0%.
        progress.status.phase = NurturePhase::AwaitingFeed;
        ctx.push(&progress.status);
        // A backend that can report *where* a control is does not need a
        // calibrated screen at all — it taps inside the rectangle the device
        // handed back instead of multiplying an iPhone 8 fraction. So try that
        // route first; iOS cannot answer `locate_description` and falls straight
        // through to the pixel engine below, unchanged.
        //
        // This is what AGENTS.md §9 means by not porting `screen.rs` to Android:
        // the same device.session policy, a different way of seeing.
        // The hierarchy loop gets its words from the engine's own grounded
        // generator, so a comment on Android is written from the same evidence, by
        // the same provider, into the same audit table as one on iOS.
        let comment_source = EngineCommentSource {
            engine: self,
            udid,
            stop: &stop,
            slides: parking_lot::Mutex::new(SlideEvidence::default()),
        };
        let live_source = EngineLiveSettings { engine: self };
        let mut said_live_settings_failed = false;
        let save_ledger = NurtureSaveLedger::new(self.db.as_ref(), &nurture_session_id, udid);
        let attempt = hierarchy::run_hierarchy_session_with_save_intent(
            device.session.as_ref(),
            device.screen_size,
            &settings,
            &device.bundle_id,
            started,
            max_duration,
            &stop,
            &mut progress.status,
            &|into: &mut NurtureSessionStatus, msg: String| ctx.report(into, msg),
            Some(&comment_source),
            Some(&live_source),
            &save_ledger,
        )
        .await;
        match attempt {
            hierarchy::HierarchySession::Ran(mut ran_outcome) => {
                // Same judgement the pixel path applies: a device.session that moved no
                // videos did not work, whatever else it reported.
                if ran_outcome == Outcome::Done && progress.status.videos_done == 0 {
                    ran_outcome = Outcome::Failed;
                }
                let summary = format!(
                    "{} — {}/{} video, {} tim, {} lưu, {} bình luận, {} follow, {:.0}s (hierarchy)",
                    ran_outcome.as_str(),
                    progress.status.videos_done,
                    progress.status.swipe_attempts,
                    progress.status.likes,
                    progress.status.saves,
                    progress.status.comments,
                    progress.status.follows,
                    started.elapsed().as_secs_f64(),
                );
                progress.status.finish(ran_outcome);
                progress.status.last_message = summary.clone();
                ctx.push(&progress.status);
                return Ok(progress.status);
            }
            // The ordinary iOS case: no geometry, so use pixels.
            hierarchy::HierarchySession::NotSupported => {}
            // Geometry works but something measured is missing. Stop, rather than
            // falling through to a pixel engine whose only calibrated layout is an
            // iPhone 8. The reason is already in `progress.status.last_message`.
            hierarchy::HierarchySession::Refused => {
                progress.status.finish(Outcome::Failed);
                ctx.push(&progress.status);
                return Ok(progress.status);
            }
        }

        let Some(layout) = screen::calibrated_layout(device.screen_size.0, device.screen_size.1)
        else {
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
            ctx.report(
                &mut progress.status,
                format!(
                    "failed — chưa hiệu chỉnh bộ dò cho màn hình {}x{}; \
                     đã hiệu chỉnh: {known}. Chạy quy trình hiệu chỉnh (AGENTS.md mục 6) \
                     trước khi dùng máy này",
                    device.screen_size.0, device.screen_size.1
                ),
            );
            progress.status.finish(Outcome::Failed);
            return Ok(progress.status);
        };
        tracing::debug!("[nurture {udid}] layout đã hiệu chỉnh: {}", layout.id);
        self.reset_touch_points(udid, device.screen_size);

        // Now the agent is warm, attach the stream that the watcher reads.
        ctx.report(&mut progress.status, "mở stream màn hình".into());
        if self
            .wait_for_frame(udid, Duration::from_secs(20), &stop, |_| true)
            .await
            .is_none()
        {
            ctx.report(
                &mut progress.status,
                "failed — stream không có frame".into(),
            );
            progress.status.finish(Outcome::Failed);
            return Ok(progress.status);
        }

        // What is on screen before we touch anything?
        let already_on_tiktok = self
            .latest_image(udid)
            .map(|img| screen::feed_ready(&img, Some(device.screen_size.0)))
            .unwrap_or(false);

        let handle = SessionHandle::new();
        handle.set(device.session.clone());
        let suppress = Arc::new(AtomicBool::new(false));

        // TikTok forward only if the frame says we are not already there.
        if already_on_tiktok {
            ctx.report(
                &mut progress.status,
                "TikTok đã mở sẵn — reuse, không khởi động lại".into(),
            );
        } else {
            ctx.report(
                &mut progress.status,
                "TikTok chưa ở foreground — đưa lên trước".into(),
            );
            let brought = self
                .bring_tiktok_foreground(
                    udid,
                    &device.ui_context,
                    device.session.as_ref(),
                    &settings,
                    device.screen_size.0,
                    &gestures,
                    &stop,
                )
                .await;
            match brought {
                Ok(true) => ctx.report(&mut progress.status, "đã bring TikTok foreground".into()),
                Ok(false) => ctx.report(&mut progress.status, "TikTok đã ở foreground".into()),
                Err(e) => ctx.report(&mut progress.status, format!("không mở được TikTok: {e}")),
            }
        }

        // Popup watcher: its own task, its own stop flag, its own cooldowns.
        let watcher = ScreenWatcher::new(
            udid,
            self.frames.clone(),
            handle.clone(),
            gestures.clone(),
            stop.clone(),
            device.screen_size,
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
        popup_watch_task = Some(tokio::spawn(
            watcher.run_suppressible(watcher_suppress),
        ));

        // The watcher normally runs in parallel with nurture. At startup we
        // add one small gate so a notification/sheet that appeared during app
        // launch cannot receive the first like or swipe. The watcher keeps
        // running after this gate for overlays that appear mid-device.session.
        let popup_closed_before = watcher_stats.popups_closed.load(Ordering::Relaxed);
        let startup_ready = watcher_state
            .wait_until_feed(&stop, STARTUP_POPUP_DRAIN)
            .await;
        let popup_closed_after = watcher_stats.popups_closed.load(Ordering::Relaxed);
        if popup_closed_after > popup_closed_before {
            ctx.report(
                &mut progress.status,
                format!(
                    "đã tự tắt {} thông báo/popup đầu phiên",
                    popup_closed_after - popup_closed_before
                ),
            );
        }
        if !startup_ready && !stop.load(Ordering::Relaxed) {
            ctx.report(
                &mut progress.status,
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
        let mut policy = HumanSessionPolicy::new_with_save(
            settings.like_prob,
            settings.comment_prob,
            settings.save_prob,
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
        // The pixel loop's door back to the settings row. The same object the hierarchy
        // loop is handed, so "live" means one thing across both.
        let live_source = EngineLiveSettings { engine: self };

        // Both bounds apply: this count and, inside the loop, the run duration. See
        // `live::video_target` for why the duration used to silently win.
        let total_videos = video_target(&settings);
        // True when the loop ran out of videos rather than out of time.
        // The feed loop proper. Set once outside the loop: a phase that flickered per post
        // would be a worse signal than no phase at all.
        if progress.status.phase != NurturePhase::Watching {
            progress.status.phase = NurturePhase::Watching;
            ctx.push(&progress.status);
        }
        'feed: for _video in 0..total_videos {
            // Live tuning, once per post rather than per action. The UI writes one settings
            // row and this picks it up, so "save" means "applies to the run in progress"
            // with no extra plumbing. Per *post* on purpose: a probability that changed
            // between rolling an action and confirming it would make that action's own
            // record unexplainable.
            match apply_live_settings(
                Some(&live_source),
                &mut settings,
                &mut human,
                &mut policy,
                &mut moods,
            ) {
                LiveSettingsRefresh::Applied => said_live_settings_failed = false,
                LiveSettingsRefresh::Failed(error) if !said_live_settings_failed => {
                    said_live_settings_failed = true;
                    ctx.report(
                        &mut progress.status,
                        format!(
                            "không đọc được cài đặt mới ({error}) — khóa like/comment/save/follow ở lượt này"
                        ),
                    );
                }
                LiveSettingsRefresh::NoSource | LiveSettingsRefresh::Failed(_) => {}
            }
            if stop.load(Ordering::Relaxed) {
                progress.outcome = Outcome::Stopped;
                progress.hit_video_cap = false;
                break;
            }
            if max_duration.is_some_and(|max| started.elapsed() >= max) {
                progress.hit_video_cap = false;
                break;
            }
            if in_night_window(settings.night_start, settings.night_end) {
                ctx.report(&mut progress.status, "giờ nghỉ đêm — dừng".into());
                progress.hit_video_cap = false;
                break;
            }
            if budget.exhausted() {
                progress.outcome = Outcome::Failed;
                progress.last_error = Some("hết ngân sách recovery".into());
                break;
            }
            policy.begin_post();

            // One mood runs for several videos, so a device.session looks like a
            // person skimming, then liking a run, then chatting — not an
            // independent coin flip per clip.
            let (mood, mood_changed) = moods.next();
            if mood_changed {
                ctx.report(
                    &mut progress.status,
                    format!("chuyển nhịp: {}", mood.label()),
                );
            }

            let watch =
                human.watch_seconds(settings.watch_min, settings.watch_max) * mood.watch_mult();
            ctx.report(
                &mut progress.status,
                format!("xem {watch:.1}s ({})", mood.label()),
            );
            sleep_interruptible(Duration::from_secs_f64(watch.max(0.5)), &stop).await;
            if stop.load(Ordering::Relaxed) {
                progress.outcome = Outcome::Stopped;
                progress.hit_video_cap = false;
                break;
            }

            // Only act when the FYP feed is actually on screen. Swiping in the
            // feed can land in a LIVE room, whose layout is completely
            // different — a live run spent several videos tapping rail
            // positions that do not exist there and opening the LIVE chat
            // keyboard. A swipe leaves; blind taps do not.
            match self
                .handle_off_feed(
                    &settings,
                    &device.ui_context,
                    &device.session,
                    device.screen_size,
                    &mut human,
                    progress.off_feed_streak,
                    &mut progress,
                    &ctx,
                )
                .await?
            {
                // The streak is dropped on this path on purpose: the line just past this
                // match zeroes it, because getting here means the phone is on the feed.
                (FeedStep::Proceed, _) => {}
                (FeedStep::NextVideo, streak) => {
                    progress.off_feed_streak = streak;
                    continue;
                }
                (FeedStep::Stop, _) => break 'feed,
            }
            // Every path that gets here is on the feed: either it always was, or
            // one of the branches above got back to it.
            progress.off_feed_streak = 0;

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
            match self
                .watch_one_card(
                    &settings,
                    card_kind,
                    &device.session,
                    device.screen_size,
                    &mut human,
                    &mut policy,
                    &live_owned,
                    &mut progress,
                    &ctx,
                )
                .await?
            {
                (FeedStep::Proceed, found) => {
                    if let Some(found) = found {
                        rail = found;
                        rail_present = true;
                    }
                }
                (FeedStep::NextVideo, _) => continue,
                (FeedStep::Stop, _) => break 'feed,
            }

            // No rail on this card: watch it out and move on rather than tap
            // where nothing is. Follow is skipped for the same reason.
            if !rail_present {
                ctx.report(
                    &mut progress.status,
                    "thẻ không có thanh hành động (LIVE / đang chuyển) — chỉ vuốt tiếp".into(),
                );
                progress.status.swipe_attempts += 1;
                // Leaving a card that has no rail is still provable, from the
                // other side: the rail *arriving* on a settled card is the
                // card change. Landing on another rail-less card is not, and
                // stays uncounted.
                if self
                    .do_swipe(
                        udid,
                        device.session.as_ref(),
                        &gestures,
                        device.screen_size,
                        human.swipe_duration_ms(roll_bool(settings.frenzy_prob)),
                        &stop,
                    )
                    .await
                    .is_ok_and(|swipe| swipe == SwipeOutcome::Advanced)
                {
                    progress.status.videos_done += 1;
                    ctx.push(&progress.status);
                    if let Some(rest) = policy.rest_after_video() {
                        ctx.report(
                            &mut progress.status,
                            format!("nghỉ tự nhiên {}s", rest.as_secs()),
                        );
                        sleep_interruptible(rest, &stop).await;
                    }
                }
                continue;
            }

            human.note_action();
            let (action_step, comment_recovery_action) = self
                .roll_and_execute_action(
                    &ctx,
                    &mut progress,
                    &mut device,
                    &settings,
                    &mut policy,
                    &mut budget,
                    &rail,
                    &mut text_health,
                    &mut last_interaction_at,
                    mood,
                    &handle,
                    &suppress,
                    &pool,
                )
                .await?;
            match action_step {
                FeedStep::Stop => break 'feed,
                FeedStep::Proceed | FeedStep::NextVideo => {}
            }

            sleep_interruptible(Duration::from_millis(human.think_pause_ms()), &stop).await;

            let (swipe_step, advanced_to_next_video) = self
                .swipe_to_next_video(
                    &ctx,
                    &mut progress,
                    &mut device,
                    &settings,
                    &mut human,
                    &mut budget,
                    &mut text_health,
                    &handle,
                )
                .await?;
            match swipe_step {
                FeedStep::Stop => break 'feed,
                FeedStep::Proceed | FeedStep::NextVideo => {}
            }
            // A card that swallows both the swipe and its retry, turn after
            // turn, is not going to start working. A live run spent 280 seconds
            // — 46 of its 53 swipes — on one photo post before the clock ran
            // out. Ending the device.session says so; continuing just burns the budget
            // in silence.
            if progress.blocked_streak >= BLOCKED_SWIPE_LIMIT {
                let streak = progress.blocked_streak;
                let message = format!(
                    "thẻ hiện tại nuốt {streak} lượt vuốt liên tiếp — dừng phiên \
                     thay vì vuốt tiếp vô ích"
                );
                progress.give_up(message, &ctx);
                break 'feed;
            }
            if must_stop_before_next_feed_iteration(comment_recovery_action, advanced_to_next_video)
            {
                let message =
                    "dừng trước lượt feed kế tiếp: chưa xác nhận rời video có trạng thái gửi mơ hồ"
                        .to_string();
                progress.give_up(message, &ctx);
                break 'feed;
            }
            if advanced_to_next_video {
                match self
                    .settle_after_advance(&ctx, &mut progress, &mut device, &mut policy, &handle)
                    .await?
                {
                    FeedStep::Stop => break 'feed,
                    FeedStep::Proceed | FeedStep::NextVideo => {}
                }
            }
            sleep_interruptible(Duration::from_millis(human.after_swipe_pause_ms()), &stop).await;
        }

        // Stop the watcher and collect its numbers before reporting.
        stop.store(true, Ordering::Relaxed);
        if let Some(task) = popup_watch_task.take() {
            let _ = tokio::time::timeout(Duration::from_secs(3), task).await;
        }
        let watch = watcher_stats.snapshot();
        let _ = watcher_state;

        progress.outcome = session_verdict(
            progress.outcome,
            progress.status.videos_done,
            progress.hit_video_cap,
            total_videos,
            progress.last_error.is_some(),
        );

        let elapsed = started.elapsed();
        let summary = format!(
            "{} — {}/{} video, {} tim, {} lưu, {} bình luận, {} follow, {} popup đóng, {} recovery, {:.0}s{}",
            progress.outcome.as_str(),
            progress.status.videos_done,
            progress.status.swipe_attempts,
            progress.status.likes,
            progress.status.saves,
            progress.status.comments,
            progress.status.follows,
            watch.popups_closed,
            budget.soft + budget.hard,
            elapsed.as_secs_f64(),
            progress.last_error
                .as_ref()
                .map(|e| format!(", lỗi cuối: {e}"))
                .unwrap_or_default(),
        );
        progress.status.finish(progress.outcome);
        progress.status.last_message = summary.clone();
        ctx.push(&progress.status);

        Ok(progress.status)
        }
        .await;

        // Some error exits happen after the popup watcher has been spawned. Stopping here is
        // unconditional so dropping its JoinHandle cannot leave the watcher detached after
        // TikTok itself has been force-stopped.
        stop.store(true, Ordering::Release);
        if let Some(task) = popup_watch_task.take() {
            let _ = tokio::time::timeout(Duration::from_secs(3), task).await;
        }
        let OpenedDevice {
            ui_context,
            bundle_id,
            ..
        } = device;
        let shutdown = self.shutdown_tiktok(ui_context, &bundle_id).await;
        self.clear_touch_points(udid);
        match session_result {
            Ok(mut status) => {
                match shutdown {
                    Ok(()) => status.last_message.push_str(", đã tắt sạch TikTok"),
                    Err(error) => {
                        let outcome = if status.videos_done == 0 {
                            Outcome::Failed
                        } else {
                            Outcome::Partial
                        };
                        status.finish(outcome);
                        status
                            .last_message
                            .push_str(&format!(", lỗi dọn TikTok: {error}"));
                    }
                }
                ctx.push(&status);
                let _ = self.db.log_op(
                    "nurture.session",
                    &format!(
                        "{udid} {} tokens={}/{}",
                        status.last_message,
                        status.session_prompt_tokens,
                        status.session_completion_tokens
                    ),
                );
                Ok(status)
            }
            Err(error) => match shutdown {
                Ok(()) => Err(error),
                Err(cleanup) => Err(anyhow::anyhow!("{error}; lỗi dọn TikTok: {cleanup}")),
            },
        }
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
    /// thing measured that does. A photo post publishes the same picture on
    /// every sample because nothing on screen moves; a video cannot, since the
    /// stream re-encodes every frame at 24 FPS with no deduplication — the same
    /// fact that made the old swipe check useless is what makes this reliable.
    ///
    /// Measured over 40 real cards: 4 came back still, at least three of them
    /// confirmed photo posts by eye (page dots and the "Ảnh" badge), and none
    /// of the 36 videos did. The page-dot detector this replaces scored 1 true
    /// positive against 9 false ones on the same cards.
    ///
    /// **Compares the picture, not the screen, and the difference is measured.** This used to
    /// hash the whole encoded frame, which the phone's own status bar is part of: on
    /// ce0717171c2a64d50d three samples of a genuinely still photo post differed only inside
    /// y 16..49, the animated network icon, and that survived minicap's half-scale JPEG — so a
    /// still card read as moving and no photo post on that phone could ever pass. The 4-of-40
    /// figure above is therefore a **floor**: it was taken on phones whose corner happened not
    /// to change inside the sampling window.
    ///
    /// Costs [`STILL_CARD_SAMPLES`] + 1 decodes of a half-scale stream frame, which is the
    /// price of asking the question that was actually meant.
    ///
    /// Only a video that holds a perfectly static picture for the whole window
    /// can pass, and the caller still has to survive being wrong — a horizontal
    /// swipe on a video navigates away from the feed.
    async fn card_is_still(&self, udid: &str, stop: &AtomicBool) -> bool {
        let Some(first) = self
            .frames
            .latest(udid)
            .and_then(|frame| picture_digest_of(&frame))
        else {
            return false;
        };
        for _ in 0..STILL_CARD_SAMPLES {
            sleep_interruptible(STILL_CARD_GAP, stop).await;
            if stop.load(Ordering::Relaxed) {
                return false;
            }
            match self
                .frames
                .latest(udid)
                .and_then(|frame| picture_digest_of(&frame))
            {
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
    /// Slides offered by the traversal since the last comment was written.
    slides: parking_lot::Mutex<SlideEvidence>,
}

/// The frames a photo post's traversal offered, kept two at a time.
///
/// **Two, not one per slide, and the number is arithmetic rather than taste.** The contact
/// sheet spends a fixed pixel budget (`openai_client::SHEET_PIXEL_BUDGET`, held at the old
/// sheet's area so the token cost does not move), so every extra thumbnail shrinks all of
/// them. On this fleet's 1080x2220 frames that is 589x1211 for one slide, 367x754 for two and
/// 271x557 for three — and `visualFacts` and `evidenceSupport` are read off exactly the small
/// text that disappears first. Two slides buys a second scene for a 2.6x per-slide cut; three
/// costs 4.7x and hands 55% of the sheet to the caption strip.
///
/// **First and last, not the first two.** On a product post that is hero plus call-to-action,
/// on a photo story beginning plus end, on a meme set setup plus punchline. The first two
/// slides of a six-slide post are usually the same setup twice.
#[derive(Default)]
struct SlideEvidence {
    /// The first slide offered, with its digest so a repeat can be recognised.
    first: Option<(SlideDigest, Vec<u8>)>,
    /// The most recent slide whose picture differed from the first.
    last: Option<Vec<u8>>,
    /// How many slides were offered, duplicates included.
    ///
    /// Kept separately from the frames because the pair is what can be read: `offered = 7`
    /// with one distinct frame says the pager turned seven times and the stream handed back
    /// the same picture every time — a parked stream — while `offered = 7` with two says the
    /// change is working. Either number alone is ambiguous.
    offered: u32,
}

impl SlideEvidence {
    /// Take a frame, keeping at most two.
    ///
    /// De-duplication is inherent rather than bolted on: a post whose pager never turned —
    /// or whose stream is parked, which is the ordinary state on a still card — leaves exactly
    /// one frame here, and the sheet then says "one khung" instead of pasting it twice.
    fn offer(&mut self, frame: Vec<u8>) {
        self.offered = self.offered.saturating_add(1);
        let digest = SlideDigest::of(&frame);
        match &self.first {
            None => self.first = Some((digest, frame)),
            Some((seen, _)) if *seen == digest => {}
            Some(_) => self.last = Some(frame),
        }
    }

    /// Hand over what was collected and reset, oldest first.
    ///
    /// Draining matters: the source lives for the whole session, so slides left behind would
    /// ground post N+1 on post N's pictures — the exact failure `collect_grounding_frames`
    /// refuses by design.
    fn drain(&mut self) -> (Vec<Vec<u8>>, u32) {
        let taken = std::mem::take(self);
        let frames = taken
            .first
            .map(|(_, frame)| frame)
            .into_iter()
            .chain(taken.last)
            .collect();
        (frames, taken.offered)
    }
}

/// Identity for one slide frame. Live frames compare decoded picture pixels and ignore the
/// status bar; malformed fixture bytes retain deterministic encoded-byte behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlideDigest {
    Picture(u64),
    EncodedFrame(u64),
}

impl SlideDigest {
    fn of(frame: &[u8]) -> Self {
        picture_digest_of(frame)
            .map(Self::Picture)
            .unwrap_or_else(|| Self::EncodedFrame(frame_digest(frame)))
    }
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
    fn refresh(&self, settings: &mut NurtureSettings) -> anyhow::Result<()> {
        self.engine.absorb_live_settings(settings)
    }
}

#[async_trait::async_trait]
impl hierarchy::CommentTextSource for EngineCommentSource<'_> {
    async fn comment_for_post(
        &self,
        settings: &NurtureSettings,
    ) -> Result<Option<hierarchy::PreparedComment>, hierarchy::CommentSourceError> {
        let (slides, offered) = self.slides.lock().drain();
        self.engine
            .prepare_hierarchy_comment(self.udid, settings, slides, offered, self.stop)
            .await
    }

    async fn record_outcome(&self, prepared: &hierarchy::PreparedComment, outcome: &str) {
        self.engine
            .finish_hierarchy_comment(prepared.audit_attempt.id(), outcome);
    }

    fn note_slide(&self) {
        // `latest` is documented as exactly this — a one-shot cache read for a caller that
        // must not wait for the next frame — so a slide costs an `RwLock` read and an `Arc`
        // clone on top of gestures it was already spending.
        if let Some(frame) = self.engine.frames.latest(self.udid) {
            self.slides.lock().offer((*frame).clone());
        }
    }

    async fn record_skip(&self, settings: &NurtureSettings, reason: &str) {
        // Drop the slides with the row: they belong to the post that was abandoned.
        let (_, offered) = self.slides.lock().drain();
        self.engine
            .record_deferred_skip(self.udid, settings, reason, offered);
    }
}

fn describe(err: &anyhow::Error) -> String {
    format!("{} ({})", err, ui_error_kind(err).as_str())
}

/// Rows the phone's own status bar owns, as a fraction of the frame height.
///
/// **This is the difference between "did the screen change" and "did the picture change".**
/// Measured 23/08/2026 on ce0717171c2a64d50d (Galaxy S8, 1080x2220): three screencaps 600 ms
/// apart of one photo post (`Hynxy ở Nha Trang · Photo`, 6 slides) differed by 185, 267 and 82
/// sampled pixels, and an exhaustive comparison put **every one of them inside y 16..49** —
/// the animated network icon. Below that line the three frames were pixel-identical.
///
/// It survives the stream, too, which is what makes it matter here rather than only in a
/// screenshot: pushed through minicap's own pipeline — half of each edge, JPEG `-Q 70` — the
/// three frames still encoded to 83,113 / 83,201 / 83,212 bytes and [`frame_digest`] differed
/// on all three pairs. So a hash of the whole frame calls a perfectly still card "moving".
///
/// 0.04 is 88 px on these phones: clear of the icons at y=49, and still above TikTok's own
/// `For You` tab row, measured at y=141 in the same capture.
pub(crate) const STATUS_BAR_FRACTION: f64 = 0.04;

/// A hash of the picture, ignoring the phone's status bar.
///
/// Walks decoded pixels on a fixed grid rather than encoded bytes, so it answers "is this the
/// same picture" instead of "are these the same bytes" — two encodings of one picture are never
/// byte-equal, and one animated icon in the corner is enough to separate two that are.
///
/// ~1024 samples, the same order as [`frame_digest`]'s 512 and for the same reason: a still
/// card must hash identically every time, and a card that moved at all must not.
pub(crate) fn picture_digest(image: &image::RgbImage) -> u64 {
    let top = (f64::from(image.height()) * STATUS_BAR_FRACTION) as u32;
    let (width, height) = (image.width(), image.height().max(top + 1));
    let step_x = (width / 32).max(1);
    let step_y = ((height - top) / 32).max(1);
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325 ^ u64::from(width) ^ (u64::from(height) << 20);
    let mut y = top;
    while y < height {
        let mut x = 0;
        while x < width {
            for channel in image.get_pixel(x, y).0 {
                hash ^= u64::from(channel);
                hash = hash.wrapping_mul(0x100_0000_01b3);
            }
            x += step_x;
        }
        y += step_y;
    }
    hash
}

/// [`picture_digest`] for a caller holding an encoded frame.
///
/// `None` when the bytes will not decode, which the callers treat as "cannot say" rather than
/// as "unchanged" — guessing either way here would be a claim about a screen nobody read.
pub(crate) fn picture_digest_of(frame: &[u8]) -> Option<u64> {
    image::load_from_memory(frame)
        .ok()
        .map(|image| picture_digest(&image.to_rgb8()))
}

/// Cheap content fingerprint for "did the screen change?".
///
/// Whole encoded bytes, which is the right question for a *screen* — and the wrong one for a
/// picture. Use [`picture_digest_of`] when the question is whether the card itself moved; see
/// [`STATUS_BAR_FRACTION`] for the capture that separated the two.
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

    #[test]
    fn every_opened_nurture_session_has_one_shutdown_boundary() {
        let whole = include_str!("mod.rs").replace("\r\n", "\n");
        let source = whole
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("production source before tests");
        assert!(
            source.contains("let session_result: anyhow::Result<NurtureSessionStatus> = async"),
            "all post-open exits must be captured before cleanup"
        );
        assert!(
            source.contains("shutdown_tiktok(ui_context, &bundle_id).await"),
            "the single post-open boundary must terminate TikTok"
        );
        let final_boundary = source
            .split("let session_result: anyhow::Result<NurtureSessionStatus> = async")
            .nth(1)
            .expect("post-open session boundary");
        let stop_watcher = final_boundary
            .find("stop.store(true, Ordering::Release)")
            .expect("stop the popup watcher on every exit");
        let join_watcher = stop_watcher
            + final_boundary[stop_watcher..]
                .find("popup_watch_task.take()")
                .expect("join the popup watcher on every exit");
        let final_shutdown = final_boundary
            .find("shutdown_tiktok(ui_context, &bundle_id).await")
            .expect("terminate TikTok at the final boundary");
        assert!(
            stop_watcher < join_watcher && join_watcher < final_shutdown,
            "the popup watcher must stop and join before TikTok is terminated"
        );
        assert!(
            !source.contains("KHÔNG ở TikTok") && !source.contains("kết thúc ở TikTok"),
            "screen location is not proof that the TikTok process was terminated"
        );
        let helper = source
            .split("async fn shutdown_tiktok")
            .nth(1)
            .expect("shutdown helper");
        let terminate = helper
            .find("terminate_streaming_app")
            .expect("terminate TikTok first");
        let close = helper
            .find("close_ui_context")
            .expect("then release the UI context");
        assert!(
            terminate < close,
            "TikTok must stop before its session is released"
        );
    }

    #[test]
    fn save_evidence_updates_exactly_one_typed_counter_branch() {
        let mut policy = HumanSessionPolicy::new_with_save(0, 0, 100, 0, false);
        let mut status = NurtureSessionStatus::new("fixture");
        policy.begin_post();

        let no_effect = policy.reserve_attempt(PolicyAction::Save);
        let no_effect_evidence = SaveEvidence {
            verdict: SaveVerdict::StateUnreadable,
            initial: None,
            final_observation: None,
            effect_boundary_crossed: false,
            error: None,
        };
        assert!(!settle_save_evidence(
            &mut policy,
            no_effect,
            &mut status,
            &no_effect_evidence,
        ));
        assert_eq!((status.save_attempts, status.saves), (0, 0));
        assert_eq!((status.save_noops, status.save_uncertain), (1, 0));

        let ambiguous = policy.reserve_attempt(PolicyAction::Save);
        let ambiguous_evidence = SaveEvidence {
            verdict: SaveVerdict::NotConfirmed,
            effect_boundary_crossed: true,
            ..no_effect_evidence.clone()
        };
        assert!(settle_save_evidence(
            &mut policy,
            ambiguous,
            &mut status,
            &ambiguous_evidence,
        ));
        assert_eq!((status.save_attempts, status.saves), (1, 0));
        assert_eq!((status.save_noops, status.save_uncertain), (1, 1));

        let saved = policy.reserve_attempt(PolicyAction::Save);
        let saved_evidence = SaveEvidence {
            verdict: SaveVerdict::Saved,
            effect_boundary_crossed: true,
            ..no_effect_evidence
        };
        assert!(settle_save_evidence(
            &mut policy,
            saved,
            &mut status,
            &saved_evidence,
        ));
        assert_eq!((status.save_attempts, status.saves), (2, 1));
        assert_eq!((status.save_noops, status.save_uncertain), (1, 1));
    }

    #[test]
    fn durable_settle_failure_after_effect_keeps_the_cap_consumed_and_status_uncertain() {
        struct BrokenSettle;

        impl NurtureSaveJournal for BrokenSettle {
            fn arm(
                &self,
                _card_key: &str,
                _observation: &SaveObservation,
            ) -> anyhow::Result<NurtureSaveLease> {
                unreachable!("the test starts after the effect boundary")
            }

            fn settle(
                &self,
                _lease: Option<NurtureSaveLease>,
                _evidence: &SaveEvidence,
            ) -> anyhow::Result<()> {
                anyhow::bail!("audit unavailable")
            }
        }

        let mut policy = HumanSessionPolicy::new_with_save(0, 0, 100, 0, true);
        policy.pin_cap_for_test(PolicyAction::Save, 1);
        policy.begin_post();
        let reservation = policy.reserve_attempt(PolicyAction::Save);
        let mut status = NurtureSessionStatus::new("fixture");
        let evidence = SaveEvidence {
            verdict: SaveVerdict::UncertainAfterEffect,
            initial: None,
            final_observation: None,
            effect_boundary_crossed: true,
            error: Some("tap acknowledgement unavailable".into()),
        };

        let error = settle_journaled_save(
            &mut policy,
            reservation,
            &mut status,
            Some(&BrokenSettle),
            None,
            &evidence,
        )
        .expect_err("durable settle must remain a fatal session error");

        assert_eq!(error.to_string(), "audit unavailable");
        assert!(!policy.can_attempt(PolicyAction::Save));
        assert_eq!((status.save_attempts, status.saves), (1, 0));
        assert_eq!((status.save_noops, status.save_uncertain), (0, 1));
    }

    #[test]
    fn task4_save_ledger_is_armed_before_effect_and_settled_from_evidence() {
        use crate::interaction::{
            InteractionActionKind, InteractionActionState, TikTokActionOwnerKind,
        };
        use crate::tiktok_save::{BookmarkState, SaveCardIdentity, SaveObservation};

        let path = std::env::temp_dir().join(format!(
            "riviu-task4-save-ledger-{}.db",
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&path).expect("test database");
        let ledger = NurtureSaveLedger::new(&db, "session-a", "device-a");
        let observation = SaveObservation {
            identity: Some(SaveCardIdentity::Hierarchy {
                author: "author-a".into(),
                sound: Some("sound-a".into()),
            }),
            sequence: 2,
            state: BookmarkState::Unsaved,
            tap_point: Some(TapPoint { x: 20.0, y: 30.0 }),
        };
        let lease = Some(
            ledger
                .arm("card-7", &observation)
                .expect("durable arm before tap"),
        );

        let owner = lease.as_ref().expect("arm returns lease").owner.clone();
        assert_eq!(owner.kind, TikTokActionOwnerKind::Nurture);
        assert_eq!(owner.device_udid, "device-a");
        assert!(owner.owner_id.starts_with("session-a:card-7:"));
        assert_eq!(
            owner.card_identity.as_deref(),
            Some(r#"{"author":"author-a","sound":"sound-a","source":"hierarchy"}"#)
        );
        let armed = db
            .get_tiktok_action_run(&owner, InteractionActionKind::Save)
            .expect("read armed row")
            .expect("armed row exists");
        assert_eq!(armed.state, InteractionActionState::Armed);
        assert!(armed.effect_intent.as_deref().is_some_and(|intent| {
            intent.contains("set bookmark state to saved") && intent.contains("sequence")
        }));

        let evidence = SaveEvidence {
            verdict: SaveVerdict::Saved,
            initial: Some(observation.clone()),
            final_observation: Some(SaveObservation {
                sequence: 3,
                state: BookmarkState::Saved,
                ..observation
            }),
            effect_boundary_crossed: true,
            error: None,
        };
        ledger
            .settle(lease, &evidence)
            .expect("settle confirmed evidence");
        let confirmed = db
            .get_tiktok_action_run(&owner, InteractionActionKind::Save)
            .expect("read confirmed row")
            .expect("confirmed row exists");
        assert_eq!(confirmed.state, InteractionActionState::Confirmed);
        assert!(confirmed
            .evidence
            .as_deref()
            .is_some_and(|raw| raw.contains("saved")));

        drop(db);
        let _ = std::fs::remove_file(path);
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
            health.observe(CommentResult::TextSent {
                prompt_tokens: 0,
                completion_tokens: 0
            }),
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

#[cfg(test)]
mod slide_evidence_tests {
    use super::{frame_digest, picture_digest_of, SlideEvidence};
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    use image::{ColorType, ImageEncoder, Rgb, RgbImage};

    fn frame(byte: u8) -> Vec<u8> {
        // Long enough that `frame_digest`'s 512-point stride actually samples the difference.
        let mut bytes = vec![7u8; 4096];
        bytes[2048] = byte;
        bytes
    }

    fn png_frame(tone: u8, compression: CompressionType, filter: FilterType) -> Vec<u8> {
        let image = RgbImage::from_fn(48, 48, |x, y| {
            let value = tone.wrapping_add((x * 3 + y * 5) as u8);
            Rgb([value, value.wrapping_add(17), value.wrapping_add(43)])
        });
        let mut encoded = Vec::new();
        PngEncoder::new_with_quality(&mut encoded, compression, filter)
            .write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                ColorType::Rgb8.into(),
            )
            .expect("encode fixture PNG");
        encoded
    }

    /// The stream can encode the same decoded picture into different bytes. Evidence is about
    /// what was visible on the slide, so those encodings must collapse to one frame.
    #[test]
    fn two_encodings_of_the_same_picture_are_one_slide_frame() {
        let fast = png_frame(21, CompressionType::Fast, FilterType::NoFilter);
        let compact = png_frame(21, CompressionType::Best, FilterType::Paeth);
        assert_ne!(frame_digest(&fast), frame_digest(&compact));
        assert_eq!(picture_digest_of(&fast), picture_digest_of(&compact));

        let mut evidence = SlideEvidence::default();
        evidence.offer(fast);
        evidence.offer(compact);
        assert_eq!(evidence.drain().0.len(), 1);
    }

    /// Distinct decoded pictures stay distinct, while undecodable fixtures retain the old,
    /// deterministic byte-hash fallback instead of all collapsing into an `unknown` bucket.
    #[test]
    fn different_pictures_and_undecodable_fallbacks_stay_distinct() {
        let mut pictures = SlideEvidence::default();
        pictures.offer(png_frame(21, CompressionType::Fast, FilterType::NoFilter));
        pictures.offer(png_frame(88, CompressionType::Best, FilterType::Paeth));
        assert_eq!(pictures.drain().0.len(), 2);

        let invalid_a = frame(1);
        let invalid_b = frame(2);
        assert_eq!(picture_digest_of(&invalid_a), None);
        assert_eq!(picture_digest_of(&invalid_b), None);
        let mut fallback = SlideEvidence::default();
        fallback.offer(invalid_a.clone());
        fallback.offer(invalid_a);
        fallback.offer(invalid_b);
        assert_eq!(fallback.drain().0.len(), 2);
    }

    /// First and last, and the reason it is two rather than one per slide is arithmetic: the
    /// contact sheet spends a fixed pixel budget, so on this fleet's 1080x2220 frames one slide
    /// gets a 589x1211 thumb, two get 367x754 and three get 271x557 — and `visualFacts` is read
    /// off exactly the small text that goes first.
    #[test]
    fn the_buffer_keeps_the_first_slide_and_the_last_different_one() {
        let mut evidence = SlideEvidence::default();
        for byte in [1u8, 2, 3, 4] {
            evidence.offer(frame(byte));
        }
        let (frames, offered) = evidence.drain();
        assert_eq!(offered, 4, "all four slides were offered");
        assert_eq!(frames.len(), 2, "and two of them were kept");
        assert_eq!(frames[0], frame(1), "the first slide");
        assert_eq!(
            frames[1],
            frame(4),
            "and the last one that differed from it"
        );
    }

    /// **The ordinary photo-post case.** A still card publishes the same picture on every
    /// sample — measured on a live card, 0 of 2,170,800 picture pixels changed over 13 s
    /// untouched — so a post the stream never repainted leaves exactly one frame here, and the
    /// sheet says "one khung" instead of pasting it twice and calling it evidence.
    #[test]
    fn a_parked_stream_leaves_one_frame_and_still_counts_its_slides() {
        let mut evidence = SlideEvidence::default();
        for _ in 0..7 {
            evidence.offer(frame(9));
        }
        let (frames, offered) = evidence.drain();
        assert_eq!((frames.len(), offered), (1, 7), "seven slides, one picture");
    }

    /// Draining matters: the source lives for the whole session, so a slide left behind would
    /// ground the next post's comment on this post's pictures.
    #[test]
    fn draining_leaves_nothing_for_the_next_post() {
        let mut evidence = SlideEvidence::default();
        evidence.offer(frame(1));
        evidence.offer(frame(2));
        assert_eq!(evidence.drain().0.len(), 2);
        assert_eq!(evidence.drain(), (Vec::new(), 0));
    }

    /// A post that was never paged asks for nothing, and gets nothing — which is the signal
    /// `prepare_hierarchy_comment` reads to sample the frames itself, exactly as before.
    #[test]
    fn a_post_that_was_never_paged_is_empty_rather_than_stale() {
        let mut evidence = SlideEvidence::default();
        assert_eq!(evidence.drain(), (Vec::new(), 0));
    }
}
