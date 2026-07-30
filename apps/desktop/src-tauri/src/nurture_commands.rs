use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use riviu_core::{
    DeviceControlPlane, DeviceWorkOwner, NurtureEngine, NurtureSessionStatus, NurtureSettings,
};
use tauri::{AppHandle, Emitter, State};

use crate::command_error::CommandError;
use crate::state::AppState;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
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
    if settings.like_prob + settings.comment_prob > 100 {
        return Err(format!(
            "like_prob ({}) + comment_prob ({}) > 100",
            settings.like_prob, settings.comment_prob
        ));
    }
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
    if udids.is_empty() {
        return Err("Chưa chọn thiết bị".into());
    }
    let settings = state
        .db
        .get_nurture_settings()
        .map_err(CommandError::operation)?;
    preflight_comment_job(&state.control, &udids, &settings).await?;
    let started = state
        .nurture
        .start_many(
            app,
            state.nurture_engine.clone(),
            udids,
            settings,
            duration_minutes.map(|m| Duration::from_secs(m as u64 * 60)),
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
}
