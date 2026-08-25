//! Clearing TikTok's onboarding pages off phones nobody is driving.
//!
//! The problem this exists for was measured on 23/08/2026: of fourteen phones attached,
//! **six** were sitting on `com.ss.android.ugc.aweme.journey.NewUserJourneyActivity` behind
//! "TikTok is better with friends!". Nothing in the app could clear them, because every
//! ladder in this project ran *inside* a nurture session — so a phone that was blocked
//! before a session started stayed blocked, and the session it eventually got spent its
//! whole thirty-second window discovering that.
//!
//! So: the same ladder ([`riviu_core::feed_ladder`]), run on a slow tick against phones
//! that are idle. Not a second ladder — the rungs, their order and the argument for each
//! live in that module and this file owns only the *budget* and the *scheduling*.
//!
//! ## The four rules this thing lives under
//!
//! 1. **It never competes.** It asks with `open_manual_session`, which takes the exclusive
//!    lease without waiting; a phone held by nurture, an interaction, a script, a repair
//!    or the operator's own control overlay is skipped in silence.
//!    [`DeviceWorkOwner::IdleSweep`] may not queue.
//! 2. **It never parks a stream.** `open_manual_session` is the overlay's own path,
//!    `try_acquire_exclusive_keeping_stream` underneath, so the tiles keep painting. The
//!    parking version of the same call would black fourteen tiles every sweep.
//! 3. **It never touches a phone outside TikTok.** [`feed_ladder::foreground_labels`]
//!    refuses on anything else, and no rung is probed before that refusal.
//! 4. **It has a ceiling.** AGENTS.md §9.67 measured what an uncapped per-device
//!    background actor costs on this fleet: 33 producer starts at two phones and **291**
//!    at twenty, because each restart costs adb and CPU, which makes more devices miss
//!    their window, which triggers more work. [`MAX_CONCURRENT`] is that lesson applied
//!    before the fact rather than after.
//!
//! Turn it off with `RIVIU_IDLE_SWEEP=off`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use riviu_core::feed_ladder::{self, LadderRefusal, LadderSpend, LadderStep};
use riviu_core::tiktok_labels::TikTokControls;
use riviu_core::{
    DeviceControlPlane, DevicePlatform, DeviceRegistry, DeviceStatus, DeviceWorkOwner,
    SessionLogBook,
};
use tokio::sync::Semaphore;

/// How often the fleet is looked at.
///
/// Forty-five seconds, chosen against what it is competing with rather than for
/// responsiveness. A phone stuck on an onboarding page has been stuck for hours by the
/// time anybody notices, so clearing it in forty-five seconds versus five makes no
/// difference to the operator — while a five-second tick would mean fourteen `dumpsys`
/// reads every five seconds forever, on the same USB bus the tiles are streaming over.
const SWEEP_INTERVAL: Duration = Duration::from_secs(45);

/// Devices worked on at the same time. See rule 4 in the module docs.
const MAX_CONCURRENT: usize = 3;

/// Ladder steps allowed per device per visit.
///
/// Three, because three is what the measured journey costs end to end: `Skip` on the
/// friend-sync page, `Skip` on its confirmation, `Done` on the suggested-follow list. A
/// phone needing a fourth is not a phone this sweeper understands, and the next tick is
/// forty-five seconds away — which is the right response to not understanding something.
const STEPS_PER_VISIT: u32 = 3;

/// Shortening that is a regression rather than a tuning choice: the phones this was built
/// for would then need one sweep *per tap*, forty-five seconds apart. A compile-time
/// assertion rather than a test, because it lives beside the constant it guards and
/// cannot be left behind when somebody edits the number.
const _: () = assert!(
    STEPS_PER_VISIT >= 3,
    "Skip -> Skip -> Done is three taps; a smaller budget cannot finish the journey \
     measured on 23/08/2026 in one visit"
);

/// Back presses allowed per visit, once patience has been earned.
const BACKS_PER_VISIT: u32 = 1;

