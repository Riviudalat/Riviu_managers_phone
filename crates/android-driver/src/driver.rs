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
    /// serial -> the last thing we proved about its agent.
    ///
    /// `DeviceDriver::cached_agent_status` is synchronous and Flow's preflight reads it, so
    /// what the async paths learn has to be left somewhere a non-async reader can find it.
    /// The iOS driver keeps the same map for the same reason.
    agent_statuses: Mutex<HashMap<String, riviu_core::AgentStatus>>,
}

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

    /// Claim the exclusive right to start a producer for `serial`.
    fn claim_start(&self, serial: &str) -> anyhow::Result<StartClaim<'_>> {
        if !self.starting.lock().insert(serial.to_string()) {
            anyhow::bail!("a minicap start for {serial} is already in flight");
        }
        Ok(StartClaim {
            starting: &self.starting,
            serial: serial.to_string(),
        })
    }

    /// Refuse unless the driver owns no producer for `serial` and none is being
    /// born.
    ///
    /// Both halves matter: a producer *starting* would publish into the generation
    /// a handoff is about to hand out.
    async fn producer_absent(&self, serial: &str) -> anyhow::Result<()> {
        if self.starting.lock().contains(serial) {
            anyhow::bail!("a minicap start for {serial} is already in flight");
        }
        if self.streams.lock().await.contains_key(serial) {
            anyhow::bail!(
                "{serial} still owns a minicap producer; stop_owned_stream must run first"
            );
        }
        Ok(())
    }

    /// What size the producer's frames come out at.
    ///
    /// **Native, not half, and that is a correctness choice rather than a quality one.**
    ///
    /// Flow measures in device pixels. A compiled coordinate records the size of the image
    /// it was picked against, `flow::executor::validate_geometry` refuses to dispatch
    /// unless the runtime frame matches the device's qualified geometry, and
    /// `FrameRegionChanged` evidence names a rectangle in frame pixels. This producer ran
    /// at `Projection::half` from the start, so on a 1080x2220 phone every frame was
    /// 540x1110 and that check could never pass: image-coordinate taps and the Flow
    /// inspector's coordinate picker were both unreachable on Android no matter what else
    /// was fixed.
    ///
    /// Nothing pays for this that was not already paying. The Android tile grid does not use
    /// minicap at all -- it is on the H.264 view path -- and `background_sample_candidate`
    /// returns false for Android, so the only consumers of these frames are the ones that
    /// measure them. The AI comment path is unaffected in either direction:
    /// `openai_client::make_contact_sheet` resizes every frame to 375x667 before a provider
    /// sees it, so the token bill does not depend on what the phone captured.
    ///
    /// If Android tiles ever move back onto minicap, this is the line to revisit: half the
    /// edge is a quarter of the bytes, and twenty tiles is where that mattered.
    fn producer_projection(screen: (u32, u32)) -> crate::frames::Projection {
        crate::frames::Projection::native(screen.0, screen.1)
    }

    /// Spawn minicap for `serial`, publishing into exactly `generation`.
    ///
    /// Never advances a generation and never holds a lock across the adb work. The
    /// step order is the port-hygiene contract: the APK push happens before any
    /// port is taken, the forward happens exactly once, and the producer is only
    /// registered at the very end so a failed start leaves nothing to clean up.
    ///
    /// Returns whether a decoded frame was observed — always `false` for
    /// [`StreamReadiness::BestEffort`], which does not wait for one.
    async fn spawn_producer(
        &self,
        serial: &str,
        generation: u64,
        readiness: StreamReadiness,
    ) -> anyhow::Result<bool> {
        let sink = self.sink()?;
        let apk = self.minicap_apk.clone().ok_or_else(|| {
            anyhow!(
                "no minicap apk configured: set RIVIU_MINICAP_APK to DeviceFarmer's \
                 noarch/minicap.apk (AGENTS.md 9)"
            )
        })?;

        let screen = crate::frames::device_screen(&self.adb, serial).await?;
        let options =
            crate::frames::MinicapOptions::for_device(serial, Self::producer_projection(screen));
        // Push before taking a port, so a push failure strands nothing.
        crate::frames::ensure_apk(&self.adb, serial, &apk).await?;

        if readiness == StreamReadiness::DecodedFrame {
            self.refuse_undrivable_screen(serial).await?;
        }

        let mut child = tokio::process::Command::new(self.adb.path())
            .args([
                "-s",
                serial,
                "shell",
                &crate::frames::launch_command(&options),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("spawn minicap")?;

        // Forward exactly once. `adb forward tcp:0` allocates a *new* host port on
        // every call, so retrying the forward alongside the connect leaks one port
        // per attempt — measured: four stranded forwards to the same socket after
        // a single launch. Only the connect is retried, because minicap binds its
        // socket a beat after `app_process` starts.
        let host_port = crate::frames::forward(&self.adb, serial, &options.socket).await?;
        let mut connected = None;
        let mut last_error = None;
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(250)).await;
            if child.try_wait().ok().flatten().is_some() {
                crate::frames::remove_forward(&self.adb, serial, host_port)
                    .await
                    .ok();
                anyhow::bail!("minicap exited before it accepted a connection");
            }
            match crate::frames::MinicapStream::connect(host_port).await {
                Ok(stream) => {
                    connected = Some(stream);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let Some(mut stream) = connected else {
            // Give the port back before surfacing the failure.
            crate::frames::remove_forward(&self.adb, serial, host_port)
                .await
                .ok();
            let _ = child.kill().await;
            return Err(
                last_error.unwrap_or_else(|| anyhow!("minicap never accepted a connection"))
            );
        };
        let banner = stream.banner().clone();
        tracing::info!(
            serial,
            host_port,
            generation,
            ?readiness,
            banner = ?banner,
            "minicap frame source started"
        );

        // The interaction path needs to know a real frame landed. A oneshot rather
        // than polling the hub: a *parked* frame from before this producer is still
        // in the hub's cache, so watching the cache would accept a pre-session frame
        // as proof of a stream started after it.
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let mut ready_tx = (readiness == StreamReadiness::DecodedFrame).then_some(ready_tx);
        let udid = serial.to_string();
        let publisher = Arc::clone(&sink);
        let reader = tokio::spawn(async move {
            let sink = publisher;
            loop {
                match stream.next_frame().await {
                    Ok(frame) => {
                        // A frame that does not decode is skipped, not published,
                        // while we are still waiting for the first one.
                        let qualifies =
                            ready_tx.is_some() && riviu_core::frame_source::decodes_as_jpeg(&frame);
                        if ready_tx.is_some() && !qualifies {
                            tracing::debug!(
                                udid,
                                generation,
                                bytes = frame.len(),
                                "skipping an undecodable candidate first frame"
                            );
                            continue;
                        }
                        // A stale generation is the signal to stop, not an error:
                        // a newer stream owns this device now.
                        if !sink.publish_if_current(&udid, generation, frame) {
                            tracing::info!(udid, generation, "minicap reader superseded; stopping");
                            return;
                        }
                        if qualifies {
                            if let Some(sender) = ready_tx.take() {
                                let _ = sender.send(());
                            }
                        }
                    }
                    Err(error) => {
                        tracing::warn!(udid, generation, %error, "minicap reader stopped");
                        return;
                    }
                }
            }
        });

        let mut first_frame_observed = false;
        if readiness == StreamReadiness::DecodedFrame {
            let started = std::time::Instant::now();
            match tokio::time::timeout(INTERACTION_FIRST_FRAME_TIMEOUT, ready_rx).await {
                Ok(Ok(())) => {
                    first_frame_observed = true;
                    tracing::info!(
                        serial,
                        generation,
                        ms = started.elapsed().as_millis(),
                        "minicap first decoded frame accepted"
                    );
                }
                // Tear down rather than reporting `Ok(first_frame_observed: false)`:
                // every consumer in core treats `false` as fatal, so returning it
                // would buy nothing except a live orphan producer.
                _ => {
                    reader.abort();
                    let _ = child.kill().await;
                    crate::frames::remove_forward(&self.adb, serial, host_port)
                        .await
                        .ok();
                    anyhow::bail!(
                        "minicap produced no decodable frame in {}s for {serial}: the display may \
                         not have changed (minicap publishes on change), or the projection is \
                         wrong. banner {}x{} virtual {}x{} orient {}, host port {host_port}, \
                         sink generation {}",
                        INTERACTION_FIRST_FRAME_TIMEOUT.as_secs(),
                        banner.real_width,
                        banner.real_height,
                        banner.virtual_width,
                        banner.virtual_height,
                        banner.orientation,
                        sink.generation(serial)
                    );
                }
            }
        }

        // Registered last, so nothing above needs undoing on failure.
        self.streams.lock().await.insert(
            serial.to_string(),
            StreamProducer {
                generation,
                host_port,
                child,
                reader,
                device_pid: banner.pid,
            },
        );
        Ok(first_frame_observed)
    }

    /// What the phone says about its own display, for a caller that needs to explain
    /// itself rather than act.
    ///
    /// `None` is unknown, never "asleep": this fleet spans Android 9 to 15 and they do
    /// not print the same `dumpsys` bodies. Exposed because the desktop's view watchdog
    /// logs through `log` while this crate emits `tracing`, which currently reaches no
    /// sink — so the only way an operator sees *why* a view went silent is for the app
    /// layer to ask and say it.
    pub async fn display_is_awake(&self, serial: &str) -> Option<bool> {
        let power = self.adb.shell(serial, "dumpsys power").await.ok()?;
        adb::parse_display_awake(&power)
    }

    /// Wake the screen before capturing it, because a sleeping one encodes nothing.
    ///
    /// [`Self::refuse_undrivable_screen`] already knew this for minicap and *refuses*;
    /// the view path must not, and the difference is the caller. Nurture is asking to
    /// drive a phone and a refusal sends the operator to unlock it. The tile grid is
    /// asking to watch every phone at once: refusing there gives a black tile and a
    /// watchdog that restarts the encoder every five seconds forever, which is exactly
    /// what a sleeping Redmi did on 14/08/2026 until one keyevent fixed it.
    ///
    /// Best effort on purpose. A phone that will not wake may still have a screen worth
    /// capturing, and trading a working tile for none because a keyevent failed is a
    /// worse outcome than a dim one. Logged at info when the display really was asleep,
    /// so the watchdog's "published nothing" line has a cause next to it instead of
    /// repeating anonymously.
    async fn wake_display_for_capture(&self, serial: &str) {
        let awake = match self.adb.shell(serial, "dumpsys power").await {
            Ok(power) => adb::parse_display_awake(&power),
            Err(_) => None,
        };
        if !adb::should_wake_before_capture(awake) {
            return;
        }
        match self.adb.shell(serial, adb::WAKE_KEYEVENT).await {
            Ok(_) => {
                if awake == Some(false) {
                    tracing::info!(%serial, "display was asleep; woke it before capturing");
                }
            }
            Err(error) => {
                tracing::warn!(%serial, %error, "could not wake before capturing");
            }
        }
    }

    /// Refuse a screen minicap cannot compose from, before anything is spawned.
    ///
    /// Two separate conditions, and the second is the one that bites: measured on a
    /// locked Redmi Note 12 (11/08/2026), `dumpsys power` reported
    /// `mWakefulness=Awake` and `mScreenOnFully=true` while the phone sat on its
    /// lock screen and nothing could be foregrounded. Wakefulness alone passes a
    /// phone no driver can drive.
    ///
    /// An unreadable `dumpsys` is **unknown**, never a refusal — the fleet spans
    /// Android 9 to 15 and they do not print the same bodies.
    async fn refuse_undrivable_screen(&self, serial: &str) -> anyhow::Result<()> {
        if let Ok(power) = self.adb.shell(serial, "dumpsys power").await {
            if adb::parse_display_awake(&power) == Some(false) {
                anyhow::bail!(
                    "{serial} has its display asleep; minicap composes nothing while the screen \
                     is off. Wake the phone and retry"
                );
            }
        }
        if let Ok(window) = self.adb.shell(serial, "dumpsys window").await {
            if adb::parse_keyguard_locked(&window) == Some(true) {
                anyhow::bail!(
                    "{serial} is on the lock screen. The screen may be on, but no app can be \
                     brought to the foreground until it is unlocked"
                );
            }
        }
        Ok(())
    }

    /// Kill a feed and drop its forward. Best effort by design: the caller has
    /// already removed it from the registry, so failing here must not strand the
    /// device with a producer nobody owns.
    async fn stop_producer(&self, serial: &str, mut producer: StreamProducer) -> bool {
        producer.reader.abort();
        // Ignore the kill error: the child may already have been reaped by an
        // earlier `try_wait`, and that is a stopped child, not a failure.
        let _ = producer.child.start_kill();
        let confirmed = matches!(
            tokio::time::timeout(CHILD_EXIT_TIMEOUT, producer.child.wait()).await,
            Ok(Ok(_))
        );
        if !confirmed {
            tracing::warn!(
                serial,
                device_pid = producer.device_pid,
                "could not confirm the minicap child exited"
            );
        }
        if let Err(error) =
            crate::frames::remove_forward(&self.adb, serial, producer.host_port).await
        {
            tracing::warn!(serial, port = producer.host_port, %error, "could not remove the minicap forward");
        }
        confirmed
    }

    /// Remove whatever producer we own for `serial` and kill it.
    ///
    /// `true` means the driver is confirmed to own no live producer afterwards —
    /// **including when it owned none to begin with**. That is not laxity: the
    /// control plane's `StreamStopProof::confirms_stop` requires
    /// `child_stopped && new > old`, and reporting `false` for "there was nothing to
    /// stop" would quarantine the lease on every teardown that follows a failed
    /// stream start. iOS answers the same way.
    async fn take_and_stop_producer(&self, serial: &str) -> bool {
        let producer = self.streams.lock().await.remove(serial);
        match producer {
            Some(producer) => self.stop_producer(serial, producer).await,
            None => true,
        }
    }

    /// The one place a teardown advances a generation.
    ///
    /// `retain_last_frame` distinguishes park from stop: both must make every frame
    /// the dead producer still holds unpublishable, but park keeps the tile's last
    /// image instead of blanking it.
    async fn teardown_stream(
        &self,
        serial: &str,
        retain_last_frame: bool,
    ) -> anyhow::Result<riviu_core::stream_budget::StreamStopProof> {
        let sink = self.sink()?;
        let child_stopped = self.take_and_stop_producer(serial).await;
        // Read the old generation separately: `FrameSink` returns only the new one,
        // deliberately. Safe because every advance for this serial happens either in
        // the producer-map critical section or under a start claim, and the control
        // plane holds a per-UDID operation lock across the whole sequence.
        let old_generation = sink.generation(serial);
        let new_generation = if retain_last_frame {
            sink.park_and_advance(serial)
        } else {
            sink.clear_and_advance(serial)
        };
        if child_stopped {
            // Recording the stop lets the plane's recovery path start a session
            // straight after a stop without confirming the handoff again.
            self.interaction.record_stopped(serial, new_generation);
        } else {
            self.interaction.clear(serial);
        }
        Ok(riviu_core::stream_budget::StreamStopProof {
            old_generation,
            new_generation,
            child_stopped,
        })
    }

    /// Start or reuse the tile feed for one device.
    ///
    /// Reuses a live producer whose generation is still current, which is what keeps
    /// a repeated `ensure_stream` from restarting a working stream — the same rule
    /// the iOS path follows.
    async fn ensure_minicap_locked(&self, serial: &str) -> anyhow::Result<()> {
        let sink = self.sink()?;
        let claim = self.claim_start(serial)?;

        let reusable = {
            let mut streams = self.streams.lock().await;
            match streams.get_mut(serial) {
                Some(existing) => {
                    let alive = existing
                        .child
                        .try_wait()
                        .map(|status| status.is_none())
                        .unwrap_or(false);
                    alive
                        && existing.generation == sink.generation(serial)
                        && !existing.reader.is_finished()
                }
                None => false,
            }
        };
        if reusable {
            return Ok(());
        }
        // Whatever is there is stale; killing it happens outside the map lock.
        self.take_and_stop_producer(serial).await;

        let generation = sink.clear_and_advance(serial);
        let started = self
            .spawn_producer(serial, generation, StreamReadiness::BestEffort)
            .await;
        drop(claim);
        started.map(|_| ())
    }

    /// Stop the feed for one device, if we own one.
    pub async fn stop_minicap(&self, serial: &str) {
        self.take_and_stop_producer(serial).await;
    }

    fn view_sink(&self) -> anyhow::Result<Arc<dyn crate::view::ViewSink>> {
        self.view_sink.lock().clone().ok_or_else(|| {
            anyhow!(
                "no view sink is wired to the Android driver; call set_view_sink before \
                 starting a view stream"
            )
        })
    }

    fn claim_view_start(&self, serial: &str) -> anyhow::Result<StartClaim<'_>> {
        if !self.view_starting.lock().insert(serial.to_string()) {
            anyhow::bail!("a scrcpy view start for {serial} is already in flight");
        }
        Ok(StartClaim {
            starting: &self.view_starting,
            serial: serial.to_string(),
        })
    }

    /// A `start_view_stream` that still holds the claim. The keeper must not
    /// treat this as a silent producer — there has been no packet yet.
    pub fn view_start_in_flight(&self, serial: &str) -> bool {
        self.view_starting.lock().contains(serial)
    }

    /// Live producer at either preset, or a start that still holds the claim.
    /// The desktop keeper must not spawn a tile while overlay retune is mid-flight.
    pub async fn view_is_active(&self, serial: &str) -> bool {
        if self.view_start_in_flight(serial) {
            return true;
        }
        self.view_is_running(serial, crate::scrcpy::ViewPreset::Tile)
            .await
            || self
                .view_is_running(serial, crate::scrcpy::ViewPreset::Overlay)
                .await
    }

    /// Whether this serial already has a live view at `preset`.
    pub async fn view_is_running(&self, serial: &str, preset: crate::scrcpy::ViewPreset) -> bool {
        let mut views = self.views.lock().await;
        match views.get_mut(serial) {
            Some(existing) => {
                let alive = existing
                    .child
                    .try_wait()
                    .map(|status| status.is_none())
                    .unwrap_or(false);
                alive
                    && existing.preset == preset
                    && !existing.reader.is_finished()
                    && existing.generation > 0
            }
            None => false,
        }
    }

    /// Start or retune the scrcpy view. Same process, new options.
    ///
    /// A producer that is already painting is **kept until the replacement has a keyframe**
    /// (see [`ViewStart`]) rather than stopped up front. That is what the operator feels when
    /// they open a phone: the picture keeps moving through the switch instead of freezing for
    /// the length of a spawn. Does not touch minicap or `StreamBudgetManager`.
    pub async fn start_view_stream(
        &self,
        serial: &str,
        preset: crate::scrcpy::ViewPreset,
    ) -> anyhow::Result<u64> {
        let sink = self.view_sink()?;
        let claim = self.claim_view_start(serial)?;
        self.desired_presets
            .lock()
            .insert(serial.to_string(), preset);
        if self.view_is_running(serial, preset).await {
            return Ok(sink.generation(serial));
        }
        // "Is something alive on this serial" rather than "is it at the preset we want":
        // anything still running is a picture worth keeping until the new one is proven.
        let replacing = self.views.lock().await.contains_key(serial);
        let start = if replacing {
            ViewStart::Replace
        } else {
            ViewStart::Fresh {
                generation: sink.advance(serial),
            }
        };
        self.spawn_view(serial, start, preset).await?;
        drop(claim);
        Ok(sink.generation(serial))
    }

    /// Stop the view for one serial. `true` when nothing is left running,
    /// including when there was nothing to stop.
    pub async fn stop_view_stream(&self, serial: &str) -> bool {
        // Deliberately does NOT forget the desired preset. Measured: it used to, and the
        // watchdog's restart path is stop-then-start (state.rs), so every restart read back
        // the default and an open overlay silently dropped to the tile encode -- observed
        // live as `gen=5 tile 216x480` while the overlay was still on screen.
        //
        // The desire belongs to the operator having an overlay open, not to a producer's
        // lifetime. It is overwritten, never cleared: closing the overlay asks for `tile`,
        // which is the same insert.
        self.take_and_stop_view(serial).await
    }

    /// What this serial should be restarted at. `Tile` for anything never asked for, which
    /// is the pre-existing behaviour for every device the operator has not opened.
    pub fn desired_view_preset(&self, serial: &str) -> crate::scrcpy::ViewPreset {
        self.desired_presets
            .lock()
            .get(serial)
            .copied()
            .unwrap_or(crate::scrcpy::ViewPreset::Tile)
    }

    /// Set the quality and frame rate new views will start with.
    ///
    /// Does **not** touch running producers. Restarting sixteen encoders because a
    /// slider moved is a fleet-wide stall the operator did not ask for, so the caller
    /// decides which views to restart and when — see `set_view_preset`.
    pub fn set_view_tuning(
        &self,
        grid: riviu_core::StreamQuality,
        focus: riviu_core::StreamQuality,
        fps: u32,
    ) {
        *self.view_tuning.lock() = ViewTuningChoice { grid, focus, fps };
    }

    /// Retune by restarting the same producer. Not a second `app_process`.
    pub async fn set_view_preset(
        &self,
        serial: &str,
        preset: crate::scrcpy::ViewPreset,
    ) -> anyhow::Result<u64> {
        self.start_view_stream(serial, preset).await
    }

    pub async fn stop_all_views(&self) {
        let serials: Vec<String> = self.views.lock().await.keys().cloned().collect();
        for serial in serials {
            self.take_and_stop_view(&serial).await;
        }
    }

    async fn take_and_stop_view(&self, serial: &str) -> bool {
        let producer = self.views.lock().await.remove(serial);
        match producer {
            Some(producer) => self.stop_view_producer(serial, producer).await,
            None => true,
        }
    }

    async fn stop_view_producer(&self, serial: &str, mut producer: ViewProducer) -> bool {
        producer.reader.abort();
        // The control socket goes first, and shut down rather than merely dropped.
        // `DesktopConnection.shutdown` on the device closes all three sockets; giving its
        // reader a clean EOF is what stops a teardown that races a `write_all` from leaving
        // a half-written message behind — and a half-written message on this stream is not a
        // lost byte, it is `ControlProtocolException` on a server we are about to kill
        // anyway, but which would log a fatal error and confuse the next reader of the log.
        producer.control_drain.abort();
        if let Ok(mut socket) = producer.control.try_lock() {
            let _ = socket.shutdown().await;
        }
        let _ = producer.child.start_kill();
        let confirmed = matches!(
            tokio::time::timeout(CHILD_EXIT_TIMEOUT, producer.child.wait()).await,
            Ok(Ok(_))
        );
        if !confirmed {
            tracing::warn!(serial, "could not confirm the scrcpy child exited");
        }
        if let Err(error) =
            crate::frames::remove_forward(&self.adb, serial, producer.host_port).await
        {
            tracing::warn!(
                serial,
                port = producer.host_port,
                %error,
                "could not remove the scrcpy forward"
            );
        }
        confirmed
    }

    /// Kill leftover *our* 3.3.4 rows. The encoder argv has `Server 3.3.4`
    /// and not the JAR path (`CLASSPATH` is environ). A grep for the JAR
    /// only hits the `sh -c` wrapper and leaves OMX held — Note 8 then
    /// hellos without an IDR. Never match GenFarmer (`Server 2.4`).
    async fn stop_our_scrcpy_leftovers(&self, serial: &str) {
        let listing = self
            .adb
            .shell(serial, crate::scrcpy::LEFTOVER_LIST_SCRIPT)
            .await
            .unwrap_or_default();
        let mut unique = Vec::new();
        for pid in listing
            .split_whitespace()
            .filter_map(|token| token.parse::<u32>().ok())
            .filter(|pid| *pid > 0)
        {
            if !unique.contains(&pid) {
                unique.push(pid);
            }
        }
        for pid in &unique {
            let _ = self.adb.shell(serial, &format!("kill {pid}")).await;
        }
        if unique.is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;

        // Confirm, then escalate. `kill` is SIGTERM, and a server blocked inside
        // MediaCodec does not have to honour it -- measured on the Redmi, two
        // `app_process` were still holding the encoder after this function had already
        // run and reported nothing, because it never looked again. A survivor is not
        // harmless: it keeps the hardware encoder, so the fresh server we are about to
        // start fails `MediaCodec.configure` and the tile stays black.
        //
        // One escalation, not a loop: if SIGKILL does not take, the process is unkillable
        // by us and retrying cannot change that, so say so and let the spawn attempt
        // produce the real error.
        let survivors = self
            .adb
            .shell(serial, crate::scrcpy::LEFTOVER_LIST_SCRIPT)
            .await
            .unwrap_or_default();
        let survivors: Vec<u32> = survivors
            .split_whitespace()
            .filter_map(|token| token.parse::<u32>().ok())
            .filter(|pid| *pid > 0 && unique.contains(pid))
            .collect();
        if survivors.is_empty() {
            return;
        }
        tracing::warn!(
            serial,
            ?survivors,
            "scrcpy server ignored SIGTERM; sending SIGKILL"
        );
        for pid in &survivors {
            let _ = self.adb.shell(serial, &format!("kill -9 {pid}")).await;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    async fn spawn_view(
        &self,
        serial: &str,
        start: ViewStart,
        preset: crate::scrcpy::ViewPreset,
    ) -> anyhow::Result<()> {
        let sink = self.view_sink()?;
        let server = self.scrcpy_server.clone().ok_or_else(|| {
            anyhow!(
                "no scrcpy server configured: set RIVIU_SCRCPY_SERVER or ship \
                 sidecars/android/noarch/scrcpy-server (AGENTS.md 9.50)"
            )
        })?;

        // Read once per spawn: a producer keeps whatever tuning it started with, so a
        // settings change takes effect on the next restart rather than half-way through
        // an encode.
        let tuning = {
            let guard = self.view_tuning.lock();
            // The overlay is one phone filling a window; a tile is one of twenty. They are
            // different pictures at different sizes, so they get the operator's two separate
            // quality choices rather than sharing one.
            let quality = match preset {
                crate::scrcpy::ViewPreset::Tile => guard.grid.clone(),
                crate::scrcpy::ViewPreset::Overlay => guard.focus.clone(),
            };
            preset.tuned(quality, guard.fps)
        };

        // Timed step by step, because "a start takes about eleven seconds" is not something
        // anyone can act on. Measured on this fleet a preset switch left the operator with
        // **17.8 s of no frames at all** after double-clicking a phone, and the only way to
        // know which of these five adb round trips to attack is to charge each of them.
        let spawn_started = std::time::Instant::now();
        self.wake_display_for_capture(serial).await;
        let woke = spawn_started.elapsed();

        crate::scrcpy::ensure_server(&self.adb, serial, &server).await?;
        let served = spawn_started.elapsed();
        // NOT on the replace path, and this is load-bearing rather than an optimisation: the
        // sweep matches every 3.3.4 server of ours on the device, and on that path one of
        // them is the producer still painting the operator's screen. Sweeping here would kill
        // the picture we are going through all this to preserve.
        if matches!(start, ViewStart::Fresh { .. }) {
            self.stop_our_scrcpy_leftovers(serial).await;
        }
        let swept = spawn_started.elapsed();

        // Drop forwards left over from a run that never cleaned up. Every failure path
        // below removes its own forward, so this is not for the current process -- it is
        // for the previous one. `adb forward` lives in the adb server, so a crash, a
        // force-quit, or a kill that skips `stop_view_producer` leaves the forward behind
        // with nothing to remove it, and `prune_forwards` cannot find it because it
        // matches the socket name exactly while scrcpy randomises the `scid`. Measured
        // after several development restarts: five stranded forwards across two phones,
        // each to a dead socket, plus two orphaned `app_process` on one of them.
        //
        // `keep` is every port a live producer holds, which is what makes this safe to
        // run on a device that is already streaming into another window.
        let live_ports: Vec<u16> = self
            .views
            .lock()
            .await
            .values()
            .map(|producer| producer.host_port)
            .collect();
        crate::frames::prune_scrcpy_forwards(
            &self.adb,
            serial,
            crate::scrcpy::FORWARD_PREFIX,
            &live_ports,
        )
        .await;
        let pruned = spawn_started.elapsed();

        let scid = (rand::random::<u32>() & 0x7fff_ffff).max(1);
        // Device listens (`tunnel_forward`). Spawn first. This Windows adb
        // refuses the abstract socket if nothing is bound yet, so a TCP
        // opened before listen EOFs and never becomes the video socket.
        // Retry TCP only while dummy has not arrived (`NotListening`).
        let mut child = tokio::process::Command::new(self.adb.path());
        child
            .args([
                "-s",
                serial,
                "shell",
                &crate::scrcpy::launch_command(scid, tuning),
            ])
            .stdin(Stdio::null())
            // Piped, not null. `Ln.i` goes to FD 1, so discarding stdout threw away the
            // server's account of itself -- which encoder it chose, the `Device: [...]` line,
            // and `Video capture reset`. A handshake that hangs instead of exiting then left
            // no host-side evidence whatsoever; the one measured instance ran six minutes
            // with nothing logged (AGENTS.md 9.71).
            //
            // Safe against the obvious hazard: the pipe is only ever read by
            // `scrcpy_exit_detail`, which runs on failure paths and then the child is killed.
            // A healthy server logs a handful of lines at startup and then nothing, so the
            // pipe cannot fill in normal operation -- and if it ever did, the writer blocking
            // is the server, not this process.
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        child.creation_flags(0x0800_0000);
        let mut child = child.spawn().context("spawn scrcpy-server")?;

        let spawned = spawn_started.elapsed();
        let host_port = match crate::scrcpy::forward(&self.adb, serial, scid).await {
            Ok(port) => port,
            Err(error) => {
                let _ = child.kill().await;
                return Err(error);
            }
        };
        let forwarded = spawn_started.elapsed();
        let mut stream = None;
        let mut control = None;
        let mut last_error = None;
        for attempt in 0..40 {
            if child.try_wait().ok().flatten().is_some() {
                crate::frames::remove_forward(&self.adb, serial, host_port)
                    .await
                    .ok();
                anyhow::bail!(
                    "scrcpy-server exited before it accepted a connection{}",
                    scrcpy_exit_detail(&mut child).await
                );
            }
            match crate::scrcpy::ScrcpyStream::try_accept(host_port).await {
                Ok((accepted, accepted_control)) => {
                    stream = Some(accepted);
                    control = Some(accepted_control);
                    break;
                }
                Err(crate::scrcpy::AcceptError::NotListening(error)) => {
                    last_error = Some(error);
                    if attempt + 1 < 40 {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
                Err(crate::scrcpy::AcceptError::Protocol(error)) => {
                    crate::frames::remove_forward(&self.adb, serial, host_port)
                        .await
                        .ok();
                    // The server's own words BEFORE the kill, on this path too. A protocol
                    // failure is exactly the case where it usually has something to say and
                    // is usually still alive to say it.
                    let said = scrcpy_exit_detail(&mut child).await;
                    let _ = child.kill().await;
                    return Err(error.context(format!("scrcpy handshake failed{said}")));
                }
            }
        }
        let Some(mut stream) = stream else {
            crate::frames::remove_forward(&self.adb, serial, host_port)
                .await
                .ok();
            let said = scrcpy_exit_detail(&mut child).await;
            let _ = child.kill().await;
            let error = last_error.unwrap_or_else(|| anyhow!("scrcpy never accepted a connection"));
            return Err(error.context(format!(
                "scrcpy never accepted a connection after 40 attempts{said}"
            )));
        };
        // Set in the same arm as `stream`, so this cannot be reached without it.
        let control = control.expect("try_accept returns both sockets or neither");
        let first =
            match tokio::time::timeout(Duration::from_secs(8), stream.next_sync_sample()).await {
                Ok(Ok(sample)) => sample,
                Ok(Err(error)) => {
                    crate::frames::remove_forward(&self.adb, serial, host_port)
                        .await
                        .ok();
                    let said = scrcpy_exit_detail(&mut child).await;
                    let _ = child.kill().await;
                    return Err(error.context(format!("scrcpy stream failed{said}")));
                }
                Err(_) => {
                    crate::frames::remove_forward(&self.adb, serial, host_port)
                        .await
                        .ok();
                    let said = scrcpy_exit_detail(&mut child).await;
                    let _ = child.kill().await;
                    anyhow::bail!("scrcpy produced no keyframe after the hello{said}");
                }
            };

        // The swap point, and it is deliberately *here* rather than before the spawn.
        //
        // Everything above can fail, and until this line the producer the operator is
        // watching is untouched: a failed replacement costs them nothing, where the old
        // order left the device dark. From here on the new stream is proven -- it has a
        // keyframe in hand -- so the handover is a hand-off rather than a gamble.
        let generation = match start {
            ViewStart::Fresh { generation } => generation,
            ViewStart::Replace => {
                self.take_and_stop_view(serial).await;
                sink.advance(serial)
            }
        };
        let swapped = spawn_started.elapsed();

        tracing::info!(
            serial,
            host_port,
            generation,
            preset = preset.as_str(),
            codec = stream.hello.codec,
            device = %stream.hello.device_name,
            width = first.width,
            height = first.height,
            key = first.key,
            bytes = first.bytes.len(),
            idr = crate::scrcpy::annexb_has_idr(&first.bytes),
            sps = crate::scrcpy::annexb_has_sps(&first.bytes),
            // Cumulative, so each is "by the time this step finished". Differences are the
            // per-step cost; the total is what the operator waits when a preset switch takes
            // their picture away.
            wake_ms = woke.as_millis() as u64,
            jar_ms = served.as_millis() as u64,
            sweep_ms = swept.as_millis() as u64,
            prune_ms = pruned.as_millis() as u64,
            spawn_ms = spawned.as_millis() as u64,
            forward_ms = forwarded.as_millis() as u64,
            // How long the old producer kept painting before it was handed over. On a
            // replace this is the whole spawn, and it is time the operator spent looking at
            // a *live* picture rather than a frozen one.
            swap_ms = swapped.as_millis() as u64,
            replaced = matches!(start, ViewStart::Replace),
            total_ms = spawn_started.elapsed().as_millis() as u64,
            "scrcpy view started"
        );

        let udid = serial.to_string();
        let publisher = Arc::clone(&sink);
        let frame_size = Arc::new(AtomicU32::new(pack_frame_size(first.width, first.height)));
        let reader_frame_size = Arc::clone(&frame_size);
        let first_packet = crate::view::ViewPacket {
            udid: udid.clone(),
            generation,
            kind: crate::view::ViewKind::H264,
            width: first.width,
            height: first.height,
            key: first.key,
            bytes: first.bytes,
        };
        if !publisher.publish(first_packet) {
            crate::frames::remove_forward(&self.adb, serial, host_port)
                .await
                .ok();
            let _ = child.kill().await;
            anyhow::bail!("view sink refused the first scrcpy sample");
        }
        let reader = tokio::spawn(async move {
            loop {
                match stream.next_sample().await {
                    Ok(sample) => {
                        // Before publishing, not after: a touch that races the publish should
                        // see the newer size, because the *server* has already moved to it.
                        reader_frame_size.store(
                            pack_frame_size(sample.width, sample.height),
                            Ordering::Release,
                        );
                        let packet = crate::view::ViewPacket {
                            udid: udid.clone(),
                            generation,
                            kind: crate::view::ViewKind::H264,
                            width: sample.width,
                            height: sample.height,
                            key: sample.key,
                            bytes: sample.bytes,
                        };
                        if !publisher.publish(packet) {
                            tracing::info!(udid, generation, "scrcpy view superseded; stopping");
                            return;
                        }
                    }
                    Err(error) => {
                        tracing::warn!(udid, generation, %error, "scrcpy view reader stopped");
                        return;
                    }
                }
            }
        });

        // Split so the write half can live behind its own lock while a task reads the other
        // end. `into_split` rather than `split` because the two halves outlive this function
        // in different places.
        let (mut control_read, control_write) = control.into_split();
        let control_write = Arc::new(tokio::sync::Mutex::new(control_write));
        let drain_serial = serial.to_string();
        let control_drain = tokio::spawn(async move {
            let mut scratch = [0u8; 1024];
            // Read and discard, never parse. The only thing that arrives is a clipboard
            // notification we did not ask for; the one thing that would be fatal is
            // objecting to a message type we do not know, and a reader that never
            // interprets cannot object.
            while let Ok(read) = control_read.read(&mut scratch).await {
                if read == 0 {
                    break;
                }
            }
            tracing::debug!(serial = %drain_serial, "scrcpy control socket closed");
        });

        self.views.lock().await.insert(
            serial.to_string(),
            ViewProducer {
                generation,
                preset,
                host_port,
                child,
                reader,
                frame_size,
                control: control_write,
                control_drain,
            },
        );
        Ok(())
    }

    /// Put one touch event on the phone, in the coordinate space of the picture on screen.
    ///
    /// `image_w`/`image_h` are the dimensions the *caller* was looking at when the operator
    /// moved their finger. They are not passed on: the message declares this host's latest
    /// observed frame size and the coordinates are rescaled into it. The device compares the
    /// declared size against what it is encoding and drops the event outright when they
    /// differ, so a caller one generation behind would otherwise lose the touch entirely
    /// rather than land it a few pixels off.
    ///
    /// `Ok(false)` means no producer — the overlay is not streaming this phone, so there is
    /// nothing to touch and nothing has gone wrong.
    pub async fn inject_touch(
        &self,
        serial: &str,
        action: crate::scrcpy::TouchAction,
        x: f64,
        y: f64,
        image_w: f64,
        image_h: f64,
    ) -> anyhow::Result<bool> {
        if !(image_w > 0.0 && image_h > 0.0) {
            anyhow::bail!("touch needs the size of the picture it came from");
        }
        let (control, packed) = {
            let views = self.views.lock().await;
            match views.get(serial) {
                Some(producer) => (
                    Arc::clone(&producer.control),
                    producer.frame_size.load(Ordering::Acquire),
                ),
                None => return Ok(false),
            }
        };
        let (frame_w, frame_h) = unpack_frame_size(packed);
        if frame_w == 0 || frame_h == 0 {
            anyhow::bail!("no frame seen from {serial} yet");
        }
        // Clamped, because a pointer can leave the element between two samples and a
        // coordinate outside the picture is a coordinate outside the phone.
        let scaled_x = (x / image_w * f64::from(frame_w)).round();
        let scaled_y = (y / image_h * f64::from(frame_h)).round();
        let clamped_x = scaled_x.clamp(0.0, f64::from(frame_w - 1)) as i32;
        let clamped_y = scaled_y.clamp(0.0, f64::from(frame_h - 1)) as i32;
        let message = crate::scrcpy::inject_touch(action, clamped_x, clamped_y, frame_w, frame_h);
        let mut socket = control.lock().await;
        // ONE `write_all`, under the lock, for the same reason as RESET_VIDEO: the reader on
        // the device has no framing, so an interleaved write desynchronises it permanently
        // and takes the video down with it.
        socket
            .write_all(&message)
            .await
            .with_context(|| format!("send touch to {serial}"))?;
        socket
            .flush()
            .await
            .with_context(|| format!("flush touch to {serial}"))?;
        Ok(true)
    }

    /// Ask the phone for a fresh keyframe, without restarting anything.
    ///
    /// This is what the control socket is for. The alternative cure for a decoder that has
    /// stopped producing frames is a full producer restart, measured at ~11.5 s of black
    /// tile on this fleet; a keyframe request is one byte and the server answers by logging
    /// `Video capture reset` and emitting a fresh IDR. Measured over a 75 s soak: twelve
    /// requests, twelve resets, video flowing throughout.
    ///
    /// `Ok(false)` means there is no producer to ask — not a failure, just nothing to do.
    ///
    /// The `views` lock is released before the write. Holding it across a socket send would
    /// let one unresponsive phone stall the keeper's reconciliation of the whole fleet.
    pub async fn request_keyframe(&self, serial: &str) -> anyhow::Result<bool> {
        let control = {
            let views = self.views.lock().await;
            match views.get(serial) {
                Some(producer) => Arc::clone(&producer.control),
                None => return Ok(false),
            }
        };
        let message = crate::scrcpy::reset_video();
        let mut socket = control.lock().await;
        // ONE `write_all`, under the lock. The device's reader has no framing, so a partial
        // or interleaved write desynchronises it permanently — and that is not a lost
        // message, it is the whole server going down, video included.
        socket
            .write_all(&message)
            .await
            .with_context(|| format!("send RESET_VIDEO to {serial}"))?;
        socket
            .flush()
            .await
            .with_context(|| format!("flush RESET_VIDEO to {serial}"))?;
        Ok(true)
    }

    fn host_port(&self, serial: &str) -> u16 {
        let mut ports = self.ports.lock();
        if let Some(port) = ports.get(serial) {
            return *port;
        }
        let next = HOST_PORT_BASE + ports.len() as u16;
        ports.insert(serial.to_string(), next);
        next
    }

    fn agent_base(&self, serial: &str) -> String {
        format!("http://127.0.0.1:{}", self.host_port(serial))
    }

    /// Point a host port at the agent's port on the device.
    async fn forward(&self, serial: &str) -> anyhow::Result<()> {
        let forward_spec = format!("tcp:{}", self.host_port(serial));
        let device_spec = format!("tcp:{AGENT_DEVICE_PORT}");
        self.adb
            .device(
                serial,
                &["forward", &forward_spec, &device_spec],
                adb::DEFAULT_TIMEOUT,
            )
            .await
            .context("open the adb forward to the agent")?;
        self.forwarded.lock().insert(serial.to_string());
        Ok(())
    }

    /// Whether an agent is reachable for this device, answered honestly.
    ///
    /// Devices we have never forwarded report `false` rather than borrowing
    /// somebody else's agent.
    ///
    /// The retry is not defensive padding. The tunnel is not durable: an adb
    /// server restart drops every forward while the on-device agent keeps
    /// running, and at the HTTP layer that is indistinguishable from a dead
    /// agent. Measured — the instrumentation runner exited, `adb forward
    /// --list` came back empty, `ps` still showed the server alive, and one
    /// re-forward brought `/status` straight back. Without this the tile would
    /// flap to not-ready every time the adb server bounces. The extra adb call
    /// only happens on the failing path.
    async fn agent_ready(&self, serial: &str) -> bool {
        if !self.forwarded.lock().contains(serial) {
            return false;
        }
        let base = self.agent_base(serial);
        if AgentClient::is_ready(&base).await {
            return true;
        }
        if self.forward(serial).await.is_err() {
            return false;
        }
        AgentClient::is_ready(&base).await
    }

    /// The pid of a package, or `None` when it is not running.
    ///
    /// `pidof` exits non-zero for an absent process, which the adb wrapper
    /// reports as a command failure. Absence is an answer here, not an error —
    /// propagating it made `inspect_app_process` fail precisely when it was
    /// asked about a stopped app, which is the case it exists to describe.
    ///
    /// `bundle_id` must already have passed [`adb::validate_package_name`];
    /// every public caller checks it before reaching here.
    async fn pid_of(&self, serial: &str, bundle_id: &str) -> Option<u64> {
        self.adb
            .shell(serial, &format!("pidof {bundle_id}"))
            .await
            .ok()
            .and_then(|stdout| adb::parse_pidof(&stdout))
    }

    async fn screen_size(&self, serial: &str) -> anyhow::Result<(f64, f64)> {
        let stdout = self.adb.shell(serial, "wm size").await?;
        let (width, height) = adb::parse_wm_size(&stdout)
            .ok_or_else(|| anyhow!("could not read the screen size from 'wm size'"))?;
        Ok((f64::from(width), f64::from(height)))
    }

    /// `versionName` and `versionCode` for one installed package.
    ///
    /// A package that is not installed is a distinct outcome from one whose dump could not
    /// be parsed, and both say so: Flow's preflight message is the only thing an operator
    /// gets when a run refuses, so "TikTok is not installed" must not arrive as "could not
    /// read the version".
    async fn package_identity(
        &self,
        serial: &str,
        package: &str,
    ) -> anyhow::Result<crate::capability::PackageIdentity> {
        let package = adb::validate_package_name(package)?;
        let dumpsys = self
            .adb
            .shell(serial, &format!("dumpsys package {package}"))
            .await
            .with_context(|| format!("read the installed record for {package} on {serial}"))?;
        let version = riviu_core::tiktok_labels::parse_version_name(&dumpsys);
        let build = riviu_core::tiktok_labels::parse_version_code(&dumpsys);
        match (version, build) {
            (Some(version), Some(build)) => Ok(crate::capability::PackageIdentity {
                package: package.to_string(),
                version: version.to_string(),
                build: build.to_string(),
            }),
            _ if !dumpsys.contains(&format!("Package [{package}]")) => {
                Err(anyhow!("{package} is not installed on {serial}"))
            }
            _ => Err(anyhow!(
                "{package} is installed on {serial} but its version could not be read from \
                 `dumpsys package`"
            )),
        }
    }

    /// SHA-256 of an installed package's APK, computed on the device.
    ///
    /// `pm path` then `sha256sum`, in one shell round trip — measured 225 ms end to end on
    /// an SM-G955F, which is affordable on a path that runs once per device per Flow run.
    ///
    /// The two are chained on the phone rather than here so the path never crosses back
    /// through the host: `pm path` prints `package:/data/app/…/base.apk`, and a serial with
    /// two installed splits would otherwise need the host to decide which line to hash.
    async fn installed_apk_sha256(&self, serial: &str, package: &str) -> anyhow::Result<String> {
        let package = adb::validate_package_name(package)?;
        let stdout = self
            .adb
            .shell(
                serial,
                &format!("sha256sum \"$(pm path {package} | head -n 1 | cut -d: -f2)\""),
            )
            .await
            .with_context(|| format!("hash the installed {package} APK on {serial}"))?;
        let digest = stdout.split_whitespace().next().unwrap_or_default();
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(anyhow!(
                "could not hash the installed {package} APK on {serial}: `sha256sum` \
                 answered {stdout:?}"
            ));
        }
        Ok(digest.to_ascii_lowercase())
    }

    /// The screen as it is being rendered right now, rotation included.
    ///
    /// **`dumpsys display`, not `wm size`.** The latter reports the display's base
    /// configuration, which has no orientation in it, so a landscape phone answers with its
    /// portrait dimensions and every coordinate derived from them is wrong (AGENTS.md
    /// §9.59, and the doc on [`adb::parse_display_geometry`]).
    async fn display_geometry(&self, serial: &str) -> anyhow::Result<adb::DisplayGeometry> {
        let stdout = self.adb.shell(serial, "dumpsys display").await?;
        adb::parse_display_geometry(&stdout).ok_or_else(|| {
            anyhow!(
                "could not read the current display geometry from `dumpsys display` on {serial}"
            )
        })
    }

    /// The instrumentation component this driver starts to bring the agent up.
    fn agent_runner() -> String {
        format!("{AGENT_TEST_PACKAGE}/{AGENT_RUNNER}")
    }

    /// `(model, release)` — what a capability snapshot calls product type and OS version.
    ///
    /// Read fresh rather than taken from the cached `DeviceInfo`, because that one carries
    /// a model *hint* from `adb devices -l` which can be the codename (`dream2lte`) rather
    /// than the marketing model (`SM-G955F`), and this value is hashed into a device
    /// profile id that has to mean the same thing every time it is computed.
    async fn device_identity(&self, serial: &str) -> anyhow::Result<(String, String)> {
        let stdout = self
            .adb
            .shell(
                serial,
                &format!(
                    "getprop ro.product.model; echo {sep}; getprop ro.build.version.release",
                    sep = FIELD_SEPARATOR
                ),
            )
            .await?;
        let mut sections = stdout.split(FIELD_SEPARATOR);
        let model = sections.next().unwrap_or_default().trim().to_string();
        let release = sections.next().unwrap_or_default().trim().to_string();
        if model.is_empty() || release.is_empty() {
            return Err(anyhow!(
                "could not read the model and Android release from {serial}"
            ));
        }
        Ok((model, release))
    }

    /// Remember what we last proved about a serial's agent, for the synchronous readers.
    ///
    /// `DeviceDriver::cached_agent_status` cannot await, and Flow's preflight reads it to
    /// decide whether the phone has a usable control surface. So every path that learns
    /// something about the agent records it here, exactly as the iOS driver does with
    /// `agent_statuses`.
    fn publish_agent_status(&self, status: riviu_core::AgentStatus) {
        self.agent_statuses
            .lock()
            .insert(status.udid.clone(), status);
    }

    fn agent_status_for(
        &self,
        serial: &str,
        state: riviu_core::AgentState,
        identity: Option<&crate::capability::PackageIdentity>,
        message: Option<String>,
    ) -> riviu_core::AgentStatus {
        let ready = state == riviu_core::AgentState::Ready;
        riviu_core::AgentStatus {
            udid: serial.to_string(),
            state,
            artifact_id: AGENT_PACKAGE.to_string(),
            artifact_version: identity
                .map(|value| value.version.clone())
                .unwrap_or_default(),
            bundle_id: AGENT_PACKAGE.to_string(),
            protocol_version: crate::capability::PROTOCOL_VERSION,
            // What the agent can do is a property of this driver, not of the install: the
            // uiautomator2 server on any phone this project drives does all four, and the
            // measurements behind that claim are in `agent.rs`. Reporting them only when
            // ready keeps a phone that cannot be driven from advertising capabilities.
            features: if ready {
                ["stream", "tap", "swipe", "text"]
                    .iter()
                    .map(|value| value.to_string())
                    .collect()
            } else {
                Vec::new()
            },
            installed_version: identity.map(|value| value.version.clone()),
            installed_build: identity.map(|value| value.build.clone()),
            // No token to be ready or not: the uiautomator2 server has no auth. What this
            // stands for on Android is the same thing `protected_auth_ready` stands for in
            // the snapshot — the control surface answered and could see.
            auth_ready: ready,
            mjpeg_ready: ready,
            session_ready: ready,
            message,
        }
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

    pub async fn open_session(&self, udid: &str) -> anyhow::Result<AndroidUiSession> {
        let agent = self.ensure_agent(udid).await?;
        let screen = {
            let cache = self
                .screens
                .lock()
                .entry(udid.to_string())
                .or_default()
                .clone();
            // Seed from `wm size` only when there is nothing cached. It is the right *seed*
            // -- available before the agent is primed -- and the wrong *refresh*, because it
            // does not follow rotation (see `ScreenCache`). Re-opening a session for a phone
            // we already know now costs no adb round trip at all.
            if cache.peek().is_none() {
                cache.store(self.screen_size(udid).await?);
            }
            cache
        };
        let helper = match self.try_attach_helper(udid).await {
            Ok(helper) => helper,
            Err(error) => {
                tracing::warn!(
                    serial = udid,
                    %error,
                    "Riviu helper is not attached; clipboard stays unsupported"
                );
                None
            }
        };
        Ok(
            // `new` still takes a tuple so the public constructor is unchanged; the shared
            // handle replaces the private cache it seeds.
            AndroidUiSession::new(agent, self.adb.clone(), udid.to_string(), (0.0, 0.0))
                .with_screen_cache(screen)
                .with_helper(helper),
        )
    }

    /// Attach the helper when it is already on the phone, or when an APK is
    /// configured so we can install it. Missing both is normal and not an
    /// error — nurture must not die because clipboard is unavailable.
    async fn try_attach_helper(
        &self,
        serial: &str,
    ) -> anyhow::Result<Option<crate::riviu_agent::HelperClient>> {
        let cached = self.helpers.lock().get(serial).cloned();
        if let Some(helper) = cached {
            if helper.is_alive().await {
                return Ok(Some(helper));
            }
            self.helpers.lock().remove(serial);
        }
        let installed = self
            .adb
            .shell(serial, &format!("pm path {}", crate::riviu_agent::PACKAGE))
            .await
            .unwrap_or_default()
            .contains("package:");
        if !installed && self.riviu_agent_apk.is_none() {
            return Ok(None);
        }
        let helper = crate::riviu_agent::HelperClient::ensure(
            self.adb.clone(),
            serial,
            self.riviu_agent_apk.as_deref(),
        )
        .await?;
        self.helpers
            .lock()
            .insert(serial.to_string(), helper.clone());
        Ok(Some(helper))
    }

    /// Make sure the agent is installed, running and forwarded.
    async fn ensure_agent(&self, serial: &str) -> anyhow::Result<AgentClient> {
        let base = self.agent_base(serial);
        self.forward(serial).await?;

        // Reuse the session we already have. Opening a second one costs the whole
        // fleet: see the note on `Self::agents`.
        let cached = self.agents.lock().get(serial).cloned();
        if let Some(agent) = cached {
            if agent.is_alive().await {
                return Ok(agent);
            }
            // Dead, and still registered on the device. Ask the server to forget it
            // before opening another, or the leak this cache exists to stop happens
            // one session at a time anyway.
            let _ = agent.close().await;
            self.agents.lock().remove(serial);
        }

        // A server that answers `/status` usually just needs a fresh session — that is the
        // rotten-session case `AgentClient::recycle` documents. But `/status` does not prove
        // the accessibility tree is readable, and when it is not, a new session against the
        // same server is just as blind: measured on an SM-N950F on 12/08/2026, where an
        // out-of-band `uiautomator dump` had taken `UiAutomation` away and `open_session`
        // happily returned a 4040 ms session whose every element query then blocked.
        //
        // So the new session has to prove itself, and the fall-through is to restart the
        // instrumentation rather than to hand back something that cannot see.
        if AgentClient::is_ready(&base).await {
            // **Both ways of failing here lead to the same restart, and until 17/08/2026 one
            // of them led nowhere.** Losing `UiAutomation` has two presentations, and this
            // path only ever handled the first:
            //
            //   1. the session opens and every query blocks — caught by `is_alive`;
            //   2. the session does not open at all, `SessionNotCreatedException:
            //      java.lang.IllegalStateException: UiAutomation not connected!`, in 137 ms.
            //
            // Reproduced on this fleet with an out-of-band `adb shell uiautomator dump`: the
            // phone lands in (1), the restart runs, and afterwards it sits in (2) — where the
            // `?` on this line returned the Java exception straight to the operator and the
            // recovery below was unreachable, because proving the server broken required a
            // session and the breakage was that no session could be had. Every tap failed,
            // forever, and nothing ever tried to fix it.
            //
            // A server that answers `/status` and will not give a session is wedged whatever
            // the message says, so the failure is not inspected: it is treated exactly like a
            // blind session.
            let opened = self.open_and_cache_agent(serial, &base).await;
            match opened {
                Ok(agent) if agent.is_alive().await => return Ok(agent),
                Ok(agent) => {
                    let _ = agent.close().await;
                    self.agents.lock().remove(serial);
                }
                Err(error) => {
                    tracing::warn!(
                        serial,
                        %error,
                        "the agent answers /status but will not open a session"
                    );
                    self.agents.lock().remove(serial);
                }
            }
            // A restart we already tried and that did not take. Refuse rather than repeat:
            // the holder of `UiAutomation` is on the phone, and a second restart inside the
            // window races the same holder for another twenty seconds of the operator's
            // time. Failing here is not giving up — it is the difference between one clear
            // message and a minute of silence.
            if let Some(since) = self.since_instrumentation_restart(serial) {
                if since < INSTRUMENTATION_RESTART_COOLDOWN {
                    let quiet_for = INSTRUMENTATION_RESTART_COOLDOWN - since;
                    // Said out loud, not just returned. The error reaches whoever made this
                    // call; the log is what tells the next person why a phone spent ten
                    // minutes refusing every gesture without a single restart in sight.
                    tracing::warn!(
                        serial,
                        since_s = since.as_secs(),
                        quiet_for_s = quiet_for.as_secs(),
                        "refusing to restart the instrumentation again inside its cooldown"
                    );
                    anyhow::bail!(
                        "the agent on {serial} is listening but cannot read the accessibility \
                         tree, and its instrumentation was already restarted {:.0}s ago \
                         without fixing it. Something else on the phone is holding \
                         UiAutomation — an `adb shell uiautomator dump` or another automation \
                         tool. Not restarting again for another {:.0}s.",
                        since.as_secs_f64(),
                        quiet_for.as_secs_f64()
                    );
                }
            }
            tracing::warn!(
                serial,
                "the agent answers /status but cannot read the accessibility tree — \
                 restarting the instrumentation. Something else may be holding \
                 UiAutomation (an `adb shell uiautomator dump` does this)"
            );
            self.note_instrumentation_restart(serial);
            let started = std::time::Instant::now();
            self.restart_instrumentation(serial).await?;
            let recovered = self.instrument_and_wait(serial, &base).await;
            // Logged either way, because the cost of this path is the whole reason it now
            // has a cooldown and nobody should have to induce the fault to find it again.
            tracing::info!(
                serial,
                ms = started.elapsed().as_millis() as u64,
                ok = recovered.is_ok(),
                "instrumentation restart finished"
            );
            return recovered;
        }

        let installed = self
            .adb
            .shell(serial, &format!("pm list packages {AGENT_PACKAGE}"))
            .await
            .unwrap_or_default();
        if !installed.contains(AGENT_PACKAGE) {
            self.install_agent_apks(serial).await?;
        }

        self.instrument_and_wait(serial, &base).await
    }

    /// How long since this device's instrumentation was last restarted for blindness.
    fn since_instrumentation_restart(&self, serial: &str) -> Option<Duration> {
        self.instrumentation_restarts
            .lock()
            .get(serial)
            .map(|at| at.elapsed())
    }

    fn note_instrumentation_restart(&self, serial: &str) {
        self.instrumentation_restarts
            .lock()
            .insert(serial.to_string(), std::time::Instant::now());
    }

    /// Start the runner and wait for a session that can actually read the screen.
    async fn instrument_and_wait(&self, serial: &str, base: &str) -> anyhow::Result<AgentClient> {
        self.spawn_instrumentation(serial)?;
        // The server binds its port a beat after the runner starts.
        for _ in 0..AGENT_READY_POLLS {
            if AgentClient::is_ready(base).await {
                let agent = self.open_and_cache_agent(serial, base).await?;
                if agent.is_alive().await {
                    return Ok(agent);
                }
                // Bound to the port but blind. Reported rather than retried forever: a
                // second restart would race the same holder of `UiAutomation`, and the
                // operator needs to know something else on the phone has it.
                let _ = agent.close().await;
                self.agents.lock().remove(serial);
                return Err(anyhow!(
                    "the agent on {serial} is listening but cannot read the accessibility \
                     tree even after a restart. Something else holds UiAutomation — an \
                     `adb shell uiautomator dump`, or another automation tool on the phone"
                ));
            }
            tokio::time::sleep(AGENT_READY_POLL_EVERY).await;
        }
        Err(anyhow!(
            "the agent on {serial} did not answer /status within {:.0} seconds",
            AGENT_READY_WAIT.as_secs_f64()
        ))
    }

    /// Stop the running instrumentation so the next start is a clean one.
    ///
    /// Force-stopping both halves is what actually recovered the phone by hand on
    /// 12/08/2026 — `open_session` then re-instrumented and answered in 4040 ms. The
    /// server holds `UiAutomation` for its lifetime, so nothing short of ending the
    /// process gets it back.
    /// Push and install both halves of the uiautomator2 instrumentation.
    ///
    /// Until 16/08/2026 this was a message telling the operator to install two APKs the
    /// app did not ship. Measured on a freshly plugged 20-device Galaxy S8 box: video
    /// worked on 20/20 because `scrcpy-server` is bundled and pushed, and control worked
    /// on **0/20** because nothing pushed these. Telling someone to install a file that is
    /// not in the box is not an error message, it is a missing feature.
    ///
    /// `-r -g -t`: reinstall over a stale copy, grant the runtime permissions the server
    /// needs without a dialog, and allow a test-only APK -- the `androidTest` half is
    /// built with `android:testOnly`, which `pm install` refuses by default.
    async fn install_agent_apks(&self, serial: &str) -> anyhow::Result<()> {
        let Some((server, test)) = self.agent_apks.as_ref() else {
            return Err(anyhow!(
                "the agent is not installed on {serial} and this build has no agent APK                  to install. Set RIVIU_AGENT_SERVER_APK and RIVIU_AGENT_TEST_APK, or use                  an installer that bundles them"
            ));
        };
        // Server first: the test APK declares an instrumentation targeting the server's
        // package, and installing it against a missing target fails on some builds.
        for (apk, package) in [(server, AGENT_PACKAGE), (test, AGENT_TEST_PACKAGE)] {
            let path = apk.to_string_lossy().to_string();
            tracing::info!(serial, package, apk = %path, "installing the uiautomator2 agent");
            self.adb
                .run(
                    &["-s", serial, "install", "-r", "-g", "-t", &path],
                    INSTALL_TIMEOUT,
                )
                .await
                .with_context(|| format!("install {package} on {serial} from {path}"))?;
        }
        // Prove it rather than trust the exit code: `pm install` has been observed to
        // report success for a package that is not then listed.
        let installed = self
            .adb
            .shell(serial, &format!("pm list packages {AGENT_PACKAGE}"))
            .await
            .unwrap_or_default();
        anyhow::ensure!(
            installed.contains(AGENT_PACKAGE),
            "installed the agent on {serial} but `pm list packages` still does not show              {AGENT_PACKAGE}"
        );
        Ok(())
    }

    async fn restart_instrumentation(&self, serial: &str) -> anyhow::Result<()> {
        for package in [AGENT_PACKAGE, AGENT_TEST_PACKAGE] {
            if let Err(error) = self
                .adb
                .shell(serial, &format!("am force-stop {package}"))
                .await
            {
                tracing::warn!(serial, package, %error, "could not stop the agent half");
            }
        }
        // The port stays bound for a moment after the process goes.
        tokio::time::sleep(Duration::from_millis(600)).await;
        Ok(())
    }

    /// Open one session and remember it for this serial.
    async fn open_and_cache_agent(&self, serial: &str, base: &str) -> anyhow::Result<AgentClient> {
        let agent = AgentClient::connect(serial, base).await?;
        self.agents.lock().insert(serial.to_string(), agent.clone());
        Ok(agent)
    }

    /// Drop and delete the cached session for one device.
    ///
    /// Awaits, so it cannot be called from `invalidate_ui_session`, which is
    /// synchronous. That one only forgets the entry; this is the path that also
    /// tells the device.
    pub async fn close_agent(&self, serial: &str) {
        let agent = self.agents.lock().remove(serial);
        if let Some(agent) = agent {
            if let Err(error) = agent.close().await {
                tracing::warn!(serial, %error, "could not delete the agent session");
            }
        }
    }

    /// Start the instrumentation runner and let it keep running.
    ///
    /// `am instrument -w` blocks for the life of the server, so the child is
    /// detached deliberately rather than awaited.
    fn spawn_instrumentation(&self, serial: &str) -> anyhow::Result<()> {
        let mut command = tokio::process::Command::new(self.adb.path());
        command
            .args([
                "-s",
                serial,
                "shell",
                "am",
                "instrument",
                "-w",
                "-e",
                "disableAnalytics",
                "true",
                &format!("{AGENT_TEST_PACKAGE}/{AGENT_RUNNER}"),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        command.creation_flags(0x0800_0000);
        command
            .spawn()
            .with_context(|| format!("start the agent on {serial}"))?;
        Ok(())
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
        let mut unauthorized = Vec::new();
        for line in lines {
            match line.state {
                AdbDeviceState::Device => {
                    let adb = self.adb.clone();
                    inflight.push(tokio::spawn(probe_device(adb, line.serial, line.model)));
                }
                // Report it, do not hide it. A phone whose USB-debugging prompt
                // has not been accepted is a normal fleet state with an obvious
                // fix, and dropping it from the list makes it look unplugged.
                AdbDeviceState::Unauthorized => unauthorized.push(DeviceInfo {
                    udid: line.serial.clone(),
                    name: line.model.clone().unwrap_or_else(|| line.serial.clone()),
                    model: line.model.unwrap_or_default(),
                    platform: riviu_core::DevicePlatform::Android,
                    os_version: String::new(),
                    connection: ConnectionKind::Usb,
                    status: DeviceStatus::Pairing,
                    battery: None,
                    wda_ready: false,
                    wda_expires_at: None,
                    stream_url: None,
                    tile_stream_state: Default::default(),
                    last_error: Some(
                        "USB debugging not allowed yet — accept the prompt on the device".into(),
                    ),
                }),
                AdbDeviceState::Offline | AdbDeviceState::Other => {}
            }
        }

        let mut devices = Vec::with_capacity(inflight.len() + unauthorized.len());
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
        devices.extend(unauthorized);
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
        tokio::fs::write(dest, &png)
            .await
            .with_context(|| format!("write {}", dest.display()))?;
        Ok(dest.to_path_buf())
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
                    // Not obtainable over adb at any price worth paying. See the doc on
                    // `InstalledApp`.
                    label: None,
                });
            }
        }
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
        loop {
            let observed = match riviu_core::driver::UiSession::active_app_bundle(&session).await {
                Ok(package) if package == bundle_id => break,
                Ok(package) => package,
                Err(error) => format!("<unreadable: {error}>"),
            };
            if std::time::Instant::now() >= deadline {
                self.interaction.clear(udid);
                anyhow::bail!(
                    "{bundle_id} did not reach the foreground on {udid} within {}s; the phone is \
                     showing {observed}. A locked screen does this — `monkey` reports success and \
                     nothing moves",
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
}
