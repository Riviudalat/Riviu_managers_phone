use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use riviu_core::{NurtureEngine, NurtureSessionStatus, NurtureSettings};
use tauri::{AppHandle, Emitter, State};

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
) -> Result<Vec<String>, String> {
    if udids.is_empty() {
        return Err("Chưa chọn thiết bị".into());
    }
    let settings = state.db.get_nurture_settings().map_err(err)?;
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
    stops: Mutex<HashMap<String, Arc<AtomicBool>>>,
    status: Mutex<HashMap<String, NurtureSessionStatus>>,
}

impl NurtureRuntime {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(NurtureRuntimeInner {
                stops: Mutex::new(HashMap::new()),
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
        if let Some(flag) = self.inner.stops.lock().get(udid) {
            flag.store(true, Ordering::Relaxed);
        }
    }

    pub fn stop_all(&self) {
        for flag in self.inner.stops.lock().values() {
            flag.store(true, Ordering::Relaxed);
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
            {
                let map = self.inner.status.lock();
                if map.get(&udid).map(|s| s.running).unwrap_or(false) {
                    continue;
                }
            }
            let stop = Arc::new(AtomicBool::new(false));
            self.inner.stops.lock().insert(udid.clone(), stop.clone());
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
                if stagger > 0 {
                    tokio::time::sleep(Duration::from_secs(stagger as u64)).await;
                }
                let result = engine
                    .run_session(
                        &udid_clone,
                        settings,
                        stop,
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
                    .await;
                let final_status = match result {
                    Ok(mut s) => {
                        s.running = false;
                        s
                    }
                    Err(e) => NurtureSessionStatus {
                        udid: udid_clone,
                        running: false,
                        videos_done: 0,
                        likes: 0,
                        comments: 0,
                        follows: 0,
                        last_message: format!("error: {e}"),
                        session_usd: 0.0,
                    },
                };
                runtime.set_status(final_status.clone());
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