/// Consecutive visits that must find nothing measurable before Back is allowed.
///
/// The ladder holds Back until the caller says so, and this is what that means here.
/// `await_feed` earns it with ten seconds of waiting inside one session; a sweeper has no
/// session to wait inside, so it earns it by seeing the same unreadable screen on two
/// separate visits ninety seconds apart. A phone mid-transition, mid-splash or briefly
/// unreadable is not answered with a keypress on the strength of one look.
const BLIND_VISITS_BEFORE_BACK: u32 = 2;

/// What the sweeper remembers about one phone between visits.
#[derive(Default)]
struct DeviceMemo {
    /// The resolved label set, and the package it was resolved for.
    ///
    /// Cached because resolving it is the expensive part: `app_version` shells out to
    /// `dumpsys package`, measured at 1–2 s on this fleet, and paying that per device per
    /// sweep would make the sweeper the heaviest thing on the bus. Keyed by package so a
    /// phone whose TikTok is replaced re-resolves rather than using the old build's
    /// strings.
    labels: Option<(String, TikTokControls)>,
    /// Consecutive visits that found no measured control at all.
    blind_visits: u32,
    /// The last refusal already written to the log, so a permanently unmeasured build
    /// says so once rather than once every forty-five seconds forever.
    said_refusal: Option<String>,
}

/// The background sweep.
#[derive(Clone)]
pub struct IdleSweeper {
    control: Arc<DeviceControlPlane>,
    registry: DeviceRegistry,
    log: SessionLogBook,
    memos: Arc<Mutex<HashMap<String, DeviceMemo>>>,
    permits: Arc<Semaphore>,
}

impl IdleSweeper {
    pub fn new(
        control: Arc<DeviceControlPlane>,
        registry: DeviceRegistry,
        log: SessionLogBook,
    ) -> Self {
        Self {
            control,
            registry,
            log,
            memos: Arc::new(Mutex::new(HashMap::new())),
            permits: Arc::new(Semaphore::new(MAX_CONCURRENT)),
        }
    }

    /// Whether the operator has switched this off.
    ///
    /// An environment variable rather than a settings row, and deliberately so: this is a
    /// kill switch for a background actor that touches real phones, and it must work on a
    /// build that will not start or a database that will not open. `off`, `0` and `false`
    /// all mean off; anything else, including unset, means on.
    pub fn enabled() -> bool {
        !matches!(
            std::env::var("RIVIU_IDLE_SWEEP")
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "off" | "0" | "false" | "no"
        )
    }

