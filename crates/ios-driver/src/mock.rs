use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use image::{ImageBuffer, Rgb};
use parking_lot::Mutex;
use riviu_core::{
    ConnectionKind, DeviceDriver, DeviceInfo, DeviceStatus, SwipeGesture, TapPoint, UiSession,
    STREAM_FPS,
};
use tokio::sync::RwLock;

use crate::stream::{encode_rgb_jpeg, StreamHub};

#[derive(Clone)]
pub struct MockIosDriver {
    devices: Arc<RwLock<Vec<DeviceInfo>>>,
    streams: StreamHub,
    taps: Arc<Mutex<HashMap<String, Vec<TapPoint>>>>,
}

impl MockIosDriver {
    pub fn new() -> Self {
        let now = Utc::now();
        let devices = vec![
            DeviceInfo {
                udid: "MOCK-IPHONE-01".into(),
                name: "iPhone Mock 01".into(),
                model: "iPhone15,2".into(),
                ios_version: "18.2".into(),
                connection: ConnectionKind::Mock,
                status: DeviceStatus::Ready,
                battery: Some(86),
                wda_ready: true,
                wda_expires_at: Some(now + ChronoDuration::days(5)),
                stream_url: Some("mock://MOCK-IPHONE-01".into()),
                last_error: None,
            },
            DeviceInfo {
                udid: "MOCK-IPHONE-02".into(),
                name: "iPhone Mock 02".into(),
                model: "iPhone14,5".into(),
                ios_version: "17.6".into(),
                connection: ConnectionKind::Mock,
                status: DeviceStatus::Ready,
                battery: Some(62),
                wda_ready: true,
                wda_expires_at: Some(now + ChronoDuration::days(2)),
                stream_url: Some("mock://MOCK-IPHONE-02".into()),
                last_error: None,
            },
            DeviceInfo {
                udid: "MOCK-IPHONE-03".into(),
                name: "iPhone Mock 03".into(),
                model: "iPhone13,2".into(),
                ios_version: "16.7".into(),
                connection: ConnectionKind::Mock,
                status: DeviceStatus::Preparing,
                battery: Some(41),
                wda_ready: false,
                wda_expires_at: Some(now - ChronoDuration::days(1)),
                stream_url: None,
                last_error: Some("WDA signing expired".into()),
            },
        ];
        let streams = StreamHub::new();
        let driver = Self {
            devices: Arc::new(RwLock::new(devices)),
            streams: streams.clone(),
            taps: Arc::new(Mutex::new(HashMap::new())),
        };
        driver.spawn_mock_streams();
        driver
    }

    fn spawn_mock_streams(&self) {
        let devices = self.devices.clone();
        let streams = self.streams.clone();
        tokio::spawn(async move {
            let mut tick: u64 = 0;
            let frame_interval = Duration::from_millis(1000 / STREAM_FPS as u64);
            loop {
                let list = devices.read().await.clone();
                for (i, device) in list.iter().enumerate() {
                    if matches!(
                        device.status,
                        DeviceStatus::Disconnected | DeviceStatus::Error
                    ) && !device.wda_ready
                    {
                        continue;
                    }
                    if let Ok(jpeg) = render_mock_frame(&device.name, i, tick) {
                        streams.publish(&device.udid, jpeg);
                    }
                }
                tick = tick.wrapping_add(1);
                tokio::time::sleep(frame_interval).await;
            }
        });
    }

    pub fn stream_hub(&self) -> StreamHub {
        self.streams.clone()
    }
}

impl Default for MockIosDriver {
    fn default() -> Self {
        Self::new()
    }
}

fn render_mock_frame(name: &str, index: usize, tick: u64) -> anyhow::Result<Vec<u8>> {
    let width = 390u32;
    let height = 844u32;
    let mut img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::new(width, height);
    let base: [[u8; 3]; 3] = [
        [24, 28, 42],
        [32, 24, 40],
        [20, 36, 34],
    ];
    let palette = base[index % 3];
    let pulse = ((tick % 48) as u8).saturating_mul(2);
    for y in 0..height {
        for x in 0..width {
            let edge = x < 8 || x > width - 9 || y < 8 || y > height - 9;
            let bar = y > 60 && y < 120;
            let color = if edge {
                Rgb([40, 44, 58])
            } else if bar {
                Rgb([
                    palette[0].saturating_add(pulse / 2),
                    palette[1].saturating_add(30),
                    palette[2].saturating_add(60),
                ])
            } else {
                Rgb([
                    palette[0].saturating_add((y / 20) as u8),
                    palette[1],
                    palette[2].saturating_add((x / 30) as u8),
                ])
            };
            img.put_pixel(x, y, color);
        }
    }
    // Simple status strip representing "live @ 24fps"
    for x in 20..(20 + ((tick % 350) as u32).min(350)) {
        for y in 780..800 {
            img.put_pixel(x, y, Rgb([80, 200, 140]));
        }
    }
    let _ = name; // name shown in UI chrome, not burned into pixels for perf
    encode_rgb_jpeg(&img, 70)
}

