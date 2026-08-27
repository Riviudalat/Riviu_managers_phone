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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
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

/// At most this many of the slots above may be held by a **file transfer** at once.
///
/// A sub-cap inside the global cap, not a second lane beside it: the table above says the fleet
/// stops getting faster past twelve concurrent adb processes, so adding a parallel lane would
/// push the total past the number that was measured. A transfer takes one of the twelve *and*
/// one of these four.
///
/// **Why transfers need their own number at all.** `pull_device_path` and `push_device_file`
/// pass a 300-second timeout, and the permit is held for the whole command. With one cap,
/// twelve simultaneous exports hold every slot for up to five minutes, and during that window
/// every other adb call in the process queues behind them: device probes, screenshots,
/// `ensure_stream`, every nurture and interaction action, the boot roster scan. The farm looks
/// frozen while nothing is wrong with it. Four leaves eight slots for work that answers in
/// milliseconds.
const ADB_MAX_TRANSFERS: usize = 4;

/// Which lane a call belongs to: work that answers in milliseconds, or work that moves bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdbLane {
    Interactive,
    Transfer,
}

/// The serial a call is aimed at and the verb it invokes, read off the argv.
///
/// `-s <serial>` is prepended by [`AdbProgram::device`] and absent otherwise, so neither can be
/// read at a fixed index. Pure, and separate from the slot machinery, because "which phone is
/// this for" is the fact the per-serial queue depends on and getting it wrong would silently
/// serialise nothing.
fn adb_target<'a>(args: &[&'a str]) -> (Option<&'a str>, Option<&'a str>) {
    let mut rest = args;
    let mut serial = None;
    if let [flag, value, tail @ ..] = rest {
        if *flag == "-s" {
            serial = Some(*value);
            rest = tail;
        }
    }
    (serial, rest.first().copied())
}

/// `pull` and `push` move bytes; everything else answers a question.
fn adb_lane(verb: Option<&str>) -> AdbLane {
    match verb {
        Some("pull") | Some("push") => AdbLane::Transfer,
        _ => AdbLane::Interactive,
    }
}

/// The cap is **global**, because the thing it rations is global: one adb server per host.
///
/// Two `AdbProgram` values are two handles onto the same server, so a per-instance limit
/// would bound nothing. `detect_driver` alone builds one instance per candidate path while
/// probing.
fn adb_slots() -> &'static Semaphore {
    static SLOTS: OnceLock<Semaphore> = OnceLock::new();
    SLOTS.get_or_init(|| Semaphore::new(ADB_MAX_CONCURRENT))
}

fn adb_transfer_slots() -> &'static Semaphore {
    static SLOTS: OnceLock<Semaphore> = OnceLock::new();
    SLOTS.get_or_init(|| Semaphore::new(ADB_MAX_TRANSFERS))
}

/// One queue per phone, each one deep enough for exactly one call.
///
/// **The global cap alone let two calls reach the same phone at once, and that is a
/// correctness problem rather than a throughput one.** Two `adb shell` scripts interleaving on
/// one device is not slower, it is a different thing happening. GenFarmer's own adb layer is
/// two-tier for this reason -- a sequential queue per serial, then a global cap -- and the
/// survey of it in this repo says plainly that Riviu was missing the first half.
///
/// Never evicted. The key space is serials, so it is bounded by the hardware on the bench
/// (twenty phones, plus a wifi-adb address each at most), every value is a one-permit
/// semaphore, and removing an entry while a permit is outstanding would break the mutual
/// exclusion it exists to provide.
fn adb_device_queues() -> &'static parking_lot::Mutex<HashMap<String, Arc<Semaphore>>> {
    static QUEUES: OnceLock<parking_lot::Mutex<HashMap<String, Arc<Semaphore>>>> = OnceLock::new();
    QUEUES.get_or_init(|| parking_lot::Mutex::new(HashMap::new()))
}

/// One phone's adb queue, held without taking a global slot.
///
/// **For the three callers that start a child which outlives the call.** `spawn_instrumentation`,
/// minicap and the scrcpy server must not hold an [`AdbSlot`], because the child never returns
/// and a permit it holds never comes back -- that is the deadlock the two-tier queue exists to
/// avoid, and `nothing_spawns_adb_outside_the_queue_without_saying_why` declares them exempt
/// for exactly that reason.
///
/// But "must not hold the global slot" is not "may interleave on the phone". Startup is the
/// window where interleaving hurts most: while `am instrument -w` is acquiring `UiAutomation`,
/// a concurrent gesture that finds the serial's queue free opens a second adb transport to the
/// same phone, and the gesture then reaches a session that is half-created or already dead.
/// This is the middle ground: mutual exclusion per phone, no global slot, released as soon as
/// the child has crossed its readiness check.
///
/// Safe to hold across an await **only** if nothing in that window takes an adb permit for the
/// same serial -- otherwise it deadlocks the phone. Traced for the instrumentation path on
/// 27/08/2026: `is_ready`, `connect`, `is_alive` and `close` are all `reqwest` over the
/// already-established forward, so no adb call happens between the spawn and readiness.
///
/// Found by an independent review on 27/08/2026.
pub struct AdbDeviceHold {
    _device: tokio::sync::OwnedSemaphorePermit,
}

/// Take one phone's adb queue and nothing else. See [`AdbDeviceHold`].
pub async fn hold_device_queue(serial: &str) -> AdbDeviceHold {
    let queue = {
        let mut queues = adb_device_queues().lock();
        Arc::clone(
            queues
                .entry(serial.to_string())
                .or_insert_with(|| Arc::new(Semaphore::new(1))),
        )
    };
    AdbDeviceHold {
        _device: queue
            .acquire_owned()
            .await
            .expect("a per-device adb queue is never closed"),
    }
}

/// Everything one adb call holds while it runs. Dropping it releases all of them.
struct AdbSlot {
    _device: Option<tokio::sync::OwnedSemaphorePermit>,
    _transfer: Option<tokio::sync::SemaphorePermit<'static>>,
    _global: tokio::sync::SemaphorePermit<'static>,
}

/// Wait for everything this call needs, and say so if the wait was long.
///
/// **Acquisition order is fixed and load-bearing: phone, then transfer sub-cap, then global.**
/// Most specific first. Taking the global slot before waiting on a phone's own queue would let
/// several calls queued behind one busy phone sit on global slots doing nothing, which is the
/// starvation this is meant to prevent rather than cause. A fixed order is also what makes the
/// three levels deadlock-free.
async fn enter_adb_slot(serial: Option<&str>, what: &str, lane: AdbLane) -> AdbSlot {
    let waiting_since = Instant::now();

    let device = match serial {
        Some(serial) => {
            let queue = {
                let mut queues = adb_device_queues().lock();
                Arc::clone(
                    queues
                        .entry(serial.to_string())
                        .or_insert_with(|| Arc::new(Semaphore::new(1))),
                )
            };
            Some(
                queue
                    .acquire_owned()
                    .await
                    .expect("a per-device adb queue is never closed"),
            )
        }
        None => None,
    };

    let transfer = match lane {
        AdbLane::Transfer => Some(
            adb_transfer_slots()
                .acquire()
                .await
                .expect("the adb transfer semaphore is never closed"),
        ),
        AdbLane::Interactive => None,
    };

    let global = adb_slots()
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
            transfers = ADB_MAX_TRANSFERS,
            command = what,
            serial = serial.unwrap_or("-"),
            "waited for an adb slot; the host is running its cap of concurrent adb calls"
        );
    }

    AdbSlot {
        _device: device,
        _transfer: transfer,
        _global: global,
    }
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
        let (serial, verb) = adb_target(args);
        let _slot = enter_adb_slot(serial, verb.unwrap_or("adb"), adb_lane(verb)).await;
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
        let _slot = enter_adb_slot(Some(serial), "shell", AdbLane::Interactive).await;
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
        // Only a *successful* read is eligible to be one half of an agreeing pair. A
        // failed read is not a fleet of zero phones, and letting it stand in for one is
        // how two errors in a row used to produce `stable: true, devices: []`.
        let mut previous: Option<Vec<AdbDeviceLine>> = None;
        let mut attempts = 0u32;
        // The last thing actually read from adb, so a failure reports the fleet it knew
        // rather than an empty one it invented.
        let mut last_good: Option<Vec<AdbDeviceLine>> = None;
        // No initialiser: both match arms below assign it before the deadline check
        // reads it, so an initial `None` would be a dead write -- and clippy runs with
        // `-D warnings` here.
        let mut failure: Option<String>;
        loop {
            attempts += 1;
            match self.run(&["devices", "-l"], DEFAULT_TIMEOUT).await {
                Ok(stdout) => {
                    let reading = parse_devices(&stdout);
                    failure = None;
                    if let Some(before) = previous.as_ref() {
                        if same_fleet(before, &reading) {
                            return DeviceListReading {
                                devices: reading,
                                stable: true,
                                attempts,
                                failure: None,
                            };
                        }
                    }
                    last_good = Some(reading.clone());
                    previous = Some(reading);
                }
                Err(error) => {
                    tracing::warn!(%error, "adb devices failed while waiting for a stable list");
                    failure = Some(error.to_string());
                    // Drop the pending comparison: the next successful read has nothing
                    // trustworthy to agree *with*, so it must not short-circuit to stable.
                    previous = None;
                }
            }
            if started.elapsed() >= deadline {
                return DeviceListReading {
                    devices: last_good.unwrap_or_default(),
                    stable: false,
                    attempts,
                    failure,
                };
            }
            tokio::time::sleep(settle).await;
        }
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

/// The three device-identity values, checked before they reach a **root** shell.
///
/// `set_device_identity` pastes these into `su -c "…"`, and inside those double quotes `$(…)`
/// and backticks still substitute — so a value like `x"; sh -c 'curl …|sh'; #` is not a bad
/// serial, it is root code execution on the phone. The shipped UI generates all three locally,
/// but they arrive as three free `Option<String>` on a registered Tauri command, so the gap is
/// at the trust boundary rather than behind it.
///
/// Rejecting beats escaping, the same call this file already makes for package names and device
/// paths: all three have narrow, fully specified grammars, so anything outside is a mistake or
/// an attack and neither should be quoted and run.
mod identity {
    use anyhow::anyhow;

    /// 16 lowercase hex digits — the shape `settings get secure android_id` returns.
    pub fn validate_android_id(value: &str) -> anyhow::Result<&str> {
        if value.len() == 16 && value.chars().all(|ch| ch.is_ascii_hexdigit()) {
            Ok(value)
        } else {
            Err(anyhow!("not a valid android_id (16 hex digits): {value:?}"))
        }
    }

    /// Alphanumerics, and the two separators Samsung/Xiaomi serials actually use.
    pub fn validate_serial_no(value: &str) -> anyhow::Result<&str> {
        let ok = (1..=64).contains(&value.len())
            && value
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
        if ok {
            Ok(value)
        } else {
            Err(anyhow!("not a valid serial number: {value:?}"))
        }
    }

