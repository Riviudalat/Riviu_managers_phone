//! `DeviceDriver` for Android over adb plus a resident on-device agent.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context};
use async_trait::async_trait;
use parking_lot::Mutex;
use riviu_core::driver::{AppProcessState, DeviceDriver, ProcessAbsenceProof, UiSession};
use riviu_core::{ConnectionKind, DeviceInfo, DeviceStatus};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::adb::{self, AdbDeviceState, AdbProgram};
use crate::agent::AgentClient;
use crate::session::AndroidUiSession;

/// Package names of the agent halves, as published by Appium.
const AGENT_PACKAGE: &str = "io.appium.uiautomator2.server";
const AGENT_TEST_PACKAGE: &str = "io.appium.uiautomator2.server.test";
const AGENT_RUNNER: &str = "androidx.test.runner.AndroidJUnitRunner";
/// The agent's own listening port on the device. Fixed by the server.
const AGENT_DEVICE_PORT: u16 = 6790;
/// First host port we forward to. One per device, allocated on first use.
const HOST_PORT_BASE: u16 = 6790;
/// Separator between sections of the batched inventory shell call.
///
/// Letters and underscores only, deliberately. The device shell interprets
/// this string: an earlier `--8<--` had its `<` taken as an input redirection,
/// so no separator was ever printed and every field parsed into the first one.
const FIELD_SEPARATOR: &str = "RIVIU_NEXT_FIELD";

/// How long the interaction handoff waits for one decoded frame.
///
/// Same 12 s the iOS path allows. It is a *failure* deadline, not a fallback: see
/// [`StreamReadiness::DecodedFrame`].
const INTERACTION_FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(12);
/// How long to wait for the target app to actually reach the foreground.
///
/// **Measured, and the old 5 s was wrong by a factor of four.** A cold TikTok start on
/// the SM-N950F (Android 8) reached the foreground at 15,9 / 19,7 / 19,4 s across three
/// runs from `am force-stop` (12/08/2026), and 26,9 s on a fourth with coarser polling.
/// With 5 s the app refused every cold start — the operator saw
/// `did not reach the foreground … the phone is showing <unreadable>` for a phone that
/// was launching TikTok perfectly well, and the natural next move was to go looking for
/// a broken `dumpsys` command. It was not broken; the deadline was.
///
/// Still a deadline and not a fallback: a locked screen makes `monkey` report success
/// while nothing moves, and that has to end in a refusal rather than a session driving a
/// lock screen. 40 s is the slowest measured start plus room, on the oldest phone here.
const FOREGROUND_PROOF_TIMEOUT: Duration = Duration::from_secs(40);
const FOREGROUND_PROOF_POLL: Duration = Duration::from_millis(250);
/// How long a system dialog gets to go away after Back has been pressed at it.
///
/// Long enough for a dialog that Back *does* dismiss to be gone and the app to be back in
/// front, short enough that a dialog Back cannot dismiss — measured: Android permission
/// dialogs — does not eat the whole foreground deadline before anyone is told.
const DIALOG_GRACE: Duration = Duration::from_secs(5);

/// Is the package in front a **system dialog standing over the target app**, rather than a
/// different app the phone wandered off to?
///
/// The distinction decides whether waiting can possibly help. A launcher in front means the
/// launch did not take, and a retry or a longer deadline is the answer. One of these means
/// the launch *did* take and something is standing on top of it — it will still be standing
/// there when the deadline expires, so the whole window gets spent watching a screen that
/// was never going to move. Measured exactly once, and it cost a phone: a whole-fleet
/// nurture run on 18/08/2026 lost ce0717171c2a64d50d to
/// `com.google.android.packageinstaller/…GrantPermissionsActivity`.
///
/// Recovery is **Back**, never a tap, for the same reason `await_feed` presses Back at a
/// modal: the labelled button on a permission dialog *grants*, and granting a permission on
/// a real account is not a decision a recovery path gets to make.
///
/// Three names because the component moved between Android versions and this fleet spans
/// them: `com.android.packageinstaller` up to Android 9, `com.google.android.packageinstaller`
/// on Google builds, `com.android.permissioncontroller` from Android 10.
fn dialog_over_app(observed: &str) -> bool {
    if matches!(
        observed,
        "com.google.android.packageinstaller"
            | "com.android.packageinstaller"
            | "com.android.permissioncontroller"
    ) {
        return true;
    }
    // **The keyboard picker, which has no package and blocks the app completely.**
    //
    // Measured 25/08/2026: one phone failed every twenty-phone run with `did not reach the
    // foreground within 40s`, and `dumpsys window` showed `mCurrentFocus=Window{… Select
    // input method}` — Android's IME chooser, sitting over TikTok. Dismissing it by hand with
    // one Back let that phone join the very next run, and it came back during the run after,
    // so it is worth clearing automatically rather than once. Unlike a permission dialog this
    // one grants nothing, and Back is measured to close it.
    //
    // The caller reads the window's name out of `ForegroundWindow::System` rather than out of
    // an error message, so this compares against the name Android gives it.
    observed == "Select input method"
}
#[cfg(test)]
mod dialog_tests {
    use super::dialog_over_app;

    /// **The keyboard picker blocks the app and carries no package.**
    ///
    /// Measured 25/08/2026: one phone failed every twenty-phone run with `did not reach the
    /// foreground within 40s` while `dumpsys window` showed `Select input method` over
    /// TikTok. It reaches this predicate inside the *error text* of a foreground read, not as
    /// a package, so the equality match against installer packages could never see it.
    #[test]
    fn the_keyboard_picker_counts_as_a_dialog_over_the_app() {
        // The name as `ForegroundWindow::System` carries it — what the caller now passes.
        assert!(dialog_over_app("Select input method"));
        assert!(dialog_over_app("com.android.permissioncontroller"));
    }

    /// And an ordinary app in front is not a dialog — including one whose name merely
    /// mentions input.
    #[test]
    fn an_app_in_the_foreground_is_not_a_dialog() {
        assert!(!dialog_over_app("com.ss.android.ugc.trill"));
        assert!(!dialog_over_app("com.example.inputmethod.keyboard"));
    }
}

/// `pm install` of an 18 MB APK over USB, with room for a slow phone.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(180);
/// How long a killed minicap child gets to actually exit before we stop claiming
/// it is gone.
const CHILD_EXIT_TIMEOUT: Duration = Duration::from_secs(2);

/// How often, and how many times, a freshly started instrumentation is asked whether it has
/// bound its port yet.
const AGENT_READY_POLLS: u32 = 40;
const AGENT_READY_POLL_EVERY: Duration = Duration::from_millis(250);
/// Longest [`AndroidDriver::instrument_and_wait`] will wait for the port. Derived so the
/// message the operator reads cannot drift away from the loop that produced it.
const AGENT_READY_WAIT: Duration =
    Duration::from_millis(AGENT_READY_POLL_EVERY.as_millis() as u64 * AGENT_READY_POLLS as u64);

/// What one trip through the blind-agent path costs before the caller learns anything.
///
/// Two element queries that will not answer — one on the session that turned out blind, one
/// on its replacement — and [`AGENT_READY_WAIT`] between them waiting for the restarted
/// server to bind. The queries dominate, and neither is ours to shorten: see
/// [`AgentClient::BLIND_QUERY_COST`].
const INSTRUMENTATION_ATTEMPT_COST: Duration =
    Duration::from_secs(AgentClient::BLIND_QUERY_COST.as_secs() * 2 + AGENT_READY_WAIT.as_secs());

/// Quiet window after an instrumentation restart that did not fix the device.
///
/// **Derived, not chosen**: two attempts' worth. A cooldown shorter than one attempt would
/// let the next restart begin while the last is still polling for a port, which is the storm
/// this exists to stop rather than the cure for it.
///
/// Erring long is nearly free, and that is worth saying because it looks careless. This is
/// only ever consulted when a device is blind *again* after a restart — a device the restart
/// fixed never reaches the check, because its liveness query answers and the recovery path
/// is never entered. So the window can only delay a second restart for a device whose cause
/// is off-host (another tool holding `UiAutomation`), and that cause does not clear on any
/// schedule of ours.
///
/// The shape matters more than the number: every recovery action needs a window, and Riviu
/// had one only for view producers (`VIEW_RESTART_BACKOFF`, in the desktop's watchdog).
/// docs/re/genfarmer README §12.2 records the same pattern applied to every recovery
/// GenFarmer has — 30 s to recreate an adb client, 45 s between server kills, ten minutes
/// after five failed reconnects.
const INSTRUMENTATION_RESTART_COOLDOWN: Duration =
    Duration::from_secs(INSTRUMENTATION_ATTEMPT_COST.as_secs() * 2);

/// How honest a stream start has to be about frames.
///
/// The split exists because minicap only publishes when the display changes
/// (`crate::frames`), so "no frame" means different things on the two paths that
/// start it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamReadiness {
    /// The operator's tile. A phone sitting on a static screen legitimately
    /// produces nothing, and the desktop sampler has its own freshness watchdog
    /// and `Stale` label for that. Demanding a frame here would report a working
    /// phone as broken.
    BestEffort,
    /// The interaction handoff. One JPEG that *decodes* must be accepted by the
    /// sink at the expected generation, or the start fails — the caller has just
    /// foregrounded a playing TikTok feed, so silence means something is wrong.
    DecodedFrame,
}

/// Exclusive right to start a producer for one serial.
///
/// Released in `Drop`, which is the point: a start future cancelled mid-flight —
/// nurture's stop flag aborting the task — must not strand the serial as
/// permanently "starting". That is also why the claim set is a sync mutex.
struct StartClaim<'a> {
    starting: &'a Mutex<HashSet<String>>,
    serial: String,
}

impl Drop for StartClaim<'_> {
    fn drop(&mut self) {
        self.starting.lock().remove(&self.serial);
    }
}

#[derive(Debug, Clone, Default)]
pub struct AndroidDriverConfig {
    /// Explicit path to `adb`. Falls back to `RIVIU_ADB_PATH`,
    /// `ANDROID_SDK_ROOT`/`ANDROID_HOME`, `PATH`, then [`Self::bundled_adb_path`].
    pub adb_path: Option<PathBuf>,
    /// DeviceFarmer's `noarch/minicap.apk`, the JPEG frame source. Falls back to
    /// `RIVIU_MINICAP_APK`, then [`Self::bundled_minicap_apk`].
    pub minicap_apk: Option<PathBuf>,
    /// The `adb` shipped inside our own installer, verified against
    /// `android-tools-manifest.json` before it gets here.
    ///
    /// **Lowest priority of all**, and a separate field for exactly that reason:
    /// putting the bundled path into [`Self::adb_path`] would make it outrank
    /// `RIVIU_ADB_PATH` and every SDK variable, because those are only consulted when
    /// the configured field is empty. A field the operator cannot outrank is not a
    /// safety net, it is a hijack.
    pub bundled_adb_path: Option<PathBuf>,
    /// The `minicap.apk` shipped inside our own installer, same precedence rule.
    ///
    /// Also a separate field, and for a sharper version of the same reason: setting
    /// `minicap_apk` directly would break **both** overrides at once, since the
    /// configured value is preferred over `RIVIU_MINICAP_APK`. See AGENTS.md 9.27 —
    /// keeping that env override working is part of the decision to bundle at all.
    pub bundled_minicap_apk: Option<PathBuf>,
    /// Explicit path to the scrcpy 3.3.4 server JAR. Falls back to
    /// `RIVIU_SCRCPY_SERVER`, then [`Self::bundled_scrcpy_server`].
    pub scrcpy_server: Option<PathBuf>,
    /// The `scrcpy-server` shipped inside the installer. Lowest priority, same
    /// reason as [`Self::bundled_minicap_apk`].
    pub bundled_scrcpy_server: Option<PathBuf>,
    /// Explicit path to the Riviu helper APK (`com.riviu.agent`). Falls back to
    /// `RIVIU_ANDROID_AGENT_APK`, then [`Self::bundled_riviu_agent_apk`].
    pub riviu_agent_apk: Option<PathBuf>,
    /// The helper APK shipped inside the installer. Lowest priority, same
    /// reason as [`Self::bundled_minicap_apk`].
    pub bundled_riviu_agent_apk: Option<PathBuf>,
    /// Explicit path to `appium-uiautomator2-server.apk`. Falls back to
    /// `RIVIU_AGENT_SERVER_APK`, then [`Self::bundled_agent_server_apk`].
    pub agent_server_apk: Option<PathBuf>,
    /// The server APK shipped inside the installer. Lowest priority, same reason as
    /// [`Self::bundled_minicap_apk`].
    pub bundled_agent_server_apk: Option<PathBuf>,
    /// Explicit path to the `androidTest` half. Falls back to `RIVIU_AGENT_TEST_APK`,
    /// then [`Self::bundled_agent_test_apk`].
    ///
    /// Both halves or neither: the runner lives in the test APK and `am instrument` names
    /// it, so a device with only the server installed refuses exactly as if it had nothing.
    pub agent_test_apk: Option<PathBuf>,
    /// The test APK shipped inside the installer. Lowest priority.
    pub bundled_agent_test_apk: Option<PathBuf>,
}