    /// Run forever, one pass per [`SWEEP_INTERVAL`].
    pub async fn run(self) {
        if !Self::enabled() {
            log::info!("idle sweeper off (RIVIU_IDLE_SWEEP)");
            return;
        }
        let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
        // A tick missed while a slow sweep was in flight must not queue up and fire back
        // to back the moment it finishes — that is how a background actor turns a slow
        // fleet into a busy one.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            self.sweep_once().await;
        }
    }

    /// One pass over the fleet.
    async fn sweep_once(&self) {
        // **Rule 1, kept for work that is about to start as well as work already running.**
        //
        // Taking each lease without waiting yields to a phone somebody else holds *now*, and
        // a campaign reaches its phones seconds apart — so the ones it has not got to yet
        // look idle and this would take them. Measured 25/08/2026 across three twenty-phone
        // runs: assignments failed with `device … is busy with IdleSweep`, a different phone
        // each time, always one the campaign had not reached. Standing down for the length of
        // a campaign costs a tick of background tidying; not standing down costs a comment the
        // operator asked for.
        if riviu_core::interaction_campaign::any_campaign_running() {
            return;
        }
        let candidates: Vec<String> = self
            .registry
            .list()
            .into_iter()
            .filter(|device| sweepable(device.platform, &device.status))
            .map(|device| device.udid)
            .collect();
        if candidates.is_empty() {
            return;
        }
        // Forget phones that have gone away, so the memo map cannot grow without bound
        // across a long uptime of plugging and unplugging.
        self.memos
            .lock()
            .retain(|udid, _| candidates.iter().any(|live| live == udid));

        // Joined rather than fire-and-forget: the tick must not start a second sweep over
        // the same phones while the first is still working through them.
        let mut visits = tokio::task::JoinSet::new();
        for udid in candidates {
            let sweeper = self.clone();
            visits.spawn(async move {
                // The ceiling is taken here rather than around the whole sweep, so three
                // phones are in flight at a time and the rest queue instead of the fleet
                // going one at a time or all at once.
                let Ok(_permit) = sweeper.permits.clone().acquire_owned().await else {
                    return;
                };
                sweeper.visit(&udid).await;
            });
        }
        while visits.join_next().await.is_some() {}
    }

    /// One phone, one visit.
    async fn visit(&self, udid: &str) {
        // Not `device_lease`: that lends the control overlay's lease when the operator has
        // one open, which is exactly the phone this must not touch. Asking for its own
        // lease means an open overlay refuses us, which is the answer we want.
        let Ok(context) = self
            .control
            .open_manual_session(udid, DeviceWorkOwner::IdleSweep)
            .await
        else {
            // Busy with real work, or the agent is not up. Both are ordinary and neither
            // is worth a log line every forty-five seconds.
            return;
        };
        let session = match self.control.session(&context) {
            Ok(session) => session,
            Err(_) => {
                let _ = self.control.close_manual_session(context);
                return;
            }
        };

        self.walk_ladder(udid, session.as_ref()).await;

        if let Err(error) = self.control.close_manual_session(context) {
            log::warn!("idle sweep could not release {udid}: {error}");
        }
    }

    /// The ladder itself, with this caller's budget around it.
    async fn walk_ladder(&self, udid: &str, session: &dyn riviu_core::UiSession) {
        let labels = match self.labels_for(udid, session).await {
            Ok(labels) => labels,
            Err(refusal) => {
                self.say_refusal_once(udid, &refusal);
                return;
            }
        };
        self.forget_refusal(udid);

        let mut spend = LadderSpend::new(BACKS_PER_VISIT);
        spend.allow_back = self.blind_visits(udid) >= BLIND_VISITS_BEFORE_BACK;

        let mut acted = false;
        for _ in 0..STEPS_PER_VISIT {
            let step = feed_ladder::step(session, labels, &mut spend).await;
            if step == LadderStep::OnFeed {
                if acted {
                    self.log.record(udid, "đã đưa máy về feed");
                }
                self.clear_blind_visits(udid);
                return;
            }
            if let Some(line) = step.says() {
                self.log.record(udid, &line);
            }
            if !step.acted() {
                // Nothing matched. Count the visit as blind so Back is eventually earned,
                // and stop — re-probing the same unchanged screen twice in one visit only
                // spends round trips.
                self.count_blind_visit(udid);
                return;
            }
            acted = true;
            // The screen is mid-transition after a tap. This is the same 1 s the session
            // ladder waits, for the same reason.
            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        // Budget spent without reaching the feed. Say so — a phone that needs more than
        // the measured journey's three steps is the case worth a human's eye, and the
        // silent version of this is how six phones sat blocked unnoticed in the first
        // place.
        self.count_blind_visit(udid);
        self.log.record(
            udid,
            "đã dọn hết mức cho phép mà chưa thấy feed — chờ lượt quét sau",
        );
    }

    /// The label set for this phone, resolved once per package.
    async fn labels_for(
        &self,
        udid: &str,
        session: &dyn riviu_core::UiSession,
    ) -> Result<TikTokControls, LadderRefusal> {
        // The foreground read is cheap and must happen every visit anyway: it is both the
        // "is this TikTok" gate and the cache key. Only the expensive half — language and
        // `dumpsys package` for the version — is skipped on a hit.
        let package = session.active_app_bundle().await.unwrap_or_default();
        let cached = self.memos.lock().get(udid).and_then(|memo| {
            memo.labels
                .as_ref()
                .filter(|(cached_for, _)| *cached_for == package)
                .map(|(_, labels)| *labels)
        });
        if let Some(labels) = cached {
            return Ok(labels);
        }
        let labels = feed_ladder::foreground_labels(session).await?;
        self.memos
            .lock()
            .entry(udid.to_string())
            .or_default()
            .labels = Some((package, labels));
        Ok(labels)
    }

    fn blind_visits(&self, udid: &str) -> u32 {
        self.memos
            .lock()
            .get(udid)
            .map(|m| m.blind_visits)
            .unwrap_or(0)
    }

    fn count_blind_visit(&self, udid: &str) {
        let mut memos = self.memos.lock();
        let memo = memos.entry(udid.to_string()).or_default();
        memo.blind_visits = memo.blind_visits.saturating_add(1);
    }

    fn clear_blind_visits(&self, udid: &str) {
        if let Some(memo) = self.memos.lock().get_mut(udid) {
            memo.blind_visits = 0;
        }
    }

    /// Write a refusal to the log the first time it is seen, and not again while it holds.
    ///
    /// An unmeasured build refuses on every sweep forever. Saying it once is information;
    /// saying it every forty-five seconds fills the two hundred lines this device's log
    /// keeps and pushes out the history somebody opened it to read.
    fn say_refusal_once(&self, udid: &str, refusal: &LadderRefusal) {
        let Some(line) = refusal.says() else {
            return;
        };
        let mut memos = self.memos.lock();
        let memo = memos.entry(udid.to_string()).or_default();
        if memo.said_refusal.as_deref() == Some(line.as_str()) {
            return;
        }
        memo.said_refusal = Some(line.clone());
        drop(memos);
        self.log.record(udid, &line);
    }

    fn forget_refusal(&self, udid: &str) {
        if let Some(memo) = self.memos.lock().get_mut(udid) {
            memo.said_refusal = None;
        }
    }
}