    /// `xx:xx:xx:xx:xx:xx`. `ip link set … address` takes nothing else.
    pub fn validate_mac(value: &str) -> anyhow::Result<&str> {
        let mut octets = 0usize;
        for octet in value.split(':') {
            octets += 1;
            if octet.len() != 2 || !octet.chars().all(|ch| ch.is_ascii_hexdigit()) {
                return Err(anyhow!("not a valid MAC address: {value:?}"));
            }
        }
        if octets == 6 {
            Ok(value)
        } else {
            Err(anyhow!("not a valid MAC address: {value:?}"))
        }
    }
}

pub use identity::{validate_android_id, validate_mac, validate_serial_no};

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

/// Not `Copy`, because [`Self::Other`] carries the word adb actually printed.
///
/// Carrying it is the point: `recovery`, `sideload`, `bootloader` and `no permissions` all
/// land here, and each has a different fix. A variant that forgets which one it was can
/// only produce a message nobody can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdbDeviceState {
    /// Authorised and usable.
    Device,
    /// Plugged in, but the USB-debugging prompt has not been accepted. Measured
    /// on the fleet: one device sat in this state, so it is a normal condition
    /// to report rather than an error to hide.
    Unauthorized,
    /// Known to adb and not answering — the usual shape of a phone whose cable or hub has
    /// dropped, and the usual shape of one that is mid-reboot.
    Offline,
    /// Anything else adb printed, kept verbatim.
    Other(String),
}

impl AdbDeviceState {
    /// What to tell an operator looking at a phone in this state.
    ///
    /// `None` for a device that is simply usable. Everything else has a sentence, because a
    /// device that cannot be driven and cannot say why is a device that looks unplugged.
    pub fn operator_reason(&self) -> Option<String> {
        match self {
            Self::Device => None,
            Self::Unauthorized => {
                Some("USB debugging not allowed yet — accept the prompt on the device".to_string())
            }
            Self::Offline => Some(
                "adb sees this device but it is not answering — check the cable or the USB hub, \
                 or wait if it is rebooting"
                    .to_string(),
            ),
            Self::Other(state) => Some(format!(
                "adb reports this device as `{state}`, which cannot be driven"
            )),
        }
    }
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

/// One `adb devices -l` reading, with whether it settled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceListReading {
    pub devices: Vec<AdbDeviceLine>,
    /// Two consecutive **successful** reads agreed. `false` means the deadline passed
    /// while the fleet was still changing, or a read failed — the list is the last
    /// thing seen, not a promise.
    pub stable: bool,
    pub attempts: u32,
    /// Why the last read could not be trusted, when it could not.
    ///
    /// **Separate from `stable` because an empty fleet and a failed read are not the
    /// same claim, and the old code could not tell them apart.** `Err` from `adb
    /// devices` became `Vec::new()`, so two consecutive failures compared equal and
    /// returned `stable: true` with zero devices — a *confident* report that every
    /// phone had been unplugged. `list_devices` then handed that to
    /// `DeviceRegistry::upsert_many`, which replaces the whole vector, and the entire
    /// fleet vanished from the grid on a transient adb hiccup. The `!stable` warning
    /// that was supposed to catch this never fired, because nothing was unstable:
    /// both readings agreed, and both were fabricated.
    ///
    /// Found by an independent review on 27/08/2026, in a function whose own comment
    /// already reasoned about "an unstable reading" — the case it had not considered
    /// is a reading that never happened.
    pub failure: Option<String>,
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
                other => AdbDeviceState::Other(other.to_string()),
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

/// The rendered display as it is *right now*, rotation included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayGeometry {
    /// Pixels across the screen in its current orientation.
    pub width: u32,
    /// Pixels down the screen in its current orientation.
    pub height: u32,
    /// Density in dpi. Divide by 160 for the density-independent scale factor.
    pub density: u32,
    /// `Surface.ROTATION_*` as an index: 0, 1, 2, 3.
    pub rotation: u8,
}

/// Parse `dumpsys display` for the size, density and rotation in force.
///
/// **This exists because `wm size` cannot answer the question.** Measured 16/08/2026 on
/// SM-G955F, turned to landscape with Settings in front: `wm size` kept saying
/// `Override size: 1080x2220` while `dumpsys display` moved to `real 2220 x 1080`.
/// `wm size` reports the display's base configuration, which has no orientation in it at
/// all (AGENTS.md §9.59). Anything that needs the geometry a coordinate was picked
/// against has to read it here.
///
/// `mOverrideDisplayInfo` first, `mBaseDisplayInfo` as the fallback, for the same reason
/// [`parse_wm_size`] prefers the override line: every phone on this fleet reports
/// `real 1440 x 2960, density 560` as its base and `real 1080 x 2220, density 420` as its
/// override, and the override is what is rendered. Reading the base puts every derived
/// coordinate 33% out.
///
/// Parsed a line at a time rather than by matching the `DisplayInfo{...}` block, because
/// that block contains nested braces (`modes [{id=1, ...}]`) — a `[^}]*}` scan stops
/// inside `modes` and never reaches `rotation` or `density`. All three values sit on the
/// one line, so the line is the unit.
pub fn parse_display_geometry(stdout: &str) -> Option<DisplayGeometry> {
    fn from_line(line: &str) -> Option<DisplayGeometry> {
        // `real W x H`, not `app W x H`: the same line also carries `app`, `largest app`
        // and `smallest app`, which exclude the system bars and are smaller.
        let (width, height) = after(line, "real ").and_then(size_pair)?;
        let density = after(line, "density ")?
            .split_whitespace()
            .next()?
            .parse()
            .ok()?;
        let rotation = after(line, "rotation ")?
            .split_whitespace()
            .next()?
            .trim_end_matches(',')
            .parse()
            .ok()
            .filter(|value| *value < 4)?;
        (width > 0 && height > 0 && density > 0).then_some(DisplayGeometry {
            width,
            height,
            density,
            rotation,
        })
    }

    fn after<'a>(line: &'a str, key: &str) -> Option<&'a str> {
        line.find(key).map(|at| &line[at + key.len()..])
    }

    fn size_pair(rest: &str) -> Option<(u32, u32)> {
        let mut parts = rest.split_whitespace();
        let width = parts.next()?.parse().ok()?;
        if parts.next()? != "x" {
            return None;
        }
        let height = parts.next()?.trim_end_matches(',').parse().ok()?;
        Some((width, height))
    }

    for key in ["mOverrideDisplayInfo=", "mBaseDisplayInfo="] {
        if let Some(found) = stdout
            .lines()
            .filter(|line| line.contains(key))
            .find_map(from_line)
        {
            return Some(found);
        }
    }
    None
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
    match parse_foreground_window(stdout) {
        ForegroundWindow::App(package) => Some(package),
        ForegroundWindow::System(_) | ForegroundWindow::Unreadable => None,
    }
}

/// What actually owns the screen, as `dumpsys window` describes it.
///
/// [`parse_current_focus_package`] answers `None` for two situations that are nothing alike,
/// and telling them apart is the whole reason this exists. Measured across all fourteen
/// phones on 23/08/2026:
///
/// ```text
/// mCurrentFocus=Window{be9279f u0 com.ss.android.ugc.trill/…SplashActivity}   -> App
/// mCurrentFocus=Window{4b5766b u0 StatusBar}                                  -> System
/// ```
///
/// The second is what a **locked** phone shows: the keyguard hands focus to `StatusBar`
/// (or `NotificationShade` on other builds), which carries no `package/activity` pair, so
/// the `/` split finds nothing and the old function reported "no foreground app". Two of the
/// fourteen were in exactly that state, and the nurture session they were given failed with
/// *"the phone is showing &lt;unreadable: could not read the foreground package&gt;"* — a
/// sentence that describes the parser rather than the phone. The phone was on its lock
/// screen, which is a thing the app can fix; "unreadable" is not.
///
/// [`Self::Unreadable`] stays a distinct answer because it is a distinct fact: no
/// `mCurrentFocus` line said anything at all, which means adb answered but this build does
/// not print what we expected. That is a reason to stop, not to press keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForegroundWindow {
    /// An app activity is focused; the payload is its package.
    App(String),
    /// A window with no package — the keyguard's `StatusBar`, a notification shade, a
    /// system dialog. The payload is the window name, verbatim, so a log line can say
    /// which one.
    System(String),
    /// Nothing on any `mCurrentFocus` line could be read.
    Unreadable,
}

/// Read [`ForegroundWindow`] out of `dumpsys window` (or `dumpsys window windows`).
///
/// Scans **every** `mCurrentFocus` line for the same reason
/// [`parse_current_focus_package`] does: a multi-display phone prints one per display and
/// the unfocused ones say `null`, so the first line is not the answer. An app anywhere in
/// the list wins over a system window, because a phone with TikTok focused on one display
/// and a shade open on another is a phone TikTok is running on.
/// The activity half of the focused window, when there is one.
///
/// `parse_foreground_window` answers the package and drops the activity on the floor, which
/// is the right shape for "is my app in front" and the wrong one for "is my app *ready*".
/// TikTok's splash screen belongs to the same package as its feed, so a proof that only reads
/// the package is satisfied by a phone that has started the app and shown nothing yet.
///
/// Measured 25/08/2026, twenty-phone run: eight phones sat on
/// `com.ss.android.ugc.aweme.splash.SplashActivity` and five of them failed the interaction
/// with `no_baseline` — "could not read an author label", which describes the symptom and
/// sends the operator looking in the wrong place.
pub fn parse_foreground_activity(stdout: &str) -> Option<String> {
    for line in stdout.lines().filter(|line| line.contains("mCurrentFocus")) {
        let Some(after) = line.split_once("mCurrentFocus=") else {
            continue;
        };
        let inside = after
            .1
            .trim()
            .trim_start_matches("Window{")
            .trim_end_matches('}');
        let Some(what) = inside.split_whitespace().nth(2) else {
            continue;
        };
        if let Some((_package, activity)) = what.split_once('/') {
            let activity = activity.trim_end_matches('}').trim();
            if !activity.is_empty() {
                return Some(activity.to_string());
            }
        }
    }
    None
}

/// Whether an activity name is a launch/splash screen rather than the app proper.
///
/// A name match, and deliberately narrow: the only thing it must catch is the state where the
/// package is up and the app has not drawn itself yet. A build that names its splash something
/// else simply behaves as before — the caller falls through to its own timeout.
pub fn is_splash_activity(activity: &str) -> bool {
    activity.to_ascii_lowercase().contains("splash")
}

pub fn parse_foreground_window(stdout: &str) -> ForegroundWindow {
    let mut system: Option<String> = None;
    for line in stdout.lines().filter(|line| line.contains("mCurrentFocus")) {
        // `mCurrentFocus=Window{<hash> u0 <what>}` — take what is inside the braces after
        // the user id, then decide by whether it carries a `package/activity` pair.
        let Some(after) = line.split_once("mCurrentFocus=") else {
            continue;
        };
        let inside = after
            .1
            .trim()
            .trim_start_matches("Window{")
            .trim_end_matches('}');
        let Some(what) = inside.split_whitespace().nth(2) else {
            continue;
        };
        if what == "null" || what.is_empty() {
            continue;
        }
        match what.split_once('/') {
            Some((package, _activity)) if !package.is_empty() => {
                return ForegroundWindow::App(package.to_string())
            }
            // A name with no slash is a system window. Remember it, but keep looking: an
            // app on another display outranks it.
            _ => {
                if system.is_none() {
                    system = Some(what.trim_end_matches('}').to_string());
                }
            }
        }
    }
    system.map_or(ForegroundWindow::Unreadable, ForegroundWindow::System)
}

