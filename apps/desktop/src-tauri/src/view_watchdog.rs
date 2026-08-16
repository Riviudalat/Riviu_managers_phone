//! One decision about whether an Android view is broken, and one gate in front of the cure.
//!
//! There used to be two watchdogs asking the same question from different evidence and
//! acting on it independently:
//!
//! * the Rust keeper in [`crate::state`], which measured **bytes arriving**
//!   ([`crate::view_hub::ViewHub::last_packet_age`]) and restarted the producer;
//! * a detector in `viewStore.ts`, which measured **frames actually painted** and, when it
//!   was allowed to act, called `view_ensure`.
//!
//! Neither evidence is sufficient alone, which is why both existed. Bytes cannot tell a dead
//! decoder from a phone whose screen is not changing — scrcpy encodes only on change, so a
//! static lock screen is legitimately silent, and AGENTS.md 9.64 §2 measured the byte rule
//! staying quiet for **8 minutes** while nothing painted. Paints cannot be trusted on their
//! own either: the reporter lives in a WebView that can be closed, throttled or reloaded,
//! and a rule that reads "no report" as "broken" would restart a fleet nobody is watching.
//!
//! What made two watchdogs actively harmful rather than merely redundant is that they had
//! two backoffs, two histories and no shared accounting, so the same phone could be
//! restarted by both. AGENTS.md 9.67 measured where that ends: 33 producer starts at two
//! phones, **291** at twenty, because each restart costs adb and CPU, which makes more
//! devices miss their paint window, which triggers more restarts. That is why the frontend
//! detector was left reporting-only (`AUTO_RESTART_ON_STALL = false`) and why the note says
//! it may act again **only** alongside a fleet-wide concurrency ceiling.
//!
//! So this module is that merge. The frontend keeps the one job it is uniquely able to do —
//! reporting what it painted — and stops being a recovery actor. The keeper takes both
//! kinds of evidence, reaches one verdict ([`view_verdict`]), and every restart in the app,
//! automatic or operator-initiated, goes through [`restart_android_view`], which cannot be
//! called without a permit from [`ViewRecoveryGate`]. The ceiling is not a flag anybody can
//! forget to check; it is an argument the function will not compile without.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use riviu_core::DeviceRegistry;
use serde::Deserialize;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::command_error::CommandError;

/// How long a running producer may publish **nothing** before it is called broken.
///
/// Raised from 5 s after measuring what 5 s actually costs. scrcpy encodes only when the
/// screen CHANGES, so a phone sitting on a static home screen publishes its first keyframe
/// and then legitimately nothing at all. At 5 s with no cooldown the keeper read that as a
/// fault and restarted forever: one run reached **generation 569 on a single Redmi** while
/// the phone was demonstrably healthy — screencap 2.7 MB, keyguard drawn, focus on the
/// launcher — against generation 12 on the other phone in the same run. Every restart also
/// took the exclusive start claim, which is what made the operator's overlay request come
/// back "already in flight" and left the overlay on the tile encode.
///
/// This rule can only ever prove "no samples arrived". It cannot tell a dead reader from a
/// screen that is not moving. [`VIEW_PAINT_STALL`] is the rule that can, so this stays the
/// coarse backstop it should always have been — and it is the **only** rule that still
/// works when no window is open to report paints at all.
pub(crate) const ANDROID_VIEW_SILENCE: Duration = Duration::from_secs(45);

/// How long packets may keep arriving while nothing is drawn before that is called broken.
///
/// The predicate matters more than the number, and the first version of this got it wrong:
/// it treated "no frames drawn" as the fault. A phone parked on a static screen paints
/// nothing for minutes and is perfectly healthy. **Arrivals climbing while paints stay
/// flat** is the condition that actually means broken, and it needs both counters.
///
/// 12 s rather than 6: at 24 fps a healthy stream paints within a frame or two of an
/// arrival, so anything this long is not merely a slow decoder.
pub(crate) const VIEW_PAINT_STALL: Duration = Duration::from_secs(12);

/// How old a paint report may be before it stops counting as evidence of anything.
///
/// The frontend reports on its own 2 s tick. Anything past this means the window is closed,
/// reloading, or throttled by the OS — and the correct reading of a missing reporter is
/// **"no paint evidence"**, never "stalled". Degrading to the byte rule is what keeps a
/// backgrounded WebView from restarting a fleet; treating silence from the reporter as a
/// fault would rebuild the 9.67 feedback loop with an extra hop in it.
///
/// Three ticks of headroom, so one skipped report — a GC pause, a slow frame — does not
/// flip a healthy fleet onto the coarse rule.
pub(crate) const VIEW_PAINT_REPORT_STALE: Duration = Duration::from_secs(6);