/// One running minicap feed, owned so a second `ensure_stream` reuses it and a
/// stop can prove the child is gone.
struct StreamProducer {
    /// The hub generation this producer publishes into. A producer whose
    /// generation is stale must die rather than publish (`FrameSink`).
    generation: u64,
    host_port: u16,
    child: tokio::process::Child,
    reader: tokio::task::JoinHandle<()>,
    /// minicap's own pid on the device, from its banner.
    ///
    /// Recorded for diagnosis, not used to kill: killing by pid needs a
    /// `/proc/<pid>/cmdline` guard against pid reuse and costs an adb round trip on
    /// every teardown, and the failure it would defend against — a stranded
    /// device-side minicap — has not been observed. A stranded one would hold the
    /// per-serial abstract socket and the next start would adopt it, still
    /// producing real frames of the real screen.
    device_pid: u32,
}

/// One running scrcpy view. Separate from [`StreamProducer`]: a phone can keep
/// this H.264 encode while nurture owns a minicap JPEG producer.
/// The operator's quality choices, one per preset, plus the frame rate they share.
#[derive(Debug, Clone)]
struct ViewTuningChoice {
    /// What a grid tile encodes at.
    grid: riviu_core::StreamQuality,
    /// What the overlay encodes at. Higher by default: it is one phone at a time, which is
    /// exactly what makes the larger encode affordable.
    focus: riviu_core::StreamQuality,
    /// Shared, because it is a property of what the fleet can deliver rather than of how big
    /// the picture is.
    fps: u32,
}

/// Width and height in one atomic, so a touch can never be built from one frame's width and
/// the next frame's height.
///
/// Samples carry these as `u32` and the control message declares them as `u16`. Saturating
/// rather than truncating: a `as u16` on a hypothetical 70000-pixel frame would wrap to a
/// small number that looks perfectly plausible on the wire, and the touch would land in the
/// wrong place instead of being refused. Nothing on this fleet comes near it — `MAX_LONG_EDGE`
/// is 832 — which is exactly why a silent wrap would never be found.
fn pack_frame_size(width: u32, height: u32) -> u32 {
    let clamp = |value: u32| u16::try_from(value).unwrap_or(u16::MAX);
    (u32::from(clamp(width)) << 16) | u32::from(clamp(height))
}

fn unpack_frame_size(packed: u32) -> (u16, u16) {
    ((packed >> 16) as u16, (packed & 0xffff) as u16)
}

/// Whether this spawn is taking over from a producer that is still painting.
///
/// **The picture must not go away while the operator is looking at it.** Opening the overlay
/// switches preset, which means a new encode, and the old shape of this — stop, advance the
/// generation, then spawn — left the canvas frozen on its last tile frame for the whole
/// spawn. Measured on this fleet: **1.7 s** of a stuck picture every time a phone is opened,
/// which is what "vẫn có delay" was.
///
/// `Replace` keeps the live producer running through the spawn and only swaps once the new
/// stream has a keyframe in hand, so nothing on screen ever stops moving. Two scrcpy servers
/// briefly share the device, which AGENTS.md 9.50 warned about — but that warning was about
/// GenFarmer's 2.4 server, and this was measured rather than assumed: on a Galaxy S8+
/// (Exynos, the fleet's fussiest encoder) a second 3.3.4 server connected alongside a live
/// one returned its config packet and a real IDR **284 ms** after connect.
///
/// A failed `Replace` is also strictly safer than the old order: the operator keeps the
/// stream they had instead of being left with a dead device.
enum ViewStart {
    /// Nothing is streaming this serial. The generation has already been advanced.
    Fresh { generation: u64 },
    /// A producer is live. Hold it until the replacement is proven, then stop it.
    Replace,
}

struct ViewProducer {
    generation: u64,
    preset: crate::scrcpy::ViewPreset,
    host_port: u16,
    child: tokio::process::Child,
    reader: tokio::task::JoinHandle<()>,
    /// Width in the high 16 bits, height in the low 16, as of the last sample read.
    ///
    /// This exists so that a touch can declare the size the server is *currently* encoding
    /// rather than the size some caller last saw. `Device.getPhysicalPoint` on the device
    /// compares the two and silently ignores the event when they differ, so a value that
    /// lags a preset change or a rotation is not a slightly-off tap — it is no tap at all.
    ///
    /// Packed into one atomic rather than kept behind the `views` mutex because the reader
    /// task writes it on every frame and `inject_touch` reads it on every pointer sample;
    /// neither should ever wait on the other, and the pair must move together or a touch
    /// could be built from one frame's width and the next frame's height.
    frame_size: Arc<AtomicU32>,
    /// The write half of the scrcpy control socket.
    ///
    /// Behind its own async mutex, and that is a correctness requirement rather than a
    /// style: `ControlMessageReader` on the device has **no framing**, so two interleaved
    /// writes desynchronise it permanently — and a desynchronised control stream is not a
    /// dropped message, it is `ControlProtocolException` -> `Looper.quitSafely()`, which
    /// kills the video too. One message, one `write_all`, one lock.
    ///
    /// Separate from `views` so the lock is never held across the send: `views` is taken by
    /// every keeper tick and holding it through a socket write would make a slow phone stall
    /// the reconciliation of every other one.
    control: Arc<tokio::sync::Mutex<tokio::net::tcp::OwnedWriteHalf>>,
    /// Reads the device→host half and throws it away.
    ///
    /// With `clipboard_autosync` left at its default the phone sends a message every time
    /// its clipboard changes. Measured: holding that socket unread for 75 s while the
    /// clipboard changed twelve times did not disturb the server — `DeviceMessageSender`
    /// offers onto a bounded queue and drops rather than blocking. So this is insurance, not
    /// a load-bearing part: it keeps the socket honest over hours rather than minutes, and it
    /// costs one idle task per phone.
    ///
    /// Tolerant by construction — it never parses, so it cannot object to a message type it
    /// does not know, and objecting is the one thing that would be fatal.
    control_drain: tokio::task::JoinHandle<()>,
}

/// What one phone's apps are called and what they look like, plus the package set it was read
/// for. See [`AndroidDriver::app_descriptions`] for the measurements that make it worth
/// keeping.
struct AppDescriptionCache {
    fingerprint: u64,
    rows: Vec<crate::riviu_agent::HelperApp>,
}

/// A hash of *which* packages a description covers.
///
/// Sorted first, so the same set in a different order is the same fingerprint: adb's listing
/// order is not stable across calls and an order-sensitive key would miss the cache every
/// time. Installing or removing an app changes the set and therefore the key, which is the
/// only event that should invalidate the names.
fn package_set_fingerprint(packages: &[String]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut sorted: Vec<&str> = packages.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    sorted.hash(&mut hasher);
    hasher.finish()
}

/// Copy labels and icons onto the rows adb produced, joining on the package name.
///
/// An empty label is dropped rather than written: a row labelled with an empty string reads
/// as an app called nothing, where `None` reads as "the package name is all we have".
fn apply_app_descriptions(
    apps: &mut [riviu_core::InstalledApp],
    described: &[crate::riviu_agent::HelperApp],
) {
    let by_package: HashMap<&str, &crate::riviu_agent::HelperApp> = described
        .iter()
        .map(|row| (row.package.as_str(), row))
        .collect();
    for app in apps.iter_mut() {
        let Some(row) = by_package.get(app.bundle_id.as_str()) else {
            continue;
        };
        if !row.label.trim().is_empty() {
            app.label = Some(row.label.clone());
        }
        if let Some(icon) = &row.icon_png_base64 {
            app.icon_png_base64 = Some(icon.clone());
        }
    }
}

pub struct AndroidDriver {
    adb: AdbProgram,
    minicap_apk: Option<PathBuf>,
    scrcpy_server: Option<PathBuf>,
    /// Where frames go. Injected by the composition root, which owns the hub —
    /// this crate must not depend on whichever crate that is.
    frame_sink: Mutex<Option<Arc<dyn riviu_core::FrameSink>>>,
    /// Where H.264 view samples go. Missing is a configuration error for the
    /// view path, not a reason to refuse minicap.
    view_sink: Mutex<Option<Arc<dyn crate::view::ViewSink>>>,
    /// serial -> the minicap feed we started for it.
    ///
    /// Held **only** across map access — `get` / `insert` / `remove` — never across
    /// the adb work. It used to wrap the whole start, including an `ensure_apk`
    /// push with a 120 s timeout, so one phone opening a stream blocked every other
    /// phone's start *and* stop. [`AndroidDriver::starting`] is what makes the
    /// short critical section safe.
    streams: tokio::sync::Mutex<HashMap<String, StreamProducer>>,
    /// serial -> the scrcpy view we started for it. Held only across map
    /// access, same rule as [`Self::streams`].
    views: tokio::sync::Mutex<HashMap<String, ViewProducer>>,
    /// Preset each serial was last *asked* for, which is not the same as the one running.
    ///
    /// Separate from `views` on purpose: the watchdog restarts a dead producer, and at that
    /// moment there is no producer left to read the preset off. It used to hard-code
    /// `Tile`, so an overlay the operator had open silently dropped back to the tile encode
    /// a few seconds later and the picture went soft again with nothing to point at.
    desired_presets: parking_lot::Mutex<HashMap<String, crate::scrcpy::ViewPreset>>,
    /// Serials with a view start in flight.
    view_starting: Mutex<HashSet<String>>,
    /// The operator's quality and frame-rate choice for the tile grid.
    ///
    /// Held here rather than threaded through `start_view_stream` because every caller
    /// wants the current value and none of them wants to know it: the watchdog, the
    /// tile Start button and the overlay all start views, and passing tuning through
    /// each would make three places able to disagree about it. Set from the app when
    /// stream settings are saved; read when a producer is spawned, so a change reaches
    /// a tile on its next restart and never mid-stream.
    /// Per preset, because the two are displayed at very different sizes and the operator
    /// gets a control for each. One shared pair meant the overlay silently encoded at the
    /// grid's quality and `focus_quality` had no reader at all — a settings row that stored
    /// a value and changed nothing.
    view_tuning: Mutex<ViewTuningChoice>,
    /// Serials with a producer start in flight.
    ///
    /// The atomic claim that replaces holding [`Self::streams`] across the slow
    /// work. A second start for the same serial is **refused**, not queued: a
    /// hidden queue is the fleet-wide serialisation this removes.
    starting: Mutex<HashSet<String>>,
    /// Driver-side proof that stopped → session → stream is being followed.
    ///
    /// Catches a generation drift at the step that broke, so the operator sees
    /// which one rather than core's opaque `StopProofMismatch`.
    interaction: riviu_core::InteractionLifecycleRegistry,
    /// serial -> the agent session we already opened, reused rather than remade.
    ///
    /// **This cache is a correctness fix, not an optimisation.** `AgentClient::connect`
    /// POSTs `/session`, and opening one per `open_session` degrades the on-device
    /// server: measured on a Redmi Note 12 (11/08/2026), after about ten accumulated
    /// sessions every element query rose from ~150 ms to the server's hardcoded
    /// root-`AccessibilityNodeInfo` timeout — 10 000+ ms and then `absent`, which
    /// reads exactly like a wrong locator. Force-stopping the instrumentation
    /// restored 118–425 ms immediately. See [`AgentClient::close`].
    agents: Mutex<HashMap<String, AgentClient>>,
    /// serial -> when this device's instrumentation was last restarted for blindness.
    ///
    /// The restart itself is already bounded *within* one call: it happens once, and a
    /// second blind session is reported rather than retried. What was missing is the bound
    /// **across** calls. When something else on the phone holds `UiAutomation` and does not
    /// give it back, every gesture the operator makes walks the whole recovery again —
    /// open a session, wait out the 5 s liveness proof, restart the instrumentation, poll
    /// up to 10 s for the port, prove the new session, fail. Tapping three times buys three
    /// restarts and a minute of nothing.
    ///
    /// So a restart that did not fix the device buys a quiet window before the next one is
    /// allowed. Inside it the caller fails immediately and says why, which is both faster
    /// and more use than grinding: the fix is on the phone, not in another restart.
    ///
    /// This is the "windowed cooldown on recovery actions" half of docs/re/genfarmer
    /// §12.6 — the half Riviu had for view producers (`VIEW_RESTART_BACKOFF`) and nowhere
    /// else.
    instrumentation_restarts: Mutex<HashMap<String, std::time::Instant>>,
    /// serial -> the resolved TikTok package, memoised.
    ///
    /// `pm list packages` is a 1–2 s adb round trip per candidate and this sits on the
    /// path to every session. Invalidated by `refresh_device`, because a build can be
    /// installed or removed while the app is running.
    tiktok_packages: Mutex<HashMap<String, String>>,
    /// serial -> the phone's current screen size, shared with every session for it.
    ///
    /// One handle per serial rather than a copy per session, for the same reason `agents`
    /// is keyed this way: invalidating has to reach the sessions already handed out.
    /// `session.rs` used to hold this as a plain tuple captured at open, so a rotation made
    /// every later coordinate wrong and nothing said so.
    screens: Mutex<HashMap<String, crate::session::ScreenCache>>,
    /// serial -> forwarded host port.
    ports: Mutex<HashMap<String, u16>>,
    /// Serials for which *we* established the `adb forward`.
    ///
    /// Readiness may only be probed for these. A host port that we allocated
    /// but never forwarded is not silent — it may already be forwarded to some
    /// other device, in which case probing it reports that device's agent as
    /// this one's. Measured: a Xiaomi with no agent installed came back
    /// `agent=true` because it drew a port an S8+ was using.
    forwarded: Mutex<HashSet<String>>,
    /// Helper APK used for clipboard / MediaStore. Missing is normal: a source
    /// checkout has no pinned binary until someone builds
    /// `sidecars/riviu-android-agent`.
    riviu_agent_apk: Option<PathBuf>,
    /// Both halves of the uiautomator2 instrumentation, resolved once at construction.
    agent_apks: Option<(PathBuf, PathBuf)>,
    /// serial -> a live helper client. Same reuse rule as [`Self::agents`]:
    /// opening a second forward per session leaks a host port.
    helpers: Mutex<HashMap<String, crate::riviu_agent::HelperClient>>,
    /// serial -> app names and icons the helper already described, keyed on the exact set of
    /// packages they were read for.
    ///
    /// **Measured on 23021RAAEG, 21/08/2026, and the numbers are why this cache exists:**
    /// labels for all 539 packages cost 4 559 ms and 47 KB; labels plus 48 px icons for the
    /// 162 user-partition packages cost 3 599 ms and 535 KB. That is per `PackageManager`
    /// call on the device, roughly 8 ms an app, and it is not something a faster wire makes
    /// better. A menu that spends four seconds every time it opens is a menu nobody opens
    /// twice, so the answer is kept until the package set itself changes — installing or
    /// removing an app is exactly when it must be re-read, and nothing else is.
    app_descriptions: Mutex<HashMap<String, AppDescriptionCache>>,
    /// serial -> the last thing we proved about its agent.
    ///
    /// `DeviceDriver::cached_agent_status` is synchronous and Flow's preflight reads it, so
    /// what the async paths learn has to be left somewhere a non-async reader can find it.
    /// The iOS driver keeps the same map for the same reason.
    agent_statuses: Mutex<HashMap<String, riviu_core::AgentStatus>>,
}

