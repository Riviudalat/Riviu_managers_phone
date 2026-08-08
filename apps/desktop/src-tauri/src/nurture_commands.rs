use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use riviu_core::{
    DeviceControlPlane, DeviceWorkOwner, FrameSource, NurtureEngine, NurtureSessionStatus,
    NurtureSettings,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, State};

use crate::command_error::CommandError;
use crate::state::AppState;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NurtureApiTestResult {
    pub udid: String,
    pub comment: String,
    pub caption: Option<String>,
    pub context_confidence: u8,
    pub relevance: u8,
    pub evidence_support: u8,
    pub frame_sha256: String,
    pub model: String,
    pub base_url_host: String,
    pub evidence_mode: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub usd: f64,
}

fn validate_nurture_settings(settings: &NurtureSettings) -> Result<(), String> {
    if !(1..=10_000).contains(&settings.num_videos) {
        return Err("num_videos phải nằm trong khoảng 1..=10000".into());
    }
    if !(1..=100).contains(&settings.num_rounds) {
        return Err("num_rounds phải nằm trong khoảng 1..=100".into());
    }
    if settings.like_prob.saturating_add(settings.comment_prob) > 100 {
        return Err(format!(
            "like_prob ({}) + comment_prob ({}) > 100",
            settings.like_prob, settings.comment_prob
        ));
    }
    if !(4..=30).contains(&settings.max_comment_words) {
        return Err("max_comment_words phải nằm trong khoảng 4..=30".into());
    }
    if settings.follow_prob > 100 || settings.frenzy_prob > 100 {
        return Err("follow_prob và frenzy_prob phải nằm trong khoảng 0..=100".into());
    }
    if settings.comment_prob > 0 && settings.api_key.trim().is_empty() {
        return Err("Đã bật bình luận nhưng API key còn trống".into());
    }
    if settings.base_url.trim().is_empty() || settings.model.trim().is_empty() {
        return Err("Base URL và model AI không được để trống".into());
    }
    if !settings.watch_min.is_finite()
        || !settings.watch_max.is_finite()
        || settings.watch_min <= 0.0
        || settings.watch_max < settings.watch_min
        || settings.watch_max > 120.0
    {
        return Err("Khoảng thời gian xem video phải trong 0..120 giây và min <= max".into());
    }
    if !(15..=1_440).contains(&settings.schedule_every_minutes) {
        return Err("schedule_every_minutes phải nằm trong khoảng 15..=1440".into());
    }
    if !(15..=360).contains(&settings.schedule_duration_minutes) {
        return Err("schedule_duration_minutes phải nằm trong khoảng 15..=360".into());
    }
    Ok(())
}

#[tauri::command]
pub fn nurture_get_settings(state: State<'_, AppState>) -> Result<NurtureSettings, String> {
    state.db.get_nurture_settings().map_err(err)
}

#[tauri::command]
pub fn nurture_save_settings(
    state: State<'_, AppState>,
    settings: NurtureSettings,
) -> Result<NurtureSettings, String> {
    validate_nurture_settings(&settings)?;
    let _admission = state.ensure_accepting_work()?;
    let prev = state.db.get_nurture_settings().unwrap_or_default();
    state.db.save_nurture_settings(&settings).map_err(err)?;
    // When schedule is (re)enabled, schedule the next tick from now.
    if settings.schedule_enabled
        && (!prev.schedule_enabled
            || prev.schedule_every_minutes != settings.schedule_every_minutes)
    {
        let every = settings.schedule_every_minutes.max(1) as i64;
        let next = (chrono::Utc::now() + chrono::Duration::minutes(every)).to_rfc3339();
        let _ = state.db.set_setting("nurture.schedule.next_run_at", &next);
    }
    if !settings.schedule_enabled {
        let _ = state.db.set_setting("nurture.schedule.next_run_at", "");
    }
    let _ = state.db.log_op("nurture.settings", &settings.model);
    Ok(settings)
}