/// Minimum gap between two recovery attempts on the same device, doubling per consecutive
/// attempt up to [`VIEW_RESTART_MAX_BACKOFF`].
///
/// A restart costs roughly 45 s of real downtime on this fleet, so a flat cooldown shorter
/// than that guarantees the next attempt re-arms while the previous one is still running —
/// measured, and the result was a phone torn down about once a minute forever. One base
/// interval already clears a single restart; the second attempt (120 s) clears two.
pub(crate) const VIEW_RESTART_BACKOFF: Duration = Duration::from_secs(60);
/// Ceiling on the per-device backoff. Past this a phone is checked occasionally rather than
/// hammered: one that has not recovered in ten minutes needs an operator, not another
/// restart.
pub(crate) const VIEW_RESTART_MAX_BACKOFF: Duration = Duration::from_secs(600);

/// Floor between two attempts to retune a device to a different preset.
///
/// Shorter than the restart backoff on purpose: a retune follows an operator action — they
/// opened or closed an overlay — so it should feel immediate, and the first attempt is not
/// delayed at all. The floor exists only so a phone that cannot encode the larger preset is
/// not retried on every 2 s tick forever.
pub(crate) const VIEW_RETUNE_FLOOR: Duration = Duration::from_secs(15);

/// Frames a device must draw after a recovery before its backoff is considered cleared.
///
/// One frame is not recovery, and treating it as such defeated the backoff completely:
/// measured over three overlay open/close cycles, a stream painted a frame or two after
/// each restart and then stopped, which reset the counter, so every stall logged
/// "attempt 1" and the loop ran 33 times. 48 frames is ~2 s at 24 fps — long enough that a
/// stream which merely twitched does not count as healthy.
pub(crate) const SUSTAINED_PAINT_FRAMES: u64 = 48;

/// How many phones may be recovering at once, fleet-wide.
///
/// This is the precondition AGENTS.md 9.67 named and never had a number for. It was measured
/// rather than picked — `crates/android-driver/examples/view_concurrency_bench.rs`, twenty
/// Galaxy S8/S8+, and AGENTS.md 9.72 has the table — and **the measurement refused the
/// reason the ceiling was originally wanted**:
///
/// | at once | p50 to first keyframe | p90 | wall for 20 |
/// |---|---|---|---|
/// | 1 | 11.4 s | 12.9 s | 230.0 s |
/// | 4 | 11.4 s | 13.3 s | 59.3 s |
/// | 20 | 11.5 s | 13.3 s | 14.9 s |
///
/// Per-start latency is **flat** from one to twenty. One adb server takes twenty concurrent
/// scrcpy spawns without slowing a single one down, so the story that 9.67's 291 restarts
/// came from starts competing for adb is simply not true, and a ceiling sold as protecting
/// throughput would be protecting something that was never at risk. (The same run also
/// dates the folklore: a *clean* start reaches its first keyframe in ~11.5 s, not the ~44 s
/// of 9.64 — that figure is the restart path inside a loaded app, and quoting it for a cold
/// start overstates the cost of recovering by four times.)
///
/// What the loop actually is: a restart makes the device paint nothing for ~12 s, which is
/// [`VIEW_PAINT_STALL`], so the restart re-arms the very rule that ordered it. That is a
/// self-triggering loop, and it is killed by two other things in this module — evidence is
/// scoped to a generation and thrown away when the producer is replaced, and the per-device
/// backoff clears a whole restart before the next attempt.
///
/// So this ceiling is kept for the one property it does buy, stated honestly: **a bound on
/// how much of the fleet can go dark at once for a cure we are not certain of.** Four is a
/// fifth of this fleet, and it is also where the arithmetic already sits — twenty devices on
/// a 60 s backoff can sustain 20/60 s of attempts, which at ~11.5 s each is ~3.8 running
/// concurrently, so this binds during a fleet-wide burst and costs nothing in steady state.
///
/// That framing is also why [`start_android_view`] is outside it: a device with no producer
/// has nothing to lose, so rationing its start protects nothing and only makes the fleet
/// slower to come up.
pub(crate) const DEFAULT_VIEW_RECOVERY_CONCURRENCY: usize = 4;
/// Bounds on the env override. Above this there is no ceiling worth the name; below one
/// nothing could ever recover. The upper bound is deliberately well under a fleet: the
/// bench says a larger farm would still not slow down, but "all of them at once" is the
/// state this exists to prevent, whatever the throughput allows.
const MAX_VIEW_RECOVERY_CONCURRENCY: usize = 8;

/// How long an operator-initiated recovery waits for a permit before it is refused.
///
/// It has to be bounded rather than patient: `graceful_shutdown` blocks on
/// `wait_for_mutating_commands`, and this waits while holding a `CommandAdmission`, so an
/// unbounded wait turns quitting the app into a 44 s hang. A refusal an operator can retry
/// is better than a window that will not close.
const VIEW_ADMIT_WAIT: Duration = Duration::from_secs(5);

