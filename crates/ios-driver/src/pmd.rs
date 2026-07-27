use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
use riviu_core::{
    ConnectionKind, DeviceDriver, DeviceInfo, DeviceStatus, FrameSource, SwipeGesture, TapPoint,
    UiError, UiErrorKind, UiSession, STREAM_FPS,
};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

use crate::stream::StreamHub;
use crate::supervisor::{DeviceOwned, OwnedChild, ProcessRegistry, Role, SlotMap};
use crate::telemetry;
use crate::wda::WdaClient;

/// First local port used for the USB tunnel to device WDA (8100).
const WDA_LOCAL_PORT_BASE: u16 = 18100;
/// Distinct WDA-control tunnels, one per device. Room for a large phone farm.
const WDA_LOCAL_PORT_SPAN: u16 = 64;

const AGENT_BUNDLE: &str = "com.riviu.managersphone.agent.xctrunner";

#[derive(Clone)]
pub struct PmdIosDriver {
    python: PathBuf,
    script: PathBuf,
    streams: StreamHub,
    wda_host: String,
    mjpeg_port: u16,
    slots: SlotMap,
    registry: ProcessRegistry,
    /// Local control port per UDID. Assignments are sticky, so a device keeps
    /// its port across reconnects and can never collide with another device.
    ports: Arc<Mutex<HashMap<String, u16>>>,
    /// Cached WDA session per UDID.
    sessions: Arc<Mutex<HashMap<String, WdaClient>>>,
}

