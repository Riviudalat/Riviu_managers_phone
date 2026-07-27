use std::path::PathBuf;

use riviu_core::{AutomationScript, DeviceInfo, JobRecord, StreamSettings, SwipeGesture, TapPoint};
use riviu_script_engine::{example_script_json, parse_script};
use tauri::State;

use crate::state::AppState;

#[tauri::command]
pub async fn list_devices(state: State<'_, AppState>) -> Result<Vec<DeviceInfo>, String> {
    Ok(state.registry.list())
}

#[tauri::command]
pub async fn refresh_devices(state: State<'_, AppState>) -> Result<Vec<DeviceInfo>, String> {
    let devices = state.driver.list_devices().await.map_err(err)?;
    for d in &devices {
        let _ = state.driver.ensure_stream(&d.udid).await;
    }
    let devices = state.driver.list_devices().await.map_err(err)?;
    state.registry.upsert_many(devices.clone());
    Ok(devices)
}

#[tauri::command]
pub async fn prepare_device(state: State<'_, AppState>, udid: String) -> Result<DeviceInfo, String> {
    state
        .registry
        .set_status(&udid, riviu_core::DeviceStatus::Preparing, None);
    state.driver.prepare_device(&udid).await.map_err(err)?;
    let url = state.driver.ensure_stream(&udid).await.map_err(err)?;
    let mut device = state
        .driver
        .refresh_device(&udid)
        .await
        .map_err(err)
        .or_else(|_| {
            state
                .registry
                .get(&udid)
                .ok_or_else(|| "device missing".to_string())
        })?;
    device.status = riviu_core::DeviceStatus::Ready;
    device.wda_ready = true;
    device.stream_url = Some(url);
    state.registry.upsert(device.clone());
    Ok(device)
}

#[tauri::command]
pub async fn install_ipa(
    state: State<'_, AppState>,
    udid: String,
    path: String,
) -> Result<(), String> {
    state
        .driver
        .install_app(&udid, &PathBuf::from(path))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn uninstall_app(
    state: State<'_, AppState>,
    udid: String,
    bundle_id: String,
) -> Result<(), String> {
    state
        .driver
        .uninstall_app(&udid, &bundle_id)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn screenshot(
    state: State<'_, AppState>,
    udid: String,
) -> Result<String, String> {
    let dest = state
        .artifacts_dir
        .join("screenshots")
        .join(format!("{udid}-{}.jpg", chrono::Utc::now().timestamp_millis()));
    let path = state.driver.screenshot(&udid, &dest).await.map_err(err)?;
    Ok(path.display().to_string())
}

#[tauri::command]
pub async fn syslog(
    state: State<'_, AppState>,
    udid: String,
    lines: Option<usize>,
) -> Result<String, String> {
    state
        .driver
        .syslog_tail(&udid, lines.unwrap_or(100))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn reboot_device(state: State<'_, AppState>, udid: String) -> Result<(), String> {
    state.driver.reboot(&udid).await.map_err(err)
}

#[tauri::command]
pub async fn device_tap(
    state: State<'_, AppState>,
    udid: String,
    x: f64,
    y: f64,
    image_w: Option<f64>,
    image_h: Option<f64>,
) -> Result<(), String> {
    let session = state.driver.start_ui_session(&udid).await.map_err(err)?;
    match (image_w, image_h) {
        (Some(w), Some(h)) if w > 0.0 && h > 0.0 => {
            session.tap_image(x, y, w, h).await.map_err(err)
        }
        _ => session.tap(TapPoint { x, y }).await.map_err(err),
    }
}

#[tauri::command]
pub async fn device_swipe(
    state: State<'_, AppState>,
    udid: String,
    gesture: SwipeGesture,
    image_w: Option<f64>,
    image_h: Option<f64>,
) -> Result<(), String> {
    let session = state.driver.start_ui_session(&udid).await.map_err(err)?;
    match (image_w, image_h) {
        (Some(w), Some(h)) if w > 0.0 && h > 0.0 => session
            .swipe_image(gesture.from, gesture.to, w, h, gesture.duration_ms)
            .await
            .map_err(err),
        _ => session.swipe(gesture).await.map_err(err),
    }
}

#[tauri::command]
pub async fn device_type_text(
    state: State<'_, AppState>,
    udid: String,
    text: String,
) -> Result<(), String> {
    let session = state.driver.start_ui_session(&udid).await.map_err(err)?;
    session.type_text(&text).await.map_err(err)
}

#[tauri::command]
pub async fn device_home(state: State<'_, AppState>, udid: String) -> Result<(), String> {
    let session = state.driver.start_ui_session(&udid).await.map_err(err)?;
    session.home().await.map_err(err)
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
) -> Result<(), String> {
    let scale = matches!((image_w, image_h), (Some(w), Some(h)) if w > 0.0 && h > 0.0);
    for udid in udids {
        let session = state.driver.start_ui_session(&udid).await.map_err(err)?;
        match kind.as_str() {
            "tap" => {
                let x = x.unwrap_or(0.0);
                let y = y.unwrap_or(0.0);
                if scale {
                    session
                        .tap_image(x, y, image_w.unwrap(), image_h.unwrap())
                        .await
                        .map_err(err)?;
                } else {
                    session.tap(TapPoint { x, y }).await.map_err(err)?;
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
                        .map_err(err)?;
                } else {
                    session
                        .swipe(SwipeGesture {
                            from,
                            to,
                            duration_ms: 300,
                        })
                        .await
                        .map_err(err)?;
                }
            }
            "type" => {
                session
                    .type_text(text.as_deref().unwrap_or(""))
                    .await
                    .map_err(err)?;
            }
            "home" => {
                session.home().await.map_err(err)?;
            }
            _ => return Err(format!("unknown group input kind: {kind}")),
        }
    }
    Ok(())
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
    Ok(state
        .streams
        .latest(&udid)
        .map(|bytes| base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes.as_slice())))
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
pub fn get_apple_id(state: State<'_, AppState>) -> riviu_core::AppleIdConfig {
    state.signing.apple_id_config()
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
pub async fn resign_wda(state: State<'_, AppState>, udid: String) -> Result<String, String> {
    state
        .registry
        .set_status(&udid, riviu_core::DeviceStatus::Preparing, None);
    match state
        .signing
        .sign_and_install_wda(&udid, &state.wda_bundle)
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
            let _ = state.driver.ensure_stream(&udid).await;
            Ok(result.message)
        }
        Err(err) => {
            let msg = err.to_string();
            state.registry.set_status(
                &udid,
                riviu_core::DeviceStatus::Error,
                Some(msg.clone()),
            );
            Err(msg)
        }
    }
}

#[tauri::command]
pub async fn bulk_resign_wda(
    state: State<'_, AppState>,
    udids: Vec<String>,
) -> Result<Vec<String>, String> {
    let results = state
        .signing
        .bulk_resign(&udids, &state.wda_bundle)
        .await
        .map_err(err)?;
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
