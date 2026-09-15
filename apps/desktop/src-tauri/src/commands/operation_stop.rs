use super::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tauri::Manager;
static ACTIVE_STOPS: std::sync::LazyLock<parking_lot::Mutex<HashSet<String>>> =
    std::sync::LazyLock::new(|| parking_lot::Mutex::new(HashSet::new()));
struct StopGuard(String);
impl Drop for StopGuard {
    fn drop(&mut self) {
        ACTIVE_STOPS.lock().remove(&self.0);
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StopDeviceResult {
    pub udid: String,
    pub closed: bool,
    pub message: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationStopResult {
    pub operation_id: String,
    pub state: String,
    pub devices: Vec<StopDeviceResult>,
}

fn key(id: &str) -> String {
    format!("operation.stop.result:{id}")
}

#[tauri::command]
pub fn operation_stop_status(
    state: State<'_, AppState>,
    operation_id: String,
) -> Result<Option<OperationStopResult>, CommandError> {
    let result: Option<OperationStopResult> = state
        .db
        .get_setting(&key(&operation_id))
        .map_err(err)?
        .map(|raw| serde_json::from_str(&raw).map_err(err))
        .transpose()?;
    Ok(result.map(|mut result| {
        if result.state == "stopping" && !ACTIVE_STOPS.lock().contains(&operation_id) {
            result.state = "needsAttention".into();
            for d in &mut result.devices {
                if !d.closed {
                    d.message = "Lần dừng trước bị gián đoạn; bấm Dừng để kiểm tra lại".into();
                }
            }
        }
        result
    }))
}

#[tauri::command]
pub async fn operation_stop(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    operation_id: String,
) -> Result<OperationStopResult, CommandError> {
    let admission = state.ensure_accepting_work()?;
    let detail = super::jobs::read_operation_run(&state, &operation_id)?
        .ok_or_else(|| err("Không tìm thấy tác vụ"))?;
    use riviu_core::OperationRunKind as Kind;
    let source = detail.summary.source_id.clone();
    let mut udids: Vec<_> = detail
        .items
        .iter()
        .filter_map(|item| item.udid.clone())
        .collect();
    udids.sort();
    udids.dedup();
    let result = OperationStopResult {
        operation_id: operation_id.clone(),
        state: "stopping".into(),
        devices: udids
            .iter()
            .map(|udid| StopDeviceResult {
                udid: udid.clone(),
                closed: false,
                message: "Đang dừng và chờ nhả máy".into(),
            })
            .collect(),
    };
    // Claim before spawning. Repeated clicks share the current stop request.
    if let Some(raw) = state.db.get_setting(&key(&operation_id)).map_err(err)? {
        let current: OperationStopResult = serde_json::from_str(&raw).map_err(err)?;
        if (current.state == "stopping" && ACTIVE_STOPS.lock().contains(&operation_id))
            || current.state == "closed"
        {
            return Ok(current);
        }
    }
    {
        let mut active = ACTIVE_STOPS.lock();
        if active.contains(&operation_id) {
            return Ok(result);
        }
        state
            .db
            .set_setting(
                &key(&operation_id),
                &serde_json::to_string(&result).map_err(err)?,
            )
            .map_err(err)?;
        active.insert(operation_id.clone());
    }
    let guard = StopGuard(operation_id.clone());
    let initial = result.clone();
    tauri::async_runtime::spawn(async move {
        let _guard = guard;
        let _admission = admission;
        let state = app.state::<AppState>();
        let mut result = result;
        let cancelled: Result<(), CommandError> = async {
            match detail.summary.kind {
                Kind::Nurture => {
                    // Stop only the exact run captured by the monitor, never a newer session.
                    for status in state
                        .nurture
                        .list_status()
                        .into_iter()
                        .filter(|s| riviu_core::nurture_source_id(s) == source)
                    {
                        state.nurture.stop(&status.udid);
                    }
                }
                Kind::Publish => {
                    state.db.stop_publish_operation(&source).map_err(err)?;
                }
                Kind::Interaction => {
                    state.db.cancel_interaction_campaign(&source).map_err(err)?;
                }
                Kind::Script => {
                    state
                        .jobs
                        .cancel(uuid::Uuid::parse_str(&source).map_err(err)?);
                }
                Kind::Flow => {
                    state
                        .flows
                        .cancel_run(uuid::Uuid::parse_str(&source).map_err(err)?)
                        .map_err(err)?;
                }
                Kind::Orchestration => {
                    crate::orchestration_commands::orchestration_cancel_run(
                        app.clone(),
                        app.state(),
                        source.clone(),
                    )
                    .await?;
                }
                Kind::AppInstall | Kind::MaterialTransfer => {
                    crate::farm_commands::operation_cancel_batch(
                        app.state(),
                        operation_id.clone(),
                    )?;
                }
            }
            Ok(())
        }
        .await;
        if let Err(error) = cancelled {
            result.state = "failed".into();
            for device in &mut result.devices {
                device.message = error.message.to_string();
            }
        } else {
            let futures = udids
                .iter()
                .map(|udid| close_stopped_device(&state, &operation_id, udid));
            result.devices = futures_util::future::join_all(futures).await;
            result.state = if result.devices.iter().all(|d| d.closed) {
                "closed"
            } else {
                "needsAttention"
            }
            .into();
        }
        if let Ok(raw) = serde_json::to_string(&result) {
            let _ = state.db.set_setting(&key(&operation_id), &raw);
        }
    });
    Ok(initial)
}

async fn close_stopped_device(
    state: &AppState,
    operation_id: &str,
    udid: &str,
) -> StopDeviceResult {
    let control = &state.control;
    let db = &state.db;
    let result: Result<(), String> = async {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            match control
                .try_acquire_exclusive_keeping_stream(udid, DeviceWorkOwner::ManualControl)
                .await
            {
                Ok(context) => {
                    let closed: Result<(), String> = async {
                        // A queued or newly started operation after cancellation still owns
                        // its target scope even during a gap between device leases.
                        for id in db
                            .operation_source_ids(None, None)
                            .map_err(|e| e.to_string())?
                        {
                            if id == operation_id {
                                continue;
                            }
                            if let Some(other) = super::jobs::read_operation_run(state, &id)
                                .map_err(|e| e.message.to_string())?
                            {
                                if !other.summary.state.is_terminal()
                                    && other.items.iter().any(|item| {
                                        item.udid.as_deref() == Some(udid)
                                            && !item.state.is_terminal()
                                    })
                                {
                                    return Err(format!(
                                        "Máy còn tác vụ khác: {}",
                                        other.summary.title
                                    ));
                                }
                            }
                        }
                        // Other campaigns' unresolved uploads are never part of this cancellation.
                        let guard = db.publish_device_guard(udid).map_err(|e| e.to_string())?;
                        let own = operation_id.strip_prefix("publish:");
                        if guard
                            .blocking
                            .iter()
                            .any(|hold| Some(hold.campaign_id.as_str()) != own)
                        {
                            return Err("Máy còn bài đăng của tác vụ khác; chưa đóng TikTok".into());
                        }
                        let package = control
                            .resolve_tiktok_package(udid)
                            .await
                            .map_err(|e| e.to_string())?;
                        control
                            .terminate_app(&context, &package)
                            .await
                            .map_err(|e| e.to_string())?;
                        if control.reports_element_bounds(udid) {
                            let home = control
                                .device_shell(&context, "input keyevent KEYCODE_HOME")
                                .await
                                .map_err(|e| e.to_string())?;
                            if home.exit_code != 0 {
                                return Err("TikTok đã tắt nhưng chưa về màn hình chính".into());
                            }
                        }
                        Ok(())
                    }
                    .await;
                    let released = control
                        .close_exclusive_context(context)
                        .map_err(|e| e.to_string());
                    closed?;
                    released?;
                    return Ok(());
                }
                Err(riviu_core::DeviceControlError::Busy(_))
                    if std::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await
                }
                Err(error) => return Err(error.to_string()),
            }
        }
    }
    .await;
    StopDeviceResult {
        udid: udid.into(),
        closed: result.is_ok(),
        message: result
            .err()
            .unwrap_or_else(|| "Đã dừng; TikTok đã tắt".into()),
    }
}