impl PmdIosDriver {
    pub async fn probe(sidecar_dir: PathBuf) -> anyhow::Result<Self> {
        let script = sidecar_dir.join("riviu_pmd.py");
        if !script.exists() {
            anyhow::bail!("missing sidecar script {}", script.display());
        }
        let python = find_python().await?;
        let output = Command::new(&python)
            .arg(&script)
            .arg("ping")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;
        if !output.status.success() && output.status.code() != Some(2) {
            anyhow::bail!(
                "pmd ping failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let driver = Self::build(python, script);
        // A previous crash can leave a relay holding the port and an XCTest
        // runner holding the device. Reclaim ours before touching any device.
        for what in driver.registry.reclaim_orphans().await {
            tracing::warn!("reclaimed orphaned child process: {what}");
            telemetry::record_event("-", "reclaim_orphan", 0, &what);
        }
        Ok(driver)
    }

    pub fn degraded() -> Self {
        Self::build(PathBuf::from("python3"), PathBuf::new())
    }

    fn build(python: PathBuf, script: PathBuf) -> Self {
        Self {
            python,
            script,
            streams: StreamHub::new(),
            wda_host: "127.0.0.1".into(),
            mjpeg_port: 9100,
            slots: SlotMap::default(),
            registry: ProcessRegistry::new(state_dir()),
            ports: Arc::new(Mutex::new(HashMap::new())),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn stream_hub(&self) -> StreamHub {
        self.streams.clone()
    }

    async fn run_json(&self, args: &[&str]) -> anyhow::Result<serde_json::Value> {
        if self.script.as_os_str().is_empty() {
            anyhow::bail!(
                "pymobiledevice3 sidecar not configured — install python3 + pymobiledevice3"
            );
        }
        let output = Command::new(&self.python)
            .arg(&self.script)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() {
            // Prefer the structured JSON error on stdout over urllib3 noise.
            let detail = serde_json::from_str::<serde_json::Value>(stdout.trim())
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(String::from))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    let s = stderr.trim();
                    if s.is_empty() {
                        stdout.trim().to_string()
                    } else {
                        s.lines()
                            .rev()
                            .find(|l| {
                                !l.contains("NotOpenSSLWarning") && !l.contains("warnings.warn")
                            })
                            .unwrap_or(s)
                            .to_string()
                    }
                });
            anyhow::bail!("pmd {}: {}", args.first().unwrap_or(&""), detail);
        }
        Ok(serde_json::from_str(stdout.trim()).unwrap_or(serde_json::json!({ "ok": true })))
    }

    /// The sticky control port for a device.
    fn port_for(&self, udid: &str) -> u16 {
        let mut ports = self.ports.lock();
        if let Some(p) = ports.get(udid) {
            return *p;
        }
        let taken: HashSet<u16> = ports.values().copied().collect();
        let port = (0..WDA_LOCAL_PORT_SPAN)
            .map(|i| WDA_LOCAL_PORT_BASE + i)
            .find(|p| !taken.contains(p))
            .unwrap_or(WDA_LOCAL_PORT_BASE);
        ports.insert(udid.to_string(), port);
        port
    }

    async fn wda_http_reachable(&self, port: u16) -> bool {
        let url = format!("http://{}:{}/status", self.wda_host, port);
        // Keep-alive hangs after some relay wedges; always close. 5 s because a
        // healthy /status can still take 1–3 s under USB load.
        let Ok(client) = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .pool_max_idle_per_host(0)
            .build()
        else {
            return false;
        };
        match client
            .get(&url)
            .header(reqwest::header::CONNECTION, "close")
            .send()
            .await
        {
            Ok(resp) => resp.status().as_u16() < 500,
            Err(_) => false,
        }
    }

    /// Ensure the control relay is up. Caller holds the device lock.
    async fn ensure_relay_locked(&self, udid: &str, owned: &mut DeviceOwned) -> anyhow::Result<u16> {
        if self.script.as_os_str().is_empty() {
            anyhow::bail!("sidecar missing");
        }
        let port = self.port_for(udid);

        // Reuse a live relay. Transient /status blips right after an app launch
        // must NOT tear it down — that is what used to spawn a second relay.
        let proxy_alive = owned.proxy.as_mut().map_or(false, |c| !c.has_exited());
        if owned.wda_port == Some(port) && proxy_alive && !owned.force_restart {
            for _ in 0..4 {
                if self.wda_http_reachable(port).await {
                    return Ok(port);
                }
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
        }

        // Something is already serving this port and we have no child of our
        // own: adopt it rather than start a competing relay.
        if owned.proxy.is_none() && !owned.force_restart && self.wda_http_reachable(port).await {
            owned.wda_port = Some(port);
            tracing::info!("adopting WDA already listening on {port} for {udid}");
            return Ok(port);
        }

        self.teardown_proxy_locked(owned).await;
        self.sessions.lock().remove(udid);

        let started = Instant::now();
        let force = owned.force_restart;
        let result = self.spawn_proxy_locked(udid, port, force, owned).await;
        telemetry::record_event(
            udid,
            if force { "relay_restart" } else { "relay_start" },
            started.elapsed().as_millis() as u32,
            &match &result {
                Ok(_) => "ok".to_string(),
                Err(e) => e.to_string(),
            },
        );
        result?;

        for _ in 0..12 {
            if self.wda_http_reachable(port).await {
                owned.wda_port = Some(port);
                owned.force_restart = false;
                return Ok(port);
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        self.teardown_proxy_locked(owned).await;
        anyhow::bail!(
            "Không kết nối được WDA qua USB (relay {port} mở nhưng /status im lặng). \
             Mở khoá iPhone, Trust developer, rồi bấm Start/Agent."
        )
    }

    async fn spawn_proxy_locked(
        &self,
        udid: &str,
        port: u16,
        force_restart: bool,
        owned: &mut DeviceOwned,
    ) -> anyhow::Result<()> {
        let mut args = vec![
            "wda-proxy".to_string(),
            "--udid".into(),
            udid.to_string(),
            "--local-port".into(),
            port.to_string(),
            "--bundle-id".into(),
            AGENT_BUNDLE.into(),
        ];
        if force_restart {
            args.push("--restart-wda".into());
        }
        // Never pipe stderr without a reader — the buffer fills and Python
        // deadlocks. Send it to a file when someone is debugging a relay that
        // dies right after coming up.
        let stderr = match std::env::var("RIVIU_PROXY_LOG") {
            Ok(path) if !path.is_empty() => std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map(Stdio::from)
                .unwrap_or_else(|_| Stdio::null()),
            _ => Stdio::null(),
        };
        let mut child: Child = Command::new(&self.python)
            .arg(&self.script)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(stderr)
            .kill_on_drop(true)
            .spawn()?;

        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("wda-proxy stdout missing"))?;

        // A restart kills the device-side runner first and waits for iOS to
        // reap it, so it needs a longer window than a plain start.
        let ready_window = if force_restart {
            Duration::from_secs(110)
        } else {
            Duration::from_secs(55)
        };
        let ready = tokio::time::timeout(ready_window, async {
            let mut buf = Vec::new();
            let mut byte = [0u8; 1];
            loop {
                stdout.read_exact(&mut byte).await?;
                if byte[0] == b'\n' {
                    break;
                }
                buf.push(byte[0]);
                if buf.len() > 8192 {
                    anyhow::bail!("wda-proxy ready line too long");
                }
            }
            let line = String::from_utf8_lossy(&buf);
            let v: serde_json::Value = serde_json::from_str(line.trim())
                .map_err(|e| anyhow::anyhow!("wda-proxy ready parse: {e} ({line})"))?;
            if v.get("ok").and_then(|x| x.as_bool()) != Some(true) {
                anyhow::bail!(
                    "{}",
                    v.get("error").and_then(|e| e.as_str()).unwrap_or(line.trim())
                );
            }
            Ok::<(), anyhow::Error>(())
        })
        .await
        .unwrap_or_else(|_| Err(anyhow::anyhow!("timeout chờ wda-proxy")));

        if let Err(e) = ready {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(e);
        }

        owned.proxy = Some(OwnedChild::adopt(
            &self.registry,
            udid,
            Role::Proxy,
            child,
            &format!("wda-proxy --udid {udid}"),
        ));
        Ok(())
    }

    async fn teardown_proxy_locked(&self, owned: &mut DeviceOwned) {
        if let Some(mut proxy) = owned.proxy.take() {
            proxy.shutdown().await;
        }
        owned.wda_port = None;
    }

    /// Kill the relay *and* the device-side runner, then let the next call
    /// bring both back. Reserved for a confirmed wedge — never a slow probe.
    async fn recycle_locked(&self, udid: &str, owned: &mut DeviceOwned) {
        let started = Instant::now();
        self.sessions.lock().remove(udid);
        let port = owned.wda_port;
        self.teardown_proxy_locked(owned).await;
        owned.force_restart = true;
        if let Some(port) = port {
            for _ in 0..24 {
                if !self.wda_http_reachable(port).await {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
        telemetry::record_event(
            udid,
            "hard_recycle",
            started.elapsed().as_millis() as u32,
            "relay + runner torn down",
        );
    }

    async fn ensure_stream_locked(
        &self,
        udid: &str,
        owned: &mut DeviceOwned,
    ) -> anyhow::Result<()> {
        if self.script.as_os_str().is_empty() {
            anyhow::bail!("sidecar missing");
        }
        if owned.stream.as_mut().map_or(false, |c| !c.has_exited()) {
            return Ok(());
        }
        if let Some(mut dead) = owned.stream.take() {
            dead.shutdown().await;
        }

        let mut child: Child = Command::new(&self.python)
            .arg(&self.script)
            .args([
                "stream",
                "--udid",
                udid,
                "--fps",
                &STREAM_FPS.to_string(),
                "--quality",
                "55",
                "--mode",
                "auto",
                "--wda-bundle",
                AGENT_BUNDLE,
                "--mjpeg-port",
                &self.mjpeg_port.to_string(),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;

        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("stream stdout missing"))?;
        let streams = self.streams.clone();
        let udid_owned = udid.to_string();
        tokio::spawn(async move {
            let mut len_buf = [0u8; 4];
            loop {
                if stdout.read_exact(&mut len_buf).await.is_err() {
                    break;
                }
                let len = u32::from_be_bytes(len_buf) as usize;
                if len == 0 || len > 8_000_000 {
                    break;
                }
                let mut frame = vec![0u8; len];
                if stdout.read_exact(&mut frame).await.is_err() {
                    break;
                }
                streams.publish(&udid_owned, frame);
            }
            tracing::info!("stream ended for {udid_owned}");
        });

        owned.stream = Some(OwnedChild::adopt(
            &self.registry,
            udid,
            Role::Stream,
            child,
            &format!("stream --udid {udid}"),
        ));
        // Give the first frame a moment; don't fail if it is slow.
        tokio::time::sleep(Duration::from_millis(200)).await;
        Ok(())
    }

    /// Get a usable WDA session, creating relay/runner/session as needed.
    /// Caller holds the device lock.
    async fn session_locked(
        &self,
        udid: &str,
        owned: &mut DeviceOwned,
    ) -> anyhow::Result<WdaClient> {
        let port = self.ensure_relay_locked(udid, owned).await?;

        let cached = self.sessions.lock().get(udid).cloned();
        if let Some(client) = cached {
            if client.port() == port && client.session_alive().await {
                return Ok(client);
            }
            self.sessions.lock().remove(udid);
        }

        let mut last_err: Option<UiError> = None;
        for attempt in 0..2 {
            let mut client = WdaClient::new(&self.wda_host, port, udid);
            match client.create_session().await {
                Ok(()) => {
                    // create_session succeeding is enough — window/size can
                    // false-negative under USB load right after session init.
                    self.sessions
                        .lock()
                        .insert(udid.to_string(), client.clone());
                    return Ok(client);
                }
                Err(e) => last_err = Some(e),
            }
            if attempt == 0 {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
        Err(anyhow::Error::new(last_err.unwrap_or_else(|| {
            UiError::new(UiErrorKind::Session, "session.create", "unavailable")
        })))
    }
}

fn state_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("riviu-managers-phone")
}

async fn find_python() -> anyhow::Result<PathBuf> {
    for candidate in ["python3", "python"] {
        if let Ok(output) = Command::new(candidate).arg("--version").output().await {
            if output.status.success() {
                return Ok(PathBuf::from(candidate));
            }
        }
    }
    anyhow::bail!("python3 not found")
}

struct PmdUiSession {
    client: WdaClient,
    mjpeg_url: String,
}

#[async_trait]
impl UiSession for PmdUiSession {
    async fn tap(&self, point: TapPoint) -> anyhow::Result<()> {
        self.client.tap(point).await.map_err(anyhow::Error::new)
    }

    async fn tap_native(&self, point: TapPoint) -> anyhow::Result<()> {
        self.client
            .tap_native(point)
            .await
            .map_err(anyhow::Error::new)
    }

    async fn swipe(&self, gesture: SwipeGesture) -> anyhow::Result<()> {
        self.client.swipe(gesture).await.map_err(anyhow::Error::new)
    }

    async fn tap_image(&self, x: f64, y: f64, image_w: f64, image_h: f64) -> anyhow::Result<()> {
        let point = self.client.to_points(x, y, image_w, image_h).await?;
        self.client.tap(point).await.map_err(anyhow::Error::new)
    }

    async fn swipe_image(
        &self,
        from: TapPoint,
        to: TapPoint,
        image_w: f64,
        image_h: f64,
        duration_ms: u64,
    ) -> anyhow::Result<()> {
        let from = self
            .client
            .to_points(from.x, from.y, image_w, image_h)
            .await?;
        let to = self.client.to_points(to.x, to.y, image_w, image_h).await?;
        self.client
            .swipe(SwipeGesture {
                from,
                to,
                duration_ms,
            })
            .await
            .map_err(anyhow::Error::new)
    }

    async fn type_text(&self, text: &str) -> anyhow::Result<()> {
        self.client
            .type_text(text)
            .await
            .map_err(anyhow::Error::new)
    }

    async fn home(&self) -> anyhow::Result<()> {
        self.client.home().await.map_err(anyhow::Error::new)
    }

    async fn find_and_tap(&self, accessibility_id: &str) -> anyhow::Result<()> {
        self.client
            .find_and_tap(accessibility_id)
            .await
            .map_err(anyhow::Error::new)
    }

    async fn assert_visible(&self, accessibility_id: &str) -> anyhow::Result<()> {
        self.client
            .assert_visible(accessibility_id)
            .await
            .map_err(anyhow::Error::new)
    }

    async fn dismiss_alert(&self) -> anyhow::Result<()> {
        self.client
            .dismiss_alert()
            .await
            .map_err(anyhow::Error::new)
    }

    async fn healthy(&self) -> bool {
        self.client.health_quick().await
    }

    async fn window_size(&self) -> anyhow::Result<(f64, f64)> {
        self.client.window_size().await.map_err(anyhow::Error::new)
    }

    async fn launch_app_foreground(&self, bundle_id: &str) -> anyhow::Result<()> {
        self.client
            .activate_app(bundle_id)
            .await
            .map_err(anyhow::Error::new)
    }

    async fn active_app_bundle(&self) -> anyhow::Result<String> {
        self.client
            .active_app_bundle()
            .await
            .map_err(anyhow::Error::new)
    }

    async fn screenshot_png(&self) -> anyhow::Result<Vec<u8>> {
        self.client
            .screenshot_png()
            .await
            .map_err(anyhow::Error::new)
    }

    fn stream_url(&self) -> Option<String> {
        Some(self.mjpeg_url.clone())
    }
}

#[async_trait]
impl DeviceDriver for PmdIosDriver {
    async fn list_devices(&self) -> anyhow::Result<Vec<DeviceInfo>> {
        if self.script.as_os_str().is_empty() {
            return Ok(Vec::new());
        }
        let value = match self.run_json(&["list"]).await {
            Ok(v) => v,
            Err(_) => return Ok(Vec::new()),
        };
        let devices = value
            .get("devices")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::new();
        for d in devices {
            let udid = d
                .get("udid")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if udid.is_empty() {
                continue;
            }
            let conn = match d.get("connection").and_then(|v| v.as_str()).unwrap_or("usb") {
                "wifi" | "network" => ConnectionKind::Wifi,
                _ => ConnectionKind::Usb,
            };
            let streaming = FrameSource::latest(&self.streams, &udid).is_some();
            out.push(DeviceInfo {
                udid: udid.clone(),
                name: d
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("iPhone")
                    .to_string(),
                model: d
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown")
                    .to_string(),
                ios_version: d
                    .get("iosVersion")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string(),
                connection: conn,
                status: if streaming {
                    DeviceStatus::Ready
                } else {
                    DeviceStatus::Connected
                },
                battery: d.get("battery").and_then(|v| v.as_u64()).map(|v| v as u8),
                wda_ready: streaming,
                wda_expires_at: None,
                stream_url: if streaming {
                    Some(format!("screenshot-stream://{udid}"))
                } else {
                    None
                },
                last_error: d
                    .get("pairingError")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            });
        }
        Ok(out)
    }

    async fn refresh_device(&self, udid: &str) -> anyhow::Result<DeviceInfo> {
        self.list_devices()
            .await?
            .into_iter()
            .find(|d| d.udid == udid)
            .ok_or_else(|| anyhow::anyhow!("device not found"))
    }

    async fn install_app(&self, udid: &str, path: &Path) -> anyhow::Result<()> {
        self.run_json(&[
            "install",
            "--udid",
            udid,
            "--ipa",
            &path.display().to_string(),
        ])
        .await?;
        Ok(())
    }

    async fn uninstall_app(&self, udid: &str, bundle_id: &str) -> anyhow::Result<()> {
        self.run_json(&["uninstall", "--udid", udid, "--bundle-id", bundle_id])
            .await?;
        Ok(())
    }

    async fn screenshot(&self, udid: &str, dest: &Path) -> anyhow::Result<PathBuf> {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        self.run_json(&[
            "screenshot",
            "--udid",
            udid,
            "--out",
            &dest.display().to_string(),
        ])
        .await?;
        Ok(dest.to_path_buf())
    }

    async fn syslog_tail(&self, udid: &str, lines: usize) -> anyhow::Result<String> {
        let value = self
            .run_json(&["syslog", "--udid", udid, "--lines", &lines.to_string()])
            .await?;
        Ok(value
            .get("log")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string())
    }

    /// Launch by bundle id via the Instruments channel.
    ///
    /// Held under the device lock because this competes with the WDA relay for
    /// the same usbmux connection — running the two concurrently is what wedged
    /// usbmux in live tests #5–#7. Prefer `UiSession::launch_app_foreground`
    /// (WDA activate) when a session already exists; this is the cold path.
    async fn launch_app(&self, udid: &str, bundle_id: &str) -> anyhow::Result<()> {
        let slot = self.slots.get(udid);
        let _owned = slot.owned.lock().await;
        let started = Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(12),
            self.run_json(&["launch", "--udid", udid, "--bundle-id", bundle_id]),
        )
        .await
        .unwrap_or_else(|_| Err(anyhow::anyhow!("launch timeout ({bundle_id})")));
        telemetry::record_event(
            udid,
            "launch_app",
            started.elapsed().as_millis() as u32,
            if result.is_ok() { "ok" } else { "failed" },
        );
        result?;
        Ok(())
    }

    async fn terminate_app(&self, udid: &str, bundle_id: &str) -> anyhow::Result<()> {
        self.run_json(&["terminate", "--udid", udid, "--bundle-id", bundle_id])
            .await?;
        Ok(())
    }

    async fn reboot(&self, udid: &str) -> anyhow::Result<()> {
        self.run_json(&["reboot", "--udid", udid]).await?;
        Ok(())
    }

    async fn start_ui_session(&self, udid: &str) -> anyhow::Result<Box<dyn UiSession>> {
        let slot = self.slots.get(udid);
        let mut owned = slot.owned.lock().await;
        let client = self.session_locked(udid, &mut owned).await?;
        Ok(Box::new(PmdUiSession {
            client,
            mjpeg_url: WdaClient::mjpeg_url(&self.wda_host, self.mjpeg_port),
        }))
    }

    async fn ui_session_cached(&self, udid: &str) -> bool {
        self.sessions.lock().contains_key(udid)
    }

    async fn invalidate_ui_session(&self, udid: &str) {
        // Soft: drop the cached session id only. Force-killing the agent on
        // every reopen caused Instruments death spirals when health probes
        // false-negatived.
        self.sessions.lock().remove(udid);
    }

    async fn recycle_ui_transport(&self, udid: &str) {
        let slot = self.slots.get(udid);
        let mut owned = slot.owned.lock().await;
        self.recycle_locked(udid, &mut owned).await;
    }

    async fn ensure_stream(&self, udid: &str) -> anyhow::Result<String> {
        let slot = self.slots.get(udid);
        let mut owned = slot.owned.lock().await;
        // The control plane owns WDA; the stream must never start or kill the
        // runner, or it tears down nurture mid-gesture.
        let _ = self.ensure_relay_locked(udid, &mut owned).await;
        self.ensure_stream_locked(udid, &mut owned).await?;
        Ok(format!("auto-stream://{udid}"))
    }

    async fn prepare_device(&self, udid: &str) -> anyhow::Result<()> {
        self.ensure_stream(udid).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_device_gets_its_own_sticky_control_port() {
        let driver = PmdIosDriver::degraded();
        let a = driver.port_for("udid-a");
        let b = driver.port_for("udid-b");
        assert_ne!(a, b, "two devices must not share a relay port");
        assert_eq!(a, driver.port_for("udid-a"), "assignments are sticky");
        assert!((WDA_LOCAL_PORT_BASE..WDA_LOCAL_PORT_BASE + WDA_LOCAL_PORT_SPAN).contains(&a));
        assert!((WDA_LOCAL_PORT_BASE..WDA_LOCAL_PORT_BASE + WDA_LOCAL_PORT_SPAN).contains(&b));
    }

    /// Two logical jobs on one device must serialise on the slot lock rather
    /// than both proceeding to spawn a relay.
    #[tokio::test]
    async fn a_second_job_for_the_same_device_queues() {
        let driver = PmdIosDriver::degraded();
        let slot = driver.slots.get("udid-a");
        let held = slot.owned.lock().await;
        assert!(driver.slots.get("udid-a").owned.try_lock().is_err());
        assert!(driver.slots.get("udid-b").owned.try_lock().is_ok());
        drop(held);
        assert!(driver.slots.get("udid-a").owned.try_lock().is_ok());
    }
}
