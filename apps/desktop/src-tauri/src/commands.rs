use std::path::PathBuf;

use riviu_core::{
    AutomationScript, DeviceControlPlane, DeviceExclusiveContext, DeviceInfo, DeviceWorkOwner,
    InteractionSessionKind, JobRecord, StreamSettings, SwipeGesture, TapPoint, UiWithStreamContext,
};
use riviu_script_engine::{example_script_json, parse_script};
use serde::Serialize;
use tauri::State;

use crate::command_error::CommandError;
use crate::state::AppState;

const TIKTOK_BUNDLE_ID: &str = "com.ss.iphone.ugc.Ame";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupInputReport {
    pub completed_udids: Vec<String>,
    pub skipped: Vec<GroupInputSkip>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupInputSkip {
    pub udid: String,
    pub code: String,
    pub current_owner: Option<DeviceWorkOwner>,
}

async fn open_ui_context(
    control: &DeviceControlPlane,
    udid: &str,
    owner: DeviceWorkOwner,
    kind: InteractionSessionKind,
) -> Result<UiWithStreamContext, CommandError> {
    let exclusive = control
        .try_acquire_exclusive(udid, owner)
        .await
        .map_err(CommandError::from)?;
    continue_ui_context(control, exclusive, kind).await
}

async fn continue_ui_context(
    control: &DeviceControlPlane,
    exclusive: DeviceExclusiveContext,
    kind: InteractionSessionKind,
) -> Result<UiWithStreamContext, CommandError> {
    let (exclusive, capacity) = control
        .reserve_ui_capacity(exclusive)
        .await
        .map_err(CommandError::from)?;
    let session = control
        .start_interaction_session(exclusive, TIKTOK_BUNDLE_ID, kind)
        .await
        .map_err(CommandError::from)?;
    control
        .start_reserved_stream(session, capacity)
        .await
        .map_err(CommandError::from)
}

async fn prepare_ui_with_control(
    control: &DeviceControlPlane,
    udid: &str,
) -> Result<(), CommandError> {
    let exclusive = control
        .try_acquire_exclusive(udid, DeviceWorkOwner::ManualControl)
        .await
        .map_err(CommandError::from)?;
    control
        .repair_agent_install_only(&exclusive)
        .await
        .map_err(CommandError::from)?;
    let context = continue_ui_context(control, exclusive, InteractionSessionKind::Ordinary).await?;
    control
        .close_ui_context(context)
        .await
        .map_err(CommandError::from)?;
    Ok(())
}

#[tauri::command]
pub async fn list_devices(state: State<'_, AppState>) -> Result<Vec<DeviceInfo>, String> {
    Ok(state.registry.list())
}

#[tauri::command]
pub async fn refresh_devices(state: State<'_, AppState>) -> Result<Vec<DeviceInfo>, CommandError> {
    let devices = state
        .control
        .list_devices()
        .await
        .map_err(CommandError::from)?;
    state.registry.upsert_many(devices.clone());
    Ok(devices)
}

#[tauri::command]
pub async fn prepare_device(
    state: State<'_, AppState>,
    udid: String,
) -> Result<DeviceInfo, CommandError> {
    state
        .registry
        .set_status(&udid, riviu_core::DeviceStatus::Preparing, None);
    prepare_ui_with_control(&state.control, &udid).await?;
    let mut device = state
        .control
        .refresh_device(&udid)
        .await
        .map_err(CommandError::from)
        .or_else(|_| {
            state
                .registry
                .get(&udid)
                .ok_or_else(|| CommandError::operation("device missing"))
        })?;
    device.status = riviu_core::DeviceStatus::Ready;
    device.wda_ready = true;
    device.stream_url = None;
    device.tile_stream_state = riviu_core::TileStreamState::Parked;
    state.registry.upsert(device.clone());
    Ok(device)
}

#[tauri::command]
pub async fn install_ipa(
    state: State<'_, AppState>,
    udid: String,
    path: String,
) -> Result<(), CommandError> {
    let context = state
        .control
        .try_acquire_exclusive(&udid, DeviceWorkOwner::Repair)
        .await
        .map_err(CommandError::from)?;
    state
        .control
        .install_app(&context, &PathBuf::from(path))
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn uninstall_app(
    state: State<'_, AppState>,
    udid: String,
    bundle_id: String,
) -> Result<(), CommandError> {
    let context = state
        .control
        .try_acquire_exclusive(&udid, DeviceWorkOwner::Repair)
        .await
        .map_err(CommandError::from)?;
    state
        .control
        .uninstall_app(&context, &bundle_id)
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn screenshot(state: State<'_, AppState>, udid: String) -> Result<String, CommandError> {
    let dest = state.artifacts_dir.join("screenshots").join(format!(
        "{udid}-{}.jpg",
        chrono::Utc::now().timestamp_millis()
    ));
    let context = state
        .control
        .try_acquire_exclusive(&udid, DeviceWorkOwner::ManualControl)
        .await
        .map_err(CommandError::from)?;
    let path = state
        .control
        .screenshot(&context, &dest)
        .await
        .map_err(CommandError::from)?;
    Ok(path.display().to_string())
}

#[tauri::command]
pub async fn syslog(
    state: State<'_, AppState>,
    udid: String,
    lines: Option<usize>,
) -> Result<String, CommandError> {
    let context = state
        .control
        .try_acquire_exclusive(&udid, DeviceWorkOwner::ManualControl)
        .await
        .map_err(CommandError::from)?;
    state
        .control
        .syslog_tail(&context, lines.unwrap_or(100))
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn reboot_device(state: State<'_, AppState>, udid: String) -> Result<(), CommandError> {
    let context = state
        .control
        .try_acquire_exclusive(&udid, DeviceWorkOwner::Repair)
        .await
        .map_err(CommandError::from)?;
    state
        .control
        .reboot(&context)
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn device_tap(
    state: State<'_, AppState>,
    udid: String,
    x: f64,
    y: f64,
    image_w: Option<f64>,
    image_h: Option<f64>,
) -> Result<(), CommandError> {
    let context = open_ui_context(
        &state.control,
        &udid,
        DeviceWorkOwner::ManualControl,
        InteractionSessionKind::Ordinary,
    )
    .await?;
    let session = state
        .control
        .streaming_session(&context)
        .map_err(CommandError::from)?;
    let result = match (image_w, image_h) {
        (Some(w), Some(h)) if w > 0.0 && h > 0.0 => session.tap_image(x, y, w, h).await,
        _ => session.tap(TapPoint { x, y }).await,
    };
    let cleanup = state.control.close_ui_context(context).await;
    result.map_err(CommandError::operation)?;
    cleanup.map_err(CommandError::from)?;
    Ok(())
}

#[tauri::command]
pub async fn device_swipe(
    state: State<'_, AppState>,
    udid: String,
    gesture: SwipeGesture,
    image_w: Option<f64>,
    image_h: Option<f64>,
) -> Result<(), CommandError> {
    let context = open_ui_context(
        &state.control,
        &udid,
        DeviceWorkOwner::ManualControl,
        InteractionSessionKind::Ordinary,
    )
    .await?;
    let session = state
        .control
        .streaming_session(&context)
        .map_err(CommandError::from)?;
    let result = match (image_w, image_h) {
        (Some(w), Some(h)) if w > 0.0 && h > 0.0 => {
            session
                .swipe_image(gesture.from, gesture.to, w, h, gesture.duration_ms)
                .await
        }
        _ => session.swipe(gesture).await,
    };
    let cleanup = state.control.close_ui_context(context).await;
    result.map_err(CommandError::operation)?;
    cleanup.map_err(CommandError::from)?;
    Ok(())
}

#[tauri::command]
pub async fn device_type_text(
    state: State<'_, AppState>,
    udid: String,
    text: String,
) -> Result<(), CommandError> {
    let kind = if state.control.requires_fresh_text_session() {
        InteractionSessionKind::FreshText
    } else {
        InteractionSessionKind::Ordinary
    };
    let context =
        open_ui_context(&state.control, &udid, DeviceWorkOwner::ManualControl, kind).await?;
    let session = state
        .control
        .streaming_session(&context)
        .map_err(CommandError::from)?;
    let result = session.type_text(&text).await;
    let cleanup = state.control.close_ui_context(context).await;
    result.map_err(CommandError::operation)?;
    cleanup.map_err(CommandError::from)?;
    Ok(())
}

#[tauri::command]
pub async fn device_home(state: State<'_, AppState>, udid: String) -> Result<(), CommandError> {
    let context = open_ui_context(
        &state.control,
        &udid,
        DeviceWorkOwner::ManualControl,
        InteractionSessionKind::Ordinary,
    )
    .await?;
    let session = state
        .control
        .streaming_session(&context)
        .map_err(CommandError::from)?;
    let result = session.home().await;
    let cleanup = state.control.close_ui_context(context).await;
    result.map_err(CommandError::operation)?;
    cleanup.map_err(CommandError::from)?;
    Ok(())
}

#[tauri::command]
pub async fn group_input(
    state: State<'_, AppState>,
    udids: Vec<String>,
    kind: String,
    x: Option<f64>,
    y: Option<f64>,
    to_x: Option<f64>,
    to_y: Option<f64>,
    text: Option<String>,
    image_w: Option<f64>,
    image_h: Option<f64>,
) -> Result<GroupInputReport, CommandError> {
    if !matches!(kind.as_str(), "tap" | "swipe" | "type" | "home") {
        return Err(CommandError::operation(format!(
            "unknown group input kind: {kind}"
        )));
    }
    let scale = matches!((image_w, image_h), (Some(w), Some(h)) if w > 0.0 && h > 0.0);
    let mut report = GroupInputReport {
        completed_udids: Vec::new(),
        skipped: Vec::new(),
    };
    for udid in udids {
        let session_kind = if kind == "type" && state.control.requires_fresh_text_session() {
            InteractionSessionKind::FreshText
        } else {
            InteractionSessionKind::Ordinary
        };
        let context = match open_ui_context(
            &state.control,
            &udid,
            DeviceWorkOwner::GroupSync,
            session_kind,
        )
        .await
        {
            Ok(context) => context,
            Err(error) if error.code == "DeviceBusy" => {
                report.skipped.push(GroupInputSkip {
                    udid,
                    code: error.code,
                    current_owner: error.current_owner,
                });
                continue;
            }
            Err(error) => return Err(error),
        };
        let session = state
            .control
            .streaming_session(&context)
            .map_err(CommandError::from)?;
        let action = match kind.as_str() {
            "tap" => {
                let x = x.unwrap_or(0.0);
                let y = y.unwrap_or(0.0);
                if scale {
                    session
                        .tap_image(x, y, image_w.unwrap(), image_h.unwrap())
                        .await
                } else {
                    session.tap(TapPoint { x, y }).await
                }
            }
            "swipe" => {
                let from = TapPoint {
                    x: x.unwrap_or(0.0),
                    y: y.unwrap_or(0.0),
                };
                let to = TapPoint {
                    x: to_x.unwrap_or(0.0),
                    y: to_y.unwrap_or(0.0),
                };
                if scale {
                    session
                        .swipe_image(from, to, image_w.unwrap(), image_h.unwrap(), 300)
                        .await
                } else {
                    session
                        .swipe(SwipeGesture {
                            from,
                            to,
                            duration_ms: 300,
                        })
                        .await
                }
            }
            "type" => session.type_text(text.as_deref().unwrap_or("")).await,
            "home" => session.home().await,
            _ => unreachable!("group input kind was validated"),
        };
        let cleanup = state.control.close_ui_context(context).await;
        action.map_err(CommandError::operation)?;
        cleanup.map_err(CommandError::from)?;
        report.completed_udids.push(udid);
    }
    Ok(report)
}

#[tauri::command]
pub fn get_stream_settings(state: State<'_, AppState>) -> StreamSettings {
    state.stream_settings.read().clone()
}

#[tauri::command]
pub fn set_stream_settings(state: State<'_, AppState>, settings: StreamSettings) -> StreamSettings {
    let mut s = settings;
    s.fps = riviu_core::STREAM_FPS;
    *state.stream_settings.write() = s.clone();
    s
}

#[tauri::command]
pub fn latest_frame(state: State<'_, AppState>, udid: String) -> Result<Option<String>, String> {
    Ok(state.streams.latest(&udid).map(|bytes| {
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes.as_slice())
    }))
}

#[tauri::command]
pub fn list_jobs(state: State<'_, AppState>) -> Result<Vec<JobRecord>, String> {
    state.jobs.list_jobs(100).map_err(err)
}

#[tauri::command]
pub async fn run_script(
    state: State<'_, AppState>,
    script_json: String,
    udids: Vec<String>,
) -> Result<JobRecord, String> {
    let script: AutomationScript = parse_script(&script_json).map_err(err)?;
    state.jobs.enqueue(script, udids).await.map_err(err)
}

#[tauri::command]
pub fn cancel_job(state: State<'_, AppState>, job_id: String) -> Result<(), String> {
    let id = uuid::Uuid::parse_str(&job_id).map_err(err)?;
    state.jobs.cancel(id);
    Ok(())
}

#[tauri::command]
pub fn list_scripts(state: State<'_, AppState>) -> Result<Vec<(String, String)>, String> {
    state.db.list_scripts().map_err(err)
}

#[tauri::command]
pub fn save_script(
    state: State<'_, AppState>,
    name: String,
    body_json: String,
) -> Result<(), String> {
    parse_script(&body_json).map_err(err)?;
    state.db.save_script(&name, &body_json).map_err(err)
}

#[tauri::command]
pub fn example_script() -> String {
    example_script_json().to_string()
}

#[tauri::command]
pub fn get_apple_id(state: State<'_, AppState>) -> Result<riviu_core::AppleIdConfig, String> {
    state.signing.apple_id_config().map_err(err)
}

#[tauri::command]
pub fn set_apple_id(
    state: State<'_, AppState>,
    email: String,
    password: String,
) -> Result<(), String> {
    state.signing.save_apple_id(&email, &password).map_err(err)
}

#[tauri::command]
pub fn clear_apple_id(state: State<'_, AppState>) -> Result<(), String> {
    state.signing.clear_apple_id().map_err(err)
}

#[tauri::command]
pub async fn resign_wda(state: State<'_, AppState>, udid: String) -> Result<String, CommandError> {
    // Legacy stock tooling only; this agent does not provide trusted TikTok text input.
    state
        .registry
        .set_status(&udid, riviu_core::DeviceStatus::Preparing, None);
    let _context = state
        .control
        .try_acquire_exclusive(&udid, DeviceWorkOwner::Repair)
        .await
        .map_err(CommandError::from)?;
    match state
        .signing
        .sign_and_install_wda(&udid, &state.legacy_wda_bundle)
        .await
    {
        Ok(result) => {
            if let Some(mut device) = state.registry.get(&udid) {
                device.wda_ready = true;
                device.wda_expires_at = Some(result.expires_at);
                device.status = riviu_core::DeviceStatus::Ready;
                device.last_error = None;
                state.registry.upsert(device);
            }
            Ok(result.message)
        }
        Err(err) => {
            let msg = err.to_string();
            state
                .registry
                .set_status(&udid, riviu_core::DeviceStatus::Error, Some(msg.clone()));
            Err(CommandError::operation(msg))
        }
    }
}

#[tauri::command]
pub async fn bulk_resign_wda(
    state: State<'_, AppState>,
    udids: Vec<String>,
) -> Result<Vec<String>, CommandError> {
    let mut contexts = Vec::with_capacity(udids.len());
    for udid in &udids {
        contexts.push(
            state
                .control
                .try_acquire_exclusive(udid, DeviceWorkOwner::Repair)
                .await
                .map_err(CommandError::from)?,
        );
    }
    let results = state
        .signing
        .bulk_resign(&udids, &state.legacy_wda_bundle)
        .await
        .map_err(CommandError::operation)?;
    let mut messages = Vec::new();
    for result in results {
        if let Some(mut device) = state.registry.get(&result.udid) {
            device.wda_ready = true;
            device.wda_expires_at = Some(result.expires_at);
            device.status = riviu_core::DeviceStatus::Ready;
            device.last_error = None;
            state.registry.upsert(device);
        }
        messages.push(result.message);
    }
    Ok(messages)
}

#[tauri::command]
pub fn driver_mode(state: State<'_, AppState>) -> String {
    match state.driver_mode {
        riviu_ios_driver::DriverMode::Mock => "mock".into(),
        riviu_ios_driver::DriverMode::Pymobiledevice3 => "pymobiledevice3".into(),
    }
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use riviu_core::{DeviceWorkCoordinator, StreamBudgetManager};
    use riviu_ios_driver::MockIosDriver;

    #[tokio::test]
    async fn shared_device_owner_group_sync_reports_interaction_as_skipped() {
        let driver = MockIosDriver::new();
        let control = DeviceControlPlane::new(
            Arc::new(driver.clone()),
            Arc::new(DeviceWorkCoordinator::new()),
            Arc::new(StreamBudgetManager::default()),
        );
        let _interaction = control
            .try_acquire_exclusive("fixture", DeviceWorkOwner::Interaction)
            .await
            .expect("interaction lease");

        let error = match open_ui_context(
            &control,
            "fixture",
            DeviceWorkOwner::GroupSync,
            InteractionSessionKind::Ordinary,
        )
        .await
        {
            Ok(_) => panic!("group sync must skip an interaction-owned device"),
            Err(error) => error,
        };

        assert_eq!(error.code, "DeviceBusy");
        assert_eq!(error.current_owner, Some(DeviceWorkOwner::Interaction));
        assert_eq!(driver.ordinary_session_calls(), 0);
    }

    #[tokio::test]
    async fn prepare_stops_at_install_only_failure_before_session_or_stream() {
        let driver = MockIosDriver::new();
        driver.set_mock_repair_failure("MOCK-IPHONE-01", true);
        let control = DeviceControlPlane::new(
            Arc::new(driver.clone()),
            Arc::new(DeviceWorkCoordinator::new()),
            Arc::new(StreamBudgetManager::default()),
        );

        let error = prepare_ui_with_control(&control, "MOCK-IPHONE-01")
            .await
            .expect_err("install-only failure must abort device preparation");

        assert!(error.message.contains("install-only auth failed"));
        assert_eq!(driver.ordinary_session_calls(), 0);
        assert_eq!(driver.fresh_text_session_calls(), 0);
        assert_eq!(driver.stream_restart_calls(), 0);
    }
}