/// Run the same grounded vision pipeline as production comment preparation,
/// but stop after returning the prepared text. No device UI or comment sender
/// is opened by this command.
#[tauri::command]
pub async fn nurture_test_api(
    state: State<'_, AppState>,
    udid: String,
) -> Result<NurtureApiTestResult, String> {
    let _admission = state.ensure_accepting_work()?;
    let udid = udid.trim().to_string();
    if udid.is_empty() {
        return Err("Chọn một thiết bị để test API".into());
    }
    let settings = state.db.get_nurture_settings().map_err(err)?;
    if settings.api_key.trim().is_empty() {
        return Err("API key đang trống — lưu Cấu hình AI trước khi test".into());
    }
    if settings.base_url.trim().is_empty() || settings.model.trim().is_empty() {
        return Err("Base URL và model AI không được để trống".into());
    }

    let mut frames = Vec::with_capacity(3);
    if let Some(frame) = state.streams.latest(&udid) {
        frames.push(frame.as_ref().clone());
    }
    let mut stream = FrameSource::subscribe(&state.streams, &udid);
    let deadline = tokio::time::Instant::now() + Duration::from_millis(1500);
    while frames.len() < 3 {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, stream.next()).await {
            Ok(Some(frame)) => frames.push(frame.as_ref().clone()),
            Ok(None) | Err(_) => break,
        }
    }
    if frames.is_empty() {
        return Err(format!("Chưa có frame stream cho thiết bị {udid}"));
    }

    let direction = settings
        .ai_directions
        .split('|')
        .map(str::trim)
        .find(|value| !value.is_empty());
    let (result, evidence_mode) = if riviu_core::openai_client::provider_supports_vision(&settings)
    {
        (
            riviu_core::openai_client::prepare_grounded_comment(&settings, &frames, direction)
                .await
                .map_err(err)?,
            "vision",
        )
    } else {
        let frame = frames
            .last()
            .ok_or_else(|| "Chưa có frame stream cho thiết bị".to_string())?;
        let observations = crate::interaction_ocr::recognize(frame)
            .await
            .map_err(|error| format!("DeepSeek chỉ nhận text và OCR caption lỗi: {error}"))?;
        let caption = riviu_core::openai_client::ocr_caption(&observations).ok_or_else(|| {
            "DeepSeek chỉ nhận text; chưa đọc được caption từ frame hiện tại".to_string()
        })?;
        let frame_sha256 = sha256_hex(frame);
        (
            riviu_core::openai_client::prepare_caption_comment(
                &settings,
                &caption,
                &frame_sha256,
                direction,
            )
            .await
            .map_err(err)?,
            "ocr-caption",
        )
    };

    Ok(NurtureApiTestResult {
        udid,
        comment: result.text,
        caption: result
            .caption
            .map(|caption| caption.chars().take(240).collect()),
        context_confidence: result.context_confidence,
        relevance: result.relevance,
        evidence_support: result.evidence_support,
        frame_sha256: result.frame_sha256,
        model: result.model,
        base_url_host: result.base_url_host,
        evidence_mode: evidence_mode.into(),
        prompt_tokens: result.prompt_tokens,
        completion_tokens: result.completion_tokens,
        usd: result.usd,
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[tauri::command]
pub fn nurture_list_costs(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<riviu_core::NurtureCommentCost>, String> {
    state
        .db
        .list_nurture_comment_costs(limit.unwrap_or(100))
        .map_err(err)
}

#[tauri::command]
pub fn nurture_list_comment_attempts(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<riviu_core::NurtureCommentAttempt>, String> {
    state
        .db
        .list_nurture_comment_attempts(limit.unwrap_or(100))
        .map_err(err)
}

#[tauri::command]
pub fn nurture_cost_summary(
    state: State<'_, AppState>,
) -> Result<riviu_core::NurtureCostSummary, String> {
    state.db.nurture_cost_summary().map_err(err)
}

#[tauri::command]
pub fn nurture_session_status(
    state: State<'_, AppState>,
) -> Result<Vec<NurtureSessionStatus>, String> {
    Ok(state.nurture.list_status())
}

#[tauri::command]
pub async fn nurture_start(
    app: AppHandle,
    state: State<'_, AppState>,
    udids: Vec<String>,
    duration_minutes: Option<u32>,
) -> Result<Vec<String>, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    if udids.is_empty() {
        return Err("Chưa chọn thiết bị".into());
    }
    let settings = state
        .db
        .get_nurture_settings()
        .map_err(CommandError::operation)?;
    validate_nurture_settings(&settings).map_err(CommandError::operation)?;
    preflight_comment_job(&state.control, &udids, &settings).await?;
    // Manual starts get a varied 2–3 hour horizon so they do not all end on
    // the same fixed video count. Scheduled starts keep their explicit value.
    let run_duration = duration_minutes
        .map(|m| Duration::from_secs(m as u64 * 60))
        .or_else(|| {
            let jitter = chrono::Utc::now().timestamp_subsec_nanos() % 61;
            Some(Duration::from_secs((120 + jitter) as u64 * 60))
        });
    let started = state
        .nurture
        .start_many(
            app,
            state.nurture_engine.clone(),
            udids,
            settings,
            run_duration,
        )
        .await;
    Ok(started)
}

async fn preflight_comment_job(
    control: &DeviceControlPlane,
    udids: &[String],
    settings: &NurtureSettings,
) -> Result<(), CommandError> {
    if settings.comment_prob == 0 {
        return Ok(());
    }

    let mut failures = Vec::new();
    for udid in udids {
        let context = control
            .try_acquire_exclusive(udid, DeviceWorkOwner::Nurture)
            .await
            .map_err(CommandError::from)?;
        match control.preflight_agent(&context).await {
            Ok(status) if status.auth_ready => {}
            Ok(status) => failures.push(format!(
                "{udid}: {}",
                status
                    .message
                    .unwrap_or_else(|| format!("trạng thái {:?}", status.state))
            )),
            Err(error) => failures.push(format!("{udid}: {error}")),
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(CommandError::operation(format!(
            "Riviu Agent chưa sẵn sàng cho bình luận chữ: {}. Chạy Agent Repair rồi thử lại.",
            failures.join("; ")
        )))
    }
}

#[tauri::command]
pub fn nurture_stop(state: State<'_, AppState>, udids: Vec<String>) -> Result<(), String> {
    let _admission = state.ensure_accepting_work()?;
    if udids.is_empty() {
        state.nurture.stop_all();
    } else {
        for u in udids {
            state.nurture.stop(&u);
        }
    }
    Ok(())
}

#[derive(Clone)]
pub struct NurtureRuntime {
    inner: Arc<NurtureRuntimeInner>,
}

struct NurtureRuntimeInner {
    runs: Mutex<NurtureRuns>,
    status: Mutex<HashMap<String, NurtureSessionStatus>>,
}

struct NurtureRuns {
    accepting_starts: bool,
    stops: HashMap<String, Arc<AtomicBool>>,
}

impl NurtureRuntime {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(NurtureRuntimeInner {
                runs: Mutex::new(NurtureRuns {
                    accepting_starts: true,
                    stops: HashMap::new(),
                }),
                status: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn list_status(&self) -> Vec<NurtureSessionStatus> {
        self.inner.status.lock().values().cloned().collect()
    }

    pub fn set_status(&self, st: NurtureSessionStatus) {
        self.inner.status.lock().insert(st.udid.clone(), st);
    }

    pub fn stop(&self, udid: &str) {
        if let Some(flag) = self.inner.runs.lock().stops.get(udid) {
            flag.store(true, Ordering::Relaxed);
        }
    }

    pub fn stop_all(&self) {
        for flag in self.inner.runs.lock().stops.values() {
            flag.store(true, Ordering::Relaxed);
        }
    }

    pub fn begin_shutdown(&self) {
        let mut runs = self.inner.runs.lock();
        runs.accepting_starts = false;
        for flag in runs.stops.values() {
            flag.store(true, Ordering::Relaxed);
        }
    }

    fn reserve_start(&self, udid: &str) -> Option<Arc<AtomicBool>> {
        let mut runs = self.inner.runs.lock();
        if !runs.accepting_starts || runs.stops.contains_key(udid) {
            return None;
        }
        let stop = Arc::new(AtomicBool::new(false));
        runs.stops.insert(udid.to_string(), stop.clone());
        Some(stop)
    }

    fn finish_start(&self, udid: &str, stop: &Arc<AtomicBool>) {
        let mut runs = self.inner.runs.lock();
        if runs
            .stops
            .get(udid)
            .is_some_and(|current| Arc::ptr_eq(current, stop))
        {
            runs.stops.remove(udid);
        }
    }

    async fn wait_stagger_or_stop(stop: &AtomicBool, duration: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + duration;
        loop {
            if stop.load(Ordering::Relaxed) {
                return true;
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return false;
            }
            tokio::time::sleep((deadline - now).min(Duration::from_millis(100))).await;
        }
    }

    pub async fn start_many(
        &self,
        app: AppHandle,
        engine: NurtureEngine,
        udids: Vec<String>,
        settings: NurtureSettings,
        max_duration: Option<Duration>,
    ) -> Vec<String> {
        let mut started = Vec::new();
        for (idx, udid) in udids.into_iter().enumerate() {
            let Some(stop) = self.reserve_start(&udid) else {
                continue;
            };
            let initial = NurtureSessionStatus {
                udid: udid.clone(),
                running: true,
                videos_done: 0,
                swipe_attempts: 0,
                like_attempts: 0,
                comment_attempts: 0,
                follow_attempts: 0,
                likes: 0,
                comments: 0,
                follows: 0,
                last_message: "queued".into(),
                session_usd: 0.0,
            };
            self.set_status(initial);

            let runtime = self.clone();
            let engine = engine.clone();
            let settings = settings.clone();
            let app2 = app.clone();
            let udid_clone = udid.clone();
            let task_stop = stop.clone();
            let min = settings.stagger_delay_min.min(settings.stagger_delay_max);
            let max = settings.stagger_delay_max.max(settings.stagger_delay_min);
            let stagger = if idx == 0 {
                0
            } else if max > min {
                min + (idx as u32 % (max - min + 1))
            } else {
                min
            };

            tauri::async_runtime::spawn(async move {
                let stopped_before_start =
                    Self::wait_stagger_or_stop(&task_stop, Duration::from_secs(stagger as u64))
                        .await;
                let final_status = if stopped_before_start || task_stop.load(Ordering::Acquire) {
                    NurtureSessionStatus {
                        udid: udid_clone.clone(),
                        running: false,
                        videos_done: 0,
                        swipe_attempts: 0,
                        like_attempts: 0,
                        comment_attempts: 0,
                        follow_attempts: 0,
                        likes: 0,
                        comments: 0,
                        follows: 0,
                        last_message: "stopped before start".to_string(),
                        session_usd: 0.0,
                    }
                } else {
                    match engine
                        .run_session(
                            &udid_clone,
                            settings,
                            task_stop.clone(),
                            max_duration,
                            |st| {
                                runtime.set_status(st.clone());
                                let _ = app2.emit(
                                    "riviu://event",
                                    serde_json::json!({
                                        "type": "nurtureStatus",
                                        "status": st,
                                    }),
                                );
                            },
                        )
                        .await
                    {
                        Ok(mut status) => {
                            status.running = false;
                            status
                        }
                        Err(error) => NurtureSessionStatus {
                            udid: udid_clone.clone(),
                            running: false,
                            videos_done: 0,
                            swipe_attempts: 0,
                            like_attempts: 0,
                            comment_attempts: 0,
                            follow_attempts: 0,
                            likes: 0,
                            comments: 0,
                            follows: 0,
                            last_message: format!("error: {error}"),
                            session_usd: 0.0,
                        },
                    }
                };
                runtime.set_status(final_status.clone());
                runtime.finish_start(&udid_clone, &task_stop);
                let _ = app2.emit(
                    "riviu://event",
                    serde_json::json!({
                        "type": "nurtureStatus",
                        "status": final_status,
                    }),
                );
            });
            started.push(udid);
        }
        started
    }
}

impl Default for NurtureRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use riviu_core::{
        AgentState, DeviceControlPlane, DeviceDriver, DeviceWorkCoordinator, StreamBudgetManager,
    };
    use riviu_ios_driver::MockIosDriver;

    #[test]
    fn concurrent_starts_reserve_exactly_one_stop_token_per_device() {
        let runtime = NurtureRuntime::new();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let mut workers = Vec::new();
        for _ in 0..2 {
            let runtime = runtime.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                runtime.reserve_start("same-device")
            }));
        }
        barrier.wait();
        let reservations = workers
            .into_iter()
            .map(|worker| worker.join().expect("reservation worker"))
            .collect::<Vec<_>>();

        assert_eq!(
            reservations.iter().filter(|value| value.is_some()).count(),
            1
        );
    }

    #[test]
    fn shutdown_atomically_blocks_new_starts_and_signals_existing_tokens() {
        let runtime = NurtureRuntime::new();
        let active = runtime.reserve_start("active").expect("active token");

        runtime.begin_shutdown();

        assert!(active.load(Ordering::Relaxed));
        assert!(runtime.reserve_start("late").is_none());
    }

    #[tokio::test]
    async fn stop_interrupts_stagger_before_a_device_session_can_start() {
        let stop = Arc::new(AtomicBool::new(false));
        let waiter_stop = stop.clone();
        let waiter = tokio::spawn(async move {
            NurtureRuntime::wait_stagger_or_stop(&waiter_stop, Duration::from_secs(30)).await
        });

        tokio::time::sleep(Duration::from_millis(10)).await;
        stop.store(true, Ordering::Relaxed);

        assert!(tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("stagger stop should be observed promptly")
            .expect("stagger waiter"));
    }

    #[tokio::test]
    async fn comment_job_with_unready_agent_is_rejected_before_it_is_reported_started() {
        let driver = MockIosDriver::new();
        for udid in ["needs-repair-a", "needs-repair-b"] {
            let mut status = driver.cached_agent_status(udid);
            status.state = AgentState::RepairRequired;
            status.message = Some("agent version does not match manifest".to_string());
            driver.set_mock_agent_status(status);
        }
        let control = DeviceControlPlane::new(
            Arc::new(driver.clone()),
            Arc::new(DeviceWorkCoordinator::new()),
            Arc::new(StreamBudgetManager::default()),
        );
        let runtime = NurtureRuntime::new();
        let settings = NurtureSettings {
            comment_prob: 1,
            ..Default::default()
        };

        let error = preflight_comment_job(
            &control,
            &["needs-repair-a".to_string(), "needs-repair-b".to_string()],
            &settings,
        )
        .await
        .expect_err("an unready text agent must reject the whole command");

        assert!(error.message.contains("needs-repair-a"));
        assert!(error.message.contains("needs-repair-b"));
        assert!(error.message.contains("Agent Repair"));
        assert!(runtime.list_status().is_empty());
        assert_eq!(
            driver.agent_preflight_calls(),
            0,
            "comment preflight must use install-only readiness"
        );
    }

    #[test]
    fn default_nurture_settings_pass_validation() {
        assert!(validate_nurture_settings(&NurtureSettings::default()).is_ok());
    }

    #[test]
    fn nurture_validation_rejects_unbounded_session_values() {
        let settings = NurtureSettings {
            num_videos: 10_001,
            ..NurtureSettings::default()
        };
        assert!(validate_nurture_settings(&settings)
            .expect_err("video ceiling must be bounded")
            .contains("num_videos"));

        let settings = NurtureSettings {
            schedule_duration_minutes: 10,
            ..NurtureSettings::default()
        };
        assert!(validate_nurture_settings(&settings)
            .expect_err("schedule burst must be human-sized")
            .contains("schedule_duration_minutes"));

        let settings = NurtureSettings {
            watch_max: 121.0,
            ..NurtureSettings::default()
        };
        assert!(validate_nurture_settings(&settings)
            .expect_err("watch duration must be bounded")
            .contains("thời gian xem"));
    }
}