mod agent;
mod device_ops;
mod stream;

impl AndroidDriver {
    pub fn new(config: &AndroidDriverConfig) -> anyhow::Result<Self> {
        let adb = AdbProgram::resolve(
            config.adb_path.as_deref(),
            config.bundled_adb_path.as_deref(),
        )?;
        Ok(Self::with_adb(adb, config))
    }

    /// Build around an `adb` that has already been chosen.
    ///
    /// Split out for [`detect_driver`], which proves a specific candidate answers
    /// before committing to it. Without this the probe's choice would be thrown away
    /// and re-derived by `resolve`, which picks the first candidate that merely
    /// *exists* — so a broken adb earlier in the order would win over the working one
    /// the probe just found.
    fn with_adb(adb: AdbProgram, config: &AndroidDriverConfig) -> Self {
        // config -> env -> bundled. The bundled path is last so neither override is
        // taken away by the act of shipping a copy; see `AndroidDriverConfig`.
        let minicap_apk = config
            .minicap_apk
            .clone()
            .or_else(|| {
                std::env::var("RIVIU_MINICAP_APK")
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from)
            })
            .or_else(|| config.bundled_minicap_apk.clone());
        let scrcpy_server = config
            .scrcpy_server
            .clone()
            .or_else(|| {
                std::env::var("RIVIU_SCRCPY_SERVER")
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from)
            })
            .or_else(|| config.bundled_scrcpy_server.clone());
        let riviu_agent_apk = crate::riviu_agent::resolve_apk_path(
            config.riviu_agent_apk.clone(),
            std::env::var("RIVIU_ANDROID_AGENT_APK").ok(),
            config.bundled_riviu_agent_apk.clone(),
        );
        // Both halves or neither. Half an instrumentation installs cleanly and then fails
        // at `am instrument` with the same "not installed" refusal, which sends whoever
        // debugs it looking at the wrong half.
        let agent_apks = crate::riviu_agent::resolve_apk_path(
            config.agent_server_apk.clone(),
            std::env::var("RIVIU_AGENT_SERVER_APK").ok(),
            config.bundled_agent_server_apk.clone(),
        )
        .zip(crate::riviu_agent::resolve_apk_path(
            config.agent_test_apk.clone(),
            std::env::var("RIVIU_AGENT_TEST_APK").ok(),
            config.bundled_agent_test_apk.clone(),
        ));
        Self {
            adb,
            minicap_apk,
            scrcpy_server,
            agent_apks,
            frame_sink: Mutex::new(None),
            view_sink: Mutex::new(None),
            streams: tokio::sync::Mutex::new(HashMap::new()),
            views: tokio::sync::Mutex::new(HashMap::new()),
            desired_presets: parking_lot::Mutex::new(HashMap::new()),
            view_starting: Mutex::new(HashSet::new()),
            // Medium reproduces the bitrate and size that shipped. The frame rate does
            // change: the launch used to ask for a hardcoded 30 while
            // `get_stream_settings` told the operator 24, so the UI and the encoder
            // disagreed silently. The default is now the declared rate, and the two agree.
            view_tuning: Mutex::new(ViewTuningChoice {
                grid: riviu_core::StreamQuality::Medium,
                focus: riviu_core::StreamQuality::High,
                fps: riviu_core::STREAM_FPS,
            }),
            starting: Mutex::new(HashSet::new()),
            interaction: riviu_core::InteractionLifecycleRegistry::default(),
            agents: Mutex::new(HashMap::new()),
            instrumentation_restarts: Mutex::new(HashMap::new()),
            tiktok_packages: Mutex::new(HashMap::new()),
            screens: Mutex::new(HashMap::new()),
            ports: Mutex::new(HashMap::new()),
            forwarded: Mutex::new(HashSet::new()),
            riviu_agent_apk,
            helpers: Mutex::new(HashMap::new()),
            app_descriptions: Mutex::new(HashMap::new()),
            agent_statuses: Mutex::new(HashMap::new()),
        }
    }

    /// Point this backend at the hub that orders and fans out frames.
    ///
    /// Without it `ensure_stream` refuses rather than inventing a stream nobody
    /// can read.
    pub fn set_frame_sink(&self, sink: Arc<dyn riviu_core::FrameSink>) {
        *self.frame_sink.lock() = Some(sink);
    }

    /// Point the H.264 view path at the desktop hub. Separate from
    /// [`Self::set_frame_sink`]: a missing view sink must not refuse minicap.
    pub fn set_view_sink(&self, sink: Arc<dyn crate::view::ViewSink>) {
        *self.view_sink.lock() = Some(sink);
    }

    /// The hub, or a refusal that names the fix.
    ///
    /// Never falls back to an internal counter: that would be a second source of
    /// truth for evidence ordering, and `crate::frames` forbids it. In the desktop
    /// app this cannot fail — the composition root wires the hub before the driver
    /// joins the fleet — so a failure here means a harness forgot to.
    fn sink(&self) -> anyhow::Result<Arc<dyn riviu_core::FrameSink>> {
        self.frame_sink.lock().clone().ok_or_else(|| {
            anyhow!(
                "no frame sink is wired to the Android driver; call set_frame_sink before \
                 starting a stream"
            )
        })
    }

    /// Open a session as the concrete type.
    ///
    /// `start_ui_session` boxes this. Callers that need the Android-specific
    /// surface — locator queries, element bounds — take this instead of
    /// downcasting a trait object.
    /// Drop what we believe about a serial's screen size.
    ///
    /// Cheap enough to call speculatively — the next gesture pays one agent read, and only
    /// if it actually needs a size.
    pub(crate) fn invalidate_screen(&self, serial: &str) {
        if let Some(cache) = self.screens.lock().get(serial) {
            cache.invalidate();
        }
    }
}

/// Whatever the scrcpy server has said for itself, on **any** failure path.
///
/// 3.3.4 prints the real reason for a refusal on stderr (`'=' expected` on a bad codec
/// option) and then exits; without this the tile only said "exited before it accepted".
///
/// It now reads stdout too, and is called from every failure arm rather than only from the
/// already-exited one. Both changes are the same lesson: `Ln.i` goes to FD 1, so the
/// server's account of what it chose — the encoder it picked, `Device: [...]`, and
/// `Video capture reset` — was being written into `Stdio::null()`. A handshake that hangs
/// rather than exits produced no host-side evidence at all, and the one measured instance of
/// that ran **six minutes with zero warnings** (AGENTS.md 9.71).
///
/// Non-blocking by construction: a short timeout on each pipe, because the server may still
/// be alive and holding them open. Nothing here may wait on a process that is not going to
/// exit.
async fn scrcpy_exit_detail(child: &mut tokio::process::Child) -> String {
    use tokio::io::AsyncReadExt;
    async fn drain(pipe: Option<impl tokio::io::AsyncRead + Unpin>, tag: &str) -> Vec<String> {
        let Some(mut pipe) = pipe else {
            return Vec::new();
        };
        let mut buf = Vec::new();
        let _ = tokio::time::timeout(Duration::from_millis(200), pipe.read_to_end(&mut buf)).await;
        String::from_utf8_lossy(&buf)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .take(6)
            .map(|line| format!("{tag} {line}"))
            .collect()
    }
    let mut said = drain(child.stderr.take(), "[err]").await;
    said.extend(drain(child.stdout.take(), "[out]").await);
    if said.is_empty() {
        String::new()
    } else {
        format!(": {}", said.join(" ").chars().take(400).collect::<String>())
    }
}