struct MockUiSession {
    udid: String,
    taps: Arc<Mutex<HashMap<String, Vec<TapPoint>>>>,
}

#[async_trait]
impl UiSession for MockUiSession {
    async fn tap(&self, point: TapPoint) -> anyhow::Result<()> {
        self.taps
            .lock()
            .entry(self.udid.clone())
            .or_default()
            .push(point);
        Ok(())
    }

    async fn swipe(&self, gesture: SwipeGesture) -> anyhow::Result<()> {
        let _ = gesture;
        Ok(())
    }

    async fn type_text(&self, _text: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn home(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn find_and_tap(&self, _accessibility_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn assert_visible(&self, _accessibility_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn healthy(&self) -> bool {
        true
    }

    async fn window_size(&self) -> anyhow::Result<(f64, f64)> {
        Ok((390.0, 844.0))
    }

    async fn launch_app_foreground(&self, _bundle_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn active_app_bundle(&self) -> anyhow::Result<String> {
        Ok("com.ss.iphone.ugc.Ame".into())
    }

    fn stream_url(&self) -> Option<String> {
        Some(format!("mock://{}", self.udid))
    }
}

#[async_trait]
impl DeviceDriver for MockIosDriver {
    async fn list_devices(&self) -> anyhow::Result<Vec<DeviceInfo>> {
        Ok(self.devices.read().await.clone())
    }

    async fn refresh_device(&self, udid: &str) -> anyhow::Result<DeviceInfo> {
        self.devices
            .read()
            .await
            .iter()
            .find(|d| d.udid == udid)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("device not found"))
    }

    async fn install_app(&self, _udid: &str, path: &Path) -> anyhow::Result<()> {
        if !path.exists() {
            anyhow::bail!("IPA not found: {}", path.display());
        }
        Ok(())
    }

    async fn uninstall_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn screenshot(&self, udid: &str, dest: &Path) -> anyhow::Result<PathBuf> {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let jpeg = self
            .streams
            .latest(udid)
            .map(|frame| frame.to_vec())
            .unwrap_or_else(|| render_mock_frame(udid, 0, 0).unwrap_or_default());
        // store as jpeg bytes with .png extension for simplicity in mock; write jpeg
        let out = if dest.extension().and_then(|e| e.to_str()) == Some("png") {
            dest.with_extension("jpg")
        } else {
            dest.to_path_buf()
        };
        std::fs::write(&out, jpeg)?;
        Ok(out)
    }

    async fn syslog_tail(&self, udid: &str, lines: usize) -> anyhow::Result<String> {
        let mut out = String::new();
        for i in 0..lines {
            out.push_str(&format!(
                "[{udid}] mock syslog line {i}: SpringBoard ready\n"
            ));
        }
        Ok(out)
    }

    async fn launch_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn terminate_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    async fn reboot(&self, udid: &str) -> anyhow::Result<()> {
        let mut list = self.devices.write().await;
        if let Some(d) = list.iter_mut().find(|d| d.udid == udid) {
            d.status = DeviceStatus::Preparing;
        }
        Ok(())
    }

    async fn start_ui_session(&self, udid: &str) -> anyhow::Result<Box<dyn UiSession>> {
        Ok(Box::new(MockUiSession {
            udid: udid.to_string(),
            taps: self.taps.clone(),
        }))
    }

    async fn ensure_stream(&self, udid: &str) -> anyhow::Result<String> {
        Ok(format!("mock://{udid}"))
    }

    async fn prepare_device(&self, udid: &str) -> anyhow::Result<()> {
        let mut list = self.devices.write().await;
        if let Some(d) = list.iter_mut().find(|d| d.udid == udid) {
            d.status = DeviceStatus::Ready;
            d.wda_ready = true;
            d.wda_expires_at = Some(Utc::now() + ChronoDuration::days(7));
            d.last_error = None;
            d.stream_url = Some(format!("mock://{udid}"));
        }
        Ok(())
    }
}
