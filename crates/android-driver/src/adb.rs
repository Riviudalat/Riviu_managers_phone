//! The `adb` command line, wrapped the way `pmd.rs` wraps its Python sidecar.
//!
//! Only device *lifecycle* goes through here — list, install, launch, stop,
//! screenshot, reboot. Anything inside a control loop goes through
//! [`crate::agent`] instead, because measured on the fleet a single
//! `adb shell input tap` costs 1502 ms on a Galaxy S8+: `/system/bin/input` is
//! a shell script that starts a VM per call, and batching several into one
//! `adb shell` does not help (five taps cost five times as much). At 1–2 s a
//! call this layer is fine for setup and wrong for everything else.
//! See `docs/ANDROID_PROBE_REPORT_2026-08-09.md`.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context};
use tokio::process::Command;
use tokio::sync::Semaphore;

/// How long a lifecycle call may take before we stop waiting. Generous: on the
/// S8+ fleet `screencap` alone is measured at 1.2–2.6 s and `pm install` of a
/// 17 MB APK is slower still.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// How many `adb` invocations this process runs at once.
///
/// **Measured on the 20-phone fleet while all twenty were streaming**, because an idle adb
/// server is not the one that misbehaves. Sixty `adb shell echo` round trips per level:
///
/// | at once | p50 | p90 | max | wall for 60 calls |
/// |---|---|---|---|---|
/// | 4 | 56 ms | 65 ms | 75 ms | 0.8 s |
/// | 8 | 61 ms | 71 ms | 82 ms | 0.5 s |
/// | **12** | **67 ms** | **79 ms** | **89 ms** | **0.3 s** |
/// | 16 | 68 ms | 75 ms | 97 ms | 0.3 s |
/// | 24 | 79 ms | 95 ms | 117 ms | 0.2 s |
/// | 32 | 86 ms | 111 ms | 132 ms | 0.2 s |
///
/// Nothing *fails* at any level — this is not a cliff, it is a slope. What the table says is
/// that past twelve the fleet stops getting faster (0.3 s → 0.2 s) while every individual
/// call gets slower (p90 79 ms → 111 ms). Twelve is where those two curves cross.
///
/// It matters because this process had no cap at all: measured at startup, it reached
/// **34 concurrent adb invocations**, which is squarely in the region where every call is
/// paying for the others. The operator feels that as the app being unresponsive exactly when
/// it is busiest — which is exactly when they are watching it.
///
/// **The long-lived scrcpy child is deliberately not counted here.** It is spawned directly
/// rather than through this module, and it must be: it never exits, so a permit it held would
/// never come back and the fleet would deadlock at the twelfth phone.
const ADB_MAX_CONCURRENT: usize = 12;

/// Long enough that an ordinary wait is not worth a line, short enough that a real queue is.
///
/// At the cap above, a call waits only when more than twelve are already running; the table
/// says a running call finishes in well under 100 ms, so a wait past this means something is
/// genuinely backed up rather than merely busy.
const ADB_SLOW_WAIT: Duration = Duration::from_millis(500);

/// The cap is **global**, because the thing it rations is global: one adb server per host.
///
/// Two `AdbProgram` values are two handles onto the same server, so a per-instance limit
/// would bound nothing. `detect_driver` alone builds one instance per candidate path while
/// probing.
fn adb_slots() -> &'static Semaphore {
    static SLOTS: OnceLock<Semaphore> = OnceLock::new();
    SLOTS.get_or_init(|| Semaphore::new(ADB_MAX_CONCURRENT))
}

/// Wait for a slot, and say so if the wait was long.
///
/// Returns the permit; dropping it releases the slot. Held only around the child process, so
/// a slow *device* does not hold a slot longer than its own command takes.
async fn enter_adb_slot(what: &str) -> tokio::sync::SemaphorePermit<'static> {
    let waiting_since = Instant::now();
    let permit = adb_slots()
        .acquire()
        .await
        .expect("the adb slot semaphore is never closed");
    let waited = waiting_since.elapsed();
    if waited >= ADB_SLOW_WAIT {
        // Printed rather than merely endured. A queue nobody can see is indistinguishable
        // from a slow device, and those two have opposite fixes.
        tracing::warn!(
            waited_ms = waited.as_millis() as u64,
            limit = ADB_MAX_CONCURRENT,
            command = what,
            "waited for an adb slot; the host is running its cap of concurrent adb calls"
        );
    }
    permit
}

#[derive(Debug, Clone)]
pub struct AdbProgram {
    path: PathBuf,
}

/// One place `adb` might be, and where that guess came from.
///
/// The origin exists so a refusal can say *which* of the six sources produced the
/// binary that failed. "adb is not usable" naming only a path leaves the operator
/// guessing whether the app read their `RIVIU_ADB_PATH` at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdbCandidate {
    pub path: PathBuf,
    pub origin: AdbOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdbOrigin {
    /// Passed in by the caller, from app config.
    Configured,
    /// `RIVIU_ADB_PATH`, pointing straight at the executable.
    RiviuAdbPath,
    /// `<ANDROID_SDK_ROOT>/platform-tools/adb`.
    AndroidSdkRoot,
    /// `<ANDROID_HOME>/platform-tools/adb`.
    AndroidHome,
    /// The bare name, left for the OS to find on `PATH`.
    Path,
    /// The copy shipped inside our own installer.
    Bundled,
}

impl AdbOrigin {
    /// How this source reads to an operator, in the language the UI uses.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Configured => "cấu hình của app",
            Self::RiviuAdbPath => "RIVIU_ADB_PATH",
            Self::AndroidSdkRoot => "ANDROID_SDK_ROOT",
            Self::AndroidHome => "ANDROID_HOME",
            Self::Path => "PATH",
            Self::Bundled => "bản đóng gói trong bộ cài",
        }
    }
}