/// One batched inventory call per device.
///
/// Deliberately a single `adb shell`: at roughly a second per call, one round
/// trip per field over a 16-device fleet is half a minute of listing. Measured
/// at 34 s before batching and parallelising.
async fn probe_device(adb: AdbProgram, serial: String, model_hint: Option<String>) -> DeviceInfo {
    let script = format!(
        "getprop ro.build.version.release; echo {sep}; \
         getprop ro.product.model; echo {sep}; \
         dumpsys battery | grep level",
        sep = FIELD_SEPARATOR
    );
    // A device that will not answer must say so. Swallowing the error left a
    // tile with a blank model and OS looking like an ordinary connected phone,
    // which is the same failure as reporting something that was never checked.
    let (stdout, probe_error) = match adb.shell(&serial, &script).await {
        Ok(stdout) => (stdout, None),
        Err(error) => (String::new(), Some(error.to_string())),
    };
    let fields = parse_inventory(&stdout);
    let model = match model_hint {
        Some(model) if !model.is_empty() => model,
        _ => fields.model.unwrap_or_default(),
    };
    DeviceInfo {
        udid: serial.clone(),
        name: if model.is_empty() {
            serial.clone()
        } else {
            model.clone()
        },
        model,
        // Still named `ios_version` in core; the rename to `os_version` plus a
        // `platform` tag is Pha 2 of the Android plan. Populating it with the
        // Android release is the honest reading of "OS version" until then.
        platform: riviu_core::DevicePlatform::Android,
        os_version: fields.release.unwrap_or_default(),
        connection: ConnectionKind::Usb,
        status: if probe_error.is_some() {
            DeviceStatus::Error
        } else {
            DeviceStatus::Connected
        },
        battery: fields.battery,
        wda_ready: false,
        // Android has no provisioning profile to expire. `adb install` needs no
        // per-device signing, so this stays `None`.
        wda_expires_at: None,
        stream_url: None,
        tile_stream_state: Default::default(),
        last_error: probe_error,
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Inventory {
    release: Option<String>,
    model: Option<String>,
    battery: Option<u8>,
}

fn parse_inventory(stdout: &str) -> Inventory {
    let mut sections = stdout.split(FIELD_SEPARATOR);
    let first = sections.next().unwrap_or_default().trim().to_string();
    let second = sections.next().unwrap_or_default().trim().to_string();
    let third = sections.next().unwrap_or_default();
    Inventory {
        release: (!first.is_empty()).then_some(first),
        model: (!second.is_empty()).then_some(second),
        battery: third
            .lines()
            .find_map(|line| line.trim().strip_prefix("level:"))
            .and_then(|value| value.trim().parse::<u8>().ok()),
    }
}

/// Where an Android screenshot goes, given where the caller asked for it.
///
/// Only the extension moves. The directory and stem are the caller's -- they carry the
/// serial and the timestamp that keep two phones' captures apart -- and a path that already
/// says `.png` is left exactly as it is.
fn screenshot_destination(dest: &Path) -> PathBuf {
    dest.with_extension("png")
}

/// A row for a phone adb can see but cannot drive.
///
/// Every field it can honestly fill is filled and the rest are left empty rather than
/// guessed: there is no OS version or battery to read from a device that will not answer.
/// The status is what the grid sorts and colours by, and the reason is the sentence the
/// operator acts on.
fn unusable_device(serial: &str, model: Option<String>, state: AdbDeviceState) -> DeviceInfo {
    DeviceInfo {
        udid: serial.to_string(),
        name: model.clone().unwrap_or_else(|| serial.to_string()),
        model: model.unwrap_or_default(),
        platform: riviu_core::DevicePlatform::Android,
        os_version: String::new(),
        connection: ConnectionKind::Usb,
        // `Pairing` for the one state a human can clear from the device itself;
        // `Disconnected` for the rest, which are about the cable, the hub or the mode the
        // phone booted into.
        status: match state {
            AdbDeviceState::Unauthorized => DeviceStatus::Pairing,
            _ => DeviceStatus::Disconnected,
        },
        battery: None,
        wda_ready: false,
        wda_expires_at: None,
        stream_url: None,
        tile_stream_state: Default::default(),
        last_error: state.operator_reason(),
    }
}

#[async_trait]
impl DeviceDriver for AndroidDriver {
    async fn list_devices(&self) -> anyhow::Result<Vec<DeviceInfo>> {
        // Read until two consecutive `adb devices` agree. A single reading is
        // not evidence: a server that is restarting answers one call and reports
        // a different fleet on the next, and `DeviceRegistry::upsert_many`
        // replaces the whole vector (AGENTS.md 9) — so a bad snapshot does not
        // just look wrong, it deletes devices from the fleet.
        //
        // An unstable reading is still returned rather than raised. Refusing the
        // scan would blank the Android half of a mixed fleet, which is the same
        // damage from the other direction; the honest fix is for the registry to
        // keep the previous vector on an untrusted read, and that is not built.
        let reading = self
            .adb
            .devices_stable(Duration::from_millis(250), Duration::from_millis(1500))
            .await;
        if !reading.stable {
            tracing::warn!(
                attempts = reading.attempts,
                devices = reading.devices.len(),
                "adb device list never settled; using the last reading"
            );
        }
        let lines = reading.devices;

        // Fan out: the fleet is 16 phones and every one of them costs a round
        // trip we would otherwise pay in series.
        let mut inflight = Vec::new();
        let mut unreachable_devices = Vec::new();
        for line in lines {
            match line.state {
                AdbDeviceState::Device => {
                    let adb = self.adb.clone();
                    inflight.push(tokio::spawn(probe_device(adb, line.serial, line.model)));
                }
                // **Report it, do not hide it**, and that now covers every state rather
                // than one of them. A phone whose USB-debugging prompt has not been
                // accepted is a normal fleet state with an obvious fix; so is one that has
                // gone `offline` because its cable or hub dropped, or because it is
                // mid-reboot. Dropping those from the list makes them look unplugged, which
                // is the one thing they are not — adb can see them, and it can say why.
                //
                // `offline` in particular was silently discarded, so a phone that lost its
                // connection simply vanished from the grid with no row and no reason.
                state => unreachable_devices.push(unusable_device(&line.serial, line.model, state)),
            }
        }

        let mut devices = Vec::with_capacity(inflight.len() + unreachable_devices.len());
        for handle in inflight {
            let Ok(mut device) = handle.await else {
                continue;
            };
            device.wda_ready = self.agent_ready(&device.udid).await;
            if device.wda_ready {
                device.status = DeviceStatus::Ready;
            }
            devices.push(device);
        }
        devices.extend(unreachable_devices);
        Ok(devices)
    }

    async fn refresh_device(&self, udid: &str) -> anyhow::Result<DeviceInfo> {
        // A refresh is the operator saying "look again", which is also the moment a
        // TikTok build may have been installed or removed -- or the display resolution
        // changed under us, which is the one stale-screen case an aspect-ratio check cannot
        // see, because `wm size 1080x2220` keeps the shape and moves the numbers.
        self.tiktok_packages.lock().remove(udid);
        self.invalidate_screen(udid);
        let mut device = probe_device(self.adb.clone(), udid.to_string(), None).await;
        device.wda_ready = self.agent_ready(udid).await;
        if device.wda_ready {
            device.status = DeviceStatus::Ready;
        }
        Ok(device)
    }

    async fn install_app(&self, udid: &str, path: &Path) -> anyhow::Result<()> {
        let path = path
            .to_str()
            .ok_or_else(|| anyhow!("the APK path is not UTF-8"))?;
        self.adb
            .device(
                udid,
                &["install", "-r", "-g", path],
                Duration::from_secs(300),
            )
            .await
            .map(|_| ())
    }

    async fn uninstall_app(&self, udid: &str, bundle_id: &str) -> anyhow::Result<()> {
        self.adb
            .device(udid, &["uninstall", bundle_id], adb::DEFAULT_TIMEOUT)
            .await
            .map(|_| ())
    }

    /// `screencap -p`, written under the extension it actually is.
    ///
    /// The caller names the file before it knows which backend will answer, and it names it
    /// `.jpg` because that is what the iOS path and the stream hub produce. `screencap -p`
    /// produces a PNG — the assertion two lines down has always said so — and the old code
    /// wrote those bytes straight into the `.jpg` the caller asked for, then handed back
    /// that path for the toast to display. Every Android screenshot this app has ever taken
    /// is a PNG with a lie for a file extension.
    ///
    /// Corrected here rather than at the call site because this is where the format is
    /// known, and the return value already exists for exactly this: callers use the path
    /// that comes back, not the one they passed in.
    async fn screenshot(&self, udid: &str, dest: &Path) -> anyhow::Result<PathBuf> {
        let png = self
            .adb
            .device_bytes(
                udid,
                &["exec-out", "screencap", "-p"],
                Duration::from_secs(120),
            )
            .await?;
        anyhow::ensure!(
            png.starts_with(&[0x89, b'P', b'N', b'G']),
            "screencap returned {} bytes that are not a PNG",
            png.len()
        );
        let dest = screenshot_destination(dest);
        tokio::fs::write(&dest, &png)
            .await
            .with_context(|| format!("write {}", dest.display()))?;
        Ok(dest)
    }

    async fn syslog_tail(&self, udid: &str, lines: usize) -> anyhow::Result<String> {
        self.adb
            .shell(udid, &format!("logcat -d -t {}", lines.clamp(1, 5_000)))
            .await
    }

    async fn launch_app(&self, udid: &str, bundle_id: &str) -> anyhow::Result<()> {
        let bundle_id = adb::validate_package_name(bundle_id)?;
        self.adb
            .shell(
                udid,
                &format!("monkey -p {bundle_id} -c android.intent.category.LAUNCHER 1"),
            )
            .await
            .map(|_| ())
    }

    /// Stop the app and prove it is gone.
    ///
    /// `am force-stop` says nothing about the outcome, so the proof comes from
    /// `pidof` afterwards. That is a real absence check, which is why
    /// [`Self::supports_verified_app_termination`] can honestly be true.
    async fn terminate_app(
        &self,
        udid: &str,
        bundle_id: &str,
    ) -> anyhow::Result<ProcessAbsenceProof> {
        let bundle_id = adb::validate_package_name(bundle_id)?;
        let before = self.pid_of(udid, bundle_id).await;
        self.adb
            .shell(udid, &format!("am force-stop {bundle_id}"))
            .await?;
        let after = self.pid_of(udid, bundle_id).await;
        if let Some(pid) = after {
            return Err(anyhow!(
                "{bundle_id} is still running (pid {pid}) after force-stop"
            ));
        }
        Ok(ProcessAbsenceProof {
            bundle_id: bundle_id.to_string(),
            old_pid: before,
        })
    }

    fn supports_verified_app_termination(&self, _udid: &str) -> bool {
        true
    }

    fn cached_agent_status(&self, udid: &str) -> riviu_core::AgentStatus {
        self.agent_statuses
            .lock()
            .get(udid)
            .cloned()
            .unwrap_or_else(|| riviu_core::AgentStatus::unknown(udid))
    }

    /// Bring the agent up, prove it can see, and record what was found.
    ///
    /// The proof is the point. `ensure_agent` already refuses to hand back a server that
    /// answers `/status` but cannot read the accessibility tree, so reaching the end of it
    /// is what "ready" means here; anything less is reported as needing repair, with the
    /// message the phone gave.
    async fn preflight_agent(&self, udid: &str) -> anyhow::Result<riviu_core::AgentStatus> {
        let status = match self.ensure_agent(udid).await {
            Ok(_) => {
                let identity = self.package_identity(udid, AGENT_PACKAGE).await.ok();
                self.agent_status_for(udid, riviu_core::AgentState::Ready, identity.as_ref(), None)
            }
            Err(error) => self.agent_status_for(
                udid,
                riviu_core::AgentState::RepairRequired,
                None,
                Some(format!("{error:#}")),
            ),
        };
        self.publish_agent_status(status.clone());
        Ok(status)
    }

    /// Repair is the same operation as preflight here, and that is not a shortcut.
    ///
    /// `ensure_agent` installs both APK halves when they are missing and restarts the
    /// instrumentation when the server has gone blind — the two things a repair could do.
    async fn repair_agent(&self, udid: &str) -> anyhow::Result<riviu_core::AgentStatus> {
        let status = self.preflight_agent(udid).await?;
        if status.state == riviu_core::AgentState::Ready {
            Ok(status)
        } else {
            Err(anyhow!(
                "{}",
                status
                    .message
                    .unwrap_or_else(|| format!("the agent on {udid} could not be repaired"))
            ))
        }
    }

    /// Install and verify the agent without opening a session or starting a stream.
    ///
    /// This is the method every product-level agent path actually calls — Settings' Check
    /// and Repair, the toolbar's bulk repair, `job_queue::run_on_device`, and the nurture
    /// comment preflight all reach it through `DeviceControlPlane`, and that indirection is
    /// deliberate: a test pins that preflight must not open a session or start a stream,
    /// because checking on a phone should not disturb the phone. Only the iOS drivers
    /// implemented it, so on an Android fleet every one of those paths answered
    /// `capability repairAgentInstallOnly is not supported by this device` — and the nurture
    /// refusal told the operator to run the Agent Repair that had just failed for the same
    /// reason. `AndroidDriver::preflight_agent` was implemented and working the whole time;
    /// nothing in the product called it.
    ///
    /// `ensure_agent` is the whole repair on this platform — it installs both APK halves
    /// when they are missing and restarts the instrumentation when the server has gone
    /// blind — and it starts no UI session and no producer, which is what the install-only
    /// contract asks for.
    async fn repair_agent_install_only(
        &self,
        udid: &str,
    ) -> anyhow::Result<riviu_core::AgentInstallProof> {
        self.ensure_agent(udid).await?;
        let agent = self.package_identity(udid, AGENT_PACKAGE).await?;
        let agent_apk_sha256 = self.installed_apk_sha256(udid, AGENT_PACKAGE).await?;
        let proof = crate::capability::install_only_proof(
            agent.clone(),
            agent_apk_sha256,
            Self::agent_runner(),
        );
        // Refuse to hand back a proof that does not satisfy the contract it names, rather
        // than let a caller store it and find out later: the digest has to be a digest, and
        // the lifecycle flags have to say that nothing was started.
        proof.validate_install_only()?;
        // Keep the synchronous readers in step, the same way `preflight_agent` does.
        let status = self.agent_status_for(udid, riviu_core::AgentState::Ready, Some(&agent), None);
        self.publish_agent_status(status);
        Ok(proof)
    }

    /// Everything Flow's preflight needs to qualify this phone for this target app.
    ///
    /// Implemented 17/08/2026. Before that the trait default returned a typed `unsupported`
    /// and every Flow run on every Android device failed here, while the UI went on listing
    /// those devices as valid targets.
    ///
    /// Ordered so the cheapest refusal comes first. A phone without the target app installed
    /// is the common miss, and finding that out costs one `dumpsys`; there is no point
    /// hashing an APK for a device that was never going to qualify.
    async fn inspect_device_for_target(
        &self,
        udid: &str,
        target_bundle_id: &str,
    ) -> anyhow::Result<riviu_core::DeviceCapabilitySnapshot> {
        let target = self.package_identity(udid, target_bundle_id).await?;
        let agent = self.package_identity(udid, AGENT_PACKAGE).await?;
        let agent_apk_sha256 = self.installed_apk_sha256(udid, AGENT_PACKAGE).await?;
        let display = self.display_geometry(udid).await?;
        let (product_type, os_version) = self.device_identity(udid).await?;
        // Last, and live: everything above describes what is installed, this asks whether
        // the control surface is answering *now*. Recorded through the same path
        // `preflight_agent` uses so a synchronous reader sees the same verdict.
        let control_surface_live =
            self.preflight_agent(udid).await?.state == riviu_core::AgentState::Ready;
        Ok(crate::capability::build_snapshot(
            crate::capability::AndroidCapabilityFacts {
                agent,
                target,
                agent_apk_sha256,
                display,
                product_type,
                os_version,
                control_surface_live,
                runner: Self::agent_runner(),
            },
        ))
    }

    /// Yes, and measured: Vietnamese reaches TikTok's comment box intact
    /// through accessibility `ACTION_SET_TEXT`.
    fn supports_text_comments(&self, _udid: &str) -> bool {
        true
    }

    /// True — every Android session answers `locate`.
    fn reports_element_bounds(&self, _udid: &str) -> bool {
        true
    }

    /// Push a campaign's files into a directory MediaStore does **not** scan.
    ///
    /// `agent_bundle_id` is unused here and that is deliberate rather than an
    /// oversight: on iOS the files go into the Agent's own sandbox over HouseArrest, so
    /// the bundle id names the destination. Android has no such sandbox in play — the
    /// staging directory is a dot-prefixed folder under `Pictures`, hidden from the
    /// media scanner and therefore from TikTok's picker, which is what preserves the
    /// contract's two-step meaning.
    async fn stage_publish_media(
        &self,
        udid: &str,
        _agent_bundle_id: &str,
        campaign_id: &str,
        source_root: &Path,
    ) -> anyhow::Result<serde_json::Value> {
        crate::publish::stage(&self.adb, udid, campaign_id, source_root).await
    }

    /// True — the import path below is real, so callers may use the native route.
    fn supports_push_media(&self, _udid: &str) -> bool {
        true
    }

    async fn prepare_publish_media(
        &self,
        udid: &str,
        campaign_id: &str,
        manifest_sha256: &str,
    ) -> anyhow::Result<serde_json::Value> {
        crate::publish::prepare(&self.adb, udid, campaign_id, manifest_sha256).await
    }

    async fn import_publish_media(
        &self,
        udid: &str,
        campaign_id: &str,
        manifest_sha256: &str,
    ) -> anyhow::Result<serde_json::Value> {
        crate::publish::import(&self.adb, udid, campaign_id, manifest_sha256).await
    }

    async fn cleanup_publish_media(
        &self,
        udid: &str,
        import_id: &str,
    ) -> anyhow::Result<serde_json::Value> {
        crate::publish::cleanup(&self.adb, udid, import_id).await
    }

    async fn pull_media(
        &self,
        udid: &str,
        dest_dir: &Path,
    ) -> anyhow::Result<riviu_core::MediaPullReport> {
        crate::publish::pull_media(&self.adb, udid, dest_dir).await
    }

    /// Which regional TikTok build this phone actually has.
    ///
    /// Reads the installed packages rather than the foreground app: at the time a
    /// caller needs this there is no session yet and the phone may be on the
    /// launcher. Foreground is the right way to *break a tie* between two installed
    /// builds, and the wrong way to resolve in the first place.
    ///
    /// Memoised because `pm list packages` is a 1–2 s adb round trip per candidate
    /// and this is on the path to every session.
    /// Two calls, both partitions, tagged rather than filtered.
    ///
    /// `cmd package` and **not** `pm`. Measured 14/08/2026 on both attached phones:
    /// `/system/bin/pm` on SDK 26 is `exec app_process … com.android.commands.pm.Pm`, one
    /// JVM start per invocation, so `pm list packages -3` costs 786–820 ms while
    /// `cmd package list packages -3` costs 274 ms; on SDK 35 `pm` is literally
    /// `cmd package "$@"` (290 ms vs 199 ms). The "1–2 s per `pm list packages`" recorded
    /// elsewhere in this file is the cost of that **wrapper**, not of the package service.
    /// Both partitions together measured 521–606 ms.
    ///
    /// `--user 0` is not optional. The Redmi carries a MIUI Second Space
    /// (`UserInfo{11:security space}`) and without it the listing returns rows from user
    /// 11 as well — apps that are not on the screen anyone is looking at.
    ///
    /// System apps are listed and tagged, never filtered out. A `-3`-only listing would
    /// omit a preinstalled TikTok and then disagree with `resolve_tiktok_package` about
    /// what is on the same phone; hiding them is the UI's visible choice.
    /// One operator-typed script, through the phone's own `sh`.
    ///
    /// No host shell is involved: `AdbProgram` spawns with argv and `Stdio::null()`
    /// stdin, so there is no `cmd.exe` and host-side escaping would be theatre. The
    /// script reaches the device exactly as typed, which is the point of an escape hatch
    /// — and the reason this is not an automation seam.
    ///
    /// Refuses an empty script rather than running one: `adb shell ""` opens an
    /// interactive shell that never returns, and the caller would see only a timeout.
    async fn device_shell(
        &self,
        udid: &str,
        script: &str,
    ) -> anyhow::Result<riviu_core::ShellOutcome> {
        // adb will not refuse this for us: measured, `adb shell ""` exits 0 with empty
        // output, so an accidental Enter on an empty box would read as a command that
        // ran and produced nothing.
        anyhow::ensure!(!script.trim().is_empty(), "empty command");
        let output = self
            .adb
            .shell_output(udid, script, adb::DEFAULT_TIMEOUT)
            .await?;
        Ok(riviu_core::ShellOutcome {
            exit_code: output.exit_code,
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    /// Ask for a rotation, then read back what the device actually did.
    ///
    /// Measured 14/08/2026 on both fleet phones, and the measurement is why this returns
    /// the observed rotation rather than `()`: **neither mechanism turned either screen.**
    /// `settings put system user_rotation 1` set the key and left `mRotation` at 0;
    /// `cmd window user-rotation lock 1` reported `lock 1` and left it at 0; and
    /// `set-ignore-orientation-request true` did not help either. A portrait-locked
    /// foreground app — a lock screen, TikTok — wins. Since TikTok is what this farm
    /// drives, a Rotate that claimed success would be a button that lies about the most
    /// common case.
    ///
    /// Both mechanisms are attempted because neither is universal: `cmd window` does not
    /// exist on the Note 8 at all (`No shell command implementation.` on SDK 26), and the
    /// `settings` keys alone are what older Android honours. Individual failures are
    /// ignored; the read-back is the only thing that decides the answer.
    async fn set_screen_rotation(&self, udid: &str, rotation: u8) -> anyhow::Result<u8> {
        anyhow::ensure!(rotation < 4, "rotation must be 0, 1, 2 or 3");
        // Invalidate BEFORE asking, not after. Once the shells below have run there is no
        // way to tell from a failed read-back whether the screen moved, so the cached size
        // has to be dropped either way — and dropping it costs one lazy agent read on the
        // next gesture, while keeping a stale one costs every coordinate after this.
        self.invalidate_screen(udid);
        // Auto-rotate has to come off first, or the sensor overrides the request on a
        // phone that is lying flat.
        let _ = self
            .adb
            .shell(udid, "settings put system accelerometer_rotation 0")
            .await;
        let _ = self
            .adb
            .shell(
                udid,
                &format!("settings put system user_rotation {rotation}"),
            )
            .await;
        // SDK 35 route. Absent on SDK 26, where this simply fails and is ignored.
        let _ = self
            .adb
            .shell(udid, &format!("cmd window user-rotation lock {rotation}"))
            .await;
        let observed = self
            .adb
            .shell(udid, "dumpsys window")
            .await
            .ok()
            .and_then(|dump| adb::parse_screen_rotation(&dump));
        observed.ok_or_else(|| {
            anyhow::anyhow!("could not read {udid} rotation back after asking for {rotation}")
        })
    }

    async fn list_installed_apps(
        &self,
        udid: &str,
    ) -> anyhow::Result<Vec<riviu_core::InstalledApp>> {
        let mut apps: Vec<riviu_core::InstalledApp> = Vec::new();
        for (flag, kind) in [
            ("-3", riviu_core::InstalledAppKind::User),
            ("-s", riviu_core::InstalledAppKind::System),
        ] {
            let stdout = self
                .adb
                .shell(udid, &format!("cmd package list packages {flag} --user 0"))
                .await?;
            for bundle_id in adb::parse_package_list(&stdout) {
                apps.push(riviu_core::InstalledApp {
                    bundle_id,
                    kind,
                    // Filled in below when the helper is there; not obtainable over adb at
                    // any price worth paying. See the doc on `InstalledApp`.
                    label: None,
                    icon_png_base64: None,
                });
            }
        }
        self.name_apps_with_helper(udid, &mut apps).await;
        Ok(apps)
    }

    async fn resolve_tiktok_package(&self, udid: &str) -> anyhow::Result<String> {
        if let Some(known) = self.tiktok_packages.lock().get(udid) {
            return Ok(known.clone());
        }
        let mut listing = String::new();
        for candidate in riviu_core::tiktok_target::measured_android_packages() {
            let candidate = adb::validate_package_name(candidate)?;
            if let Ok(stdout) = self
                .adb
                .shell(udid, &format!("pm list packages {candidate}"))
                .await
            {
                listing.push_str(&stdout);
                listing.push('\n');
            }
        }
        let resolved = match riviu_core::tiktok_target::resolve_installed_android_tiktok(&listing) {
            Ok(package) => package,
            Err(riviu_core::tiktok_target::TargetResolution::Ambiguous(found)) => {
                // Two measured builds side by side. Whichever is in front is the one
                // the operator is working with; anything else would be a coin flip.
                //
                // Read through adb rather than opening a session: resolving a package
                // must not have the side effect of creating one, and `mCurrentFocus`
                // needs no agent.
                let foreground = self
                    .adb
                    .shell(udid, "dumpsys window displays | grep mCurrentFocus")
                    .await
                    .ok()
                    .as_deref()
                    .and_then(adb::parse_current_focus_package);
                match foreground.filter(|package| found.contains(package)) {
                    Some(package) => package,
                    None => anyhow::bail!(
                        "{udid}: more than one measured TikTok build is installed ({}), and none \
                         of them is in the foreground to break the tie",
                        found.join(", ")
                    ),
                }
            }
            Err(error) => {
                return Err(anyhow!("{udid}: {error}"));
            }
        };
        self.tiktok_packages
            .lock()
            .insert(udid.to_string(), resolved.clone());
        Ok(resolved)
    }

    async fn inspect_app_process(
        &self,
        udid: &str,
        bundle_id: &str,
    ) -> anyhow::Result<AppProcessState> {
        let bundle_id = adb::validate_package_name(bundle_id)?;
        let pid = self.pid_of(udid, bundle_id).await;
        Ok(AppProcessState {
            bundle_id: bundle_id.to_string(),
            pid,
            running: pid.is_some(),
        })
    }

    async fn reboot(&self, udid: &str) -> anyhow::Result<()> {
        self.adb
            .device(udid, &["reboot"], adb::DEFAULT_TIMEOUT)
            .await
            .map(|_| ())
    }

    async fn start_ui_session(&self, udid: &str) -> anyhow::Result<Box<dyn UiSession>> {
        Ok(Box::new(self.open_session(udid).await?))
    }

    /// Read the generation the next session and stream must both use.
    ///
    /// **Reads, never advances.** The whole handoff contract rests on nothing moving
    /// between here and `start_stream_after_session`, which is how
    /// `proof.generation == handoff_generation` holds by construction rather than by
    /// copying a number around.
    ///
    /// Idempotent: the control plane re-confirms on a failed stream start to re-arm
    /// its cleanup ticket, so a second call at the same generation must succeed.
    async fn confirm_interaction_stream_stopped(
        &self,
        udid: &str,
    ) -> anyhow::Result<riviu_core::StreamHandoffProof> {
        let sink = self.sink()?;
        self.producer_absent(udid).await?;
        let generation = sink.generation(udid);
        self.interaction.record_stopped(udid, generation);
        Ok(riviu_core::StreamHandoffProof { generation })
    }

    /// Open a session with the target app proven to be in front.
    ///
    /// The foreground **proof** is the point. `monkey` exits 0 in cases where it did
    /// not do what was asked — it is the "HTTP 200" of this path — so the answer
    /// comes from `active_app_bundle`, which reads `mCurrentFocus` and is real
    /// evidence. Nothing else on the nurture path foregrounds the app, so without
    /// this the hierarchy loop would run against the launcher and report a locator
    /// failure for what is really a launch failure.
    ///
    /// `kind` produces the same session either way on Android and that is recorded
    /// rather than silently dropped: `FreshText` exists on iOS because the trusted
    /// text channel is a property of *when* the session was created, while here it
    /// is a property of the agent — `type_text` goes through accessibility
    /// `ACTION_SET_TEXT` and carries full Vietnamese diacritics regardless.
    async fn start_interaction_session(
        &self,
        udid: &str,
        bundle_id: &str,
        kind: riviu_core::InteractionSessionKind,
    ) -> anyhow::Result<Box<dyn UiSession>> {
        // Cheap, phone-free refusals first, so a caller that got the sequence wrong
        // learns before anything is touched.
        let bundle_id = adb::validate_package_name(bundle_id)?.to_string();
        let sink = self.sink()?;
        let generation = sink.generation(udid);
        let reservation = self.interaction.begin_session(udid, generation, kind)?;

        let session = self.open_session(udid).await?;
        riviu_core::driver::UiSession::launch_app_foreground(&session, &bundle_id).await?;

        let deadline = std::time::Instant::now() + FOREGROUND_PROOF_TIMEOUT;
        // One Back, then a short grace, then the truth. Measured on ce0717171c2a64d50d on
        // 18/08/2026: a whole-fleet nurture run lost exactly one phone, and it was sitting
        // under `GrantPermissionsActivity` — the launch had worked, and TikTok's *own*
        // permission dialog was standing over it, in TikTok's own task.
        //
        // Back is tried because Back is what cleared the in-app modal in `await_feed`, and
        // it is the one gesture that cannot grant anything. It was then measured **not** to
        // clear this one: Android's permission dialogs are not cancelable. So the rest of
        // the forty seconds is spent watching a screen that cannot change, and the honest
        // answer is to stop and say which screen it is. Answering the dialog is not on the
        // table: both its buttons are labelled, one of them grants a permission on a real
        // account's phone, and that is the operator's decision and not a recovery path's.
        let mut backed_at: Option<std::time::Instant> = None;
        // **The lock screen, which used to eat the whole forty seconds and then blame the
        // parser.** Measured 23/08/2026: of fourteen phones attached, two sat behind their
        // keyguard, and this loop reported *"the phone is showing <unreadable: could not read
        // the foreground package … had no mCurrentFocus line>"*. The line was there; it named
        // `StatusBar`, which carries no `package/activity` pair. So the sentence described the
        // parser rather than the phone, and the phone was in a state the app can fix.
        //
        // Latched, not polled: `screen_guard_state` is a `dumpsys window` round trip, and
        // asking every 250 ms for forty seconds would be 160 of them per phone. Consulted
        // once, exactly like `backed_at` above.
        let mut keyguard_tried = false;
        loop {
            let observed = match riviu_core::driver::UiSession::active_app_bundle(&session).await {
                // The package is in front. Give the splash a bounded moment to get out of the
                // way, then proceed either way — see `wait_out_splash`.
                Ok(package) if package == bundle_id => {
                    self.wait_out_splash(udid).await;
                    break;
                }
                Ok(package) => package,
                // **Ask what the window *is*, rather than matching on the error text.**
                //
                // A system window has no `package/activity` pair, so a foreground read fails
                // and its message is the only thing that names it. Matching that message was a
                // guess about wording and it did not fire: measured 25/08/2026, a phone
                // holding `Select input method` failed every run with the plain forty-second
                // timeout, never with the dialog path. `screen_guard_state` already parses the
                // same dump into a `ForegroundWindow`, so the name is available as data.
                Err(error) => match self.screen_guard_state(udid).await {
                    Ok(state) => match state.foreground {
                        crate::adb::ForegroundWindow::System(window) => window,
                        _ => format!("<unreadable: {error}>"),
                    },
                    Err(_) => format!("<unreadable: {error}>"),
                },
            };

            if !keyguard_tried {
                keyguard_tried = true;
                // An unreadable dump must fall through to the timeout rather than refuse:
                // `behind_lock_screen` answers `false` when it has no evidence, and
                // `parse_keyguard_locked` is explicit that unknown is not "unlocked". A
                // false refusal here would turn a working phone away.
                if let Ok(state) = self.screen_guard_state(udid).await {
                    if state.behind_lock_screen() {
                        let blocker = state.blocker().unwrap_or("lock screen").to_string();
                        tracing::info!(
                            udid,
                            blocker = %blocker,
                            "phone is behind its lock screen; dismissing before waiting out the \
                             foreground proof"
                        );
                        // `dismiss_keyguard` re-reads the keyguard and returns the honest
                        // answer, which is why it is used rather than `set_locked(false)`:
                        // that one presses two keys over the HTTP agent and verifies nothing,
                        // so a PIN-locked phone would come back looking unlocked.
                        let opened = self.dismiss_keyguard(udid).await.unwrap_or(false);
                        if opened {
                            let _ = riviu_core::driver::UiSession::launch_app_foreground(
                                &session, &bundle_id,
                            )
                            .await;
                        } else {
                            self.interaction.clear(udid);
                            anyhow::bail!(
                                "{udid} đang ở màn hình khoá ({blocker}) và không mở được bằng \
                                 phím — máy này có PIN/pattern/vân tay. Màn hình có thể đang \
                                 sáng, nhưng không app nào lên foreground được cho tới khi \
                                 nó được mở khoá bằng tay"
                            );
                        }
                    }
                }
            }

            let over_the_app = dialog_over_app(&observed);
            if over_the_app && backed_at.is_none() {
                backed_at = Some(std::time::Instant::now());
                tracing::info!(
                    udid,
                    observed = %observed,
                    "a system dialog is over the app; pressing Back once and re-launching"
                );
                let _ = riviu_core::driver::UiSession::back(&session).await;
                let _ = riviu_core::driver::UiSession::launch_app_foreground(&session, &bundle_id)
                    .await;
            }
            let now = std::time::Instant::now();
            let stuck_behind_dialog =
                over_the_app && backed_at.is_some_and(|at| now.duration_since(at) >= DIALOG_GRACE);
            if stuck_behind_dialog {
                self.interaction.clear(udid);
                anyhow::bail!(
                    "{bundle_id} is running on {udid} but {observed} is standing over it, and \
                     Back did not clear it. This is a system permission dialog in the app's own \
                     task: nothing here can answer it, because one of its buttons grants a \
                     permission on a real account. Clear it on the phone once and the phone is \
                     usable again"
                );
            }
            if now >= deadline {
                self.interaction.clear(udid);
                anyhow::bail!(
                    "{bundle_id} did not reach the foreground on {udid} within {}s; the phone is \
                     showing {observed}. A locked screen does this — `monkey` reports success \
                     and nothing moves",
                    FOREGROUND_PROOF_TIMEOUT.as_secs()
                );
            }
            tokio::time::sleep(FOREGROUND_PROOF_POLL).await;
        }

        self.interaction.complete_session(&reservation)?;
        tracing::info!(
            udid,
            bundle_id = %bundle_id,
            generation,
            ?kind,
            "android interaction session started with a foreground proof"
        );
        Ok(Box::new(session))
    }

    /// Delegate, so the target is not launched twice.
    ///
    /// The trait default calls `launch_app` and *then* `start_interaction_session`,
    /// which on Android would `monkey` TikTok twice — an extra round trip and a
    /// redundant resume. `start_interaction_session` already foregrounds and proves
    /// it. iOS overrides this for the same reason.
    async fn foreground_target_app_and_start_interaction_session(
        &self,
        udid: &str,
        bundle_id: &str,
        kind: riviu_core::InteractionSessionKind,
    ) -> anyhow::Result<Box<dyn UiSession>> {
        self.start_interaction_session(udid, bundle_id, kind).await
    }

    /// Start the interaction stream at the handoff generation, proving a frame.
    ///
    /// `first_frame_observed` is literally what the reader proved — a JPEG that
    /// decoded and was accepted by the sink at this generation. There is no path
    /// that sets it to a constant, and a timeout is an error rather than a `false`.
    async fn start_stream_after_session(
        &self,
        udid: &str,
    ) -> anyhow::Result<riviu_core::StreamStartProof> {
        let sink = self.sink()?;
        let claim = self.claim_start(udid)?;
        if self.streams.lock().await.contains_key(udid) {
            anyhow::bail!(
                "interaction stream for {udid} requires the old producer to be stopped first"
            );
        }
        let generation = sink.generation(udid);
        // Catches a drift between the handoff and now, here, with a message naming
        // the step — rather than letting core report an opaque StopProofMismatch.
        let reservation = self.interaction.reserve_stream(udid, generation)?;

        let started = self
            .spawn_producer(
                udid,
                reservation.generation(),
                StreamReadiness::DecodedFrame,
            )
            .await;
        drop(claim);
        let first_frame_observed = started?;
        self.interaction.complete_stream(&reservation)?;
        Ok(riviu_core::StreamStartProof {
            generation: reservation.generation(),
            first_frame_observed,
            stream_url: format!("auto-stream://{udid}"),
        })
    }

    /// Destructive stop: the producer dies and the tile's cached frame goes with it.
    async fn stop_owned_stream(
        &self,
        udid: &str,
    ) -> anyhow::Result<riviu_core::stream_budget::StreamStopProof> {
        self.teardown_stream(udid, false).await
    }

    /// Bounded stop: the producer dies but the tile keeps its last image.
    ///
    /// This is the path the desktop sampler takes every time it finishes a
    /// background sample. Before it existed the Android tile ended every turn in
    /// `TileStreamState::Error`, because the trait default refused.
    async fn park_owned_stream(
        &self,
        udid: &str,
    ) -> anyhow::Result<riviu_core::stream_budget::StreamStopProof> {
        self.teardown_stream(udid, true).await
    }

    /// Forget any lifecycle reservation for this device.
    ///
    /// Called by the plane's cleanup when a ticket is abandoned, and synchronous for
    /// that reason.
    fn invalidate_ui_session(&self, udid: &str) {
        self.interaction.clear(udid);
    }

    /// The operator's tile feed, started best-effort.
    ///
    /// Deliberately does **not** require a first frame, unlike the interaction
    /// handoff. minicap publishes only when the display changes, so a phone
    /// sitting on a static screen legitimately produces nothing — and the desktop
    /// sampler already has its own freshness watchdog and a `Stale` label for
    /// that. Demanding a frame here would report a working phone as broken.
    async fn ensure_stream(&self, udid: &str) -> anyhow::Result<String> {
        self.ensure_minicap_locked(udid).await?;
        // The scheme is a marker, not a fetchable URL: readers take frames from
        // the hub. Same shape the iOS path returns so the tile treats both alike.
        Ok(format!("auto-stream://{udid}"))
    }

    async fn prepare_device(&self, udid: &str) -> anyhow::Result<()> {
        // Install/auth-only, per the trait contract: no session, no producer.
        self.adb
            .device(udid, &["wait-for-device"], adb::DEFAULT_TIMEOUT)
            .await?;
        Ok(())
    }
}

/// Wrapper so the composition root can hold the driver as a trait object.
pub fn create_driver(config: &AndroidDriverConfig) -> anyhow::Result<Arc<dyn DeviceDriver>> {
    Ok(Arc::new(AndroidDriver::new(config)?))
}

/// Build the driver only when `adb` is actually usable on this host.
///
/// `AdbProgram::resolve` always succeeds — it falls back to the bare name and
/// lets the OS search `PATH` — so construction alone proves nothing. A machine
/// with no Android tooling should not carry a permanently degraded Android
/// backend in every fleet listing; the honest report is that there is no
/// backend, and why.
///
/// Returns the concrete driver rather than `Arc<dyn DeviceDriver>` so the caller
/// can still hand it a frame sink; it coerces to the trait object at the point it
/// is put in a fleet.
///
/// Every candidate in [`AdbProgram::candidates`] is tried in order and the first one
/// that answers `adb version` wins. It used to resolve a single path and probe only
/// that, which had a real bug in it: `resolve` picks the first candidate that merely
/// **exists**, so a stale `ANDROID_HOME` pointing at a deleted SDK — or, once we ship
/// one, any earlier entry that happens to be broken — made this report that Android
/// was unavailable on a machine with a perfectly good adb one position further down.
/// Trying each is also what makes the bundled copy reachable at all, since it sits
/// last on purpose.
pub async fn detect_driver(config: &AndroidDriverConfig) -> Result<Arc<AndroidDriver>, String> {
    let candidates = AdbProgram::candidates(
        config.adb_path.as_deref(),
        config.bundled_adb_path.as_deref(),
    );
    let mut refusals: Vec<String> = Vec::new();
    for candidate in candidates {
        let adb = AdbProgram::at(candidate.path.clone());
        match adb.run(&["version"], Duration::from_secs(10)).await {
            Ok(_) => return Ok(Arc::new(AndroidDriver::with_adb(adb, config))),
            Err(error) => refusals.push(format!(
                "{} ({}): {error}",
                candidate.path.display(),
                candidate.origin.label()
            )),
        }
    }
    // Every candidate named, with where it came from. The operator needs to see that
    // their `RIVIU_ADB_PATH` was read and rejected, not wonder whether it was read.
    Err(format!(
        "không có adb nào chạy được. Đã thử: {}",
        refusals.join("; ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn a_phone_adb_can_see_but_not_drive_gets_a_row_and_a_reason() {
        // `offline` was discarded outright, so a phone whose cable or hub dropped simply
        // vanished from the grid: no row, no reason, indistinguishable from unplugged. The
        // comment beside the `unauthorized` arm had already made the argument -- "report
        // it, do not hide it" -- and it applies to every state that is not `device`.
        let offline = unusable_device("ce06", Some("SM-N950F".into()), AdbDeviceState::Offline);
        assert_eq!(offline.udid, "ce06");
        assert_eq!(offline.name, "SM-N950F");
        assert_eq!(offline.status, DeviceStatus::Disconnected);
        assert!(offline
            .last_error
            .as_deref()
            .expect("a reason")
            .contains("not answering"));

        // Unauthorised keeps `Pairing`: it is the one state a human clears from the device
        // itself, and the grid treats it differently for that reason.
        let unauthorised = unusable_device("ce07", None, AdbDeviceState::Unauthorized);
        assert_eq!(unauthorised.status, DeviceStatus::Pairing);
        assert_eq!(
            unauthorised.name, "ce07",
            "no model, so the serial has to do"
        );
        assert!(unauthorised
            .last_error
            .as_deref()
            .expect("a reason")
            .contains("accept the prompt"));

        // And an unrecognised state says which one it was, because `recovery`,
        // `sideload` and `no permissions` have different fixes.
        let recovery = unusable_device("ce08", None, AdbDeviceState::Other("recovery".into()));
        assert_eq!(recovery.status, DeviceStatus::Disconnected);
        assert!(recovery
            .last_error
            .as_deref()
            .expect("a reason")
            .contains("`recovery`"));
    }

    #[test]
    fn an_android_screenshot_is_saved_under_the_extension_it_actually_is() {
        // `screencap -p` returns a PNG; the caller names the file `.jpg` because that is
        // what the iOS path and the stream hub produce. The old code wrote PNG bytes into
        // that `.jpg` and handed the path back for the toast, so every Android screenshot
        // this app has ever taken is a PNG with a lie for a file extension -- and the
        // operator is told exactly that path.
        let asked = Path::new("C:/riviu/screenshots/ce06-1755400000000.jpg");
        let actual = screenshot_destination(asked);

        assert_eq!(
            actual.extension().and_then(|value| value.to_str()),
            Some("png")
        );
        // The stem is what keeps two phones' captures apart; only the extension moves.
        assert_eq!(actual.file_stem(), asked.file_stem());
        assert_eq!(actual.parent(), asked.parent());
        // Already correct stays correct.
        assert_eq!(
            screenshot_destination(Path::new("/tmp/shot.png")),
            PathBuf::from("/tmp/shot.png")
        );
    }

    #[test]
    fn the_producer_publishes_the_pixels_the_snapshot_claims_the_device_has() {
        // The two halves of Flow's coordinate model have to agree, and until 17/08/2026
        // they did not. `inspect_device_for_target` reports the device's real geometry;
        // this producer published `Projection::half` of it. `validate_geometry` compares
        // the two and refuses on any difference, so on this fleet every image-coordinate
        // tap was a guaranteed `GeometryMismatch` and the inspector's coordinate picker
        // could never return a usable frame.
        //
        // Asserted as the relationship rather than as a constant: the point is not that
        // 1080 appears twice, it is that whatever the snapshot says the pixels are, that
        // is what comes out of the producer.
        let display = crate::adb::DisplayGeometry {
            width: 1080,
            height: 2220,
            density: 420,
            rotation: 0,
        };
        let geometry = crate::capability::qualified_geometry(display);
        let projection = AndroidDriver::producer_projection((display.width, display.height));
        assert_eq!(projection.virtual_width, geometry.pixel_width);
        assert_eq!(projection.virtual_height, geometry.pixel_height);
        // And the half it used to be is exactly what that check rejects.
        let old = crate::frames::Projection::half(display.width, display.height);
        assert_ne!(old.virtual_width, geometry.pixel_width);
    }

    #[test]
    fn the_instrumentation_cooldown_outlasts_the_attempt_it_is_bounding() {
        // The whole point of the window is that two attempts cannot overlap. A cooldown
        // shorter than one trip through the blind path would let the second restart begin
        // while the first is still polling for a port -- which is the storm, not the cure.
        //
        assert!(
            INSTRUMENTATION_RESTART_COOLDOWN >= INSTRUMENTATION_ATTEMPT_COST,
            "cooldown {INSTRUMENTATION_RESTART_COOLDOWN:?} is shorter than one attempt \
             ({INSTRUMENTATION_ATTEMPT_COST:?}), so restarts would overlap"
        );
        // The attempt cost is dominated by two queries the server will not answer, so a
        // derivation that forgot one of them would halve the window without looking wrong.
        assert!(
            INSTRUMENTATION_ATTEMPT_COST >= AgentClient::BLIND_QUERY_COST * 2,
            "an attempt pays for the blind session AND its replacement"
        );

        // And the wait the operator is told about has to be the wait the loop actually
        // performs, or the error message is fiction.
        assert_eq!(
            AGENT_READY_WAIT,
            AGENT_READY_POLL_EVERY * AGENT_READY_POLLS,
            "the derived deadline drifted from the poll loop"
        );
    }

    /// A `FrameSink` that counts which advance was used.
    ///
    /// The counters are the point: the difference between stop and park is not
    /// visible in the returned generation, only in whether the cached frame
    /// survived, so the test has to observe the call itself.
    #[derive(Default)]
    struct TestSink {
        generations: Mutex<HashMap<String, u64>>,
        cleared: AtomicU64,
        parked: AtomicU64,
    }

    impl TestSink {
        fn advance(&self, udid: &str) -> u64 {
            let mut generations = self.generations.lock();
            let entry = generations.entry(udid.to_string()).or_insert(0);
            *entry += 1;
            *entry
        }
    }

    impl riviu_core::FrameSink for TestSink {
        fn generation(&self, udid: &str) -> u64 {
            self.generations.lock().get(udid).copied().unwrap_or(0)
        }

        fn clear_and_advance(&self, udid: &str) -> u64 {
            self.cleared.fetch_add(1, Ordering::Relaxed);
            self.advance(udid)
        }

        fn park_and_advance(&self, udid: &str) -> u64 {
            self.parked.fetch_add(1, Ordering::Relaxed);
            self.advance(udid)
        }

        fn publish_if_current(&self, _udid: &str, _generation: u64, _jpeg: Vec<u8>) -> bool {
            true
        }
    }

    /// A driver whose adb path does not exist.
    ///
    /// Used to prove *ordering*: if a method returns an error on such a driver, it
    /// refused before attempting any adb call.
    fn driver_with_unrunnable_adb() -> AndroidDriver {
        let mut driver = AndroidDriver::new(&AndroidDriverConfig::default()).expect("driver");
        driver.adb = AdbProgram::unrunnable_for_test(PathBuf::from(
            "C:/riviu-nonexistent/definitely-not-adb.exe",
        ));
        driver
    }

    fn wired(sink: &Arc<TestSink>) -> AndroidDriver {
        let driver = driver_with_unrunnable_adb();
        driver.set_frame_sink(Arc::clone(sink) as Arc<dyn riviu_core::FrameSink>);
        driver
    }

    #[tokio::test]
    async fn stop_with_no_producer_still_confirms_the_stop() {
        // The trap: the control plane's `confirms_stop` needs
        // `child_stopped && new > old`, and `clean_ticket` quarantines the whole
        // lease when it does not hold. Reporting `false` for "there was nothing to
        // stop" would quarantine every teardown after a failed stream start.
        let sink = Arc::new(TestSink::default());
        let driver = wired(&sink);

        let proof = driver.stop_owned_stream("fixture").await.expect("stop");

        assert!(
            proof.child_stopped,
            "an absent producer is a stopped producer"
        );
        assert_eq!((proof.old_generation, proof.new_generation), (0, 1));
        assert!(proof.child_stopped && proof.new_generation > proof.old_generation);
    }

    #[tokio::test]
    async fn two_stops_advance_one_generation_each() {
        let sink = Arc::new(TestSink::default());
        let driver = wired(&sink);

        let first = driver.stop_owned_stream("fixture").await.expect("stop");
        let second = driver.stop_owned_stream("fixture").await.expect("stop");

        assert_eq!((first.old_generation, first.new_generation), (0, 1));
        assert_eq!((second.old_generation, second.new_generation), (1, 2));
    }

    #[tokio::test]
    async fn confirm_reads_the_generation_and_never_advances_it() {
        // AGENTS.md is explicit that the handoff read must not stop a process or
        // bump a generation. Without this the equality the plane checks would hold
        // only by luck.
        let sink = Arc::new(TestSink::default());
        let driver = wired(&sink);

        let first = driver
            .confirm_interaction_stream_stopped("fixture")
            .await
            .expect("handoff");
        let second = driver
            .confirm_interaction_stream_stopped("fixture")
            .await
            .expect("a repeated handoff is idempotent");

        assert_eq!(first.generation, second.generation);
        assert_eq!(sink.cleared.load(Ordering::Relaxed), 0);
        assert_eq!(sink.parked.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn confirm_after_stop_matches_the_stop_proof() {
        // This is the equality `start_reserved_stream` enforces.
        let sink = Arc::new(TestSink::default());
        let driver = wired(&sink);

        let stop = driver.stop_owned_stream("fixture").await.expect("stop");
        let handoff = driver
            .confirm_interaction_stream_stopped("fixture")
            .await
            .expect("handoff");

        assert_eq!(handoff.generation, stop.new_generation);
    }

    #[tokio::test]
    async fn park_keeps_the_last_frame_and_still_advances() {
        let sink = Arc::new(TestSink::default());
        let driver = wired(&sink);

        let proof = driver.park_owned_stream("fixture").await.expect("park");

        assert!(proof.new_generation > proof.old_generation);
        assert_eq!(sink.parked.load(Ordering::Relaxed), 1);
        assert_eq!(
            sink.cleared.load(Ordering::Relaxed),
            0,
            "park must not clear the tile's cached frame"
        );
    }

    #[tokio::test]
    async fn start_stream_without_a_confirm_is_refused_before_any_adb_call() {
        // The driver's adb path does not exist, so reaching adb would surface a
        // spawn error instead of this message.
        let sink = Arc::new(TestSink::default());
        let driver = wired(&sink);

        let error = driver
            .start_stream_after_session("fixture")
            .await
            .expect_err("no session reservation");

        assert!(error.to_string().contains("session reservation"), "{error}");
    }

    #[tokio::test]
    async fn the_handoff_and_the_start_refuse_when_no_frame_sink_is_wired() {
        // A harness that forgot `set_frame_sink` must be told so, not handed a
        // fabricated generation from some driver-local counter.
        let driver = driver_with_unrunnable_adb();

        for message in [
            driver
                .confirm_interaction_stream_stopped("fixture")
                .await
                .expect_err("no sink")
                .to_string(),
            driver
                .start_stream_after_session("fixture")
                .await
                .expect_err("no sink")
                .to_string(),
            driver
                .stop_owned_stream("fixture")
                .await
                .expect_err("no sink")
                .to_string(),
            driver
                .park_owned_stream("fixture")
                .await
                .expect_err("no sink")
                .to_string(),
        ] {
            assert!(message.contains("frame sink"), "{message}");
        }
    }

    #[tokio::test]
    async fn a_start_claim_blocks_a_second_start_and_is_released_on_drop() {
        let driver = driver_with_unrunnable_adb();

        let claim = driver.claim_start("fixture").expect("first claim");
        assert!(driver.claim_start("fixture").is_err());
        // A different serial is unaffected — the claim is per device, not a fleet
        // lock, which is the whole point of replacing the shared map lock.
        let other = driver.claim_start("other").expect("a different serial");
        drop(other);
        drop(claim);
        driver.claim_start("fixture").expect("released on drop");
    }

    #[tokio::test]
    async fn a_handoff_is_refused_while_a_start_is_in_flight() {
        // A producer being born would publish into the generation the handoff is
        // about to hand out.
        let sink = Arc::new(TestSink::default());
        let driver = wired(&sink);
        let _claim = driver.claim_start("fixture").expect("claim");

        let error = driver
            .confirm_interaction_stream_stopped("fixture")
            .await
            .expect_err("a start is in flight");

        assert!(error.to_string().contains("in flight"), "{error}");
    }

    #[tokio::test]
    async fn an_interaction_session_refuses_an_invalid_package_before_touching_the_phone() {
        let sink = Arc::new(TestSink::default());
        let driver = wired(&sink);

        let error = driver
            .start_interaction_session(
                "fixture",
                "com.x; rm -rf /sdcard/DCIM",
                riviu_core::InteractionSessionKind::Ordinary,
            )
            .await
            .err()
            .expect("a package name that is really a shell command must be refused");

        assert!(error.to_string().contains("package name"), "{error}");
    }

    #[tokio::test]
    async fn an_interaction_session_requires_the_handoff_first() {
        let sink = Arc::new(TestSink::default());
        let driver = wired(&sink);

        let error = driver
            .start_interaction_session(
                "fixture",
                "com.ss.android.ugc.trill",
                riviu_core::InteractionSessionKind::Ordinary,
            )
            .await
            .err()
            .expect("a session without a recorded handoff must be refused");

        assert!(error.to_string().contains("stop_owned_stream"), "{error}");
    }

    #[tokio::test]
    async fn stopping_a_producer_does_not_forget_which_preset_the_operator_asked_for() {
        // Regression, observed live: the watchdog's restart path is stop-then-start
        // (apps/desktop/src-tauri/src/state.rs), and `stop_view_stream` used to clear the
        // desired preset. So every restart read the default back and an open overlay
        // dropped to the tile encode -- logged as `gen=5 tile 216x480` with the overlay
        // still on screen. Asserted here rather than in the desktop crate because this is
        // the invariant the desktop relies on.
        let driver = AndroidDriver::new(&AndroidDriverConfig::default()).expect("driver");
        assert_eq!(
            driver.desired_view_preset("serial-a"),
            crate::scrcpy::ViewPreset::Tile,
            "a device never asked for anything must restart as a tile"
        );

        driver
            .desired_presets
            .lock()
            .insert("serial-a".to_string(), crate::scrcpy::ViewPreset::Overlay);

        // No producer is running, so this is the cheap half of stop-then-start; what
        // matters is that it leaves the recorded desire alone.
        driver.stop_view_stream("serial-a").await;
        assert_eq!(
            driver.desired_view_preset("serial-a"),
            crate::scrcpy::ViewPreset::Overlay,
            "stopping the producer must not change what the operator asked for"
        );

        // Closing the overlay overwrites rather than clears -- that is the only path back.
        driver
            .desired_presets
            .lock()
            .insert("serial-a".to_string(), crate::scrcpy::ViewPreset::Tile);
        assert_eq!(
            driver.desired_view_preset("serial-a"),
            crate::scrcpy::ViewPreset::Tile
        );
    }

    #[test]
    fn half_an_agent_is_no_agent() {
        // Both halves or neither. The runner lives in the `androidTest` APK and
        // `am instrument` names it, so a device with only the server installed refuses
        // exactly as if it had nothing -- and whoever debugs that goes looking at the
        // half that IS installed. `zip` makes the pair unrepresentable when one is
        // missing, so the refusal happens at construction with a message that says so.
        let only_server = AndroidDriverConfig {
            agent_server_apk: Some(PathBuf::from("server.apk")),
            ..Default::default()
        };
        assert!(
            AndroidDriver::new(&only_server)
                .expect("driver")
                .agent_apks
                .is_none(),
            "a server APK without its androidTest half must not look installable"
        );

        let only_test = AndroidDriverConfig {
            agent_test_apk: Some(PathBuf::from("test.apk")),
            ..Default::default()
        };
        assert!(
            AndroidDriver::new(&only_test)
                .expect("driver")
                .agent_apks
                .is_none(),
            "the androidTest half alone is not an agent either"
        );

        let both = AndroidDriverConfig {
            agent_server_apk: Some(PathBuf::from("server.apk")),
            agent_test_apk: Some(PathBuf::from("test.apk")),
            ..Default::default()
        };
        assert_eq!(
            AndroidDriver::new(&both).expect("driver").agent_apks,
            Some((PathBuf::from("server.apk"), PathBuf::from("test.apk")))
        );
    }

    #[test]
    fn each_device_gets_its_own_forwarded_port() {
        let driver = AndroidDriver::new(&AndroidDriverConfig::default()).expect("driver");
        let first = driver.host_port("serial-a");
        let second = driver.host_port("serial-b");
        assert_ne!(first, second);
        // Stable across calls: a session reconnecting must reach the same agent.
        assert_eq!(driver.host_port("serial-a"), first);
        assert_eq!(driver.host_port("serial-b"), second);
        assert_eq!(first, HOST_PORT_BASE);
    }

    #[tokio::test]
    async fn readiness_is_false_until_we_forward_that_device_ourselves() {
        // The bug this guards: allocating a port does not forward it, and the
        // port may already belong to another device's agent. Probing it then
        // reports a phone with no agent as ready.
        let driver = AndroidDriver::new(&AndroidDriverConfig::default()).expect("driver");
        let _ = driver.host_port("never-forwarded");
        assert!(!driver.agent_ready("never-forwarded").await);
    }

    #[test]
    fn the_field_separator_survives_the_device_shell() {
        // It is echoed by the shell on the phone, so any metacharacter changes
        // what runs. `--8<--` silently turned into a redirection and every
        // field collapsed into the first one.
        assert!(
            FIELD_SEPARATOR
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_'),
            "separator {FIELD_SEPARATOR:?} contains a character the device shell may interpret"
        );
        assert!(!FIELD_SEPARATOR.is_empty());
    }

    #[test]
    fn inventory_splits_into_release_model_and_battery() {
        let stdout = format!(
            "9\n{sep}\nSM-G955N\n{sep}\n  level: 58\n  scale: 100\n",
            sep = FIELD_SEPARATOR
        );
        assert_eq!(
            parse_inventory(&stdout),
            Inventory {
                release: Some("9".into()),
                model: Some("SM-G955N".into()),
                battery: Some(58),
            }
        );
    }

    #[test]
    fn inventory_tolerates_missing_sections() {
        assert_eq!(parse_inventory(""), Inventory::default());
        let partial = format!("15\n{sep}\n\n{sep}\n", sep = FIELD_SEPARATOR);
        assert_eq!(
            parse_inventory(&partial),
            Inventory {
                release: Some("15".into()),
                model: None,
                battery: None,
            }
        );
    }

    #[tokio::test]
    async fn a_failing_server_is_quoted_from_both_of_its_pipes() {
        // scrcpy writes `Ln.i` to stdout and `Ln.w`/`Ln.e` to stderr, and this used to read
        // stderr only -- with stdout going to `Stdio::null()`, so the server's account of
        // what it chose was discarded at the source. A handshake that hangs rather than
        // exits then leaves no host-side evidence at all, which is how turning on the
        // control socket produced six minutes of silence (AGENTS.md 9.71).
        let mut command = tokio::process::Command::new(if cfg!(windows) { "cmd" } else { "sh" });
        if cfg!(windows) {
            command.args(["/c", "echo picked-encoder & echo refused 1>&2"]);
        } else {
            command.args(["-c", "echo picked-encoder; echo refused 1>&2"]);
        }
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn a process that writes to both pipes");

        let detail = scrcpy_exit_detail(&mut child).await;

        assert!(detail.contains("refused"), "stderr is quoted: {detail}");
        assert!(
            detail.contains("picked-encoder"),
            "stdout is quoted too -- that is the half that used to be thrown away: {detail}"
        );
        assert!(
            detail.contains("[out]") && detail.contains("[err]"),
            "{detail}"
        );
    }

    #[tokio::test]
    async fn a_server_that_says_nothing_adds_nothing_to_the_error() {
        // The detail is appended to a message that already reads as a sentence, so an empty
        // one must be genuinely empty rather than a dangling colon.
        let mut command = tokio::process::Command::new(if cfg!(windows) { "cmd" } else { "sh" });
        if cfg!(windows) {
            command.args(["/c", "exit 1"]);
        } else {
            command.args(["-c", "exit 1"]);
        }
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn a silent process");
        assert_eq!(scrcpy_exit_detail(&mut child).await, "");
    }

    #[test]
    fn a_permission_dialog_is_told_apart_from_a_phone_that_wandered_off() {
        // The two need different answers, which is the only reason this is a function
        // rather than a longer deadline. A dialog is cleared with Back; a launcher means
        // the launch did not take and Back would only make it worse.
        for dialog in [
            "com.google.android.packageinstaller",
            "com.android.packageinstaller",
            "com.android.permissioncontroller",
        ] {
            assert!(
                dialog_over_app(dialog),
                "{dialog} stands over the app — waiting out the deadline cannot clear it"
            );
        }
        for elsewhere in [
            "com.sec.android.app.launcher",
            "com.android.systemui",
            "com.ss.android.ugc.trill",
            "",
        ] {
            assert!(
                !dialog_over_app(elsewhere),
                "{elsewhere:?} is not a dialog over the app, and Back there is a guess"
            );
        }
    }
}
