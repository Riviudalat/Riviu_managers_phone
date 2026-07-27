//! Per-device screen watcher driven by the frame stream.
//!
//! This is the answer to "a popup must close the moment it appears" without
//! either of the two approaches that failed on this USB stack:
//!
//! * polling WDA `GET /screenshot` (wedges the control relay), or
//! * checking every N videos (the user rejected the latency).
//!
//! Instead the watcher subscribes to the MJPEG frames the app is already
//! pulling for the device tile, so watching costs the control plane nothing.
//! Work is bounded by coalescing to a few frames per second and by skipping
//! frames whose bytes are unchanged — a still feed decodes nothing at all.
//!
//! Nothing here taps on a guess. A popup is acted on only after two consecutive
//! frames agree on its kind and position, gestures are serialised against the
//! nurture loop through a shared lock, and every tap is confirmed against a
//! later frame before it counts as closed.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;

use crate::driver::UiSession;
use crate::frame_source::FrameSource;
use crate::screen::{self, ScreenKind, ScreenObservation};
use crate::types::TapPoint;

/// Upper bound on classification work. The stream runs at ~7–24 FPS; a popup
/// that survives a third of a second is still closed effectively instantly, and
/// this keeps a debug build comfortable.
const MAX_ANALYSIS_FPS: u32 = 3;

/// Frames that must agree before a popup is tapped. Two consecutive matching
/// classifications rule out a mid-transition frame or a compression artefact.
const CONFIRM_FRAMES: u32 = 2;

/// A sheet's ✕ must land within this fraction of the previous sighting for the
/// two frames to count as the same popup (≈11 px on this device).
const POSITION_TOLERANCE: f64 = 0.03;

/// Quiet period after a dismissal tap, so the closing animation is not read as
/// "the popup is still there".
const TAP_COOLDOWN: Duration = Duration::from_millis(1_600);

/// How long a popup may persist after a tap before we try again.
const CONFIRM_WINDOW: Duration = Duration::from_secs(5);

/// Dismissal attempts for one continuous sighting. After this the watcher stops
/// tapping and reports, rather than drumming on a screen it does not understand.
const MAX_ATTEMPTS: u32 = 3;

/// Where the interest picker's "Bỏ qua" pill sits, in screen fractions.
///
/// Carried over from the previous implementation, which measured it at
/// 0.08–0.40 × 0.90–0.96. It is the only measurement this project has for that
/// page and it has **not** been re-verified against a live capture, so the
/// watcher confirms the result and gives up instead of re-tapping blind; a
/// frame is dumped whenever the page is seen so it can be pinned down properly.
const SKIP_PILL: (f64, f64) = (0.24, 0.93);

/// The live WDA session, shared between the nurture loop and the watcher.
///
/// The loop replaces it on reopen; the watcher always taps through whatever is
/// current, and simply does nothing while the slot is empty.
#[derive(Clone, Default)]
pub struct SessionHandle {
    inner: Arc<RwLock<Option<Arc<dyn UiSession>>>>,
}

impl SessionHandle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self) -> Option<Arc<dyn UiSession>> {
        self.inner.read().clone()
    }

    pub fn set(&self, session: Arc<dyn UiSession>) {
        *self.inner.write() = Some(session);
    }

    pub fn clear(&self) {
        *self.inner.write() = None;
    }
}

/// Counters the session summary reports.
#[derive(Default)]
pub struct WatchStats {
    pub frames_seen: AtomicU32,
    pub frames_analyzed: AtomicU32,
    pub popups_detected: AtomicU32,
    pub popups_closed: AtomicU32,
    pub popups_unresolved: AtomicU32,
    /// Total milliseconds from first sighting to a confirmed close.
    pub close_latency_ms_total: AtomicU32,
}