impl AdbProgram {
    /// Every place to look for `adb`, best first.
    ///
    /// `configured -> RIVIU_ADB_PATH -> ANDROID_SDK_ROOT -> ANDROID_HOME -> PATH ->
    /// bundled`. One implementation, because two callers need the same order and
    /// disagreeing is how the diagnostics panel comes to report a different adb than
    /// the one the fleet is actually driving.
    ///
    /// **The bundled copy is deliberately last, after `PATH`.** A machine that
    /// already has platform-tools (Android Studio, scrcpy) has that copy's adb server
    /// holding port 5037, and a client of a different revision forces
    /// `adb server version doesn't match this client; killing...`, tearing down the
    /// other tool's session. The operator's adb wins **if it runs**; ours is the
    /// safety net for a clean machine. Being last also means the bundled branch is
    /// never exercised on a developer machine — see AGENTS.md, it needs a second
    /// clean host to test.
    ///
    /// `RIVIU_ADB_PATH` points straight at the executable, which matters because a
    /// machine can have platform-tools unpacked somewhere without an SDK layout
    /// around it. Named for the repo's existing convention (`RIVIU_STREAM_CAPACITY`,
    /// `RIVIU_DEFAULT_AGENT_MODE`, `RIVIU_FRAME_DUMP`).
    pub fn candidates(configured: Option<&Path>, bundled: Option<&Path>) -> Vec<AdbCandidate> {
        let mut candidates: Vec<AdbCandidate> = Vec::new();
        if let Some(path) = configured {
            candidates.push(AdbCandidate {
                path: path.to_path_buf(),
                origin: AdbOrigin::Configured,
            });
        }
        if let Ok(direct) = std::env::var("RIVIU_ADB_PATH") {
            if !direct.trim().is_empty() {
                candidates.push(AdbCandidate {
                    path: PathBuf::from(direct.trim()),
                    origin: AdbOrigin::RiviuAdbPath,
                });
            }
        }
        for (key, origin) in [
            ("ANDROID_SDK_ROOT", AdbOrigin::AndroidSdkRoot),
            ("ANDROID_HOME", AdbOrigin::AndroidHome),
        ] {
            if let Ok(root) = std::env::var(key) {
                if !root.trim().is_empty() {
                    candidates.push(AdbCandidate {
                        path: Path::new(root.trim())
                            .join("platform-tools")
                            .join(exe_name()),
                        origin,
                    });
                }
            }
        }
        // Bare name: let the OS search PATH. Always a candidate — there is nothing to
        // stat, and whether it resolves is only knowable by running it.
        candidates.push(AdbCandidate {
            path: PathBuf::from(exe_name()),
            origin: AdbOrigin::Path,
        });
        if let Some(path) = bundled {
            candidates.push(AdbCandidate {
                path: path.to_path_buf(),
                origin: AdbOrigin::Bundled,
            });
        }
        candidates
    }

    /// Pick a binary without running anything.
    ///
    /// First candidate that exists on disk, else the bare name for the OS to search.
    /// Kept sync and infallible because [`crate::AndroidDriver::new`] is both, and a
    /// driver that exists is useful even when adb turns out to be broken — the
    /// probing version is [`crate::detect_driver`], which is the one that decides
    /// whether the backend joins the fleet.
    pub fn resolve(configured: Option<&Path>, bundled: Option<&Path>) -> anyhow::Result<Self> {
        let candidates = Self::candidates(configured, bundled);
        for candidate in &candidates {
            if candidate.origin == AdbOrigin::Path {
                continue;
            }
            if candidate.path.is_file() {
                return Ok(Self {
                    path: candidate.path.clone(),
                });
            }
        }
        Ok(Self {
            path: PathBuf::from(exe_name()),
        })
    }

