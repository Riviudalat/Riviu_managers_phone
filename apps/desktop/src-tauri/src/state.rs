use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use parking_lot::RwLock;
use riviu_core::db::Database;
use riviu_core::{
    DeviceRegistry, EventBus, JobQueue, NurtureEngine, StreamSettings, STREAM_FPS,
};
use riviu_ios_driver::{create_driver, DriverMode, StreamHub};
use riviu_signing::SigningService;
use tauri::{AppHandle, Emitter};

use crate::nurture_commands::NurtureRuntime;

pub struct AppState {
    pub registry: DeviceRegistry,
    pub events: EventBus,
    pub driver: Arc<dyn riviu_core::DeviceDriver>,
    pub streams: StreamHub,
    pub driver_mode: DriverMode,
    pub jobs: JobQueue,
    pub db: Arc<Database>,
    pub signing: SigningService,
    pub stream_settings: Arc<RwLock<StreamSettings>>,
    pub artifacts_dir: PathBuf,
    pub wda_bundle: PathBuf,
    pub nurture: NurtureRuntime,
    pub nurture_engine: NurtureEngine,
}

impl AppState {
    pub async fn bootstrap() -> anyhow::Result<Self> {
        let data = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("riviu-managers-phone");
        std::fs::create_dir_all(&data)?;
        let artifacts_dir = data.join("artifacts");
        std::fs::create_dir_all(&artifacts_dir)?;

        let db = Arc::new(Database::open(data.join("riviu.db"))?);
        let events = EventBus::new(512);
        let registry = DeviceRegistry::new(events.clone());

        let sidecar_root = resolve_sidecar_root();
        let bundle = create_driver(sidecar_root.join("pymobiledevice3")).await;
        let signing = SigningService::new(sidecar_root.join("signer"));

        let jobs = JobQueue::new(
            db.clone(),
            events.clone(),
            registry.clone(),
            bundle.driver.clone(),
            artifacts_dir.clone(),
        );

        // The engine reads the screen from the frame stream the app already
        // runs for the device tiles, so it never has to ask WDA for a
        // screenshot. `StreamHub` is the FrameSource implementation.
        let nurture_engine = NurtureEngine::new(
            db.clone(),
            bundle.driver.clone(),
            Arc::new(bundle.streams.clone()),
            artifacts_dir.clone(),
        );

        let state = Self {
            registry,
            events,
            driver: bundle.driver,
            streams: bundle.streams,
            driver_mode: bundle.mode,
            jobs,
            db,
            signing,
            stream_settings: Arc::new(RwLock::new(StreamSettings::default())),
            artifacts_dir,
            wda_bundle: sidecar_root.join("wda").join("Riviumanagersphone.ipa"),
            nurture: NurtureRuntime::new(),
            nurture_engine,
        };

        // Initial device scan + auto-start streams
        if let Ok(devices) = state.driver.list_devices().await {
            for d in &devices {
                let _ = state.driver.ensure_stream(&d.udid).await;
            }
            if let Ok(updated) = state.driver.list_devices().await {
                state.registry.upsert_many(updated);
            } else {
                state.registry.upsert_many(devices);
            }
        }

        Ok(state)
    }