impl WatchStats {
    pub fn snapshot(&self) -> WatchSummary {
        let closed = self.popups_closed.load(Ordering::Relaxed);
        WatchSummary {
            frames_seen: self.frames_seen.load(Ordering::Relaxed),
            frames_analyzed: self.frames_analyzed.load(Ordering::Relaxed),
            popups_detected: self.popups_detected.load(Ordering::Relaxed),
            popups_closed: closed,
            popups_unresolved: self.popups_unresolved.load(Ordering::Relaxed),
            mean_close_ms: if closed == 0 {
                0
            } else {
                self.close_latency_ms_total.load(Ordering::Relaxed) / closed
            },
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct WatchSummary {
    pub frames_seen: u32,
    pub frames_analyzed: u32,
    pub popups_detected: u32,
    pub popups_closed: u32,
    pub popups_unresolved: u32,
    pub mean_close_ms: u32,
}

/// The latest classification, for the nurture loop to consult without taking a
/// screenshot of its own.
#[derive(Clone)]
pub struct ScreenState {
    inner: Arc<RwLock<Option<(ScreenObservation, Instant)>>>,
}

impl Default for ScreenState {
    fn default() -> Self {
        Self {
            inner: Arc::new(RwLock::new(None)),
        }
    }
}

impl ScreenState {
    /// The most recent observation, if it is younger than `max_age`.
    pub fn recent(&self, max_age: Duration) -> Option<ScreenObservation> {
        self.inner
            .read()
            .as_ref()
            .filter(|(_, at)| at.elapsed() <= max_age)
            .map(|(obs, _)| *obs)
    }

    fn set(&self, obs: ScreenObservation) {
        *self.inner.write() = Some((obs, Instant::now()));
    }
}

type LogFn = Arc<dyn Fn(&str) + Send + Sync>;

/// Builds and owns the watcher task for one device.
pub struct ScreenWatcher {
    udid: String,
    frames: Arc<dyn FrameSource>,
    session: SessionHandle,
    gestures: Arc<tokio::sync::Mutex<()>>,
    stop: Arc<AtomicBool>,
    logical: (f64, f64),
    log: LogFn,
    pub stats: Arc<WatchStats>,
    pub state: ScreenState,
}

impl ScreenWatcher {
    pub fn new(
        udid: &str,
        frames: Arc<dyn FrameSource>,
        session: SessionHandle,
        gestures: Arc<tokio::sync::Mutex<()>>,
        stop: Arc<AtomicBool>,
        logical: (f64, f64),
        log: LogFn,
    ) -> Self {
        Self {
            udid: udid.to_string(),
            frames,
            session,
            gestures,
            stop,
            logical,
            log,
            stats: Arc::new(WatchStats::default()),
            state: ScreenState::default(),
        }
    }

    /// Run until `stop` is set. Each device gets its own task, its own stop
    /// flag and its own cooldown state, so devices never share watcher state.
    pub async fn run(self) {
        self.run_suppressible(Arc::new(AtomicBool::new(false))).await
    }

    /// As [`Self::run`], but the caller can suspend *acting* while it drives a
    /// multi-step flow of its own.
    ///
    /// The comment drawer is the case this exists for: it hides TikTok's
    /// compose bar, so the classifier legitimately stops seeing the feed, and a
    /// watcher that kept hunting for close buttons would tap inside the drawer.
    /// Classification keeps running while suppressed, so [`Self::state`] stays
    /// current for whoever is driving.
    pub async fn run_suppressible(self, suppress: Arc<AtomicBool>) {
        let min_gap = Duration::from_millis(1_000 / MAX_ANALYSIS_FPS.max(1) as u64);
        let dump = FrameDump::from_env(&self.udid);
        let mut last_analyzed: Option<Instant> = None;
        let mut last_digest: u64 = 0;
        let mut pending: Option<Sighting> = None;
        let mut awaiting: Option<Awaiting> = None;
        let mut cooldown_until: Option<Instant> = None;

        'outer: while !self.stop.load(Ordering::Relaxed) {
            let mut stream = self.frames.subscribe(&self.udid);
            loop {
                if self.stop.load(Ordering::Relaxed) {
                    break 'outer;
                }
                // A stopped session must not leave this task parked forever on
                // a stream that will never produce another frame.
                let frame = match tokio::time::timeout(Duration::from_millis(500), stream.next())
                    .await
                {
                    Ok(Some(f)) => f,
                    Ok(None) => break,
                    Err(_) => continue,
                };
                self.stats.frames_seen.fetch_add(1, Ordering::Relaxed);

                if last_analyzed.map_or(false, |t| t.elapsed() < min_gap) {
                    continue;
                }
                // A motionless feed re-sends identical bytes; decoding them
                // again cannot change the answer.
                let digest = digest(&frame);
                if digest == last_digest {
                    continue;
                }
                last_digest = digest;
                last_analyzed = Some(Instant::now());

                let Ok(img) = image::load_from_memory(&frame).map(|i| i.to_rgb8()) else {
                    continue;
                };
                let obs = screen::classify(&img, Some(self.logical.0));
                self.stats.frames_analyzed.fetch_add(1, Ordering::Relaxed);
                self.state.set(obs);
                dump.maybe_write(&frame, &obs);

                if suppress.load(Ordering::Relaxed) {
                    // Someone else owns the screen right now. Keep observing,
                    // but drop any half-built sighting so confirmation starts
                    // fresh once they hand it back.
                    pending = None;
                    continue;
                }

                match obs.kind {
                    ScreenKind::Feed => {
                        if let Some(a) = awaiting.take() {
                            let ms = a.first_seen.elapsed().as_millis() as u32;
                            self.stats.popups_closed.fetch_add(1, Ordering::Relaxed);
                            self.stats
                                .close_latency_ms_total
                                .fetch_add(ms, Ordering::Relaxed);
                            (self.log)(&format!(
                                "Xác nhận popup biến mất ({}) sau {:.1}s",
                                a.label,
                                ms as f64 / 1000.0
                            ));
                        }
                        pending = None;
                    }
                    ScreenKind::Unknown => {
                        // Not TikTok, or a frame we cannot read. Never tap on
                        // this; the nurture loop decides what to do about it.
                        pending = None;
                        if let Some(a) = &awaiting {
                            if a.first_seen.elapsed() > CONFIRM_WINDOW {
                                self.give_up(&mut awaiting);
                            }
                        }
                    }
                    ScreenKind::ClosableSheet { x, y, score } => {
                        let target = (x, y);
                        self.on_popup(
                            Sighting::sheet(target, score),
                            &mut pending,
                            &mut awaiting,
                            &mut cooldown_until,
                        )
                        .await;
                    }
                    ScreenKind::LiveRoom => {
                        self.on_popup(
                            Sighting::live(),
                            &mut pending,
                            &mut awaiting,
                            &mut cooldown_until,
                        )
                        .await;
                    }
                    ScreenKind::SystemAlert { x, y } => {
                        self.on_popup(
                            Sighting::system_alert((x, y)),
                            &mut pending,
                            &mut awaiting,
                            &mut cooldown_until,
                        )
                        .await;
                    }
                    ScreenKind::InterestPicker => {
                        self.on_popup(
                            Sighting::picker(),
                            &mut pending,
                            &mut awaiting,
                            &mut cooldown_until,
                        )
                        .await;
                    }
                }
            }
            // Stream ended (device unplugged, stream restarted) — resubscribe.
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    async fn on_popup(
        &self,
        seen: Sighting,
        pending: &mut Option<Sighting>,
        awaiting: &mut Option<Awaiting>,
        cooldown_until: &mut Option<Instant>,
    ) {
        // Still inside the post-tap animation window: say nothing, do nothing.
        if cooldown_until.map_or(false, |t| Instant::now() < t) {
            return;
        }
        // A popup that outlived its confirmation window is retried, up to a cap.
        if let Some(a) = awaiting {
            if a.first_seen.elapsed() > CONFIRM_WINDOW {
                if a.attempts >= MAX_ATTEMPTS {
                    self.give_up(awaiting);
                    return;
                }
            } else {
                return;
            }
        }

        let agreed = match pending.as_mut() {
            Some(prev) if prev.same_as(&seen) => {
                prev.count += 1;
                prev.count
            }
            _ => {
                *pending = Some(seen.clone());
                1
            }
        };
        if agreed < CONFIRM_FRAMES {
            return;
        }

        let first_seen = awaiting
            .as_ref()
            .map(|a| a.first_seen)
            .unwrap_or_else(Instant::now);
        let attempts = awaiting.as_ref().map(|a| a.attempts).unwrap_or(0);
        if attempts == 0 {
            self.stats.popups_detected.fetch_add(1, Ordering::Relaxed);
            (self.log)(&format!("Phát hiện {}", seen.describe()));
        }

        let (fx, fy) = seen.tap_target();
        let point = TapPoint {
            x: self.logical.0 * fx,
            y: self.logical.1 * fy,
        };
        let Some(session) = self.session.get() else {
            return;
        };
        // Serialised against the nurture loop's own gestures: a dismissal tap
        // must never land in the middle of a swipe.
        let outcome = {
            let _guard = self.gestures.lock().await;
            session.tap(point).await
        };
        *pending = None;
        *cooldown_until = Some(Instant::now() + TAP_COOLDOWN);
        match outcome {
            Ok(()) => {
                (self.log)(&format!("Đã đóng popup ({})", seen.label()));
                *awaiting = Some(Awaiting {
                    label: seen.label(),
                    first_seen,
                    attempts: attempts + 1,
                });
            }
            Err(e) => {
                (self.log)(&format!("Đóng popup thất bại ({}): {e}", seen.label()));
                *awaiting = Some(Awaiting {
                    label: seen.label(),
                    first_seen,
                    attempts: attempts + 1,
                });
            }
        }
    }

    fn give_up(&self, awaiting: &mut Option<Awaiting>) {
        if let Some(a) = awaiting.take() {
            self.stats.popups_unresolved.fetch_add(1, Ordering::Relaxed);
            (self.log)(&format!(
                "Popup {} chưa đóng được sau {} lần — dừng tap, chờ vòng sau",
                a.label, a.attempts
            ));
        }
    }
}

/// One classification that might become a dismissal.
#[derive(Clone)]
struct Sighting {
    kind: &'static str,
    target: (f64, f64),
    score: f64,
    count: u32,
}

impl Sighting {
    fn sheet(target: (f64, f64), score: f64) -> Self {
        Self {
            kind: "sheet",
            target,
            score,
            count: 0,
        }
    }

    fn picker() -> Self {
        Self {
            kind: "interest",
            target: SKIP_PILL,
            score: 0.0,
            count: 0,
        }
    }

    /// An iOS alert, aimed at its dismissive button. Unlike the others this
    /// one is not TikTok's, so no amount of swiping or app-switching clears it.
    fn system_alert(target: (f64, f64)) -> Self {
        Self {
            kind: "system-alert",
            target,
            score: 0.0,
            count: 0,
        }
    }

    /// A LIVE room is left through the ✕, not by swiping: a vertical swipe
    /// scrolls the room's own content, so a session that drifts in stays there.
    fn live() -> Self {
        Self {
            kind: "live",
            target: screen::LIVE_EXIT,
            score: 0.0,
            count: 0,
        }
    }

    fn same_as(&self, other: &Sighting) -> bool {
        self.kind == other.kind
            && (self.target.0 - other.target.0).abs() <= POSITION_TOLERANCE
            && (self.target.1 - other.target.1).abs() <= POSITION_TOLERANCE
    }

    fn tap_target(&self) -> (f64, f64) {
        self.target
    }

    fn label(&self) -> &'static str {
        match self.kind {
            "interest" => "Chọn chủ đề",
            "live" => "phòng LIVE",
            _ => "popup nút X",
        }
    }

    fn describe(&self) -> String {
        match self.kind {
            "interest" => "trang Chọn chủ đề — bấm Bỏ qua".to_string(),
            "live" => "đang ở phòng LIVE — bấm ✕ để về FYP".to_string(),
            _ => format!(
                "popup nút X score={:.3} tại ({:.3}, {:.3})",
                self.score, self.target.0, self.target.1
            ),
        }
    }
}

struct Awaiting {
    label: &'static str,
    first_seen: Instant,
    attempts: u32,
}

/// Cheap content fingerprint: length plus a sample of the encoded bytes. Two
/// different screens never collide in practice, and an unchanged screen always
/// matches, which is the case worth optimising.
fn digest(frame: &[u8]) -> u64 {
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

/// Optional frame capture for calibrating detectors against real popups.
/// Enabled with `RIVIU_FRAME_DUMP=<dir>`; writes at most [`Self::LIMIT`] files
/// so a long run cannot fill the disk.
struct FrameDump {
    dir: Option<std::path::PathBuf>,
    written: AtomicU32,
    last_kind: RwLock<&'static str>,
}

impl FrameDump {
    const LIMIT: u32 = 300;

    fn from_env(udid: &str) -> Self {
        let dir = std::env::var("RIVIU_FRAME_DUMP").ok().map(|d| {
            let path = std::path::PathBuf::from(d).join(&udid[..8.min(udid.len())]);
            let _ = std::fs::create_dir_all(&path);
            path
        });
        Self {
            dir: dir,
            written: AtomicU32::new(0),
            last_kind: RwLock::new(""),
        }
    }

    /// Write a frame whenever the classification changes, so the dump captures
    /// the transition into and out of every popup instead of a burst of feed.
    fn maybe_write(&self, frame: &[u8], obs: &ScreenObservation) {
        let Some(dir) = &self.dir else { return };
        let label = obs.kind.label();
        {
            let mut last = self.last_kind.write();
            if *last == label {
                return;
            }
            *last = label;
        }
        let n = self.written.fetch_add(1, Ordering::Relaxed);
        if n >= Self::LIMIT {
            return;
        }
        let name = format!("{n:04}-{label}.jpg");
        let _ = std::fs::write(dir.join(&name), frame);
        let _ = std::fs::write(
            dir.join(format!("{n:04}-{label}.txt")),
            obs.debug_line().as_bytes(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_frames_share_a_digest_and_different_ones_do_not() {
        let a = vec![7u8; 4096];
        let mut b = a.clone();
        assert_eq!(digest(&a), digest(&b));
        b[2048] = 9;
        assert_ne!(digest(&a), digest(&b), "a changed frame must be re-analysed");
    }

    #[test]
    fn a_sheet_that_moved_is_a_different_sighting() {
        let first = Sighting::sheet((0.92, 0.56), 0.99);
        let same = Sighting::sheet((0.925, 0.565), 0.98);
        let moved = Sighting::sheet((0.92, 0.70), 0.98);
        assert!(first.same_as(&same), "jitter within tolerance is one popup");
        assert!(!first.same_as(&moved), "a real move restarts confirmation");
        assert!(!first.same_as(&Sighting::picker()));
    }

    #[test]
    fn session_handle_starts_empty_and_can_be_cleared() {
        let h = SessionHandle::new();
        assert!(h.get().is_none());
        h.clear();
        assert!(h.get().is_none());
    }

    #[test]
    fn screen_state_expires_stale_observations() {
        let state = ScreenState::default();
        assert!(state.recent(Duration::from_secs(1)).is_none());
        state.set(ScreenObservation {
            kind: ScreenKind::Feed,
            evidence: Default::default(),
        });
        assert!(state.recent(Duration::from_secs(5)).is_some());
        assert!(
            state.recent(Duration::ZERO).is_none(),
            "a zero-age window must reject even a fresh reading"
        );
    }
}