    /// Use exactly this binary, no searching.
    ///
    /// For [`crate::detect_driver`], which has already proved a specific candidate
    /// answers `adb version` and must not have that choice re-derived underneath it.
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// An `adb` that is deliberately not runnable, for ordering tests.
    ///
    /// Lets a test prove a method refused **before** attempting any device call: if
    /// it had reached adb, the failure would be a spawn error instead of the
    /// expected refusal. `resolve` cannot express this — it falls back to the bare
    /// name on `PATH`, which on a developer machine usually *is* runnable.
    #[cfg(test)]
    pub(crate) fn unrunnable_for_test(path: PathBuf) -> Self {
        Self { path }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.path);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            // CREATE_NO_WINDOW: a console flash per adb call is unusable when
            // the fleet is 16 devices.
            command.creation_flags(0x0800_0000);
        }
        command
    }

    /// Run `adb <args>` and return raw stdout bytes.
    ///
    /// Anything binary — `exec-out screencap -p` above all — must come through
    /// here. Decoding stdout as text first replaces every invalid UTF-8 byte
    /// with U+FFFD, which silently corrupts a PNG into something the same
    /// order of size and no longer an image.
    pub async fn run_bytes(&self, args: &[&str], timeout: Duration) -> anyhow::Result<Vec<u8>> {
        let mut command = self.command();
        command.args(args);
        // The timeout starts AFTER the slot is acquired, deliberately. Counting queue time
        // against a command's own deadline would make a busy host look like a broken phone,
        // and the caller's timeouts are sized on what the device takes to answer.
        let _slot = enter_adb_slot(args.first().copied().unwrap_or("adb")).await;
        let output = tokio::time::timeout(timeout, command.output())
            .await
            .map_err(|_| anyhow!("adb {} timed out after {:?}", args.join(" "), timeout))?
            .with_context(|| format!("run adb {}", args.join(" ")))?;
        if output.status.success() {
            return Ok(output.stdout);
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        } else {
            stderr
        };
        Err(anyhow!("adb {} failed: {detail}", args.join(" ")))
    }

    /// Run one shell script and return stdout, stderr and the exit code together.
    ///
    /// Deliberately **not** [`Self::run_bytes`], which treats a non-zero exit as failure
    /// and throws stdout away. For an operator's own command that is a lie: measured
    /// 14/08/2026 on both fleet phones, `ls /nope/nothing` exits 1 with the message on
    /// **stderr** and stdout empty, and the same is true of `grep` with no match or
    /// `dumpsys` on an unknown service. Every one of those is a normal answer to a
    /// question, so all three fields come back and the caller decides what they mean.
    ///
    /// Still `Err` for the things that really are failures: adb not running, the device
    /// gone, or the call outliving its timeout.
    pub async fn shell_output(
        &self,
        serial: &str,
        script: &str,
        timeout: Duration,
    ) -> anyhow::Result<ShellOutput> {
        let mut command = self.command();
        command.args(["-s", serial, "shell", script]);
        let _slot = enter_adb_slot("shell").await;
        let output = tokio::time::timeout(timeout, command.output())
            .await
            .map_err(|_| anyhow!("adb shell timed out after {timeout:?}"))?
            .context("run adb shell")?;
        Ok(ShellOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    /// Run `adb <args>` and return stdout as text.
    pub async fn run(&self, args: &[&str], timeout: Duration) -> anyhow::Result<String> {
        let bytes = self.run_bytes(args, timeout).await?;
        Ok(String::from_utf8_lossy(&bytes).to_string())
    }

    /// Run a **read-only or otherwise idempotent** command, retrying a transport
    /// blip instead of surfacing it.
    ///
    /// Deliberately *not* folded into [`Self::run_bytes`]. `pm install`,
    /// `am start`, `am force-stop` and `input` are not idempotent, and a blind
    /// retry after a first attempt that actually landed installs twice or
    /// launches twice — the failure is invisible because both attempts "succeed".
    /// Retry is therefore opt-in per call site, and only [`AdbFault::Transient`]
    /// and [`AdbFault::Timeout`] are retried: an unaccepted USB-debugging prompt
    /// or an unknown serial needs a human or a rescan, not another attempt.
    pub async fn run_bytes_idempotent(
        &self,
        args: &[&str],
        timeout: Duration,
        attempts: u32,
    ) -> anyhow::Result<Vec<u8>> {
        let attempts = attempts.max(1);
        let mut last: Option<anyhow::Error> = None;
        for attempt in 1..=attempts {
            match self.run_bytes(args, timeout).await {
                Ok(bytes) => return Ok(bytes),
                Err(error) => {
                    let fault = classify_fault(&error.to_string());
                    if !fault.is_worth_retrying() || attempt == attempts {
                        return Err(error.context(format!(
                            "adb {} gave up after {attempt} attempt(s) ({fault:?})",
                            args.join(" ")
                        )));
                    }
                    tracing::warn!(
                        args = %args.join(" "),
                        attempt,
                        ?fault,
                        "retrying a transient adb failure"
                    );
                    last = Some(error);
                    tokio::time::sleep(retry_backoff(attempt)).await;
                }
            }
        }
        Err(last.unwrap_or_else(|| anyhow!("adb {} failed with no error", args.join(" "))))
    }

    /// Read `adb devices -l` until two consecutive reads agree.
    ///
    /// "adb is healthy" is not "one command returned" — a wedged or
    /// mid-restart server happily answers one call and then reports a different
    /// fleet on the next. The signal that matters is a *stable* list, so this
    /// reads, settles, and reads again.
    ///
    /// Returns what it saw either way: an unstable fleet still has to be shown
    /// to the operator rather than swallowed, so the caller decides what an
    /// unstable reading is worth (see [`DeviceListReading::stable`]).
    pub async fn devices_stable(&self, settle: Duration, deadline: Duration) -> DeviceListReading {
        let started = tokio::time::Instant::now();
        let mut previous: Option<Vec<AdbDeviceLine>> = None;
        let mut attempts = 0u32;
        loop {
            attempts += 1;
            let reading = match self.run(&["devices", "-l"], DEFAULT_TIMEOUT).await {
                Ok(stdout) => parse_devices(&stdout),
                Err(error) => {
                    tracing::warn!(%error, "adb devices failed while waiting for a stable list");
                    Vec::new()
                }
            };
            if let Some(before) = previous.as_ref() {
                if same_fleet(before, &reading) {
                    return DeviceListReading {
                        devices: reading,
                        stable: true,
                        attempts,
                    };
                }
            }
            if started.elapsed() >= deadline {
                return DeviceListReading {
                    devices: reading,
                    stable: false,
                    attempts,
                };
            }
            previous = Some(reading);
            tokio::time::sleep(settle).await;
        }
    }

    /// `adb kill-server`.
    ///
    /// **Global, and never called automatically.** It drops the adb connection
    /// of every other tool on the machine — another farm app mid-run included
    /// (`docs/re/genfarmer/README.md` §9) — and every `adb forward` on the host
    /// dies with it, so each device has to be re-forwarded afterwards. Reach for
    /// it only when an operator asked for it, and log that you did.
    pub async fn kill_server(&self) -> anyhow::Result<()> {
        tracing::warn!("adb kill-server: every tool on this machine loses its adb connection");
        self.run(&["kill-server"], DEFAULT_TIMEOUT).await?;
        Ok(())
    }

    /// Run `adb -s <serial> <args>` and return raw stdout bytes.
    pub async fn device_bytes(
        &self,
        serial: &str,
        args: &[&str],
        timeout: Duration,
    ) -> anyhow::Result<Vec<u8>> {
        let mut full: Vec<&str> = vec!["-s", serial];
        full.extend_from_slice(args);
        self.run_bytes(&full, timeout).await
    }

    /// Run `adb -s <serial> <args>`.
    pub async fn device(
        &self,
        serial: &str,
        args: &[&str],
        timeout: Duration,
    ) -> anyhow::Result<String> {
        let mut full: Vec<&str> = vec!["-s", serial];
        full.extend_from_slice(args);
        self.run(&full, timeout).await
    }

    /// Run a shell command on the device.
    pub async fn shell(&self, serial: &str, script: &str) -> anyhow::Result<String> {
        self.device(serial, &["shell", script], DEFAULT_TIMEOUT)
            .await
    }
}

/// Check a package name before it is pasted into a device shell command.
///
/// `adb shell` runs a real shell on the phone, so `bundle_id` reaches it as
/// code, not as data: a value like `x; rm -rf /sdcard/DCIM` would run. The
/// bundle id is operator-supplied — nurture settings, Flow action config — and
/// Flow documents import from JSON, so it can arrive from outside.
///
/// Rejecting beats escaping here. An Android package name has a narrow, fully
/// specified grammar (dot-separated segments of letters, digits and
/// underscores, each starting with a letter), so anything outside it is a
/// mistake or an attack, and neither should be quoted and run.
pub fn validate_package_name(bundle_id: &str) -> anyhow::Result<&str> {
    let invalid = || anyhow!("not a valid Android package name: {bundle_id:?}");
    if bundle_id.is_empty() || bundle_id.len() > 255 {
        return Err(invalid());
    }
    let mut segments = 0usize;
    for segment in bundle_id.split('.') {
        segments += 1;
        let mut chars = segment.chars();
        match chars.next() {
            Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
            _ => return Err(invalid()),
        }
        if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
            return Err(invalid());
        }
    }
    if segments < 2 {
        return Err(invalid());
    }
    Ok(bundle_id)
}

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "adb.exe"
    } else {
        "adb"
    }
}

/// One line of `adb devices -l`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdbDeviceLine {
    pub serial: String,
    pub state: AdbDeviceState,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdbDeviceState {
    /// Authorised and usable.
    Device,
    /// Plugged in, but the USB-debugging prompt has not been accepted. Measured
    /// on the fleet: one device sat in this state, so it is a normal condition
    /// to report rather than an error to hide.
    Unauthorized,
    Offline,
    Other,
}

/// Why an `adb` invocation failed, and therefore whether another attempt helps.
///
/// The point of splitting these apart is that "retry" and "tell someone" are
/// different answers. A device that has not had its USB-debugging prompt
/// accepted will fail identically forever, and burning three attempts on it just
/// delays the message the operator actually needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdbFault {
    /// The transport blipped. The same command can succeed on the next attempt.
    Transient,
    /// The command outlived its deadline.
    Timeout,
    /// The USB-debugging prompt has not been accepted on the device. A human
    /// must act.
    Unauthorized,
    /// adb does not know this serial — it was unplugged, or the scan is stale.
    UnknownDevice,
    /// Anything unrecognised. Treated as terminal on purpose: guessing that an
    /// unknown failure is transient turns one clear error into three slow ones.
    Terminal,
}

impl AdbFault {
    pub fn is_worth_retrying(self) -> bool {
        matches!(self, Self::Transient | Self::Timeout)
    }
}