/// The keys that dismiss a swipe-only keyguard, in the order they must be sent.
///
/// **Measured on this fleet on 23/08/2026, on both locked phones**: `mDreamingLockscreen`
/// went `true` → `false` and TikTok appeared in `mCurrentFocus` immediately afterwards. So
/// these phones carry no secure lock, and the pair is enough.
///
/// `KEYCODE_WAKEUP` first and never `KEYCODE_POWER`: power *toggles*, so on an
/// already-awake phone it would turn the screen off — see [`WAKE_KEYEVENT`].
/// `KEYCODE_MENU` is what actually dismisses the keyguard, and it is safe on an unlocked
/// phone: no app in this fleet's flow has a menu key binding, and Android 9 delivers it to
/// the focused window which ignores it.
///
/// **This cannot open a phone with a PIN, pattern or fingerprint**, and must not pretend
/// to. The caller re-reads [`parse_keyguard_locked`] afterwards and reports the honest
/// answer if the phone is still locked — a human has to unlock that one.
pub const KEYGUARD_DISMISS_KEYEVENTS: [&str; 2] = [
    "input keyevent KEYCODE_WAKEUP",
    "input keyevent KEYCODE_MENU",
];

/// The command that prints the Wi-Fi interface address, for WIFI-adb (feature A4). `ip addr`
/// is present on modern Android; the caller pairs it with [`parse_wlan_ipv4`].
pub const WLAN_IP_SHELL: &str = "ip -f inet addr show wlan0";

/// Parse the host's `arp -a` table (Windows format) into `(ip, mac)` pairs, for discovering
/// phones on the LAN to `adb connect` (feature A9). Header/interface lines and incomplete
/// entries are skipped; only IPv4 rows with a MAC survive.
pub fn parse_arp_table(stdout: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let mut cols = line.split_whitespace();
        let (Some(ip), Some(mac)) = (cols.next(), cols.next()) else {
            continue;
        };
        let octets: Vec<&str> = ip.split('.').collect();
        let is_ipv4 = octets.len() == 4 && octets.iter().all(|o| o.parse::<u8>().is_ok());
        // Windows prints MACs as aa-bb-cc-dd-ee-ff; require the shape so header words
        // ("Internet", "Interface:") and IPv6 rows do not slip through.
        let is_mac = mac.len() == 17 && mac.split('-').count() == 6;
        if is_ipv4 && is_mac && ip != "255.255.255.255" && !ip.ends_with(".255") {
            out.push((ip.to_string(), mac.to_string()));
        }
    }
    out
}

/// Pull the first IPv4 address out of `ip -f inet addr show wlan0`, e.g. the `192.168.1.42`
/// in `    inet 192.168.1.42/24 brd 192.168.1.255 scope global wlan0`. Loopback and
/// link-local (169.254.x) are skipped — neither is reachable for `adb connect`.
pub fn parse_wlan_ipv4(stdout: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("inet ")?;
        let cidr = rest.split_whitespace().next()?;
        let ip = cidr.split('/').next()?;
        let octets: Vec<&str> = ip.split('.').collect();
        if octets.len() != 4 || !octets.iter().all(|o| o.parse::<u8>().is_ok()) {
            return None;
        }
        if ip == "127.0.0.1" || ip.starts_with("169.254.") {
            return None;
        }
        Some(ip.to_string())
    })
}

/// Check a path before it is pasted into a device shell command, and say why not.
///
/// The companion of [`validate_package_name`], and the reasoning is the same one: `adb
/// shell` runs a real shell on the phone, so a path typed by an operator or clicked out of
/// a listing reaches it as code. What differs is how strict it can afford to be. A package
/// name has a narrow grammar; a *filename* does not — real ones on this fleet contain
/// spaces, dashes and Vietnamese diacritics (`Giao Trinh - Bai Giang - HDH`, measured on
/// 23021RAAEG), so rejecting everything outside `[A-Za-z0-9._-]` would refuse to browse the
/// phone's actual contents.
///
/// So the rule is narrower and provable instead: every path this module sends is wrapped in
/// **single quotes** by [`quote_device_path`], inside which `$`, `&`, `;`, `|`, `<`, `>` and
/// backtick are all inert.
///
/// **An apostrophe used to be rejected here, and that was the wrong half to fix.** A quote is
/// the one character a single-quoted run cannot contain, so refusing it made the quoting
/// provable — at the price of making `John's photo.jpg` impossible to list, export or delete
/// from this app at all. A file an operator can see and cannot touch is not a safety property;
/// it is a missing feature that reads as a bug. [`quote_device_path`] now emits the POSIX escape
/// for it, so the guarantee is unchanged and the file is reachable.
///
/// Still refused: control characters — a newline would survive the quoting intact, but nothing
/// on this fleet has one and a path containing one is far more likely to be a parse mistake
/// upstream than a real name — and anything not anchored at `/`, because a relative path
/// resolves against a working directory the caller never chose.
pub fn validate_device_path(path: &str) -> anyhow::Result<&str> {
    if path.is_empty() {
        anyhow::bail!("đường dẫn rỗng");
    }
    if !path.starts_with('/') {
        anyhow::bail!("đường dẫn phải bắt đầu bằng / (nhận: {path})");
    }
    if path.len() > 1024 {
        anyhow::bail!("đường dẫn dài quá 1024 ký tự");
    }
    if let Some(bad) = path.chars().find(|c| c.is_control()) {
        anyhow::bail!("đường dẫn có ký tự điều khiển U+{:04X}", bad as u32);
    }
    Ok(path)
}

/// Wrap a validated path for a device shell command, apostrophes and all.
///
/// POSIX offers no escape *inside* a single-quoted run, so an apostrophe is emitted by leaving
/// the run, passing a backslash-escaped quote, and opening a new one. The three pieces form one
/// shell word because nothing separates them.
///
/// This replaced a ban: `validate_device_path` used to refuse any path containing an apostrophe,
/// which made the quoting trivially safe and made `John's photo.jpg` unreachable from this app
/// — no listing, no export, no delete. The round-trip test proves the escape by unquoting it the
/// way a shell would, rather than by asserting a literal nobody can read.
pub fn quote_device_path(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}

/// Paths that must never be handed to `rm -rf`, whatever the operator clicked.
///
/// Not a permission model — adb already has whatever rights it has — but a guard against the
/// one gesture that cannot be undone from a UI: a delete aimed at a *root* rather than at
/// something in it. Everything below these survives; the roots themselves do not.
const UNDELETABLE_ROOTS: &[&str] = &[
    "/",
    "/sdcard",
    "/storage",
    "/storage/emulated",
    "/storage/emulated/0",
    "/storage/self",
    "/storage/self/primary",
    "/data",
    "/data/local",
    "/data/local/tmp",
    "/system",
    "/vendor",
    "/mnt",
    "/proc",
    "/sys",
    "/dev",
    "/cache",
    "/config",
    "/apex",
];

/// True when a delete aimed here would take out a whole storage root rather than a file.
/// Trailing slashes are stripped first, because `/sdcard/` and `/sdcard` are the same
/// directory and only one of them would otherwise be caught.
pub fn is_undeletable_root(path: &str) -> bool {
    let trimmed = path.trim_end_matches('/');
    let candidate = if trimmed.is_empty() { "/" } else { trimmed };
    UNDELETABLE_ROOTS.contains(&candidate)
}

/// `2026-08-19`, as `ls -la` prints the date column.
fn looks_like_ls_date(token: &str) -> bool {
    let bytes = token.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
}

/// `15:45`, as `ls -la` prints the time column.
fn looks_like_ls_time(token: &str) -> bool {
    let bytes = token.as_bytes();
    bytes.len() == 5
        && bytes[2] == b':'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 2 || b.is_ascii_digit())
}

