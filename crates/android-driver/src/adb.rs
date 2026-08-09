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
use std::time::Duration;

use anyhow::{anyhow, Context};
use tokio::process::Command;

/// How long a lifecycle call may take before we stop waiting. Generous: on the
/// S8+ fleet `screencap` alone is measured at 1.2–2.6 s and `pm install` of a
/// 17 MB APK is slower still.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub struct AdbProgram {
    path: PathBuf,
}

impl AdbProgram {
    /// Resolve `adb`, preferring an explicitly configured binary, then
    /// `RIVIU_ADB_PATH`, then the standard SDK locations, then `PATH`.
    ///
    /// `RIVIU_ADB_PATH` points straight at the executable, which matters
    /// because a machine can have platform-tools unpacked somewhere without an
    /// SDK layout around it. Without it the only way to be found is to sit in
    /// `<ANDROID_SDK_ROOT>/platform-tools/` or on `PATH`, and an operator with
    /// a loose copy has no way to say where it is.
    ///
    /// Named for the repo's existing convention (`RIVIU_STREAM_CAPACITY`,
    /// `RIVIU_DEFAULT_AGENT_MODE`, `RIVIU_FRAME_DUMP`).
    pub fn resolve(configured: Option<&Path>) -> anyhow::Result<Self> {
        let mut tried: Vec<String> = Vec::new();
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(path) = configured {
            candidates.push(path.to_path_buf());
        }
        if let Ok(direct) = std::env::var("RIVIU_ADB_PATH") {
            if !direct.trim().is_empty() {
                candidates.push(PathBuf::from(direct.trim()));
            }
        }
        for key in ["ANDROID_SDK_ROOT", "ANDROID_HOME"] {
            if let Ok(root) = std::env::var(key) {
                if !root.trim().is_empty() {
                    candidates.push(Path::new(&root).join("platform-tools").join(exe_name()));
                }
            }
        }
        for candidate in &candidates {
            if candidate.is_file() {
                return Ok(Self {
                    path: candidate.clone(),
                });
            }
            tried.push(candidate.display().to_string());
        }
        // Bare name: let the OS search PATH.
        tried.push(format!("{} (trên PATH)", exe_name()));
        Ok(Self {
            path: PathBuf::from(exe_name()),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
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

    /// Run `adb <args>` and return stdout as text.
    pub async fn run(&self, args: &[&str], timeout: Duration) -> anyhow::Result<String> {
        let bytes = self.run_bytes(args, timeout).await?;
        Ok(String::from_utf8_lossy(&bytes).to_string())
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
pub fn parse_current_focus_package(stdout: &str) -> Option<String> {
    let line = stdout.lines().find(|line| line.contains("mCurrentFocus"))?;
    let inside = line.rsplit_once('/')?.0;
    let package = inside.rsplit_once(' ')?.1;
    if package.is_empty() {
        None
    } else {
        Some(package.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