/// Keep the measured default while letting a differently-sized farm say otherwise. Invalid
/// values fail closed to the default rather than removing the ceiling by typo.
fn configured_view_recovery_concurrency() -> usize {
    match std::env::var("RIVIU_VIEW_RECOVERY_CONCURRENCY") {
        Ok(raw) => match raw.trim().parse::<usize>() {
            Ok(value) if (1..=MAX_VIEW_RECOVERY_CONCURRENCY).contains(&value) => value,
            _ => {
                log::warn!(
                    "invalid RIVIU_VIEW_RECOVERY_CONCURRENCY={raw:?}; \
                     using default {DEFAULT_VIEW_RECOVERY_CONCURRENCY}"
                );
                DEFAULT_VIEW_RECOVERY_CONCURRENCY
            }
        },
        Err(_) => DEFAULT_VIEW_RECOVERY_CONCURRENCY,
    }
}

/// Whether a device is due another attempt, given how many it has had and how long ago.
///
/// Doubling, capped. Kept as a free function so the schedule can be asserted directly
/// rather than inferred from timing.
pub(crate) fn view_restart_is_due(
    attempts: u32,
    since_last: Option<Duration>,
    base: Duration,
    max: Duration,
) -> bool {
    let Some(since) = since_last else {
        return true;
    };
    // `attempts - 1`, so the wait after the FIRST attempt is one base interval rather than
    // two. Saturating because attempts is 0 only on the never-restarted path above.
    let factor = 1u32 << attempts.saturating_sub(1).min(4);
    since >= base.saturating_mul(factor).min(max)
}

/// What the frontend saw, for one device, as of its last tick.
///
/// `generation` is what makes this evidence rather than a rumour: it says which producer
/// the counters belong to. A report from before a restart shows `received` far ahead of
/// `frames` forever, and acting on it the instant the restart completes is the exact shape
/// of the 9.67 feedback loop.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaintReport {
    pub udid: String,
    pub generation: u64,
    /// Envelopes the worker has taken in for this device.
    pub received: u64,
    /// Frames the worker has actually drawn for this device.
    pub frames: u64,
    /// Milliseconds since a frame was last drawn, as measured by the reporter's own clock.
    ///
    /// Sent as an age rather than a timestamp deliberately: the WebView's `Date.now()` and
    /// the host's `Instant` are different clocks, and comparing them across the IPC boundary
    /// is how a paused machine or a DST step becomes a fleet-wide restart.
    pub since_paint_ms: u64,
}

/// One device's paint evidence as the host holds it.
#[derive(Debug, Clone)]
pub(crate) struct PaintRecord {
    pub(crate) generation: u64,
    pub(crate) received: u64,
    pub(crate) frames: u64,
    pub(crate) since_paint: Duration,
    /// When the host recorded it — host clock, so staleness is measurable.
    pub(crate) reported_at: Instant,
}

/// The frontend's paint evidence, keyed by udid.
///
/// Written by the `view_report_paint` command, read by the keeper. Bounded by the fleet
/// size, and entries are dropped when a producer's generation advances rather than being
/// left to describe a producer that no longer exists.
#[derive(Default)]
pub struct ViewPaintLedger {
    inner: Mutex<HashMap<String, PaintRecord>>,
}

impl ViewPaintLedger {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Take one report, if it describes the producer that is currently running.
    ///
    /// A report for an older generation is dropped, and one for a newer generation clears
    /// what is held: either way the ledger never mixes counters from two producers, which
    /// is the invariant that keeps a completed restart from immediately re-triggering.
    pub(crate) fn record(&self, report: &PaintReport, current_generation: u64, now: Instant) {
        if report.generation != current_generation {
            self.inner.lock().remove(&report.udid);
            return;
        }
        self.inner.lock().insert(
            report.udid.clone(),
            PaintRecord {
                generation: report.generation,
                received: report.received,
                frames: report.frames,
                since_paint: Duration::from_millis(report.since_paint_ms),
                reported_at: now,
            },
        );
    }

    pub(crate) fn sample(&self, udid: &str) -> Option<PaintRecord> {
        self.inner.lock().get(udid).cloned()
    }

    /// How many devices have evidence fresh enough to decide anything with.
    ///
    /// Exists to be printed. The fine rule silently degrades to the coarse one when nobody
    /// is reporting — which is correct behaviour and *indistinguishable in a log* from the
    /// fine rule working, so without this the paint path could be dead for weeks and every
    /// number would still look healthy. That is the mistake AGENTS.md 9.66 cost three
    /// diagnosis rounds: a counter nobody prints is not evidence of health, it is not
    /// evidence of anything.
    pub(crate) fn fresh_count(&self, now: Instant) -> usize {
        self.inner
            .lock()
            .values()
            .filter(|record| now.duration_since(record.reported_at) < VIEW_PAINT_REPORT_STALE)
            .count()
    }

