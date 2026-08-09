//! `DeviceDriver` for Android over adb plus a resident on-device agent.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context};
use async_trait::async_trait;
use parking_lot::Mutex;
use riviu_core::driver::{AppProcessState, DeviceDriver, ProcessAbsenceProof, UiSession};
use riviu_core::{ConnectionKind, DeviceInfo, DeviceStatus};

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

#[derive(Debug, Clone, Default)]
pub struct AndroidDriverConfig {
    /// Explicit path to `adb`. Falls back to `ANDROID_SDK_ROOT`/`ANDROID_HOME`,
    /// then `PATH`.
    pub adb_path: Option<PathBuf>,
}

pub struct AndroidDriver {
    adb: AdbProgram,
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
}

impl AndroidDriver {
    pub fn new(config: &AndroidDriverConfig) -> anyhow::Result<Self> {
        let adb = AdbProgram::resolve(config.adb_path.as_deref())?;
        Ok(Self {
            adb,
            ports: Mutex::new(HashMap::new()),
            forwarded: Mutex::new(HashSet::new()),
        })
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

    /// Whether an agent is reachable for this device, answered honestly.
    ///
    /// Devices we have never forwarded report `false` rather than borrowing
    /// somebody else's agent.
    async fn agent_ready(&self, serial: &str) -> bool {
        if !self.forwarded.lock().contains(serial) {
            return false;
        }
        AgentClient::is_ready(&self.agent_base(serial)).await
    }

    /// The pid of a package, or `None` when it is not running.
    ///
    /// `pidof` exits non-zero for an absent process, which the adb wrapper
    /// reports as a command failure. Absence is an answer here, not an error —
    /// propagating it made `inspect_app_process` fail precisely when it was
    /// asked about a stopped app, which is the case it exists to describe.
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

    /// Open a session as the concrete type.
    ///
    /// `start_ui_session` boxes this. Callers that need the Android-specific
    /// surface — locator queries, element bounds — take this instead of
    /// downcasting a trait object.
    pub async fn open_session(&self, udid: &str) -> anyhow::Result<AndroidUiSession> {
        let agent = self.ensure_agent(udid).await?;
        let screen = self.screen_size(udid).await?;
        Ok(AndroidUiSession::new(
            agent,
            self.adb.clone(),
            udid.to_string(),
            screen,
        ))
    }

    /// Make sure the agent is installed, running and forwarded.
    async fn ensure_agent(&self, serial: &str) -> anyhow::Result<AgentClient> {
        let port = self.host_port(serial);
        let base = self.agent_base(serial);
        let forward_spec = format!("tcp:{port}");
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

        if AgentClient::is_ready(&base).await {
            return AgentClient::connect(&base).await;
        }

        let installed = self
            .adb
            .shell(serial, &format!("pm list packages {AGENT_PACKAGE}"))
            .await
            .unwrap_or_default();
        if !installed.contains(AGENT_PACKAGE) {
            return Err(anyhow!(
                "the agent is not installed on {serial}. Install both \
                 appium-uiautomator2-server APKs (server and \
                 debug-androidTest) and try again"
            ));
        }

        self.spawn_instrumentation(serial)?;
        // The server binds its port a beat after the runner starts.
        for _ in 0..40 {
            if AgentClient::is_ready(&base).await {
                return AgentClient::connect(&base).await;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        Err(anyhow!(
            "the agent on {serial} did not answer /status within 10 seconds"
        ))
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
    let stdout = adb.shell(&serial, &script).await.unwrap_or_default();
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
        ios_version: fields.release.unwrap_or_default(),
        connection: ConnectionKind::Usb,
        status: DeviceStatus::Connected,
        battery: fields.battery,
        wda_ready: false,
        // Android has no provisioning profile to expire. `adb install` needs no
        // per-device signing, so this stays `None`.
        wda_expires_at: None,
        stream_url: None,
        tile_stream_state: Default::default(),
        last_error: None,
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
        let stdout = self
            .adb
            .run(&["devices", "-l"], adb::DEFAULT_TIMEOUT)
            .await?;
        let lines = adb::parse_devices(&stdout);

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
                    ios_version: String::new(),
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

    /// Yes, and measured: Vietnamese reaches TikTok's comment box intact
    /// through accessibility `ACTION_SET_TEXT`.
    fn supports_text_comments(&self, _udid: &str) -> bool {
        true
    }

    async fn inspect_app_process(
        &self,
        udid: &str,
        bundle_id: &str,
    ) -> anyhow::Result<AppProcessState> {
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

    /// No frame producer yet.
    ///
    /// Deliberate: with hierarchy-based location, frames are corroboration
    /// rather than the locator, so the video pipeline is deferred until
    /// measurement shows it is needed. Failing loudly beats returning a URL
    /// that publishes nothing.
    async fn ensure_stream(&self, _udid: &str) -> anyhow::Result<String> {
        Err(anyhow!(
            "the Android driver has no frame source yet — see Pha 5 of the plan"
        ))
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
pub async fn detect_driver(config: &AndroidDriverConfig) -> Result<Arc<dyn DeviceDriver>, String> {
    let driver = AndroidDriver::new(config).map_err(|error| error.to_string())?;
    driver
        .adb
        .run(&["version"], Duration::from_secs(10))
        .await
        .map_err(|error| {
            format!(
                "adb is not usable ({}): {error}",
                driver.adb.path().display()
            )
        })?;
    Ok(Arc::new(driver))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