/// Classify a failed adb invocation from its message.
///
/// Matching is on the substrings adb itself emits. Order matters: an
/// unauthorised device also mentions the serial, so the more specific families
/// are tested first.
pub fn classify_fault(message: &str) -> AdbFault {
    let text = message.to_ascii_lowercase();
    if text.contains("timed out after") {
        return AdbFault::Timeout;
    }
    if text.contains("unauthorized") {
        return AdbFault::Unauthorized;
    }
    if text.contains("not found")
        || text.contains("no devices/emulators found")
        || text.contains("more than one device")
    {
        return AdbFault::UnknownDevice;
    }
    const TRANSIENT: [&str; 8] = [
        "device offline",
        "protocol fault",
        "connection reset",
        "device still authorizing",
        "cannot connect to daemon",
        "daemon not running",
        "broken pipe",
        "error: closed",
    ];
    if TRANSIENT.iter().any(|marker| text.contains(marker)) {
        return AdbFault::Transient;
    }
    AdbFault::Terminal
}

/// Backoff before retrying attempt `attempt` (1-based). Short on purpose: a USB
/// blip clears in milliseconds, and a lifecycle call already costs 1–2 s.
fn retry_backoff(attempt: u32) -> Duration {
    Duration::from_millis(250u64.saturating_mul(1u64 << (attempt.min(4) - 1)))
}

/// One `adb devices -l` reading, with whether it settled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceListReading {
    pub devices: Vec<AdbDeviceLine>,
    /// Two consecutive reads agreed. `false` means the deadline passed while the
    /// fleet was still changing — the list is the last thing seen, not a promise.
    pub stable: bool,
    pub attempts: u32,
}

/// Whether two readings describe the same fleet, ignoring the order adb happens
/// to print devices in.
pub fn same_fleet(left: &[AdbDeviceLine], right: &[AdbDeviceLine]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut left: Vec<&AdbDeviceLine> = left.iter().collect();
    let mut right: Vec<&AdbDeviceLine> = right.iter().collect();
    left.sort_by(|a, b| a.serial.cmp(&b.serial));
    right.sort_by(|a, b| a.serial.cmp(&b.serial));
    left.iter()
        .zip(right.iter())
        .all(|(a, b)| a.serial == b.serial && a.state == b.state)
}

/// Parse `adb devices -l`. The header line and blank lines are skipped.
pub fn parse_devices(stdout: &str) -> Vec<AdbDeviceLine> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("List of devices"))
        // adb prints daemon chatter on first contact; it has no second column.
        .filter(|line| !line.starts_with('*'))
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let serial = parts.next()?.to_string();
            let state = match parts.next()? {
                "device" => AdbDeviceState::Device,
                "unauthorized" => AdbDeviceState::Unauthorized,
                "offline" => AdbDeviceState::Offline,
                _ => AdbDeviceState::Other,
            };
            let model = parts.find_map(|token| {
                token
                    .strip_prefix("model:")
                    .map(|value| value.replace('_', " "))
            });
            Some(AdbDeviceLine {
                serial,
                state,
                model,
            })
        })
        .collect()
}

/// Screen geometry as the device reports it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenGeometry {
    pub width: u32,
    pub height: u32,
    pub density: u32,
}

/// Parse `wm size`, preferring the override when one is set.
///
/// This matters more than it looks. Every device on the fleet reports
/// `Physical size: 1440x2960` *and* `Override size: 1080x2220`, and the
/// override is what is actually rendered — every coordinate, every screenshot
/// and every geometry check is in override space. Reading the physical line
/// puts the driver 33% off on both axes.
pub fn parse_wm_size(stdout: &str) -> Option<(u32, u32)> {
    let mut physical = None;
    let mut override_size = None;
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("Physical size:") {
            physical = parse_size_pair(value);
        } else if let Some(value) = line.strip_prefix("Override size:") {
            override_size = parse_size_pair(value);
        }
    }
    override_size.or(physical)
}

fn parse_size_pair(value: &str) -> Option<(u32, u32)> {
    let (w, h) = value.trim().split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

/// Parse `wm density`, preferring the override.
pub fn parse_wm_density(stdout: &str) -> Option<u32> {
    let mut physical = None;
    let mut override_density = None;
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("Physical density:") {
            physical = value.trim().parse().ok();
        } else if let Some(value) = line.strip_prefix("Override density:") {
            override_density = value.trim().parse().ok();
        }
    }
    override_density.or(physical)
}

/// Parse the pid out of `pidof <package>`; absent output means not running.
pub fn parse_pidof(stdout: &str) -> Option<u64> {
    stdout.split_whitespace().next()?.parse().ok()
}

/// Parse the package name out of `dumpsys window windows | grep mCurrentFocus`.
/// The UI language the operator is actually looking at.
///
/// **`persist.sys.locale`, never `ro.product.locale`.** Measured on the Redmi Note
/// 12: `persist.sys.locale=vi-VN` while `ro.product.locale=en-GB`. The second is the
/// factory default, so reading it says "English" about a phone whose TikTok labels
/// are all Vietnamese — the same first-line trap as `wm size` and `mCurrentFocus`.
///
/// Returns the raw tag (`vi-VN`); reduce it with
/// `riviu_core::tiktok_labels::normalise_language`.
pub fn parse_locale(persist_locale: &str, system_locales: &str) -> Option<String> {
    for candidate in [persist_locale, system_locales] {
        // `settings get` prints `null` when unset, and that is a string, not None.
        let value = candidate.trim();
        if value.is_empty() || value.eq_ignore_ascii_case("null") {
            continue;
        }
        // `system_locales` can be a comma-separated preference list; the first is
        // the active one.
        let first = value.split(',').next().unwrap_or(value).trim();
        if !first.is_empty() {
            return Some(first.to_string());
        }
    }
    None
}

/// Whether the display is composing, from `dumpsys power`.
///
/// `None` means the line was not found — **unknown**, which callers must not treat
/// as "asleep". MIUI and Android 9 do not print the same `dumpsys` bodies, and an
/// unreadable line is not evidence the screen is off.
///
/// Not sufficient on its own: see [`parse_keyguard_locked`]. A locked phone
/// reports `Awake`.
pub fn parse_display_awake(stdout: &str) -> Option<bool> {
    stdout.lines().find_map(|line| {
        let value = line.trim().strip_prefix("mWakefulness=")?.trim();
        match value {
            "Awake" => Some(true),
            "Asleep" | "Dozing" | "Dreaming" => Some(false),
            _ => None,
        }
    })
}

/// Package names out of `cmd package list packages`, in the order the phone gave them.
///
/// **The flagless form, on purpose.** Adding `-f` makes each line
/// `package:<apkPath>=<name>`, and the apk path itself contains `=` — measured on the
/// Redmi: `~~t4zKiXKBJ07rbvGFo_JJsA==/com.microsoft.office.officehubrow-lSzImKSf8a5Gv78FCOkWUg==/base.apk`.
/// Splitting on the first `=` destroys the path and splitting on the last grabs whatever
/// came after it, which is how that flag produces a silently empty parse. Without `-f`
/// the line is exactly `package:<name>` — the shape `tiktok_target` already reads — and
/// the trap cannot occur.
///
/// A row that is not a legal package name is **dropped, not repaired**: this output is
/// device-supplied, and a malformed row means the parse has desynchronised rather than
/// that the phone has an oddly-named app. `str::lines` is required rather than splitting
/// on `\n` because adb returns CRLF.
pub fn parse_package_list(stdout: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for line in stdout.lines() {
        let Some(name) = line.trim().strip_prefix("package:") else {
            continue;
        };
        let name = name.trim();
        if validate_package_name(name).is_err() {
            continue;
        }
        if !names.iter().any(|seen| seen == name) {
            names.push(name.to_string());
        }
    }
    names
}