    /// Forget a device's evidence. Called when a producer is replaced, so the next verdict
    /// is reached on evidence from the producer that replaced it.
    pub(crate) fn clear(&self, udid: &str) {
        self.inner.lock().remove(udid);
    }
}

/// Why a device is being restarted. Carried into the log so the two rules stay
/// distinguishable in a transcript — the whole reason there were two watchdogs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ViewFault {
    /// No samples are arriving from the phone at all.
    Silent,
    /// Samples are arriving and the decoder is drawing nothing from them.
    PaintStalled,
}

impl ViewFault {
    pub(crate) fn reason(self) -> &'static str {
        match self {
            ViewFault::Silent => "published nothing",
            ViewFault::PaintStalled => "published packets that drew no frame",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ViewVerdict {
    /// A start still holds the claim. Not evidence of anything yet.
    Starting,
    Healthy,
    Restart(ViewFault),
}

/// The one decision, from both kinds of evidence.
///
/// Pure, and takes its inputs rather than reading any global, so the policy can be tested
/// without a phone, a socket or a timer. That matters more here than usual: the defect this
/// exists for was invisible precisely because nothing observable changed, and a rule that
/// can only be exercised by running the whole fleet is a rule that goes untested.
pub(crate) fn view_verdict(
    start_in_flight: bool,
    last_packet_age: Option<Duration>,
    paint: Option<&PaintRecord>,
    report_age: Option<Duration>,
) -> ViewVerdict {
    // A start that still holds the claim has not had the chance to publish anything.
    if start_in_flight {
        return ViewVerdict::Starting;
    }
    // The coarse backstop, and the only rule that survives having no window open. `None`
    // means no packet has ever been accepted for the current generation.
    if last_packet_age
        .map(|age| age >= ANDROID_VIEW_SILENCE)
        .unwrap_or(true)
    {
        return ViewVerdict::Restart(ViewFault::Silent);
    }
    let (Some(paint), Some(report_age)) = (paint, report_age) else {
        return ViewVerdict::Healthy;
    };
    // A reporter that has gone quiet is not evidence. Degrade to the byte rule above.
    if report_age >= VIEW_PAINT_REPORT_STALE {
        return ViewVerdict::Healthy;
    }
    // Never drawn is not stalled: the device is starting up, or already reported through
    // the byte rule. Treating "never" as "stalled" restarts every stream the moment it
    // appears, which is how the previous rule made the outage it was reporting.
    if paint.frames == 0 {
        return ViewVerdict::Healthy;
    }
    // The whole point: only a stream whose packets kept coming is broken. A static screen
    // stops producing packets too, and restarting it fixes nothing while costing ~45 s.
    if paint.since_paint > VIEW_PAINT_STALL && paint.received > paint.frames {
        return ViewVerdict::Restart(ViewFault::PaintStalled);
    }
    ViewVerdict::Healthy
}

/// When a device was last recovered and how many times in a row without painting since.
#[derive(Debug, Clone, Copy)]
struct RestartRecord {
    at: Instant,
    attempts: u32,
    /// `frames` at the moment the permit was taken, so "it recovered" can mean sustained
    /// painting rather than a single frame.
    frames_at_attempt: u64,
}

/// The fleet-wide ceiling on recoveries, plus the per-device backoff behind it.
///
/// Both live here because they answer the same question from different sides — *may this
/// device be restarted now* and *may anything be restarted now* — and splitting them is how
/// the app ended up with two of each.
pub struct ViewRecoveryGate {
    permits: Arc<Semaphore>,
    history: Mutex<HashMap<String, RestartRecord>>,
    limit: usize,
}

/// Proof that a recovery is admitted. [`restart_android_view`] takes one by value, so the
/// ceiling cannot be bypassed by forgetting to check it at a call site.
///
/// Held for the whole restart — not just the decision — because a producer that has spawned
/// but published nothing is still consuming the adb server this ceiling exists to protect.
pub struct ViewRecoveryPermit {
    _permit: OwnedSemaphorePermit,
    udid: String,
    gate: Arc<ViewRecoveryGate>,
}

impl Drop for ViewRecoveryPermit {
    fn drop(&mut self) {
        // Stamp the clock at RELEASE, not at admission. The backoff is meant to space out
        // attempts by the time between them, and a restart takes ~44 s; stamping at
        // admission would let the next attempt start 16 s after this one finished.
        let mut history = self.gate.history.lock();
        if let Some(record) = history.get_mut(&self.udid) {
            record.at = Instant::now();
        }
    }
}

impl ViewRecoveryGate {
    pub fn new() -> Arc<Self> {
        let limit = configured_view_recovery_concurrency();
        Arc::new(Self {
            permits: Arc::new(Semaphore::new(limit)),
            history: Mutex::new(HashMap::new()),
            limit,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_limit(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            permits: Arc::new(Semaphore::new(limit)),
            history: Mutex::new(HashMap::new()),
            limit,
        })
    }

    pub(crate) fn limit(&self) -> usize {
        self.limit
    }

    /// How many recoveries are running right now. For logging — a number that explains a
    /// deferral is the difference between a ceiling and a hang.
    pub(crate) fn in_flight(&self) -> usize {
        self.limit.saturating_sub(self.permits.available_permits())
    }

    /// The automatic lane: take a permit, or decline and let the next tick try.
    ///
    /// Backoff first, then capacity — a device that is not due yet must not consume a
    /// permit another device could have used. `None` is not an error; it means "not now",
    /// and the keeper simply moves on.
    pub(crate) fn try_admit(
        self: &Arc<Self>,
        udid: &str,
        frames_now: u64,
        base: Duration,
        max: Duration,
    ) -> Option<ViewRecoveryPermit> {
        {
            let history = self.history.lock();
            let record = history.get(udid).copied();
            if !view_restart_is_due(
                record.map(|r| r.attempts).unwrap_or(0),
                record.map(|r| r.at.elapsed()),
                base,
                max,
            ) {
                return None;
            }
        }
        // Capacity is checked AFTER the backoff so a device that is not due yet cannot
        // consume a slot another device could have used, and BEFORE the attempt is recorded
        // so a device turned away for want of a slot has not "tried" — being twentieth in
        // the queue must not push a phone further down it.
        let permit = Arc::clone(&self.permits).try_acquire_owned().ok()?;
        self.note_attempt(udid, frames_now);
        Some(ViewRecoveryPermit {
            _permit: permit,
            udid: udid.to_string(),
            gate: Arc::clone(self),
        })
    }

    /// The human lane: wait briefly, then refuse with a reason the operator can act on.
    ///
    /// No per-device backoff here. A person asking again is not a feedback loop — they can
    /// only click so fast, and refusing their second click because their first one was 40 s
    /// ago would make the app feel broken. The fleet ceiling still applies, because the adb
    /// server does not care who asked.
    pub(crate) async fn admit_operator(
        self: &Arc<Self>,
        udid: &str,
        frames_now: u64,
    ) -> Result<ViewRecoveryPermit, CommandError> {
        let permit =
            tokio::time::timeout(VIEW_ADMIT_WAIT, Arc::clone(&self.permits).acquire_owned())
                .await
                .map_err(|_| {
                    CommandError::operation(format!(
                        "đang khôi phục {} máy khác (trần {} máy một lúc) — thử lại sau vài giây",
                        self.in_flight(),
                        self.limit
                    ))
                })?
                .map_err(|_| CommandError::operation("hàng đợi khôi phục stream đã đóng"))?;
        self.note_attempt(udid, frames_now);
        Ok(ViewRecoveryPermit {
            _permit: permit,
            udid: udid.to_string(),
            gate: Arc::clone(self),
        })
    }

    fn note_attempt(&self, udid: &str, frames_now: u64) {
        let mut history = self.history.lock();
        let attempts = history.get(udid).map(|r| r.attempts).unwrap_or(0) + 1;
        history.insert(
            udid.to_string(),
            RestartRecord {
                at: Instant::now(),
                attempts,
                frames_at_attempt: frames_now,
            },
        );
    }

    /// Clear a device's backoff, but only on **sustained** painting.
    ///
    /// A restart that produced a couple of frames and stopped is the failure being retried,
    /// not a recovery from it — measured, and treating one frame as success is what made
    /// every stall log "attempt 1" while the loop ran 33 times.
    pub(crate) fn note_painted(&self, udid: &str, frames_now: u64) {
        let mut history = self.history.lock();
        let Some(record) = history.get(udid) else {
            return;
        };
        if frames_now.saturating_sub(record.frames_at_attempt) >= SUSTAINED_PAINT_FRAMES {
            history.remove(udid);
        }
    }

    /// Forget a device entirely — it left the fleet.
    pub(crate) fn forget(&self, udid: &str) {
        self.history.lock().remove(udid);
    }
}

/// Bring up a producer for a device that has none. **Takes no permit, deliberately.**
///
/// A first start is not a recovery, and the distinction is the whole reason the ceiling can
/// be as tight as it is. Nothing is being torn down here: the tile is already dark, so there
/// is no working picture to risk and no "cure we are not sure of" to bound. Rationing this
/// path buys nothing and costs a great deal — measured on this fleet, gating it made a cold
/// start of twenty phones take **55 s instead of the 15 s** the bench says twenty concurrent
/// spawns actually need (AGENTS.md 9.72).
///
/// The exclusive start claim (`view_start_in_flight`) still prevents two of these racing for
/// the same device; what is removed is only the fleet-wide count.
pub(crate) async fn start_android_view(
    android: &Arc<riviu_android_driver::AndroidDriver>,
    registry: &DeviceRegistry,
    udid: &str,
) -> Result<u64, String> {
    crate::state::set_stream_sampling(registry, udid);
    // The preset the operator asked for, not a hard-coded Tile: bringing an open overlay
    // back at the tile encode is how it used to quietly lose its resolution.
    let preset = android.desired_view_preset(udid);
    match android.start_view_stream(udid, preset).await {
        Ok(generation) => {
            crate::state::mark_android_view_live(registry, udid);
            Ok(generation)
        }
        Err(error) => {
            let message = format!("{error:#}");
            // Say it out loud, not only into the registry. This arm used to do nothing but
            // set `TileStreamState::Error`, and the tile showing an error is not something
            // anyone is watching at three in the morning: when turning on the scrcpy control
            // socket broke the handshake on all twenty phones, the app ran **six minutes with
            // zero producers and not one warning** (AGENTS.md 9.71). The failure was never
            // silent — nobody was printing it.
            log::warn!("android view for {udid} failed to start: {message}");
            crate::state::set_stream_error(registry, udid, message.clone());
            Err(message)
        }
    }
}

/// The single **restart** path: replace a producer that exists and is judged broken.
///
/// It takes the permit **by value** so a caller cannot reach it without having been
/// admitted, and holds it until the start returns: the ceiling has to bite across ticks,
/// not merely within one, because a restart outlives the tick that ordered it.
///
/// This is the path the ceiling exists for, and the difference from
/// [`start_android_view`] is exactly the thing being rationed — here a picture that may be
/// working is taken away first, on the strength of a verdict that can be wrong.
///
/// Stop-then-start rather than start-alone, and the preset is whatever the operator last
/// asked for rather than a hard-coded `Tile` — restarting an open overlay at the tile encode
/// is how it used to quietly lose its resolution a few seconds after opening
/// (AGENTS.md 9.64 §4).
pub(crate) async fn restart_android_view(
    android: &Arc<riviu_android_driver::AndroidDriver>,
    registry: &DeviceRegistry,
    paint: &Arc<ViewPaintLedger>,
    udid: &str,
    permit: ViewRecoveryPermit,
) -> Result<u64, String> {
    crate::state::set_stream_sampling(registry, udid);
    android.stop_view_stream(udid).await;
    // Evidence from the producer that is being replaced must not survive it.
    paint.clear(udid);
    let outcome = start_android_view(android, registry, udid).await;
    // Released here, after the start has returned — see the struct doc.
    drop(permit);
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(frames: u64, received: u64, since_paint: Duration) -> PaintRecord {
        PaintRecord {
            generation: 1,
            received,
            frames,
            since_paint,
            reported_at: Instant::now(),
        }
    }

    #[test]
    fn a_live_producer_is_silent_only_after_the_whole_silence_window() {
        // Written against the constant, not against a literal. The name used to say "five
        // seconds" and the body asserted 4 and 5, so raising the window meant editing a
        // test whose name then lied -- and the number had to be raised, because five
        // seconds is shorter than a static screen legitimately stays quiet.
        let window = ANDROID_VIEW_SILENCE;
        // A start still in flight is never silent, however long the clock says.
        assert_eq!(view_verdict(true, None, None, None), ViewVerdict::Starting);
        assert_eq!(
            view_verdict(true, Some(window * 10), None, None),
            ViewVerdict::Starting
        );
        // No packet ever seen, and nothing in flight, is silent immediately.
        assert_eq!(
            view_verdict(false, None, None, None),
            ViewVerdict::Restart(ViewFault::Silent)
        );
        assert_eq!(
            view_verdict(false, Some(window - Duration::from_secs(1)), None, None),
            ViewVerdict::Healthy
        );
        assert_eq!(
            view_verdict(false, Some(window), None, None),
            ViewVerdict::Restart(ViewFault::Silent)
        );
    }

    #[test]
    fn packets_arriving_while_nothing_paints_is_the_fault_bytes_cannot_see() {
        // The 8-minute blind spot of AGENTS.md 9.64 §2, now inside the same decision: bytes
        // are flowing, so the silence rule says healthy, and it is wrong.
        let byte_age = Some(Duration::from_secs(1));
        let stalled = record(200, 900, VIEW_PAINT_STALL + Duration::from_secs(1));
        assert_eq!(
            view_verdict(false, byte_age, Some(&stalled), Some(Duration::ZERO)),
            ViewVerdict::Restart(ViewFault::PaintStalled)
        );
    }

    #[test]
    fn a_static_screen_is_not_a_stall_however_long_it_sits() {
        // scrcpy encodes only when the screen changes, so a phone parked on a lock screen
        // paints nothing for minutes and is perfectly healthy. Restarting it costs ~45s and
        // fixes nothing -- measured at a restart every ~7s of uptime against ~45s of spawn,
        // leaving the stream up 14-18% of the time.
        let byte_age = Some(Duration::from_secs(1));
        // Arrivals have NOT climbed past what was painted: nothing new came in.
        let quiet = record(500, 500, Duration::from_secs(600));
        assert_eq!(
            view_verdict(false, byte_age, Some(&quiet), Some(Duration::ZERO)),
            ViewVerdict::Healthy
        );
    }

    #[test]
    fn a_device_that_has_never_painted_is_starting_up_not_stalled() {
        let byte_age = Some(Duration::from_secs(1));
        let never = record(0, 40, Duration::from_secs(120));
        assert_eq!(
            view_verdict(false, byte_age, Some(&never), Some(Duration::ZERO)),
            ViewVerdict::Healthy
        );
    }

    #[test]
    fn a_stale_report_degrades_to_the_byte_rule_instead_of_restarting_the_fleet() {
        // The reporter lives in a WebView that can be closed, reloaded or throttled by the
        // OS. "No report" must read as "no paint evidence", never as "broken" -- otherwise
        // minimising the window restarts twenty phones.
        let byte_age = Some(Duration::from_secs(1));
        let stalled = record(200, 900, VIEW_PAINT_STALL + Duration::from_secs(1));
        assert_eq!(
            view_verdict(
                false,
                byte_age,
                Some(&stalled),
                Some(VIEW_PAINT_REPORT_STALE)
            ),
            ViewVerdict::Healthy
        );
        // And the byte rule still fires underneath it, so a stale reporter loses the fine
        // rule without losing the coarse one.
        assert_eq!(
            view_verdict(
                false,
                Some(ANDROID_VIEW_SILENCE),
                Some(&stalled),
                Some(VIEW_PAINT_REPORT_STALE)
            ),
            ViewVerdict::Restart(ViewFault::Silent)
        );
    }

    #[test]
    fn evidence_from_a_replaced_producer_is_dropped_not_acted_on() {
        // The feedback shape of 9.67 with an extra hop: a report captured before a restart
        // shows received >> frames forever, so acting on it the instant the restart lands
        // restarts again, and again.
        let ledger = ViewPaintLedger::default();
        let report = PaintReport {
            udid: "a".into(),
            generation: 3,
            received: 900,
            frames: 200,
            since_paint_ms: 30_000,
        };
        // The hub has moved on to generation 4.
        ledger.record(&report, 4, Instant::now());
        assert!(ledger.sample("a").is_none());
        // Same generation: kept.
        ledger.record(&report, 3, Instant::now());
        assert!(ledger.sample("a").is_some());
        // And a bump clears what was held.
        ledger.record(&report, 5, Instant::now());
        assert!(ledger.sample("a").is_none());
    }

    #[test]
    fn the_gate_never_admits_more_than_the_ceiling_at_once() {
        // The 9.67 regression, pinned: twenty phones all faulting in the same tick must
        // produce at most `limit` restarts in flight, not twenty.
        let gate = ViewRecoveryGate::with_limit(3);
        let mut permits = Vec::new();
        for index in 0..20 {
            if let Some(permit) = gate.try_admit(
                &format!("serial-{index}"),
                0,
                VIEW_RESTART_BACKOFF,
                VIEW_RESTART_MAX_BACKOFF,
            ) {
                permits.push(permit);
            }
        }
        assert_eq!(permits.len(), 3, "the ceiling is a ceiling");
        assert_eq!(gate.in_flight(), 3);
        // Releasing one frees exactly one slot -- for a device that has not used its
        // backoff yet.
        permits.pop();
        assert_eq!(gate.in_flight(), 2);
        assert!(gate
            .try_admit(
                "serial-fresh",
                0,
                VIEW_RESTART_BACKOFF,
                VIEW_RESTART_MAX_BACKOFF
            )
            .is_some());
    }

    #[test]
    fn a_device_that_just_tried_does_not_get_a_second_permit() {
        // Without this, a phone that cannot recover would consume every permit on every
        // tick and starve the rest of the fleet -- the ceiling would bound the damage but
        // not distribute the chances.
        let gate = ViewRecoveryGate::with_limit(3);
        let first = gate.try_admit("a", 0, VIEW_RESTART_BACKOFF, VIEW_RESTART_MAX_BACKOFF);
        assert!(first.is_some());
        drop(first);
        assert!(gate
            .try_admit("a", 0, VIEW_RESTART_BACKOFF, VIEW_RESTART_MAX_BACKOFF)
            .is_none());
    }

    #[test]
    fn the_backoff_doubles_and_caps_and_a_first_attempt_never_waits() {
        // The measurement this encodes: with no cooldown and a 5s silence rule, one Redmi
        // reached generation 569 in a single run while the phone was healthy -- screencap
        // 2.7 MB, keyguard drawn, focus on the launcher -- because scrcpy encodes only when
        // the screen CHANGES and a static home screen publishes nothing at all.
        let base = VIEW_RESTART_BACKOFF;
        let max = VIEW_RESTART_MAX_BACKOFF;
        assert!(view_restart_is_due(0, None, base, max));
        // First failure waits one base interval, which already exceeds the ~45s a restart
        // itself costs -- the property that stops a restart from re-arming mid-restart.
        assert!(!view_restart_is_due(
            1,
            Some(Duration::from_secs(59)),
            base,
            max
        ));
        assert!(view_restart_is_due(
            1,
            Some(Duration::from_secs(60)),
            base,
            max
        ));
        assert!(!view_restart_is_due(
            2,
            Some(Duration::from_secs(119)),
            base,
            max
        ));
        assert!(view_restart_is_due(
            2,
            Some(Duration::from_secs(120)),
            base,
            max
        ));
        // Capped, and the shift cannot overflow however many attempts accumulate.
        assert!(view_restart_is_due(u32::MAX, Some(max), base, max));
        assert!(!view_restart_is_due(
            u32::MAX,
            Some(max - Duration::from_secs(1)),
            base,
            max
        ));
    }

    #[test]
    fn a_retune_is_immediate_the_first_time_and_floored_after_that() {
        // A retune follows an operator action -- they opened or closed an overlay -- so the
        // first attempt must not wait. The floor exists only so a phone that cannot encode
        // the larger preset is not retried on every 2s tick forever.
        let floor = VIEW_RETUNE_FLOOR;
        let max = VIEW_RESTART_MAX_BACKOFF;
        assert!(view_restart_is_due(0, None, floor, max));
        assert!(!view_restart_is_due(
            1,
            Some(floor - Duration::from_secs(1)),
            floor,
            max
        ));
        assert!(view_restart_is_due(1, Some(floor), floor, max));
        // Faster than the restart backoff, because a retune is a response to a person.
        assert!(VIEW_RETUNE_FLOOR < VIEW_RESTART_BACKOFF);
    }

    #[test]
    fn one_frame_is_not_a_recovery_but_sustained_painting_is() {
        let gate = ViewRecoveryGate::with_limit(2);
        let permit = gate
            .try_admit("a", 0, VIEW_RESTART_BACKOFF, VIEW_RESTART_MAX_BACKOFF)
            .expect("first attempt is always due");
        drop(permit);
        // A twitch: two frames since the attempt. The backoff must survive it, or every
        // stall logs "attempt 1" forever -- measured at 33 restarts over three cycles.
        gate.note_painted("a", 2);
        assert!(gate
            .try_admit("a", 2, VIEW_RESTART_BACKOFF, VIEW_RESTART_MAX_BACKOFF)
            .is_none());
        gate.note_painted("a", SUSTAINED_PAINT_FRAMES);
        assert!(gate
            .try_admit(
                "a",
                SUSTAINED_PAINT_FRAMES,
                VIEW_RESTART_BACKOFF,
                VIEW_RESTART_MAX_BACKOFF
            )
            .is_some());
    }

    #[test]
    fn the_windows_stay_separate_because_they_were_measured_separately() {
        // Collapsing 45s and 12s into one number would change the byte rule's sensitivity
        // as a side effect of tuning the paint rule. They are different questions.
        assert!(ANDROID_VIEW_SILENCE > VIEW_PAINT_STALL);
        // A restart costs ~45s on this fleet, so the backoff must clear a whole restart or
        // the next attempt re-arms mid-restart.
        assert!(VIEW_RESTART_BACKOFF >= ANDROID_VIEW_SILENCE);
        // A report has to be able to go missing for more than one tick without being read
        // as a fault, and must expire well inside the stall window it feeds.
        assert!(VIEW_PAINT_REPORT_STALE < VIEW_PAINT_STALL);
    }

    // Decided at compile time, the same way `view_hub.rs` walls its ring size: these are
    // relations between constants, so a violation should stop the build rather than wait for
    // somebody to run the suite. The env override is read once at construction, so what has
    // to hold here is that its bounds cannot be widened into "no ceiling" by a typo.
    const _: () = assert!(
        DEFAULT_VIEW_RECOVERY_CONCURRENCY >= 1,
        "a ceiling of zero means nothing ever recovers"
    );
    const _: () = assert!(
        DEFAULT_VIEW_RECOVERY_CONCURRENCY <= MAX_VIEW_RECOVERY_CONCURRENCY,
        "the default has to be reachable through the override's own bounds"
    );
    const _: () = assert!(
        MAX_VIEW_RECOVERY_CONCURRENCY < 20,
        "at or above the fleet size this stops being a ceiling: the state it exists to \
         prevent is every phone going dark at once for a cure we are not sure of"
    );
}
