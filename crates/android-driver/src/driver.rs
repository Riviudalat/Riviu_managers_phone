//! `DeviceDriver` for Android over adb plus a resident on-device agent.

use std::collections::HashMap;
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
}

impl AndroidDriver {
    pub fn new(config: &AndroidDriverConfig) -> anyhow::Result<Self> {
        let adb = AdbProgram::resolve(config.adb_path.as_deref())?;
        Ok(Self {
            adb,
            ports: Mutex::new(HashMap::new()),
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

    async fn getprop(&self, serial: &str, key: &str) -> String {
        self.adb
            .shell(serial, &format!("getprop {key}"))
            .await
            .map(|value| value.trim().to_string())
            .unwrap_or_default()
    }

    async fn screen_size(&self, serial: &str) -> anyhow::Result<(f64, f64)> {
        let stdout = self.adb.shell(serial, "wm size").await?;
        let (width, height) = adb::parse_wm_size(&stdout)
            .ok_or_else(|| anyhow!("không đọc được kích thước màn hình từ 'wm size'"))?;
        Ok((f64::from(width), f64::from(height)))
    }

    async fn battery_level(&self, serial: &str) -> Option<u8> {
        let stdout = self.adb.shell(serial, "dumpsys battery").await.ok()?;
        stdout
            .lines()
            .find_map(|line| line.trim().strip_prefix("level:"))
            .and_then(|value| value.trim().parse::<u8>().ok())
    }

    async fn describe(&self, serial: &str, model_hint: Option<&str>) -> DeviceInfo {
        let release = self.getprop(serial, "ro.build.version.release").await;
        let model = match model_hint {
            Some(model) if !model.is_empty() => model.to_string(),
            _ => self.getprop(serial, "ro.product.model").await,
        };
        let agent_ready = AgentClient::is_ready(&self.agent_base(serial)).await;
        DeviceInfo {
            udid: serial.to_string(),
            name: if model.is_empty() {
                serial.to_string()
            } else {
                model.clone()
            },
            model,
            // Still named `ios_version` in core; the rename to `os_version`
            // plus a `platform` tag is Pha 2 of the Android plan. Populating it
            // with the Android release is the honest reading of "OS version"
            // until that lands.
            ios_version: release,
            connection: ConnectionKind::Usb,
            status: if agent_ready {
                DeviceStatus::Ready
            } else {
                DeviceStatus::Connected
            },
            battery: self.battery_level(serial).await,
            wda_ready: agent_ready,
            // Android has no provisioning profile to expire. `adb install`
            // needs no per-device signing, so this is `None` forever here.
            wda_expires_at: None,
            stream_url: None,
            tile_stream_state: Default::default(),
            last_error: None,
        }
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
            .context("mở adb forward tới agent")?;

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
                "agent chưa được cài trên {serial}. Cài hai APK \
                 appium-uiautomator2-server và ...-debug-androidTest rồi thử lại"
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
            "agent trên {serial} không phản hồi /status sau 10 giây"
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
            .with_context(|| format!("khởi động agent trên {serial}"))?;
        Ok(())
    }
}

#[async_trait]
impl DeviceDriver for AndroidDriver {
    async fn list_devices(&self) -> anyhow::Result<Vec<DeviceInfo>> {
        let stdout = self
            .adb
            .run(&["devices", "-l"], adb::DEFAULT_TIMEOUT)
            .await?;
        let mut devices = Vec::new();
        for line in adb::parse_devices(&stdout) {
            match line.state {
                AdbDeviceState::Device => {
                    devices.push(self.describe(&line.serial, line.model.as_deref()).await);
                }
                // Report it, do not hide it. A phone whose USB-debugging prompt
                // has not been accepted is a normal fleet state with an obvious
                // fix, and dropping it from the list makes it look unplugged.
                AdbDeviceState::Unauthorized => devices.push(DeviceInfo {
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
                        "Chưa cho phép USB debugging — bấm Cho phép trên màn hình máy".into(),
                    ),
                }),
                AdbDeviceState::Offline | AdbDeviceState::Other => {}
            }
        }
        Ok(devices)
    }

    async fn refresh_device(&self, udid: &str) -> anyhow::Result<DeviceInfo> {
        Ok(self.describe(udid, None).await)
    }

    async fn install_app(&self, udid: &str, path: &Path) -> anyhow::Result<()> {
        let path = path
            .to_str()
            .ok_or_else(|| anyhow!("đường dẫn APK không phải UTF-8"))?;
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
        let remote = "/sdcard/riviu_screenshot.png";
        self.adb
            .shell(udid, &format!("screencap -p {remote}"))
            .await?;
        let local = dest
            .to_str()
            .ok_or_else(|| anyhow!("đường dẫn ảnh không phải UTF-8"))?;
        self.adb
            .device(udid, &["pull", remote, local], Duration::from_secs(120))
            .await?;
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
        let before = self
            .adb
            .shell(udid, &format!("pidof {bundle_id}"))
            .await
            .ok()
            .and_then(|stdout| adb::parse_pidof(&stdout));
        self.adb
            .shell(udid, &format!("am force-stop {bundle_id}"))
            .await?;
        let after = self
            .adb
            .shell(udid, &format!("pidof {bundle_id}"))
            .await
            .ok()
            .and_then(|stdout| adb::parse_pidof(&stdout));
        if let Some(pid) = after {
            return Err(anyhow!(
                "{bundle_id} vẫn chạy (pid {pid}) sau khi force-stop"
            ));
        }
        Ok(ProcessAbsenceProof {
            bundle_id: bundle_id.to_string(),
            old_pid: before,
        })
    }

    fn supports_verified_app_termination(&self) -> bool {
        true
    }

    /// Yes, and measured: Vietnamese reaches TikTok's comment box intact
    /// through accessibility `ACTION_SET_TEXT`.
    fn supports_text_comments(&self) -> bool {
        true
    }

    async fn inspect_app_process(
        &self,
        udid: &str,
        bundle_id: &str,
    ) -> anyhow::Result<AppProcessState> {
        let stdout = self.adb.shell(udid, &format!("pidof {bundle_id}")).await?;
        let pid = adb::parse_pidof(&stdout);
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
        let agent = self.ensure_agent(udid).await?;
        let screen = self.screen_size(udid).await?;
        Ok(Box::new(AndroidUiSession::new(
            agent,
            self.adb.clone(),
            udid.to_string(),
            screen,
        )))
    }

    /// No frame producer yet.
    ///
    /// Deliberate: with hierarchy-based location, frames are corroboration
    /// rather than the locator, so the video pipeline is deferred until
    /// measurement shows it is needed. Failing loudly beats returning a URL
    /// that publishes nothing.
    async fn ensure_stream(&self, _udid: &str) -> anyhow::Result<String> {
        Err(anyhow!(
            "driver Android chưa có nguồn frame — xem Pha 5 của kế hoạch"
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

    #[test]
    fn the_agent_base_url_follows_the_allocated_port() {
        let driver = AndroidDriver::new(&AndroidDriverConfig::default()).expect("driver");
        let base = driver.agent_base("serial-a");
        assert_eq!(base, format!("http://127.0.0.1:{HOST_PORT_BASE}"));
    }
}