/// Everything one shell call produced, with nothing discarded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// The rotation the device reports, from `dumpsys window`.
///
/// **Two formats, and both are in this fleet.** Measured 14/08/2026: the Redmi
/// (SDK 35) prints `mRotation=ROTATION_0` while the Note 8 (SDK 26) prints
/// `mRotation=0` and also `mCurrentRotation=0`. A parser written against either one
/// alone reads the other as unknown — the same shape as the one-line versus two-line
/// `wm size` trap already recorded for `parse_wm_size`.
///
/// `None` is unknown, never zero: zero is a real rotation and returning it for an
/// unreadable dump would tell a caller the screen is upright when nobody knows.
pub fn parse_screen_rotation(stdout: &str) -> Option<u8> {
    for key in ["mRotation=", "mCurrentRotation="] {
        for line in stdout.lines() {
            let Some(at) = line.find(key) else { continue };
            let value = line[at + key.len()..]
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_end_matches(&[',', '}'][..]);
            // Two number systems, not one. `ROTATION_90` is the *name* of
            // `Surface.ROTATION_90`, whose value is 1 — so the named form carries
            // degrees and the bare form carries the index. Parsing the digits out of the
            // name would read 270 as an impossible rotation and drop it.
            let rotation = match value.strip_prefix("ROTATION_") {
                Some("0") => Some(0),
                Some("90") => Some(1),
                Some("180") => Some(2),
                Some("270") => Some(3),
                Some(_) => None,
                None => value.parse::<u8>().ok().filter(|value| *value < 4),
            };
            if let Some(rotation) = rotation {
                return Some(rotation);
            }
        }
    }
    None
}

/// The keyevent that wakes a screen without ever putting one to sleep.
///
/// `KEYCODE_POWER` **toggles**, so on a phone that is already awake it would blank the
/// screen — the exact opposite of what a caller starting a capture wants, and a failure
/// that would look like the bug it was meant to fix. `KEYCODE_WAKEUP` only wakes, and is
/// a no-op on an awake phone.
pub const WAKE_KEYEVENT: &str = "input keyevent KEYCODE_WAKEUP";

/// Whether to send [`WAKE_KEYEVENT`] before capturing this phone's screen.
///
/// Measured 14/08/2026 on a Redmi Note 12 (Android 15) whose screen had gone to sleep:
/// scrcpy started, encoded nothing at all, and the desktop's five-second watchdog
/// restarted it every cycle forever — the producer was alive, there was simply no frame
/// to encode. One `KEYCODE_WAKEUP` and the tile went live immediately, fleet count
/// `1/2` → `2/2`. The same fact was already recorded for minicap on 11/08 in
/// `refuse_undrivable_screen`; the scrcpy view path added later never inherited it.
///
/// Unknown counts as "send it". Waking an awake phone costs one idempotent keyevent,
/// while skipping it on a sleeping one costs a permanently black tile — so when
/// `dumpsys` cannot be read, the cheap side is the safe side. This is deliberately the
/// opposite default from [`parse_display_awake`]'s callers that *refuse*: refusing on a
/// guess strands a working phone, waking on a guess costs nothing.
pub fn should_wake_before_capture(display_awake: Option<bool>) -> bool {
    !matches!(display_awake, Some(true))
}

/// Whether the lock screen is up, from `dumpsys window`.
///
/// **This is the check `mWakefulness` cannot make.** Measured on a locked Redmi
/// Note 12 (Android 15, HyperOS `OS2.0.207.0`) on 11/08/2026: `dumpsys power` said
/// `mWakefulness=Awake` and `mAwake=true mScreenOnEarly=true mScreenOnFully=true`
/// — the screen genuinely *was* on — while the phone sat on the lock screen and
/// `mCurrentFocus` read `NotificationShade`. Every attempt to foreground an app
/// silently did nothing. So a wakefulness-only pre-check passes a phone nothing
/// can drive.
///
/// Three keys carried the truth on that build, and all three are accepted because
/// no single one is guaranteed across the fleet's Android 9 → 15 range:
/// `isKeyguardShowing=true`, `mKeyguardShowing=true`, `mDreamingLockscreen=true`.
///
/// **Any** of them reading `true` means locked. That direction is deliberate: a
/// false "locked" refuses a working phone with a clear message, while a false
/// "unlocked" sends the driver to tap a lock screen. The foreground proof in
/// `start_interaction_session` is the backstop for both.
pub fn parse_keyguard_locked(stdout: &str) -> Option<bool> {
    const KEYS: [&str; 3] = [
        "isKeyguardShowing=",
        "mKeyguardShowing=",
        "mDreamingLockscreen=",
    ];
    let mut seen = false;
    for line in stdout.lines() {
        for key in KEYS {
            let Some(index) = line.find(key) else {
                continue;
            };
            let value = line[index + key.len()..]
                .split_whitespace()
                .next()
                .unwrap_or_default();
            match value {
                "true" => return Some(true),
                "false" => seen = true,
                _ => {}
            }
        }
    }
    seen.then_some(false)
}