    pub fn spawn_background_tasks(&self, app: AppHandle) {
        // Poll devices
        let driver = self.driver.clone();
        let registry = self.registry.clone();
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(3));
            loop {
                interval.tick().await;
                if let Ok(devices) = driver.list_devices().await {
                    for d in &devices {
                        if d.stream_url.is_none() {
                            let _ = driver.ensure_stream(&d.udid).await;
                        }
                    }
                    let devices = driver.list_devices().await.unwrap_or(devices);
                    // Preserve stream URLs / WDA flags from registry when PMD returns fresh list
                    let existing = registry.list();
                    let merged = devices
                        .into_iter()
                        .map(|mut d| {
                            if let Some(prev) = existing.iter().find(|e| e.udid == d.udid) {
                                if d.stream_url.is_none() {
                                    d.stream_url = prev.stream_url.clone();
                                }
                                if !d.wda_ready {
                                    d.wda_ready = prev.wda_ready;
                                }
                                d.wda_expires_at = prev.wda_expires_at.or(d.wda_expires_at);
                                if matches!(prev.status, riviu_core::DeviceStatus::Busy) {
                                    d.status = prev.status.clone();
                                } else if d.stream_url.is_some() {
                                    d.status = riviu_core::DeviceStatus::Ready;
                                    d.wda_ready = true;
                                }
                                if d.last_error.is_none() {
                                    d.last_error = prev.last_error.clone();
                                }
                            }
                            d
                        })
                        .collect();
                    registry.upsert_many(merged);
                }
            }
        });

        // Forward stream frames @ 24 FPS pacing to UI
        let streams = self.streams.clone();
        let app_frames = app.clone();
        tauri::async_runtime::spawn(async move {
            let mut rx = streams.subscribe();
            let min_gap = Duration::from_millis(1000 / STREAM_FPS as u64);
            let mut last_emit = std::collections::HashMap::<String, std::time::Instant>::new();
            loop {
                match rx.recv().await {
                    Ok((udid, jpeg)) => {
                        let now = std::time::Instant::now();
                        let allow = last_emit
                            .get(&udid)
                            .map(|t| now.duration_since(*t) >= min_gap)
                            .unwrap_or(true);
                        if !allow {
                            continue;
                        }
                        last_emit.insert(udid.clone(), now);
                        let payload = serde_json::json!({
                            "type": "streamFrame",
                            "udid": udid,
                            "jpegBase64": B64.encode(jpeg.as_slice()),
                            "fps": STREAM_FPS,
                        });
                        let _ = app_frames.emit("riviu://event", payload);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        });

        // Forward app events
        let mut ev_rx = self.events.subscribe();
        let app_ev = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                match ev_rx.recv().await {
                    Ok(event) => {
                        let _ = app_ev.emit("riviu://event", event);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        });

        // WDA expiry warnings
        let registry = self.registry.clone();
        let events = self.events.clone();
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                for d in registry.list() {
                    if let Some(exp) = d.wda_expires_at {
                        let days = (exp - chrono::Utc::now()).num_days();
                        if days <= 2 {
                            events.emit(riviu_core::AppEvent::WdaExpiryWarning {
                                udid: d.udid,
                                days_remaining: days,
                            });
                        }
                    }
                }
            }
        });

        // Local schedule runner
        let db = self.db.clone();
        let jobs = self.jobs.clone();
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                let Ok(schedules) = db.list_schedules() else {
                    continue;
                };
                let now = chrono::Utc::now();
                for mut s in schedules {
                    if !s.enabled {
                        continue;
                    }
                    let due = s
                        .next_run_at
                        .as_ref()
                        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                        .map(|t| t.with_timezone(&chrono::Utc) <= now)
                        .unwrap_or(true);
                    if !due {
                        continue;
                    }
                    if let Ok(Some(body)) = db.get_script(&s.script_name) {
                        if let Ok(script) = riviu_script_engine::parse_script(&body) {
                            let _ = jobs.enqueue(script, s.udids.clone()).await;
                            let _ = db.log_op("schedule.run", &s.name);
                        }
                    }
                    s.last_run_at = Some(now.to_rfc3339());
                    s.next_run_at = Some(
                        (now + chrono::Duration::minutes(s.every_minutes as i64)).to_rfc3339(),
                    );
                    let _ = db.upsert_schedule(&s);
                }
            }
        });

        // TikTok nurture schedule ticks
        let db = self.db.clone();
        let nurture = self.nurture.clone();
        let nurture_engine = self.nurture_engine.clone();
        let registry = self.registry.clone();
        let app_nurture = app.clone();
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                let Ok(settings) = db.get_nurture_settings() else {
                    continue;
                };
                if !settings.schedule_enabled {
                    continue;
                }
                let every = settings.schedule_every_minutes.max(1) as i64;
                let now = chrono::Utc::now();
                let due = match db.get_setting("nurture.schedule.next_run_at") {
                    Ok(Some(raw)) => chrono::DateTime::parse_from_rfc3339(&raw)
                        .map(|t| t.with_timezone(&chrono::Utc) <= now)
                        .unwrap_or(true),
                    _ => true,
                };
                if !due {
                    continue;
                }
                let mut udids = settings.schedule_udids.clone();
                if udids.is_empty() {
                    udids = registry
                        .list()
                        .into_iter()
                        .filter(|d| {
                            !matches!(
                                d.status,
                                riviu_core::DeviceStatus::Disconnected
                                    | riviu_core::DeviceStatus::Error
                            )
                        })
                        .map(|d| d.udid)
                        .collect();
                }
                if udids.is_empty() {
                    let _ = db.set_setting(
                        "nurture.schedule.next_run_at",
                        &(now + chrono::Duration::minutes(every)).to_rfc3339(),
                    );
                    continue;
                }
                let duration = Duration::from_secs(
                    settings.schedule_duration_minutes.max(1) as u64 * 60,
                );
                let started = nurture
                    .start_many(
                        app_nurture.clone(),
                        nurture_engine.clone(),
                        udids,
                        settings,
                        Some(duration),
                    )
                    .await;
                if !started.is_empty() {
                    let _ = db.log_op("nurture.schedule", &format!("{} devices", started.len()));
                }
                let _ = db.set_setting(
                    "nurture.schedule.next_run_at",
                    &(now + chrono::Duration::minutes(every)).to_rfc3339(),
                );
            }
        });
    }
}

fn resolve_sidecar_root() -> PathBuf {
    if let Ok(p) = std::env::var("RIVIU_SIDECAR_ROOT") {
        return PathBuf::from(p);
    }
    // Dev: repo sidecars/ relative to CARGO_MANIFEST_DIR (apps/desktop/src-tauri)
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("../../..")
        .join("sidecars")
        .canonicalize()
        .unwrap_or_else(|_| manifest.join("../../../sidecars"))
}