/// Whether a device is one this sweeper may look at.
///
/// A pure function so the rule is testable without a fleet. iOS is excluded by
/// construction rather than by the ladder refusing later: that platform locates controls
/// by pixels ([`riviu_core::screen_watch`]) because its accessibility tree is not readable,
/// so there is nothing here for it to do and opening a WDA session to find that out would
/// cost the one thing iOS sessions are fragile about.
fn sweepable(platform: DevicePlatform, status: &DeviceStatus) -> bool {
    if platform != DevicePlatform::Android {
        return false;
    }
    // `Busy` is somebody else's work and would be refused by the lease anyway — skipping
    // it here saves the round trip. `Ready` and `Connected` are the two states a phone
    // sits in while idle: `Ready` once the agent is up, `Connected` before it is, and the
    // second one still gets a visit because `open_manual_session` is what brings the agent
    // up and a phone stuck on an onboarding page is often exactly one that never got there.
    matches!(status, DeviceStatus::Ready | DeviceStatus::Connected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_idle_android_phones_are_swept() {
        assert!(sweepable(DevicePlatform::Android, &DeviceStatus::Ready));
        assert!(sweepable(DevicePlatform::Android, &DeviceStatus::Connected));
    }

    #[test]
    fn ios_is_never_swept() {
        for status in [
            DeviceStatus::Ready,
            DeviceStatus::Connected,
            DeviceStatus::Busy,
        ] {
            assert!(
                !sweepable(DevicePlatform::Ios, &status),
                "iOS locates by pixels and has no hierarchy ladder to run: {status:?}"
            );
        }
    }

    #[test]
    fn a_phone_that_is_not_idle_is_left_alone() {
        for status in [
            DeviceStatus::Busy,
            DeviceStatus::Disconnected,
            DeviceStatus::Pairing,
            DeviceStatus::Preparing,
            DeviceStatus::Error,
        ] {
            assert!(
                !sweepable(DevicePlatform::Android, &status),
                "{status:?} is not an idle phone"
            );
        }
    }

    #[test]
    fn the_kill_switch_takes_the_obvious_spellings() {
        // Guarded so the cases cannot interleave with another test reading the same var.
        for value in ["off", "OFF", "0", "false", "no", " off "] {
            std::env::set_var("RIVIU_IDLE_SWEEP", value);
            assert!(!IdleSweeper::enabled(), "{value:?} should switch it off");
        }
        for value in ["", "on", "1", "yes", "anything"] {
            std::env::set_var("RIVIU_IDLE_SWEEP", value);
            assert!(IdleSweeper::enabled(), "{value:?} should leave it on");
        }
        std::env::remove_var("RIVIU_IDLE_SWEEP");
        assert!(IdleSweeper::enabled(), "unset means on");
    }
}