/// Scans **every** `mCurrentFocus` line, not the first.
///
/// A device with more than one display prints one line per display and the
/// unfocused ones say `mCurrentFocus=null`. Taking the first match therefore
/// reports "no foreground app" while TikTok is plainly on screen — measured on a
/// Redmi Note 12, where `null` came first. Same shape of trap as `wm size`
/// printing two lines (§9): the first line is not the answer.
pub fn parse_current_focus_package(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .filter(|line| line.contains("mCurrentFocus"))
        .find_map(|line| {
            let inside = line.rsplit_once('/')?.0;
            let package = inside.rsplit_once(' ')?.1;
            (!package.is_empty()).then(|| package.to_string())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pending_usb_prompt_is_not_a_transport_blip() {
        // The distinction that matters: this fails the same way forever, so
        // retrying it only delays telling the operator to tap Allow.
        assert_eq!(
            classify_fault("adb -s X shell id failed: error: device unauthorized."),
            AdbFault::Unauthorized
        );
        assert!(!AdbFault::Unauthorized.is_worth_retrying());
    }

    #[test]
    fn transport_blips_are_retried_and_unknown_failures_are_not() {
        for message in [
            "adb -s X shell id failed: error: device offline",
            "adb devices failed: protocol fault (couldn't read status): connection reset",
            "adb -s X shell id failed: error: device still authorizing",
            "adb devices failed: * daemon not running; starting now",
            "adb -s X push failed: error: closed",
        ] {
            assert_eq!(classify_fault(message), AdbFault::Transient, "{message}");
            assert!(AdbFault::Transient.is_worth_retrying());
        }
        // Anything unrecognised stays terminal on purpose.
        let unknown =
            classify_fault("adb -s X install failed: INSTALL_FAILED_INSUFFICIENT_STORAGE");
        assert_eq!(unknown, AdbFault::Terminal);
        assert!(!unknown.is_worth_retrying());
    }

    #[test]
    fn a_deadline_is_retryable_but_a_missing_serial_is_a_rescan() {
        assert_eq!(
            classify_fault("adb -s X shell wm size timed out after 60s"),
            AdbFault::Timeout
        );
        assert!(AdbFault::Timeout.is_worth_retrying());
        assert_eq!(
            classify_fault("adb -s X shell id failed: error: device 'X' not found"),
            AdbFault::UnknownDevice
        );
        assert!(!AdbFault::UnknownDevice.is_worth_retrying());
    }

    #[test]
    fn fleet_equality_ignores_the_order_adb_prints() {
        let first = parse_devices("List of devices attached\na device\nb device\n");
        let reversed = parse_devices("List of devices attached\nb device\na device\n");
        assert!(same_fleet(&first, &reversed));
    }

    #[test]
    fn a_state_change_is_not_a_stable_fleet() {
        // The case this guards: the serial set is identical while a device is
        // still coming up, which is exactly when a scan must not be trusted.
        let authorizing = parse_devices("List of devices attached\na unauthorized\n");
        let ready = parse_devices("List of devices attached\na device\n");
        assert!(!same_fleet(&authorizing, &ready));
        let gained = parse_devices("List of devices attached\na device\nb device\n");
        assert!(!same_fleet(&ready, &gained));
    }

    #[test]
    fn retry_backoff_grows_and_stays_bounded() {
        assert_eq!(retry_backoff(1), Duration::from_millis(250));
        assert_eq!(retry_backoff(2), Duration::from_millis(500));
        assert_eq!(retry_backoff(3), Duration::from_millis(1000));
        // Clamped, so a large attempt count cannot shift its way to a stall.
        assert_eq!(retry_backoff(9), retry_backoff(4));
    }

    #[test]
    fn device_listing_keeps_authorized_and_unauthorized_apart() {
        let listing = "List of devices attached\n\
             10969614               unauthorized transport_id:16\n\
             ce011711c354be2005     device product:dream2lteks model:SM_G955N device:dream2lteks transport_id:6\n";
        let parsed = parse_devices(listing);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].state, AdbDeviceState::Unauthorized);
        assert_eq!(parsed[0].serial, "10969614");
        assert_eq!(parsed[0].model, None);
        assert_eq!(parsed[1].state, AdbDeviceState::Device);
        assert_eq!(parsed[1].model.as_deref(), Some("SM G955N"));
    }

    #[test]
    fn daemon_chatter_is_not_mistaken_for_a_device() {
        let listing = "* daemon not running; starting now at tcp:5037\n\
             * daemon started successfully\n\
             List of devices attached\n\
             ce011711c354be2005     device\n";
        let parsed = parse_devices(listing);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].serial, "ce011711c354be2005");
    }

    #[test]
    fn wm_size_prefers_the_override_because_that_is_what_renders() {
        // Verbatim from the fleet: reading the physical line would put every
        // coordinate 33% out.
        let stdout = "Physical size: 1440x2960\nOverride size: 1080x2220\n";
        assert_eq!(parse_wm_size(stdout), Some((1080, 2220)));
    }

    #[test]
    fn wm_size_falls_back_to_physical_when_no_override_is_set() {
        assert_eq!(
            parse_wm_size("Physical size: 1080x2400\n"),
            Some((1080, 2400))
        );
        assert_eq!(parse_wm_size("nothing useful"), None);
    }

    #[test]
    fn wm_density_prefers_the_override_too() {
        assert_eq!(
            parse_wm_density("Physical density: 640\nOverride density: 420\n"),
            Some(420)
        );
        assert_eq!(parse_wm_density("Physical density: 420\n"), Some(420));
    }

    #[test]
    fn a_real_package_name_is_accepted() {
        for good in [
            "com.zhiliaoapp.musically",
            "io.appium.uiautomator2.server.test",
            "com.ss.android.ugc.trill",
            "a.b",
            "com.example._private1",
        ] {
            assert!(validate_package_name(good).is_ok(), "{good}");
        }
    }

    #[test]
    fn anything_the_device_shell_could_act_on_is_refused() {
        // These are the shapes that matter: the value is pasted into a shell
        // command on the phone, so a separator or a substitution is code.
        for bad in [
            "com.x; rm -rf /sdcard/DCIM",
            "com.x && reboot",
            "com.x | sh",
            "com.x$(id)",
            "com.x`id`",
            "com.x\nreboot",
            "com.x y",
            "com.x'",
            "../../etc/passwd",
            "com..x",
            "1com.x",
            "",
            "nodots",
        ] {
            assert!(
                validate_package_name(bad).is_err(),
                "should have been refused: {bad:?}"
            );
        }
    }

    #[test]
    fn pidof_absent_means_not_running() {
        assert_eq!(parse_pidof("12345\n"), Some(12345));
        assert_eq!(parse_pidof("12345 12346\n"), Some(12345));
        assert_eq!(parse_pidof("\n"), None);
        assert_eq!(parse_pidof(""), None);
    }

    #[test]
    fn current_focus_yields_the_package() {
        let stdout = "  mCurrentFocus=Window{866366d u0 com.zhiliaoapp.musically/com.ss.android.ugc.aweme.splash.SplashActivity}";
        assert_eq!(
            parse_current_focus_package(stdout).as_deref(),
            Some("com.zhiliaoapp.musically")
        );
        assert_eq!(parse_current_focus_package("mCurrentFocus=null"), None);
    }

    #[test]
    fn current_focus_reads_the_regional_tiktok_and_an_empty_dump() {
        // Measured on Android 15: the SEA build is a different package, and the
        // line arrives from `dumpsys window displays` because
        // `dumpsys window windows` returns nothing there.
        let stdout = "  mCurrentFocus=Window{f567298 u0 com.ss.android.ugc.trill/com.ss.android.ugc.aweme.splash.SplashActivity}";
        assert_eq!(
            parse_current_focus_package(stdout).as_deref(),
            Some("com.ss.android.ugc.trill")
        );
        // An empty dump is the case that used to surface as a bare failure.
        assert_eq!(parse_current_focus_package(""), None);
    }

    #[test]
    fn ui_locale_comes_from_the_user_setting_not_the_factory_default() {
        // Verbatim from the Redmi Note 12: the two properties disagree, and only
        // the first describes what is on screen.
        assert_eq!(parse_locale("vi-VN", "vi-VN").as_deref(), Some("vi-VN"));
        // Falls back to the settings value when the property is unset.
        assert_eq!(parse_locale("", "vi-VN").as_deref(), Some("vi-VN"));
        // `settings get` prints the word null when unset.
        assert_eq!(parse_locale("null", "null"), None);
        // A preference list: the active locale is the first entry.
        assert_eq!(parse_locale("", "vi-VN,en-GB").as_deref(), Some("vi-VN"));
        assert_eq!(parse_locale("  ", "  "), None);
    }

    #[test]
    fn a_null_focus_line_does_not_hide_the_focused_display() {
        // Verbatim from `dumpsys window displays` on a Redmi Note 12: two
        // displays, the unfocused one first. Reading only the first line reported
        // no foreground app while TikTok was on screen.
        let stdout = "  mCurrentFocus=null\n  \
             mCurrentFocus=Window{823a4a6 u0 com.ss.android.ugc.trill/com.ss.android.ugc.aweme.splash.SplashActivity}\n";
        assert_eq!(
            parse_current_focus_package(stdout).as_deref(),
            Some("com.ss.android.ugc.trill")
        );
        // Every display unfocused is still None, not a panic.
        assert_eq!(
            parse_current_focus_package("mCurrentFocus=null\nmCurrentFocus=null\n"),
            None
        );
    }

    /// The exact `dumpsys power` body read off a **locked** Redmi Note 12 on
    /// 11/08/2026. Kept verbatim because it is the counter-example: every field
    /// here says the screen is on, and the phone could not be driven at all.
    const LOCKED_POWER: &str = "  mWakefulness=Awake\n  mWakefulnessChanging=false\n  \
                                mHoldingDisplaySuspendBlocker=true\n";

    /// The matching `dumpsys window` extract from the same locked phone.
    const LOCKED_WINDOW: &str = "    KeyguardServiceDelegate\n      showing=true\n        \
                                 mIsShowing=true\n    mIsImeShowing=false\n    mAwake=true \
                                 mScreenOnEarly=true mScreenOnFully=true\n    \
                                 mShowingDream=false mDreamingLockscreen=true\n    \
                                 isKeyguardShowing=true\n";

    #[test]
    fn wakefulness_alone_cannot_see_a_locked_phone() {
        // This is the whole reason `parse_keyguard_locked` exists. If this ever
        // starts returning `Some(false)`, the pre-check has been weakened.
        assert_eq!(parse_display_awake(LOCKED_POWER), Some(true));
        assert_eq!(parse_keyguard_locked(LOCKED_WINDOW), Some(true));
    }

    #[test]
    fn display_wakefulness_reads_the_states_that_stop_composition() {
        assert_eq!(parse_display_awake("mWakefulness=Asleep"), Some(false));
        assert_eq!(parse_display_awake("  mWakefulness=Dozing  "), Some(false));
        assert_eq!(parse_display_awake("mWakefulness=Dreaming"), Some(false));
    }

    #[test]
    fn a_package_listing_keeps_device_order_and_drops_nothing_real() {
        // Real stdout shape, CRLF as adb actually returns it.
        let stdout = "package:com.zhiliaoapp.musically\r\npackage:com.riviu.agent\r\n\
                      package:com.ss.android.ugc.trill\r\n";

        assert_eq!(
            parse_package_list(stdout),
            [
                "com.zhiliaoapp.musically",
                "com.riviu.agent",
                "com.ss.android.ugc.trill"
            ]
        );
    }

    #[test]
    fn a_listing_row_that_is_not_a_package_name_is_dropped_not_repaired() {
        // Device-supplied output. A row that fails the name rules means the parse
        // desynchronised, not that the phone has an oddly-named app — so it must not
        // reach a caller that might paste it into a device shell.
        let stdout = "package:com.good.app\n\
                      package:\n\
                      package:9bad.start\n\
                      package:nodots\n\
                      Exception occurred while executing:\n\
                      package:com.other.app\n";

        assert_eq!(
            parse_package_list(stdout),
            ["com.good.app", "com.other.app"]
        );
    }

    #[test]
    fn the_flagless_form_is_what_keeps_the_equals_trap_from_existing() {
        // With `-f` the line is `package:<apkPath>=<name>` and the apk path itself
        // contains `=`, measured on the Redmi. Such a row is not a package name, so it is
        // dropped rather than silently mis-split — the parse cannot half-succeed.
        let with_f = "package:/data/app/~~t4zKiXKBJ07rbvGFo_JJsA==/\
                      com.microsoft.office.officehubrow-lSzImKSf8a5Gv78FCOkWUg==/base.apk=\
                      com.microsoft.office.officehubrow\n";

        assert!(parse_package_list(with_f).is_empty());
    }

    #[test]
    fn a_package_listed_twice_appears_once() {
        // The two partitions are read in sequence into one list; a phone that reports a
        // package in both must not produce a duplicate row.
        let stdout = "package:com.dup.app\npackage:com.dup.app\n";

        assert_eq!(parse_package_list(stdout), ["com.dup.app"]);
    }

    #[test]
    fn rotation_is_read_from_both_formats_this_fleet_prints() {
        // Redmi SDK 35 and Note 8 SDK 26, measured verbatim.
        assert_eq!(parse_screen_rotation("  mRotation=ROTATION_0"), Some(0));
        // The named form is degrees; its value is the index. 270 degrees is index 3.
        assert_eq!(parse_screen_rotation("  mRotation=ROTATION_90"), Some(1));
        assert_eq!(parse_screen_rotation("  mRotation=ROTATION_270"), Some(3));
        assert_eq!(
            parse_screen_rotation("  mCurrentRotation=1 mRotation=1"),
            Some(1)
        );
    }

    #[test]
    fn an_unreadable_rotation_is_unknown_and_never_upright() {
        // Zero is a real rotation. Returning it for a dump nobody could read would tell
        // a caller the screen is upright when in fact nobody knows.
        assert_eq!(parse_screen_rotation(""), None);
        assert_eq!(parse_screen_rotation("mRotation=ROTATION_SIDEWAYS"), None);
        assert_eq!(parse_screen_rotation("mRotation=9"), None);
    }

    #[test]
    fn a_sleeping_or_unreadable_display_is_woken_before_capture() {
        // Unknown deliberately errs toward waking. The two costs are not symmetric: a
        // wake on an awake phone is one idempotent keyevent, while skipping it on a
        // sleeping one is a black tile and a watchdog restarting the encoder forever.
        assert!(should_wake_before_capture(Some(false)));
        assert!(should_wake_before_capture(None));
        assert!(!should_wake_before_capture(Some(true)));
    }

    #[test]
    fn the_wake_keyevent_can_never_put_a_screen_to_sleep() {
        // KEYCODE_POWER toggles, so using it here would blank an already-awake phone
        // and reproduce the very symptom this exists to remove. Pinned because the two
        // constants read almost identically at a glance.
        assert!(WAKE_KEYEVENT.contains("KEYCODE_WAKEUP"), "{WAKE_KEYEVENT}");
        assert!(!WAKE_KEYEVENT.contains("KEYCODE_POWER"), "{WAKE_KEYEVENT}");
    }

    #[test]
    fn an_unreadable_dumpsys_is_unknown_and_must_not_block() {
        // A build that prints neither key is not evidence of anything. Returning
        // `Some(false)` here would let a start proceed on a phone we cannot read;
        // returning `Some(true)` would refuse every such build outright.
        assert_eq!(parse_display_awake(""), None);
        assert_eq!(parse_display_awake("mWakefulness=SomethingNew"), None);
        assert_eq!(parse_keyguard_locked(""), None);
        assert_eq!(parse_keyguard_locked("Window #0 mCurrentFocus=null"), None);
    }

    #[test]
    fn an_unlocked_phone_reports_unlocked_on_every_accepted_key() {
        for key in [
            "isKeyguardShowing",
            "mKeyguardShowing",
            "mDreamingLockscreen",
        ] {
            assert_eq!(
                parse_keyguard_locked(&format!("    {key}=false\n")),
                Some(false),
                "{key}"
            );
            assert_eq!(
                parse_keyguard_locked(&format!("    {key}=true\n")),
                Some(true),
                "{key}"
            );
        }
    }

    #[test]
    fn one_key_saying_locked_outweighs_another_saying_unlocked() {
        // Deliberate asymmetry: refusing a working phone is a clear message, while
        // proceeding onto a lock screen taps something nobody chose.
        let mixed = "    mDreamingLockscreen=false\n    isKeyguardShowing=true\n";
        assert_eq!(parse_keyguard_locked(mixed), Some(true));
        let reversed = "    isKeyguardShowing=true\n    mDreamingLockscreen=false\n";
        assert_eq!(parse_keyguard_locked(reversed), Some(true));
    }

    #[test]
    fn a_trailing_field_on_the_same_line_does_not_bleed_into_the_value() {
        // The real body packs several fields per line
        // (`mShowingDream=false mDreamingLockscreen=true`), so the parser must stop
        // at whitespace rather than taking the rest of the line.
        assert_eq!(
            parse_keyguard_locked("    mShowingDream=false mDreamingLockscreen=true\n"),
            Some(true)
        );
        assert_eq!(
            parse_keyguard_locked("    mDreamingLockscreen=false mShowingDream=true\n"),
            Some(false)
        );
    }

    /// These read the environment rather than setting it, on purpose: mutating
    /// `ANDROID_HOME` here would race every other test in the binary, and the
    /// properties worth pinning — where `configured`, `PATH` and `bundled` sit
    /// relative to each other — hold whatever the environment happens to contain.
    mod candidate_order {
        use super::*;

        fn positions(candidates: &[AdbCandidate], origin: AdbOrigin) -> Vec<usize> {
            candidates
                .iter()
                .enumerate()
                .filter(|(_, candidate)| candidate.origin == origin)
                .map(|(index, _)| index)
                .collect()
        }

        fn only(candidates: &[AdbCandidate], origin: AdbOrigin) -> usize {
            let found = positions(candidates, origin);
            assert_eq!(found.len(), 1, "expected exactly one {origin:?}");
            found[0]
        }

        #[test]
        fn the_bundled_copy_is_tried_after_path_not_before_it() {
            // The load-bearing one. A bundled adb of a different revision than the
            // operator's kills their adb server on port 5037, so ours may only be
            // reached once everything they installed has failed to answer.
            let candidates = AdbProgram::candidates(None, Some(Path::new("/bundle/adb")));
            assert!(
                only(&candidates, AdbOrigin::Bundled) > only(&candidates, AdbOrigin::Path),
                "bundled must come after PATH, got {candidates:?}"
            );
            assert_eq!(
                candidates.last().map(|c| c.origin),
                Some(AdbOrigin::Bundled)
            );
        }

        #[test]
        fn a_configured_path_outranks_everything_including_the_bundled_copy() {
            let candidates =
                AdbProgram::candidates(Some(Path::new("/chosen/adb")), Some(Path::new("/b/adb")));
            assert_eq!(only(&candidates, AdbOrigin::Configured), 0);
        }

        #[test]
        fn path_is_always_a_candidate_because_only_running_it_can_answer() {
            // There is nothing to stat for a bare name, so it cannot be filtered out
            // the way a missing file can. Leaving it out when some SDK variable is set
            // is how a machine with adb on PATH gets reported as having none.
            assert_eq!(
                positions(&AdbProgram::candidates(None, None), AdbOrigin::Path).len(),
                1
            );
        }

        #[test]
        fn nothing_bundled_means_no_bundled_candidate_rather_than_an_empty_path() {
            // A `Some(PathBuf::new())` here would become a candidate that spawns the
            // current directory. Absent must stay absent.
            let candidates = AdbProgram::candidates(None, None);
            assert!(positions(&candidates, AdbOrigin::Bundled).is_empty());
        }

        #[test]
        fn resolve_takes_a_configured_file_that_exists() {
            // Deliberately asserted through `configured` rather than `bundled`: the
            // bundled slot sits after the SDK variables, and a CI runner has
            // `ANDROID_HOME` set to a real SDK (GitHub's windows images do), so a test
            // expecting the bundled copy to win would pass here and fail there. Same
            // code path, environment-independent claim.
            let dir = std::env::temp_dir().join("riviu-adb-resolve-test");
            std::fs::create_dir_all(&dir).expect("temp dir");
            let chosen = dir.join(exe_name());
            std::fs::write(&chosen, b"not really adb").expect("write");
            let resolved = AdbProgram::resolve(Some(&chosen), None).expect("resolve");
            assert_eq!(resolved.path(), chosen.as_path());
            std::fs::remove_file(&chosen).ok();
        }

        #[test]
        fn resolve_never_answers_with_a_path_that_does_not_exist() {
            // The property, stated so the environment cannot decide it. The first version
            // asserted the answer *equals* the bare name, which is only true when no SDK
            // variable happens to point at a real adb — and GitHub's Windows runners set
            // `ANDROID_HOME` to exactly that. It passed here and failed on CI, which is the
            // trap the sibling test's comment already warned about; I had fixed only one of
            // the two.
            //
            // What matters either way: a configured path that is not there must never come
            // back, because it cannot be spawned. The answer is an existing file or the bare
            // name for the OS to resolve, and nothing else.
            let missing = std::env::temp_dir().join("riviu-adb-absent").join("adb");
            let resolved = AdbProgram::resolve(Some(&missing), None).expect("resolve");
            assert_ne!(
                resolved.path(),
                missing.as_path(),
                "a path that does not exist must never be the answer"
            );
            assert!(
                resolved.path() == Path::new(exe_name()) || resolved.path().is_file(),
                "resolve returned {:?}, which is neither an existing file nor the bare name",
                resolved.path()
            );
        }

        #[test]
        fn every_origin_explains_itself_in_the_operator_s_language() {
            for origin in [
                AdbOrigin::Configured,
                AdbOrigin::RiviuAdbPath,
                AdbOrigin::AndroidSdkRoot,
                AdbOrigin::AndroidHome,
                AdbOrigin::Path,
                AdbOrigin::Bundled,
            ] {
                assert!(!origin.label().is_empty(), "{origin:?} has no label");
            }
        }
    }
}