/// Split a line into tokens, keeping each one's byte offset so the *name* can be taken as
/// the untouched remainder of the line. Splitting the whole line and re-joining would
/// collapse the runs of spaces inside a filename into one.
fn tokens_with_offsets(line: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (index, ch) in line.char_indices() {
        if ch.is_whitespace() {
            if let Some(from) = start.take() {
                out.push((from, &line[from..index]));
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if let Some(from) = start {
        out.push((from, &line[from..]));
    }
    out
}

/// `Jul`, as a `ls -l` prints the month column when it is not printing an ISO date.
///
/// **Not measured on this fleet, and said plainly so nobody later reads it as measured.** Both
/// ROMs that have been read here -- 23021RAAEG (Android 15) and SM-G955F (Android 9) -- print
/// `YYYY-MM-DD HH:MM`. busybox and the BSD-derived `ls` in other vendor ROMs print
/// `Mon DD HH:MM` for a recent file and `Mon DD  YYYY` for an older one.
///
/// It is handled anyway, and the reason is not the format: it is what the parser did when it
/// met a format it did not know. The fallback took token 6 as the start of the name, so an
/// unrecognised row came back **named after its own date columns** -- and that invented name is
/// what the browser showed and what `rm -rf` and `pull` were handed. Recognising one more real
/// format narrows the hole; `read_ls_listing` reporting the rest is what closes it.
fn looks_like_ls_month(token: &str) -> bool {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    MONTHS.contains(&token)
}

/// A short run of digits: the day column, or the year that replaces the time on an older file.
/// The caller decides which slot it is; this only says the token could be a number.
fn looks_like_ls_number(token: &str, max_len: usize) -> bool {
    !token.is_empty() && token.len() <= max_len && token.bytes().all(|b| b.is_ascii_digit())
}

/// Where the timestamp sits in a row, how many tokens it spans, and how it reads.
///
/// **The whole parse hangs off this.** Column widths are padded per listing and the *number* of
/// columns differs between ROMs -- some print a link-count, some do not -- so nothing can be
/// read at a fixed index. Locating the timestamp settles two things at once: the name is
/// everything after it, and **the size is the token immediately before it**.
///
/// That second part was a live bug. The size used to be read from `tokens[4]` unconditionally,
/// which is right only when the link-count column is present. Without it `tokens[4]` is the
/// *date*, `parse::<u64>()` fails, and `unwrap_or(0)` turned every file in the browser into
/// **0 B** -- silently, because a size of zero reads as a fact rather than as a failure.
/// Where the timestamp can start, and nowhere else.
///
/// **The scan used to run over every token, and a filename can contain a date.** `ls -la`
/// prints `mode links owner group size date time name`, so the date is at index 5; ROMs that
/// omit the link-count column put it at 4. Those are the only two layouts this parser handles
/// -- the `?`-metadata branch below already hard-codes 4 and 6 on the same assumption.
///
/// Unbounded, a row whose metadata is unreadable could resynchronise *inside* the name:
///
/// ```text
/// -r????????? ? ? ? ? ? report 2026-08-27 12:00 final.jpg
/// ```
///
/// The scanner found `2026-08-27 12:00` at index 7, took `report` for the size, and returned
/// the name `final.jpg` -- when the real remainder is `report 2026-08-27 12:00 final.jpg`. A
/// fabricated name is not a cosmetic bug here: it goes on to `pull` and to `rm -rf`.
///
/// Bounded tightly on purpose. A row this rejects is pushed onto `unreadable` and shown to the
/// operator as a line that could not be read, which is safe; a row it accepts wrongly produces
/// a filename nobody has. Found by an independent review on 27/08/2026.
const LS_TIMESTAMP_FIRST: usize = 4;
const LS_TIMESTAMP_LAST: usize = 5;

fn ls_timestamp(tokens: &[(usize, &str)]) -> Option<(usize, usize, String)> {
    let last = LS_TIMESTAMP_LAST.min(tokens.len().saturating_sub(1));
    for index in LS_TIMESTAMP_FIRST..=last {
        let token = tokens[index].1;
        // `2026-07-26 20:29`
        if looks_like_ls_date(token) {
            if let Some((_, time)) = tokens.get(index + 1) {
                if looks_like_ls_time(time) {
                    return Some((index, 2, format!("{token} {time}")));
                }
            }
        }
        // `Jul 11 11:16`, or `Jul 11  2024` once the file is older than six months.
        if looks_like_ls_month(token) {
            let day = tokens.get(index + 1).map(|(_, day)| *day);
            let third = tokens.get(index + 2).map(|(_, third)| *third);
            if let (Some(day), Some(third)) = (day, third) {
                if looks_like_ls_number(day, 2)
                    && (looks_like_ls_time(third) || looks_like_ls_number(third, 4))
                {
                    return Some((index, 3, format!("{token} {day} {third}")));
                }
            }
        }
    }
    None
}

/// Parse one phone's `ls -la` into rows a file browser can draw (xiaowei "Preview Mobile
/// Files").
///
/// **Measured on 23021RAAEG, Android 15, 21/08/2026** — every shape below is a line this
/// fleet actually printed, not a guess at toybox's format:
///
/// ```text
/// total 223
/// drwxrws---  2 u0_a269  media_rw  3452 2024-07-11 11:16 Alarms
/// -rwxrwx--- 1 u0_a269 media_rw 138078 2025-11-25 08:49 CV prototype.pdf
/// lrw-r--r--   1 root   root        11 2009-01-01 07:00 bin -> /system/bin
/// l?????????   ? ?      ?            ?                ? cache -> ?
/// ```
///
/// Three things in there decide the whole implementation. **Column widths are padded per
/// listing**, so nothing can be read at a fixed offset. **Names contain spaces** — and
/// worse, contain ` - ` (`Giao Trinh - Bai Giang - HDH`), so the arrow of a symlink is only
/// looked for on rows whose mode begins with `l`. And **a row the phone cannot stat prints
/// `?` for every column including a merged date/time**, which is one field short of every
/// other row: keying off a field *count* drops it, so the name is found by locating the
/// date-then-time pair and falling back to the seventh token only when there is none.
pub fn parse_ls_listing(stdout: &str) -> Vec<riviu_core::DeviceFileEntry> {
    read_ls_listing(stdout).0
}

/// Every row of a `ls -la`, and the lines that looked like rows but could not be read.
///
/// **The second half is the point.** The old parser had a single path for "no timestamp found":
/// take token 6 as the start of the name. On the one row shape that genuinely has no timestamp
/// -- the unstattable `l?????????  ? ?  ?  ? cache -> ?` -- that is right. On a row whose date
/// format the parser did not know, it is a **fabricated filename**: given
/// `-rw-r--r-- 1 root root 138078 Jul 11 11:16 photo.jpg`, token 6 is `11`, so the file was
/// called `"11 11:16 photo.jpg"` -- and that invented name then went on into `ls`, `rm -rf` and
/// `pull` as though the phone had said it.
///
/// The two cases are now told apart by what actually distinguishes them: an unstattable row
/// prints `?` in its columns. A row with no timestamp *and* no `?` is a shape this parser does
/// not understand, and it is reported instead of guessed at.
pub fn read_ls_listing(stdout: &str) -> (Vec<riviu_core::DeviceFileEntry>, Vec<String>) {
    let mut entries = Vec::new();
    let mut unreadable = Vec::new();
    for line in stdout.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with("total ") {
            continue;
        }
        // `ls: /sdcard/nope: No such file or directory` arrives here whenever the caller
        // merged the two pipes; it is a message, never a row.
        if line.starts_with("ls:") {
            continue;
        }
        let tokens = tokens_with_offsets(line);
        if tokens.len() < 7 {
            continue;
        }
        let (name_start, modified, size_at) = match ls_timestamp(&tokens) {
            Some((index, width, stamp)) => {
                let Some((offset, _)) = tokens.get(index + width) else {
                    // A timestamp with nothing after it is not a row with an empty name; it is
                    // a line this parser has misread. Say so rather than dropping it.
                    unreadable.push(line.trim().to_string());
                    continue;
                };
                (*offset, Some(stamp), index.checked_sub(1))
            }
            None => {
                if !tokens.iter().any(|(_, token)| token.contains('?')) {
                    unreadable.push(line.trim().to_string());
                    continue;
                }
                (tokens[6].0, None, Some(4))
            }
        };
        let rest = line[name_start..].trim();
        if rest.is_empty() {
            unreadable.push(line.trim().to_string());
            continue;
        }
        let mode = tokens[0].1;
        let kind = match mode.as_bytes().first() {
            Some(b'd') => riviu_core::DeviceFileKind::Directory,
            Some(b'l') => riviu_core::DeviceFileKind::Symlink,
            Some(b'-') => riviu_core::DeviceFileKind::File,
            _ => riviu_core::DeviceFileKind::Other,
        };
        // Only on a symlink row, and only the *last* arrow: a name may contain " - " but a
        // target may itself be a path with spaces, so the split has to be the final one.
        let (name, link_target) = if kind == riviu_core::DeviceFileKind::Symlink {
            match rest.rsplit_once(" -> ") {
                Some((name, target)) => (name.trim(), Some(target.trim().to_string())),
                None => (rest, None),
            }
        } else {
            (rest, None)
        };
        if name.is_empty() || name == "." || name == ".." {
            continue;
        }
        entries.push(riviu_core::DeviceFileEntry {
            name: name.to_string(),
            kind,
            size: size_at
                .and_then(|at| tokens.get(at))
                .and_then(|(_, token)| token.parse::<u64>().ok())
                .unwrap_or(0),
            modified,
            link_target,
        });
    }
    (entries, unreadable)
}

/// What a `ls -la` on a phone actually told us.
///
/// **The three-way split is the fix.** `list_device_dir` used to decide with one condition,
/// `entries.is_empty() && exit_code != 0`, and that leaves three holes -- every one of which
/// makes the browser state something false rather than fail:
///
/// 1. **exit 0 with a complaint on stderr.** Some ROMs refuse a directory and still exit 0.
///    `is_empty()` is true, `exit_code != 0` is false, so an empty list came back and the
///    browser drew **an empty folder** -- claiming the directory exists and holds nothing.
/// 2. **Some rows plus a complaint.** `ls -la` on a directory whose children are partly
///    unreadable prints what it can and complains about the rest. `is_empty()` is false, so the
///    truncated list came back and the browser drew it as **complete**.
/// 3. **A ROM that merges stderr into stdout.** Then `stderr` is empty and the exit code is 0,
///    so neither half of the condition ever fires.
///
/// So: complaints are collected from **both** pipes, and "nothing to show" is separated from
/// "nothing is here".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LsOutcome {
    /// Everything the phone has here, and it said nothing else.
    Complete(Vec<riviu_core::DeviceFileEntry>),
    /// What could be read, and what the phone said about the rest. **Never draw as complete.**
    Partial {
        entries: Vec<riviu_core::DeviceFileEntry>,
        reason: String,
    },
    /// Nothing could be read. **Not an empty folder** -- that is the distinction the single
    /// condition lost.
    Refused(String),
}

/// True for a line that is a complaint rather than a row, from either pipe.
///
/// Matched on shape rather than on an exact sentence: `ls` prefixes its own name, and the rest
/// varies by ROM and by locale. The two bodies listed are the ones this fleet prints, kept so a
/// ROM that drops the `ls:` prefix is still recognised.
fn is_ls_complaint(line: &str) -> bool {
    let line = line.trim();
    if line.is_empty() {
        return false;
    }
    line.starts_with("ls:")
        || line.contains("Permission denied")
        || line.contains("No such file or directory")
        || line.contains("Not a directory")
}

/// Read one `ls -la` result the way the caller has to act on it.
///
/// Pure, and separate from the command that produces it, because the interesting cases are
/// combinations of three inputs that are awkward to reach against a real phone -- and each one
/// of them was, until now, drawn as a fact.
pub fn classify_ls_output(stdout: &str, stderr: &str, exit_code: i32) -> LsOutcome {
    let (entries, unreadable) = read_ls_listing(stdout);

    let mut complaints: Vec<String> = stderr
        .lines()
        .chain(stdout.lines())
        .map(|line| line.trim_end_matches('\r').trim())
        .filter(|line| is_ls_complaint(line))
        .map(|line| line.to_string())
        .collect();
    complaints.dedup();

    // A row this parser could not read is exactly as much of a gap as a refused child, and the
    // operator has the same right to know: the list in front of them is short.
    if !unreadable.is_empty() {
        complaints.push(format!(
            "{} dòng không đọc được: {}",
            unreadable.len(),
            unreadable.join(" | ")
        ));
    }

    if entries.is_empty() {
        if !complaints.is_empty() {
            return LsOutcome::Refused(complaints.join("; "));
        }
        if exit_code != 0 {
            // No sentence anywhere, but the shell says it failed. Better to name the code than
            // to present the silence as an empty directory.
            return LsOutcome::Refused(format!("ls trả mã lỗi {exit_code}"));
        }
        // Genuinely nothing here. An empty directory is a real thing and has to stay sayable.
        return LsOutcome::Complete(Vec::new());
    }

    if complaints.is_empty() && exit_code == 0 {
        LsOutcome::Complete(entries)
    } else {
        let reason = if complaints.is_empty() {
            format!("ls trả mã lỗi {exit_code}")
        } else {
            complaints.join("; ")
        };
        LsOutcome::Partial { entries, reason }
    }
}

#[cfg(test)]
mod tests {
    /// A sentence an operator reads must not carry the source code's indentation.
    ///
    /// Rust joins a literal split across lines *including* the leading spaces of the next
    /// line, unless a trailing backslash swallows them. The offline reason had eighteen of
    /// them in its middle, so the grid offered "check the cable or the USB hub,
    /// or wait" with a hole in it — which reads as a rendering fault in the app rather than
    /// as advice, right at the moment somebody is trying to work out why a phone vanished.
    /// Found by scanning for the shape rather than by reading, which is the only way: it is
    /// invisible in the source, where it looks like ordinary wrapping.
    #[test]
    fn every_reason_is_one_clean_sentence() {
        for state in [
            AdbDeviceState::Device,
            AdbDeviceState::Unauthorized,
            AdbDeviceState::Offline,
            AdbDeviceState::Other("sideload".into()),
        ] {
            let Some(reason) = state.operator_reason() else {
                continue;
            };
            assert!(
                !reason.contains("  "),
                "{state:?} reason has a gap in the middle of it: {reason:?}"
            );
            assert!(
                !reason.contains('\n'),
                "{state:?} reason spans lines: {reason:?}"
            );
            assert_eq!(reason.trim(), reason, "{state:?} reason has loose edges");
        }
    }

    use super::*;

    #[test]
    fn parse_wlan_ipv4_pulls_the_usable_address() {
        let out = "12: wlan0: <UP>\n    inet 192.168.1.42/24 brd 192.168.1.255 scope global wlan0\n       valid_lft forever";
        assert_eq!(parse_wlan_ipv4(out).as_deref(), Some("192.168.1.42"));
    }

    #[test]
    fn parse_wlan_ipv4_skips_loopback_and_link_local() {
        assert_eq!(parse_wlan_ipv4("    inet 127.0.0.1/8 scope host lo"), None);
        assert_eq!(
            parse_wlan_ipv4("    inet 169.254.3.9/16 scope link wlan0"),
            None
        );
        assert_eq!(parse_wlan_ipv4("no address here"), None);
    }

    #[test]
    fn parse_arp_table_keeps_only_host_rows() {
        let out = "\nInterface: 192.168.1.10 --- 0x2\n  Internet Address      Physical Address      Type\n  192.168.1.1           aa-bb-cc-dd-ee-ff     dynamic\n  192.168.1.42          11-22-33-44-55-66     dynamic\n  192.168.1.255         ff-ff-ff-ff-ff-ff     static\n";
        let table = parse_arp_table(out);
        assert_eq!(
            table,
            vec![
                ("192.168.1.1".to_string(), "aa-bb-cc-dd-ee-ff".to_string()),
                ("192.168.1.42".to_string(), "11-22-33-44-55-66".to_string()),
            ]
        );
    }

    /// The real thing, pasted from `adb -s 10969614 shell "ls -la /sdcard/Download"` and
    /// `ls -la /` on 21/08/2026. Every awkward row this fleet has is in here: padded
    /// columns, a name with spaces, a name containing ` - `, a symlink, and the unstattable
    /// row that prints `?` for a merged date/time.
    const MEASURED_LS: &str = "total 41893\n\
-rwxrwx--- 1 u0_a269 media_rw      108 2026-07-26 20:29 .admaster_._u_i_d_f_k.txt\n\
drwxrws--- 2 u0_a269 media_rw     3452 2025-03-08 08:51 .temp_mivideo\n\
-rwxrwx--- 1 u0_a269 media_rw   138078 2025-11-25 08:49 CV prototype.pdf\n\
drwxrws--- 3 u0_a269 media_rw     3452 2025-02-15 07:39 Giao Trinh - Bai Giang - HDH\n\
lrw-r--r--   1 root   root        11 2009-01-01 07:00 bin -> /system/bin\n\
l?????????   ? ?      ?            ?                ? cache -> ?\n\
drwxr-xr-x  32 root   root       788 2009-01-01 07:00 .\n\
drwxr-xr-x  32 root   root       788 2009-01-01 07:00 ..\n";

    #[test]
    fn parse_ls_listing_reads_every_row_shape_the_fleet_prints() {
        let rows = parse_ls_listing(MEASURED_LS);
        let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                ".admaster_._u_i_d_f_k.txt",
                ".temp_mivideo",
                "CV prototype.pdf",
                "Giao Trinh - Bai Giang - HDH",
                "bin",
                "cache",
            ],
            "the `total` line, `.` and `..` are not rows"
        );
        assert_eq!(rows[0].kind, riviu_core::DeviceFileKind::File);
        assert_eq!(rows[0].size, 108);
        assert_eq!(rows[0].modified.as_deref(), Some("2026-07-26 20:29"));
        assert_eq!(rows[1].kind, riviu_core::DeviceFileKind::Directory);
        assert_eq!(
            rows[2].size, 138_078,
            "a name with a space must not shift the size column"
        );
        assert_eq!(
            rows[3].name, "Giao Trinh - Bai Giang - HDH",
            "` - ` inside a name is not a symlink arrow"
        );
        assert_eq!(rows[3].link_target, None);
        assert_eq!(rows[4].kind, riviu_core::DeviceFileKind::Symlink);
        assert_eq!(rows[4].link_target.as_deref(), Some("/system/bin"));
    }

    /// The row that keying off a field count would silently drop. It is one token short of
    /// every other row because the phone printed a single `?` where the date and the time
    /// both go — and a directory browser that hides entries it cannot stat shows a folder as
    /// emptier than it is.
    #[test]
    fn parse_ls_listing_keeps_a_row_the_phone_could_not_stat() {
        let rows = parse_ls_listing(MEASURED_LS);
        let broken = rows
            .iter()
            .find(|row| row.name == "cache")
            .expect("the unstattable row survives");
        assert_eq!(broken.kind, riviu_core::DeviceFileKind::Symlink);
        assert_eq!(broken.modified, None, "`?` is not a timestamp");
        assert_eq!(broken.size, 0);
        assert_eq!(broken.link_target.as_deref(), Some("?"));
    }

    #[test]
    fn parse_ls_listing_ignores_the_error_line_a_missing_path_prints() {
        // Measured: exit 1, this exact sentence, stdout empty. A caller that merges the
        // pipes must not end up with a file named after the complaint.
        assert!(parse_ls_listing("ls: /sdcard/nope-nothing: No such file or directory").is_empty());
    }

    /// **The fabricated-filename shape, as the row that produces it.**
    ///
    /// A row dated `Mon DD HH:MM` found no timestamp under the ISO-only parser, so it fell back
    /// to "token 6 starts the name" -- and token 6 is the day. This row came back naming a file
    /// `"11 11:16 photo.jpg"`, and that invented name is what the browser showed and what
    /// `rm -rf` and `pull` were handed.
    ///
    /// Latent rather than live on today's fleet: both ROMs measured here print the ISO date. It
    /// is pinned because the failure is silent and its blast radius is a delete.
    #[test]
    fn a_bsd_dated_row_is_read_instead_of_inventing_a_filename() {
        let (rows, unreadable) =
            read_ls_listing("-rw-r--r-- 1 root root 138078 Jul 11 11:16 photo.jpg");
        assert!(unreadable.is_empty(), "{unreadable:?}");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "photo.jpg");
        assert_eq!(rows[0].size, 138_078);
        assert_eq!(rows[0].modified.as_deref(), Some("Jul 11 11:16"));
    }

    /// **A date inside a filename is not the row's timestamp.**
    ///
    /// The scan used to run over every token, so an unstattable row whose name happens to
    /// contain a date resynchronised *inside* the name: it took `2026-08-27 12:00` at index 7
    /// for the metadata, `report` for the size, and returned `final.jpg` as the name -- when
    /// the real remainder is `report 2026-08-27 12:00 final.jpg`. That invented name is what
    /// `pull` and `rm -rf` would be handed, which is the same blast radius as
    /// `a_bsd_dated_row_is_read_instead_of_inventing_a_filename` above.
    ///
    /// With the bound in place the row falls to the `?`-metadata branch, which knows the
    /// name starts at token 6 -- so the **whole** name comes back and the unreadable columns
    /// say they are unreadable. That is a better outcome than rejecting the row, and better
    /// than this test first asserted: the first draft expected `unreadable.len() == 1` and
    /// failed, because the parser recovered the real filename instead of merely refusing to
    /// invent one.
    #[test]
    fn a_date_in_a_filename_is_not_mistaken_for_the_timestamp() {
        let (rows, unreadable) =
            read_ls_listing("-r????????? ? ? ? ? ? report 2026-08-27 12:00 final.jpg");
        assert!(unreadable.is_empty(), "{unreadable:?}");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].name, "report 2026-08-27 12:00 final.jpg",
            "the name must be the whole remainder, not the part after a date inside it"
        );
        // The columns really are unreadable on this row, and both say so rather than
        // reporting a plausible number.
        assert_eq!(rows[0].modified, None);
        assert_eq!(rows[0].size, 0);
    }

    /// Both supported layouts still parse, so the bound did not simply reject everything.
    ///
    /// The timestamp lives at index 5 when the ROM prints a link count and at 4 when it does
    /// not. A bound that was one tighter would make every row on one of the two shapes
    /// "unreadable" -- honest, but useless -- so this pins that both are still read.
    #[test]
    fn both_column_layouts_still_find_their_timestamp() {
        let (with_links, bad) =
            read_ls_listing("-rw-r--r-- 1 root root 138078 2026-08-27 12:00 a.jpg");
        assert!(bad.is_empty(), "{bad:?}");
        assert_eq!(with_links[0].name, "a.jpg");
        assert_eq!(with_links[0].size, 138_078);

        let (no_links, bad) = read_ls_listing("-rw-r--r-- root root 4096 2026-08-27 12:00 b.jpg");
        assert!(bad.is_empty(), "{bad:?}");
        assert_eq!(no_links[0].name, "b.jpg");
        assert_eq!(no_links[0].size, 4_096);
    }
    /// Older than six months and `ls` prints the year where the time goes. Same row, one
    /// different column, and it must not fall back to the guessing path either.
    #[test]
    fn a_row_dated_with_a_year_reads_too() {
        let (rows, unreadable) =
            read_ls_listing("drwxr-xr-x 2 u0_a269 media_rw 3452 Mar 8  2024 Giao Trinh - HDH");
        assert!(unreadable.is_empty(), "{unreadable:?}");
        assert_eq!(rows[0].name, "Giao Trinh - HDH");
        assert_eq!(rows[0].kind, riviu_core::DeviceFileKind::Directory);
        assert_eq!(rows[0].size, 3_452);
        assert_eq!(rows[0].modified.as_deref(), Some("Mar 8 2024"));
    }

    /// **Every file 0 B, and it would have looked like a fact.**
    ///
    /// Size used to be read from `tokens[4]`, which is the size column only when the ROM prints
    /// a link-count. Without that column `tokens[4]` is the date, the parse fails, and
    /// `unwrap_or(0)` reports zero bytes for everything -- a number an operator has no reason to
    /// doubt. Reading the slot *before* the timestamp is right on both shapes.
    ///
    /// Also latent today (both measured ROMs print the link-count), and pinned for the same
    /// reason: nothing about the wrong answer looks wrong.
    #[test]
    fn a_rom_without_a_link_count_column_still_reports_the_real_size() {
        let (rows, unreadable) =
            read_ls_listing("-rw-r--r-- root root 138078 2025-11-25 08:49 CV prototype.pdf");
        assert!(unreadable.is_empty(), "{unreadable:?}");
        assert_eq!(rows[0].name, "CV prototype.pdf");
        assert_eq!(
            rows[0].size, 138_078,
            "the size column moves with the ROM; a fixed index finds the date"
        );
    }

    /// A shape this parser does not understand must be **reported**, never named.
    ///
    /// This is the guard that keeps the fabricated-name class closed for the next unknown date
    /// format: an unstattable row prints `?`, so a row with neither a timestamp nor a `?` is a
    /// gap in the parser and says so.
    #[test]
    fn a_row_in_an_unknown_shape_is_reported_rather_than_named() {
        let (rows, unreadable) =
            read_ls_listing("-rw-r--r-- 1 root root 138078 vendredi 11 juillet photo.jpg");
        assert!(
            rows.is_empty(),
            "no name may be invented from a row we cannot read: {rows:?}"
        );
        assert_eq!(unreadable.len(), 1);
        assert!(unreadable[0].contains("photo.jpg"));
    }

    /// The measured rows keep working, including the one with no timestamp at all.
    #[test]
    fn the_measured_listing_reports_nothing_unreadable() {
        let (rows, unreadable) = read_ls_listing(MEASURED_LS);
        assert!(unreadable.is_empty(), "{unreadable:?}");
        assert_eq!(rows.len(), 6);
    }

    /// **A refused directory is not an empty one, and this is the hole that made it look like
    /// one.**
    ///
    /// Some ROMs refuse and still exit 0. The old condition was
    /// `entries.is_empty() && exit_code != 0`, so nothing fired, an empty list came back, and
    /// the browser drew an empty folder -- asserting the directory exists and holds nothing.
    #[test]
    fn a_refusal_that_exits_zero_is_still_a_refusal() {
        let outcome = classify_ls_output("", "ls: /data: Permission denied", 0);
        match outcome {
            LsOutcome::Refused(reason) => assert!(reason.contains("Permission denied")),
            other => panic!("a refusal read as {other:?}"),
        }
    }

    /// The same refusal, from a ROM that merges its pipes: stderr empty, exit code 0.
    #[test]
    fn a_refusal_merged_into_stdout_is_still_a_refusal() {
        let outcome = classify_ls_output("ls: /data: Permission denied\n", "", 0);
        assert!(matches!(outcome, LsOutcome::Refused(_)), "{outcome:?}");
    }

    /// **A short list must not read as a whole one.**
    ///
    /// `ls -la` on a directory it can only partly read prints the rows it managed and complains
    /// about the rest. `entries.is_empty()` is false there, so the old code returned the
    /// truncated list with nothing to say it was truncated.
    #[test]
    fn a_partial_listing_never_reads_as_complete() {
        let outcome = classify_ls_output(
            "-rw-r--r-- 1 root root 108 2026-07-26 20:29 readable.txt\n",
            "ls: /sdcard/Android/data/com.x: Permission denied\n",
            1,
        );
        match outcome {
            LsOutcome::Partial { entries, reason } => {
                assert_eq!(entries.len(), 1);
                assert!(reason.contains("Permission denied"));
            }
            other => panic!("a truncated listing read as {other:?}"),
        }
    }

    /// An unreadable row makes the listing partial too: the operator is short either way.
    #[test]
    fn an_unreadable_row_makes_the_listing_partial() {
        let outcome = classify_ls_output(
            "-rw-r--r-- 1 root root 108 2026-07-26 20:29 readable.txt\n\
             -rw-r--r-- 1 root root 200 vendredi 11 juillet other.txt\n",
            "",
            0,
        );
        match outcome {
            LsOutcome::Partial { entries, reason } => {
                assert_eq!(entries.len(), 1);
                assert!(reason.contains("other.txt"), "{reason}");
            }
            other => panic!("expected a partial listing, got {other:?}"),
        }
    }

    /// **An empty directory has to stay sayable.** Refusing everything would be the same defect
    /// in the other direction.
    #[test]
    fn a_genuinely_empty_directory_is_complete_and_empty() {
        assert_eq!(
            classify_ls_output("total 0\n", "", 0),
            LsOutcome::Complete(Vec::new())
        );
    }

    /// Nothing to show, nothing said, and a non-zero exit: name the code rather than present
    /// the silence as an empty folder.
    #[test]
    fn a_failing_exit_with_no_sentence_still_refuses() {
        match classify_ls_output("", "", 2) {
            LsOutcome::Refused(reason) => assert!(reason.contains('2'), "{reason}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn validate_device_path_allows_the_names_this_fleet_really_has() {
        for path in [
            "/sdcard/Download/CV prototype.pdf",
            "/sdcard/Download/Giao Trinh - Bai Giang - HDH",
            "/sdcard/DCIM/Camera",
            "/data/local/tmp/riviu-wallpaper.png",
        ] {
            assert!(
                validate_device_path(path).is_ok(),
                "{path} is an ordinary path and must be browsable"
            );
        }
    }

    /// A model of how `sh` builds **one word** out of a quoted string.
    ///
    /// Written so the escaping can be proved by round-trip instead of by asserting a literal
    /// that nobody can read: `'/sdcard/John'\\''s photo.jpg'` is unreadable as an
    /// assertion, and an unreadable assertion is how a wrong one survives review.
    fn shell_word(quoted: &str) -> String {
        let mut out = String::new();
        let mut chars = quoted.chars();
        let mut in_single = false;
        while let Some(c) = chars.next() {
            if in_single {
                if c == '\'' {
                    in_single = false;
                } else {
                    out.push(c);
                }
            } else if c == '\'' {
                in_single = true;
            } else if c == '\\' {
                // Outside quotes a backslash escapes the next character, whatever it is.
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    /// The serial and the verb are read off the argv, whichever shape it has.
    ///
    /// `AdbProgram::device` prepends `-s <serial>`; `run_bytes` is also called directly with a
    /// bare verb. Reading either at a fixed index gets one of the two wrong, and getting it
    /// wrong is silent: the per-serial queue would key on `"-s"` and serialise nothing.
    #[test]
    fn the_serial_and_the_verb_are_found_in_both_argv_shapes() {
        assert_eq!(
            adb_target(&["-s", "10969614", "pull", "/sdcard/a", "."]),
            (Some("10969614"), Some("pull"))
        );
        assert_eq!(
            adb_target(&["pull", "/sdcard/a", "."]),
            (None, Some("pull"))
        );
        assert_eq!(adb_target(&["devices"]), (None, Some("devices")));
        assert_eq!(adb_target(&[]), (None, None));
        // `-s` as the *last* token is malformed rather than a serial, and must not be read as
        // one -- the slice pattern needs two elements, so it falls through.
        assert_eq!(adb_target(&["-s"]), (None, Some("-s")));
    }

    /// Only the two verbs that move bytes get the transfer lane.
    #[test]
    fn only_pull_and_push_count_as_transfers() {
        assert_eq!(adb_lane(Some("pull")), AdbLane::Transfer);
        assert_eq!(adb_lane(Some("push")), AdbLane::Transfer);
        for verb in ["shell", "devices", "install", "reboot", "exec-out"] {
            assert_eq!(adb_lane(Some(verb)), AdbLane::Interactive, "{verb}");
        }
        assert_eq!(adb_lane(None), AdbLane::Interactive);
    }

    /// **A transfer can never take every slot, and that is the whole point of the sub-cap.**
    ///
    /// Checked at **compile time** rather than in a test body: these are constants, so the
    /// strongest available form is a build that refuses rather than a test that reports. It is
    /// the shape `view_hub` already uses for `DEVICE_BROADCAST_CAP`, and clippy asks for it by
    /// name.
    const _: () = assert!(
        ADB_MAX_TRANSFERS < ADB_MAX_CONCURRENT,
        "transfers hold a permit for up to 300 s; if they can take every slot then each probe, \
         screenshot and nurture action queues behind an export for five minutes"
    );
    const _: () = assert!(
        ADB_MAX_CONCURRENT - ADB_MAX_TRANSFERS >= 8,
        "leave enough slots that work answering in milliseconds still flows"
    );
    const _: () = assert!(
        ADB_MAX_TRANSFERS >= 1,
        "a fleet still has to be able to export a file"
    );

    /// Serialises the tests below, because the thing they inspect is **global**.
    ///
    /// The semaphores are process-wide `OnceLock` statics -- deliberately, since what they ration
    /// is one adb server per host -- so two of these tests running at once perturb each other's
    /// permit counts. That is not hypothetical: `a_transfer_takes_a_transfer_slot_and_a_global_one`
    /// passed alone and failed in the full crate run, which is exactly the flake shape this whole
    /// pass is supposed to be removing rather than adding.
    ///
    /// `tokio::sync::Mutex` rather than `std::sync::Mutex` because these tests hold the guard
    /// across `.await`. Under CI's `--test-threads=1` this is already serial and the lock costs
    /// nothing; locally it is what makes them deterministic.
    fn slot_tests_run_one_at_a_time() -> &'static tokio::sync::Mutex<()> {
        static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
    }

    /// One phone, one call at a time.
    ///
    /// Asserts on the phone's own queue rather than on timing: the permit count is the
    /// mechanism, and reading it is deterministic where racing two tasks is not.
    #[tokio::test]
    async fn a_second_call_to_one_phone_waits_for_the_first() {
        let _serial = slot_tests_run_one_at_a_time().lock().await;
        let serial = "queue-test-one-phone";
        let held = enter_adb_slot(Some(serial), "shell", AdbLane::Interactive).await;

        let queue = adb_device_queues()
            .lock()
            .get(serial)
            .cloned()
            .expect("the phone got a queue");
        assert_eq!(
            queue.available_permits(),
            0,
            "while one call runs, the phone's queue must be closed"
        );

        drop(held);
        assert_eq!(
            queue.available_permits(),
            1,
            "and open again the moment it finishes"
        );
    }

    /// Two phones do not queue behind each other. Serialising the fleet would be a far worse
    /// bug than the one this fixed; if the queues were shared this test would hang.
    #[tokio::test]
    async fn two_different_phones_run_at_the_same_time() {
        let _serial = slot_tests_run_one_at_a_time().lock().await;
        let first = enter_adb_slot(Some("queue-test-a"), "shell", AdbLane::Interactive).await;
        let second = enter_adb_slot(Some("queue-test-b"), "shell", AdbLane::Interactive).await;
        drop((first, second));
    }

    /// A transfer takes one of the four **and** one of the twelve, because the sub-cap sits
    /// inside the global cap rather than beside it -- twelve is the number that was measured.
    #[tokio::test]
    async fn a_transfer_takes_a_transfer_slot_and_a_global_one() {
        let _serial = slot_tests_run_one_at_a_time().lock().await;
        let transfers_before = adb_transfer_slots().available_permits();
        let global_before = adb_slots().available_permits();

        let held = enter_adb_slot(Some("queue-test-transfer"), "pull", AdbLane::Transfer).await;
        assert_eq!(
            adb_transfer_slots().available_permits(),
            transfers_before - 1
        );
        assert_eq!(adb_slots().available_permits(), global_before - 1);

        drop(held);
        assert_eq!(adb_transfer_slots().available_permits(), transfers_before);
        assert_eq!(adb_slots().available_permits(), global_before);
    }

    /// And an interactive call leaves the transfer sub-cap alone.
    #[tokio::test]
    async fn an_interactive_call_does_not_consume_the_transfer_sub_cap() {
        let _serial = slot_tests_run_one_at_a_time().lock().await;
        let before = adb_transfer_slots().available_permits();
        let held = enter_adb_slot(
            Some("queue-test-interactive"),
            "shell",
            AdbLane::Interactive,
        )
        .await;
        assert_eq!(adb_transfer_slots().available_permits(), before);
        drop(held);
    }

    /// **The quoting survives every character a real filename can hold — apostrophes included.**
    ///
    /// The apostrophe row is the one that used to be refused outright. It is here as a
    /// round-trip because that is the property that matters: whatever the shell does with the
    /// quoted text, it must come back out as exactly the path that went in, as **one word**.
    #[test]
    fn quoting_a_path_round_trips_through_a_shell() {
        for path in [
            "/sdcard/Download",
            "/sdcard/Download/CV prototype.pdf",
            "/sdcard/Download/Giao Trinh - Bai Giang - HDH",
            "/sdcard/Download/John's photo.jpg",
            "/sdcard/Download/a$b&c;d|e.txt",
            "/sdcard/'''",
            // The injection attempt. It is now *allowed* through the validator, and it has to
            // come back out as a filename rather than as three commands.
            "/sdcard/'; rm -rf /sdcard; echo '",
        ] {
            let quoted = quote_device_path(path);
            assert_eq!(
                shell_word(&quoted),
                path,
                "{path} did not survive quoting as {quoted}"
            );
        }
    }

    /// The validator still refuses what quoting cannot make safe, and now allows what it can.
    #[test]
    fn validate_device_path_refuses_only_what_quoting_cannot_fix() {
        // A newline is refused for a different reason than an apostrophe was: quoting preserves
        // it fine, but no name on this fleet has one and a path carrying one is much more
        // likely to be an upstream parse mistake than a file somebody wants.
        assert!(validate_device_path("/sdcard/a\nrm -rf /sdcard").is_err());
        assert!(validate_device_path("sdcard/Download").is_err(), "relative");
        assert!(validate_device_path("").is_err());
        // Inert inside single quotes, and real filenames use them — so they stay allowed.
        assert!(validate_device_path("/sdcard/Download/a$b&c;d|e.txt").is_ok());
        // **No longer refused.** The quoter escapes it; see the round-trip above.
        assert!(
            validate_device_path("/sdcard/Download/John's photo.jpg").is_ok(),
            "a file the operator can see must be a file the operator can touch"
        );
        assert!(validate_device_path("/sdcard/'; rm -rf /sdcard; echo '").is_ok());
    }

    #[test]
    fn is_undeletable_root_catches_the_roots_however_they_are_written() {
        for path in ["/", "/sdcard", "/sdcard/", "/storage/emulated/0", "/data"] {
            assert!(is_undeletable_root(path), "{path} must not be deletable");
        }
        for path in [
            "/sdcard/Download",
            "/sdcard/DCIM/Camera",
            "/data/local/tmp/x",
        ] {
            assert!(!is_undeletable_root(path), "{path} is a normal target");
        }
    }

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

    /// The shapes the frontend's generator actually produces must all pass.
    ///
    /// `RootTool` invents an identity per phone in `randomIdentity.ts`, and every one of the
    /// three values lands here before it reaches `su -c`. The two halves are written in
    /// different languages and cannot call each other, so each states the same grammar and
    /// names the other: the TypeScript half is `randomIdentity.test.ts`, which asserts the
    /// generator emits exactly the shapes below.
    ///
    /// A drift is not a security hole — the validator refusing is the safe direction — it is
    /// a batch identity change that fails on every phone after the operator pressed the
    /// button, with a per-field error and no clue that the *generator* is what moved.
    #[test]
    fn the_generated_identities_match_the_shapes_the_frontend_sends() {
        // 16 lowercase hex, from 8 random bytes.
        for generated in ["0123456789abcdef", "ffffffffffffffff", "0000000000000000"] {
            assert!(
                validate_android_id(generated).is_ok(),
                "the generator emits {generated} and this refuses it"
            );
        }

        // 12 uppercase alphanumerics from an alphabet with no ambiguous I/O/0/1, because a
        // human reads these off a label. Narrower than what this validator allows, on purpose.
        for generated in ["ABCDEFGHJKLM", "NPQRSTUVWXYZ", "23456789ABCD"] {
            assert!(
                validate_serial_no(generated).is_ok(),
                "the generator emits {generated} and this refuses it"
            );
        }

        // Six colon-separated lowercase hex octets, first octet locally administered and
        // unicast — bit 1 set, bit 0 clear — so `02`, `06`, `0a`, `0e` and so on.
        for generated in [
            "02:1a:2b:3c:4d:5e",
            "0a:ff:00:11:22:33",
            "fe:00:00:00:00:01",
        ] {
            assert!(
                validate_mac(generated).is_ok(),
                "the generator emits {generated} and this refuses it"
            );
        }
    }

    #[test]
    fn identity_values_that_a_root_shell_could_act_on_are_refused() {
        // These three are pasted into `su -c "…"`, so the bar is higher than for the package
        // name: inside double quotes `$(…)` and a backtick still substitute even though `;`
        // and `|` are already covered by the grammars. A pass here is root on the phone.
        let measured_android_id = "a1b2c3d4e5f60789";
        let measured_serial = "10969614";
        let measured_mac = "02:00:00:44:55:66";
        assert!(validate_android_id(measured_android_id).is_ok());
        assert!(validate_serial_no(measured_serial).is_ok());
        assert!(validate_mac(measured_mac).is_ok());

        for bad in [
            "a1b2c3d4e5f6078",   // 15 digits
            "a1b2c3d4e5f607890", // 17
            "a1b2c3d4e5f6078g",  // not hex
            "$(id)0123456789",
            "a1b2c3d4e5f6\"; id; #",
            "",
        ] {
            assert!(
                validate_android_id(bad).is_err(),
                "android_id should have been refused: {bad:?}"
            );
        }

        for bad in [
            "x\"; sh -c 'id'; #",
            "x$(id)",
            "x`id`",
            "x;reboot",
            "x y",
            "x\nreboot",
            "",
        ] {
            assert!(
                validate_serial_no(bad).is_err(),
                "serial should have been refused: {bad:?}"
            );
        }

        for bad in [
            "02:00:00:44:55",       // five octets
            "02:00:00:44:55:66:77", // seven
            "02:00:00:44:55:6g",
            "02:00:00:44:55:6",
            "02-00-00-44-55-66",
            "x\"; ip link set wlan0 address 00:11:22:33:44:55; #",
            "$(id):00:00:44:55:66",
            "",
        ] {
            assert!(
                validate_mac(bad).is_err(),
                "mac should have been refused: {bad:?}"
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

    /// Verbatim from `ce0717171c2a64d50d` while it sat on its lock screen, 23/08/2026 —
    /// one of the two phones whose nurture session failed calling this "unreadable".
    const LOCKED_FOCUS: &str = "  mCurrentFocus=Window{4b5766b u0 StatusBar}\n";

    /// The same line off a healthy phone in the same sweep.
    const TIKTOK_FOCUS: &str = "  mCurrentFocus=Window{be9279f u0 com.ss.android.ugc.trill/\
         com.ss.android.ugc.aweme.splash.SplashActivity}\n";

    #[test]
    fn a_locked_phone_reads_as_a_system_window_not_as_unreadable() {
        // The distinction this enum exists for. "Unreadable" describes the parser;
        // "StatusBar" describes a phone somebody can unlock.
        assert_eq!(
            parse_foreground_window(LOCKED_FOCUS),
            ForegroundWindow::System("StatusBar".into())
        );
        assert_eq!(
            parse_foreground_window(TIKTOK_FOCUS),
            ForegroundWindow::App("com.ss.android.ugc.trill".into())
        );
    }

    #[test]
    fn nothing_readable_is_its_own_answer() {
        assert_eq!(parse_foreground_window(""), ForegroundWindow::Unreadable);
        assert_eq!(
            parse_foreground_window("  mCurrentFocus=null\n"),
            ForegroundWindow::Unreadable
        );
        assert_eq!(
            parse_foreground_window("Window #0 mOwnerUid=1000\n"),
            ForegroundWindow::Unreadable
        );
    }

    /// **A splash screen is the app's package and not the app.**
    ///
    /// Measured 25/08/2026: eight phones of twenty sat on
    /// `com.ss.android.ugc.aweme.splash.SplashActivity` while the readiness proof — which
    /// reads only the package — counted them as up. Everything downstream then read an empty
    /// screen and refused with `no_baseline`, a sentence about the symptom.
    #[test]
    fn the_activity_half_is_readable_and_a_splash_is_recognised() {
        let splash = "  mCurrentFocus=Window{abc123 u0                       com.ss.android.ugc.trill/com.ss.android.ugc.aweme.splash.SplashActivity}
";
        let activity = parse_foreground_activity(splash).expect("the line names an activity");
        assert_eq!(activity, "com.ss.android.ugc.aweme.splash.SplashActivity");
        assert!(is_splash_activity(&activity));

        // The package is unchanged for both, which is exactly why the package cannot decide.
        assert_eq!(
            parse_foreground_window(splash),
            ForegroundWindow::App("com.ss.android.ugc.trill".into())
        );
    }

    /// The feed is not a splash, and a window with no activity answers `None` rather than
    /// guessing — the caller then behaves as it did before this existed.
    #[test]
    fn the_feed_is_not_a_splash_and_a_system_window_names_no_activity() {
        let main = "  mCurrentFocus=Window{abc123 u0                     com.ss.android.ugc.trill/com.ss.android.ugc.aweme.main.MainActivity}
";
        let activity = parse_foreground_activity(main).expect("the line names an activity");
        assert!(!is_splash_activity(&activity));
        assert_eq!(
            parse_foreground_activity(
                "  mCurrentFocus=Window{a u0 StatusBar}
"
            ),
            None
        );
    }

    /// The multi-display trap, restated for the new function: `null` comes first on a Redmi
    /// Note 12, so a first-line reader says "nothing focused" with TikTok on screen.
    #[test]
    fn an_app_on_any_display_outranks_null_and_a_system_window() {
        let stdout = format!("  mCurrentFocus=null\n{LOCKED_FOCUS}{TIKTOK_FOCUS}");
        assert_eq!(
            parse_foreground_window(&stdout),
            ForegroundWindow::App("com.ss.android.ugc.trill".into()),
            "a real app anywhere in the list is the answer"
        );
    }

    /// The old entry point must keep answering exactly what it always did, because callers
    /// treat its `None` as "not the app I asked about" and that reading is still correct.
    #[test]
    fn the_package_helper_still_answers_the_way_it_used_to() {
        assert_eq!(
            parse_current_focus_package(TIKTOK_FOCUS).as_deref(),
            Some("com.ss.android.ugc.trill")
        );
        assert_eq!(parse_current_focus_package(LOCKED_FOCUS), None);
        assert_eq!(parse_current_focus_package(""), None);
    }

    #[test]
    fn the_keyguard_keys_wake_before_they_dismiss_and_never_use_power() {
        assert_eq!(KEYGUARD_DISMISS_KEYEVENTS.len(), 2);
        assert!(
            KEYGUARD_DISMISS_KEYEVENTS[0].contains("KEYCODE_WAKEUP"),
            "wake first, and idempotently: {KEYGUARD_DISMISS_KEYEVENTS:?}"
        );
        assert!(
            KEYGUARD_DISMISS_KEYEVENTS[1].contains("KEYCODE_MENU"),
            "{KEYGUARD_DISMISS_KEYEVENTS:?}"
        );
        assert!(
            !KEYGUARD_DISMISS_KEYEVENTS
                .iter()
                .any(|key| key.contains("KEYCODE_POWER")),
            "POWER toggles — on an awake phone it would turn the screen off"
        );
    }

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

    /// One `dumpsys display` from SM-G955F, 17/08/2026, copied verbatim.
    ///
    /// Kept whole rather than trimmed to the interesting parts, because the parts that
    /// break parsers are the ones a summary would drop: the nested `modes [{...}]` braces
    /// and the `app`/`largest app`/`smallest app` sizes that sit beside `real`.
    const FLEET_DUMPSYS_DISPLAY: &str = concat!(
        "  mDefaultViewport=DisplayViewport{valid=true, displayId=0, orientation=0, ",
        "logicalFrame=Rect(0, 0 - 1080, 2220), deviceWidth=1440, deviceHeight=2960}\n",
        "  DisplayDeviceInfo{\"Built-in Screen\": uniqueId=\"local:0\", 1440 x 2960, ",
        "modeId 1, density 560, 522.514 x 525.762 dpi, touch INTERNAL, rotation 0, ",
        "type BUILT_IN, state ON}\n",
        "    mBaseDisplayInfo=DisplayInfo{\"Built-in Screen\", app 1440 x 2960, ",
        "real 1440 x 2960, largest app 1440 x 2960, smallest app 1440 x 2960, mode 1, ",
        "modes [{id=1, width=1440, height=2960, fps=60.000004}], colorMode -1, ",
        "rotation 0, density 560 (522.514 x 525.762) dpi, layerStack 0, state ON}\n",
        "    mOverrideDisplayInfo=DisplayInfo{\"Built-in Screen\", app 1080 x 2094, ",
        "real 1080 x 2220, largest app 2094 x 2031, smallest app 1080 x 1017, mode 1, ",
        "modes [{id=1, width=1440, height=2960, fps=60.000004}], colorMode -1, ",
        "rotation 0, density 420 (391.8855 x 394.32153) dpi, layerStack 0, state ON}\n",
    );

    #[test]
    fn the_display_read_is_the_one_that_is_rendered_not_the_panel() {
        // 1080x2220 at 420, not 1440x2960 at 560. Reading the base line would put every
        // derived coordinate 33% out on every phone in this fleet -- the same trap
        // `parse_wm_size` documents for Physical vs Override.
        let geometry = parse_display_geometry(FLEET_DUMPSYS_DISPLAY).expect("geometry");
        assert_eq!(
            geometry,
            DisplayGeometry {
                width: 1080,
                height: 2220,
                density: 420,
                rotation: 0,
            }
        );
    }

    #[test]
    fn a_rotated_display_reports_the_size_it_is_actually_showing() {
        // The whole reason this parser exists instead of `wm size`: measured 16/08/2026,
        // a landscape SM-G955F kept reporting `Override size: 1080x2220` to `wm size`
        // while `dumpsys display` moved to `real 2220 x 1080`.
        let landscape = FLEET_DUMPSYS_DISPLAY
            .replace("real 1080 x 2220", "real 2220 x 1080")
            .replace("rotation 0, density 420", "rotation 1, density 420");
        let geometry = parse_display_geometry(&landscape).expect("geometry");
        assert_eq!((geometry.width, geometry.height), (2220, 1080));
        assert_eq!(geometry.rotation, 1);
    }

    #[test]
    fn a_display_with_no_override_falls_back_to_the_panel_it_has() {
        // Not every phone sets an override. Falling back is right; guessing is not.
        let base_only = FLEET_DUMPSYS_DISPLAY
            .lines()
            .filter(|line| !line.contains("mOverrideDisplayInfo="))
            .collect::<Vec<_>>()
            .join("\n");
        let geometry = parse_display_geometry(&base_only).expect("geometry");
        assert_eq!(
            (geometry.width, geometry.height, geometry.density),
            (1440, 2960, 560)
        );
    }

    #[test]
    fn an_unreadable_display_dump_is_none_rather_than_a_plausible_guess() {
        // A snapshot built from a half-read dump would be persisted, hashed into a
        // profile id and enforced at run time. Nothing is better than nearly.
        assert!(parse_display_geometry("").is_none());
        assert!(
            parse_display_geometry("mOverrideDisplayInfo=DisplayInfo{app 1080 x 2094}").is_none()
        );
        // Size and density but no rotation: still not enough to know the orientation.
        assert!(parse_display_geometry(
            "mOverrideDisplayInfo=DisplayInfo{real 1080 x 2220, density 420 dpi}"
        )
        .is_none());
        // A zero dimension is a dump that did not mean it.
        assert!(parse_display_geometry(
            "mOverrideDisplayInfo=DisplayInfo{real 0 x 2220, rotation 0, density 420 dpi}"
        )
        .is_none());
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
    /// **Every short adb call goes through the two-tier queue; the long-lived ones must not.**
    ///
    /// The queue is per-serial ×1 inside a global ceiling, and it exists for two separate
    /// reasons: two adb commands interleaving on one phone is a correctness bug, and twelve
    /// 300 s transfers holding all twelve global permits made the whole fleet look hung.
    /// Both are invisible from a call site -- a direct `Command::new(adb.path())` compiles,
    /// runs, and works fine until a second caller shows up.
    ///
    /// Three sites spawn adb directly, and all three are right to: they start children that
    /// **never exit on their own**. This is the reasoning already written at the top of this
    /// file for scrcpy -- a permit held by a process that does not return is a permit that
    /// never comes back, and the fleet deadlocks at the twelfth phone. So the rule is not "no
    /// direct spawns" but "no *undeclared* direct spawns", and each exemption says which
    /// long-lived child it starts.
    ///
    /// Checked from both ends: a new spawn site fails until it is either queued or declared,
    /// and a declaration that no longer matches a real site fails too.
    #[test]
    fn nothing_spawns_adb_outside_the_queue_without_saying_why() {
        /// Sites that start a child which runs until something stops it, so they must not
        /// hold a queue permit while it does.
        const LONG_LIVED: [(&str, &str); 3] = [
            (
                "driver/agent.rs",
                "am instrument -w blocks for the life of the uiautomator2 server",
            ),
            (
                "driver/stream.rs",
                "minicap and the scrcpy server both run until the stream is stopped",
            ),
            (
                "adb.rs",
                "this file is the queue: enter_adb_slot is taken before the spawn",
            ),
        ];

        /// Remove every `#[cfg(test)] mod … { … }` block, keeping the rest.
        ///
        /// The repo's older scanners cut the source at the first `#[cfg(test)]` that is
        /// followed by `mod `, and that is a truncation rather than a filter. It was already
        /// known to be lossy — `agent_commands.rs` carries an item-level `#[cfg(test)]` on
        /// line 1 and the cut hid all six of its commands — but the measured spread is wider
        /// than the note describing it: **27 files** in this workspace have a first
        /// `#[cfg(test)]` that is not the trailing test module, and several are inline test
        /// modules early in a long file. `driver/mod.rs` opens `mod dialog_tests` on line 103
        /// of 2,400, so a truncating scan of that file reads 4% of it and reports green.
        ///
        /// A scanner that silently reads 4% of a file is worse than no scanner: it answers
        /// the question it was asked with the wrong evidence.
        ///
        /// The closing brace is found by indentation, which holds because `cargo fmt
        /// --all -- --check` is itself a gate: rustfmt closes a module at the same column it
        /// opened. Matching is newline-agnostic for the reason `all_commands` writes out —
        /// this repo is developed on Windows with `core.autocrlf=true`, so a needle
        /// containing a hard-coded newline finds nothing in a fresh checkout.
        fn strip_test_modules(source: &str) -> String {
            let mut kept: Vec<&str> = Vec::new();
            let mut lines = source.lines().peekable();
            while let Some(line) = lines.next() {
                if line.trim() != "#[cfg(test)]" {
                    kept.push(line);
                    continue;
                }
                // Only a module is skipped. An item-level `#[cfg(test)]` marks a test-only
                // helper that is still production-shaped code, and hiding it would recreate
                // the blind spot this function exists to close.
                let Some(next) = lines.peek() else {
                    kept.push(line);
                    continue;
                };
                if !next.trim_start().starts_with("mod ") {
                    kept.push(line);
                    continue;
                }
                let indent = line.len() - line.trim_start().len();
                let closing = format!("{}}}", " ".repeat(indent));
                for inner in lines.by_ref() {
                    if inner == closing {
                        break;
                    }
                }
            }
            kept.join("\n")
        }

        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    out.push(path);
                }
            }
        }

        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        walk(&src, &mut files);

        let mut scanned = 0usize;
        let mut spawns = 0usize;
        let mut declared: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        let mut undeclared: Vec<String> = Vec::new();

        for path in &files {
            let rel = path
                .strip_prefix(&src)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            let Ok(whole) = std::fs::read_to_string(path) else {
                continue;
            };
            // Test modules are removed rather than truncated at -- see strip_test_modules.
            // This file is exactly why: it carries an item-level `#[cfg(test)]` on
            // `unrunnable_for_test` at line 371, so a truncating cut hid its own spawn at 377
            // and the both-ends check below went red on the first run.
            let source = strip_test_modules(&whole);
            let source = source.as_str();
            scanned += 1;

            for (idx, line) in source.lines().enumerate() {
                let code = line.trim_start();
                if code.starts_with("//") || !line.contains("Command::new(") {
                    continue;
                }
                // Only spawns of the adb binary itself. `sh -c`, `arp` and the yt-dlp path
                // are different tools with different budgets.
                if !line.contains("adb.path()") && !line.contains("&self.path") {
                    continue;
                }
                spawns += 1;
                match LONG_LIVED.iter().find(|(file, _)| *file == rel) {
                    Some((file, _)) => {
                        declared.insert(file);
                    }
                    None => undeclared.push(format!("{rel}:{}", idx + 1)),
                }
            }
        }

        // A scanner that reads nothing passes every assertion below it.
        assert!(
            scanned >= 10,
            "only {scanned} source files scanned; the walk is broken"
        );
        assert!(
            spawns >= 3,
            "only {spawns} adb spawn sites found; the scan is broken"
        );
        assert!(
            undeclared.is_empty(),
            "these spawn adb without going through the queue and without declaring why: \
             {undeclared:?}\n\
             \n\
             Short calls belong in `run`/`shell`, which take a slot. Only a child that never \
             exits on its own may bypass the queue, and it has to be named in LONG_LIVED \
             with the reason -- a held permit that never returns deadlocks the fleet."
        );
        let stale: Vec<&str> = LONG_LIVED
            .iter()
            .map(|(file, _)| *file)
            .filter(|file| !declared.contains(file))
            .collect();
        assert!(
            stale.is_empty(),
            "these files are declared as spawning a long-lived adb child but no longer do: \
             {stale:?} -- drop the declaration"
        );
    }
}
