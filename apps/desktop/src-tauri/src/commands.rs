use std::path::PathBuf;
use std::sync::Arc;

use riviu_core::{
    AutomationScript, DeviceControlPlane, DeviceExclusiveContext, DeviceInfo, DeviceWorkOwner,
    HardwareKey, InteractionSessionKind, JobRecord, StreamSettings, SwipeGesture, TapPoint,
    UiSession, UiWithStreamContext,
};
use riviu_script_engine::{example_script_json, parse_script};
use serde::Serialize;
use tauri::State;

use crate::command_error::CommandError;
use crate::state::AppState;

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

async fn continue_ui_context(
    control: &DeviceControlPlane,
    exclusive: DeviceExclusiveContext,
    kind: InteractionSessionKind,
) -> Result<UiWithStreamContext, CommandError> {
    let (exclusive, capacity) = control
        .reserve_ui_capacity(exclusive)
        .await
        .map_err(CommandError::from)?;
    // Per device, not a module constant. Prepare/Open-on-Device used to hand the
    // *iOS* bundle to every backend, so on Android `start_interaction_session`
    // foregrounded nothing that exists and the foreground proof could never pass.
    // Manual tap/swipe/type/home/key do not use this helper — they go through
    // `open_manual_session` so they do not park the live preview.
    let udid = exclusive.udid().to_string();
    let target_package = control
        .resolve_tiktok_package(&udid)
        .await
        .map_err(CommandError::from)?;
    let session = control
        .start_interaction_session(exclusive, &target_package, kind)
        .await
        .map_err(CommandError::from)?;
    control
        .start_reserved_stream(session, capacity)
        .await
        .map_err(CommandError::from)
}

async fn with_manual_session<F, Fut>(
    state: &AppState,
    udid: &str,
    owner: DeviceWorkOwner,
    f: F,
) -> Result<(), CommandError>
where
    F: FnOnce(Arc<dyn UiSession>) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    if let Some(session) = state.overlay_ui_session(udid).await {
        return f(session).await.map_err(CommandError::operation);
    }
    let context = state
        .control
        .open_manual_session(udid, owner)
        .await
        .map_err(CommandError::from)?;
    let session = state
        .control
        .session(&context)
        .map_err(CommandError::from)?;
    let result = f(session).await;
    let cleanup = state.control.close_manual_session(context);
    result.map_err(CommandError::operation)?;
    cleanup.map_err(CommandError::from)?;
    Ok(())
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
    let _admission = state.ensure_accepting_work()?;
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
    let _admission = state.ensure_accepting_work()?;
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
    let _admission = state.ensure_accepting_work()?;
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupInstallResult {
    pub udid: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Install one signed IPA onto every member of a device group.
///
/// Devices are processed one at a time: each takes its own exclusive Repair
/// lease, installs, then releases before the next. A single device's failure is
/// recorded and the batch continues, so one bad device never aborts the fleet.
#[tauri::command]
pub async fn install_ipa_to_group(
    state: State<'_, AppState>,
    group_id: String,
    path: String,
) -> Result<Vec<GroupInstallResult>, CommandError> {
    let _admission = state.ensure_accepting_work()?;

    let ipa = PathBuf::from(&path);
    if !ipa.is_file() {
        return Err(CommandError::invalid_argument(format!(
            "IPA not found at {path}"
        )));
    }

    let group = state
        .db
        .list_groups()
        .map_err(CommandError::operation)?
        .into_iter()
        .find(|group| group.id == group_id)
        .ok_or_else(|| {
            CommandError::code("GroupNotFound", format!("group {group_id} not found"))
        })?;

    if group.udids.is_empty() {
        return Err(CommandError::invalid_argument("group has no devices"));
    }

    let mut results = Vec::with_capacity(group.udids.len());
    for udid in group.udids {
        let outcome = async {
            let context = state
                .control
                .try_acquire_exclusive(&udid, DeviceWorkOwner::Repair)
                .await?;
            state.control.install_app(&context, &ipa).await
        }
        .await;
        results.push(match outcome {
            Ok(()) => GroupInstallResult {
                udid,
                ok: true,
                error: None,
            },
            Err(error) => GroupInstallResult {
                udid,
                ok: false,
                error: Some(error.to_string()),
            },
        });
    }

    Ok(results)
}

/// Spike gate for restore-based unsigned installs (TrollRestore). Returns false
/// until the hardware feasibility pass in `docs/re/unsigned-install-spike.md`
/// completes. Deliberately a plain fn, not a `const` — a `const false` would make
/// the safety gates below unreachable dead code; keeping them reachable documents
/// the intended order and lets tests exercise them.
fn unsigned_install_enabled() -> bool {
    false
}

/// SPIKE ONLY — restore-based install of an unsigned IPA.
///
/// The destructive restore path is intentionally **not wired**. This command
/// exists to encode the safety gates (capability off by default, backup-first,
/// isolation from the production agent) and always refuses before touching a
/// device. See `docs/re/unsigned-install-spike.md`.
#[tauri::command]
pub async fn install_unsigned_ipa(
    state: State<'_, AppState>,
    udid: String,
    path: String,
    backup_dir: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;

    // Gate 1 — capability disabled by default.
    if !unsigned_install_enabled() {
        return Err(CommandError::code(
            "UnsignedInstallDisabled",
            "restore-based unsigned install is disabled; see docs/re/unsigned-install-spike.md",
        ));
    }

    // Gate 2 — the IPA must exist.
    if !std::path::Path::new(&path).is_file() {
        return Err(CommandError::invalid_argument(format!(
            "IPA not found at {path}"
        )));
    }

    // Gate 3 — backup-first: a prior backup must exist as a rollback path.
    if !std::path::Path::new(&backup_dir).is_dir() {
        return Err(CommandError::invalid_argument(
            "a device backup is required before a restore-based install (see backup_device)",
        ));
    }

    // Take a real exclusive lease so the intent is genuine, then refuse: the
    // destructive restore path is not wired in the spike phase.
    let _context = state
        .control
        .try_acquire_exclusive(&udid, DeviceWorkOwner::Repair)
        .await
        .map_err(CommandError::from)?;

    Err(CommandError::code(
        "UnsignedInstallSpike",
        "unsigned install execution is not wired in the spike phase; no device action taken",
    ))
}

#[tauri::command]
pub async fn uninstall_app(
    state: State<'_, AppState>,
    udid: String,
    bundle_id: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
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
    let _admission = state.ensure_accepting_work()?;
    let dest = state.artifacts_dir.join("screenshots").join(format!(
        "{udid}-{}.jpg",
        chrono::Utc::now().timestamp_millis()
    ));
    if let Some(bytes) = state.streams.latest(&udid) {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(CommandError::operation)?;
        }
        std::fs::write(&dest, bytes.as_slice()).map_err(CommandError::operation)?;
        return Ok(dest.display().to_string());
    }
    let context = state
        .control
        .try_acquire_exclusive_keeping_stream(&udid, DeviceWorkOwner::ManualControl)
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
    let _admission = state.ensure_accepting_work()?;
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
    let _admission = state.ensure_accepting_work()?;
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
pub async fn backup_device(
    state: State<'_, AppState>,
    udid: String,
    dest: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let context = state
        .control
        .try_acquire_exclusive(&udid, DeviceWorkOwner::Repair)
        .await
        .map_err(CommandError::from)?;
    state
        .control
        .backup_device(&context, std::path::Path::new(&dest))
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn restore_device(
    state: State<'_, AppState>,
    udid: String,
    src: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let context = state
        .control
        .try_acquire_exclusive(&udid, DeviceWorkOwner::Repair)
        .await
        .map_err(CommandError::from)?;
    state
        .control
        .restore_device(&context, std::path::Path::new(&src))
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
    let _admission = state.ensure_accepting_work()?;
    with_manual_session(
        &state,
        &udid,
        DeviceWorkOwner::ManualControl,
        |session| async move {
            match (image_w, image_h) {
                (Some(w), Some(h)) if w > 0.0 && h > 0.0 => session.tap_image(x, y, w, h).await,
                _ => session.tap(TapPoint { x, y }).await,
            }
        },
    )
    .await
}

#[tauri::command]
pub async fn device_swipe(
    state: State<'_, AppState>,
    udid: String,
    gesture: SwipeGesture,
    image_w: Option<f64>,
    image_h: Option<f64>,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    with_manual_session(
        &state,
        &udid,
        DeviceWorkOwner::ManualControl,
        |session| async move {
            match (image_w, image_h) {
                (Some(w), Some(h)) if w > 0.0 && h > 0.0 => {
                    session
                        .swipe_image(gesture.from, gesture.to, w, h, gesture.duration_ms)
                        .await
                }
                _ => session.swipe(gesture).await,
            }
        },
    )
    .await
}

#[tauri::command]
pub async fn device_type_text(
    state: State<'_, AppState>,
    udid: String,
    text: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    with_manual_session(
        &state,
        &udid,
        DeviceWorkOwner::ManualControl,
        |session| async move { session.type_text(&text).await },
    )
    .await
}

#[tauri::command]
pub async fn device_home(state: State<'_, AppState>, udid: String) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    with_manual_session(
        &state,
        &udid,
        DeviceWorkOwner::ManualControl,
        |session| async move { session.home().await },
    )
    .await
}

#[tauri::command]
pub async fn device_key(
    state: State<'_, AppState>,
    udid: String,
    key: HardwareKey,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    with_manual_session(
        &state,
        &udid,
        DeviceWorkOwner::ManualControl,
        move |session| async move { session.press_hardware_key(key).await },
    )
    .await
}

#[tauri::command]
pub async fn device_control_begin(
    state: State<'_, AppState>,
    udid: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state.begin_overlay_session(&udid).await
}

#[tauri::command]
pub async fn device_control_end(
    state: State<'_, AppState>,
    udid: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state.end_overlay_session(&udid).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
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
    key: Option<HardwareKey>,
) -> Result<GroupInputReport, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    if !matches!(kind.as_str(), "tap" | "swipe" | "type" | "home" | "key") {
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
        let overlay_session = state.overlay_ui_session(&udid).await;
        let owned = if overlay_session.is_none() {
            match state
                .control
                .open_manual_session(&udid, DeviceWorkOwner::GroupSync)
                .await
            {
                Ok(context) => Some(context),
                Err(error) => {
                    let error = CommandError::from(error);
                    if error.code == "DeviceBusy" {
                        report.skipped.push(GroupInputSkip {
                            udid,
                            code: error.code,
                            current_owner: error.current_owner,
                        });
                        continue;
                    }
                    return Err(error);
                }
            }
        } else {
            None
        };
        let session = match overlay_session {
            Some(session) => session,
            None => state
                .control
                .session(
                    owned
                        .as_ref()
                        .expect("group input opened a session when no overlay is held"),
                )
                .map_err(CommandError::from)?,
        };
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
            "key" => match key {
                Some(key) => session.press_hardware_key(key).await,
                None => {
                    if let Some(context) = owned {
                        state
                            .control
                            .close_manual_session(context)
                            .map_err(CommandError::from)?;
                    }
                    return Err(CommandError::operation(
                        "group input kind key requires a hardware key",
                    ));
                }
            },
            _ => unreachable!("group input kind was validated"),
        };
        if let Some(context) = owned {
            let cleanup = state.control.close_manual_session(context);
            action.map_err(CommandError::operation)?;
            cleanup.map_err(CommandError::from)?;
        } else {
            action.map_err(CommandError::operation)?;
        }
        report.completed_udids.push(udid);
    }
    Ok(report)
}

#[tauri::command]
pub fn get_stream_settings(state: State<'_, AppState>) -> StreamSettings {
    state.stream_settings.read().clone()
}

#[tauri::command]
pub fn set_stream_settings(
    state: State<'_, AppState>,
    settings: StreamSettings,
) -> Result<StreamSettings, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let mut s = settings;
    s.fps = riviu_core::STREAM_FPS;
    *state.stream_settings.write() = s.clone();
    Ok(s)
}

#[tauri::command]
pub fn latest_frame(state: State<'_, AppState>, udid: String) -> Result<Option<String>, String> {
    Ok(state.streams.latest(&udid).map(|bytes| {
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes.as_slice())
    }))
}

#[tauri::command]
pub fn view_endpoint(state: State<'_, AppState>) -> Result<Option<String>, String> {
    Ok(state.view_hub.endpoint())
}

#[tauri::command]
pub async fn view_ensure(state: State<'_, AppState>, udid: String) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let Some(android) = &state.android else {
        return Ok(());
    };
    let platform = state
        .registry
        .get(&udid)
        .map(|device| device.platform)
        .unwrap_or(riviu_core::DevicePlatform::Ios);
    if platform != riviu_core::DevicePlatform::Android {
        return Ok(());
    }
    android.stop_view_stream(&udid).await;
    android
        .start_view_stream(&udid, riviu_android_driver::ViewPreset::Tile)
        .await
        .map_err(CommandError::operation)?;
    crate::state::mark_android_view_live(&state.registry, &udid);
    Ok(())
}

#[tauri::command]
pub async fn view_set_preset(
    state: State<'_, AppState>,
    udid: String,
    preset: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let Some(android) = &state.android else {
        return Ok(());
    };
    let platform = state
        .registry
        .get(&udid)
        .map(|device| device.platform)
        .unwrap_or(riviu_core::DevicePlatform::Ios);
    if platform != riviu_core::DevicePlatform::Android {
        return Ok(());
    }
    let preset = match preset.as_str() {
        "overlay" => riviu_android_driver::ViewPreset::Overlay,
        _ => riviu_android_driver::ViewPreset::Tile,
    };
    android
        .set_view_preset(&udid, preset)
        .await
        .map_err(CommandError::operation)
        .map(|_| ())
}

#[tauri::command]
pub fn save_view_snapshot(
    state: State<'_, AppState>,
    udid: String,
    jpeg: Vec<u8>,
) -> Result<String, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    if jpeg.len() < 3 || jpeg[0] != 0xff || jpeg[1] != 0xd8 {
        return Err(CommandError::operation("view snapshot is not a JPEG"));
    }
    let dest = state.artifacts_dir.join("screenshots").join(format!(
        "{udid}-{}.jpg",
        chrono::Utc::now().timestamp_millis()
    ));
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(CommandError::operation)?;
    }
    std::fs::write(&dest, jpeg).map_err(CommandError::operation)?;
    Ok(dest.display().to_string())
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
    let _admission = state.ensure_accepting_work()?;
    let script: AutomationScript = parse_script(&script_json).map_err(err)?;
    state.jobs.enqueue(script, udids).await.map_err(err)
}

#[tauri::command]
pub fn cancel_job(state: State<'_, AppState>, job_id: String) -> Result<(), String> {
    let _admission = state.ensure_accepting_work()?;
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
    let _admission = state.ensure_accepting_work()?;
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
    let _admission = state.ensure_accepting_work()?;
    state.signing.save_apple_id(&email, &password).map_err(err)
}

#[tauri::command]
pub fn clear_apple_id(state: State<'_, AppState>) -> Result<(), String> {
    let _admission = state.ensure_accepting_work()?;
    state.signing.clear_apple_id().map_err(err)
}

#[tauri::command]
pub async fn resign_wda(state: State<'_, AppState>, udid: String) -> Result<String, CommandError> {
    let _admission = state.ensure_accepting_work()?;
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
    let _admission = state.ensure_accepting_work()?;
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

/// Why real devices cannot be listed, or `None` when the sidecar is healthy.
/// Read-only, so it needs no admission gate.
///
/// Two sources, boot snapshot first. The snapshot is the more specific fact when it
/// exists — the sidecar never started, so there is nothing to list from. The live listing
/// error is what covers everything after boot: a sidecar that started fine and then had
/// `list` fail, or answered with `{"devices": [], "error": "pymobiledevice3 not installed"}`
/// at exit code 0. That second case used to be dropped entirely, so the fleet went empty
/// and the UI said nothing (AGENTS.md 9.29).
///
/// It is asked, not stored, so it clears itself: a listing that succeeds sets it back to
/// `None` and an operator who fixes the machine stops being told it is broken without
/// restarting the app.
#[tauri::command]
pub fn driver_degraded_reason(state: State<'_, AppState>) -> Option<String> {
    if let Some(reason) = state.driver_degraded_reason.clone() {
        return Some(reason);
    }
    state.driver_list_error.as_ref().and_then(|probe| probe())
}

/// Why the Android half of the fleet is absent, or `None` when it joined.
///
/// Separate from [`driver_degraded_reason`] on purpose: "this machine has no
/// adb" and "the iOS sidecar failed" are different facts with different fixes,
/// and collapsing them into one string would send an operator looking in the
/// wrong place.
#[tauri::command]
pub fn android_unavailable_reason(state: State<'_, AppState>) -> Option<String> {
    state.android_unavailable_reason.clone()
}

/// Whether a newer release is published, and whether now is a safe moment to take it.
///
/// Two answers in one call, and deliberately so. An updater that reports "an update is
/// available" without saying "you have sixteen phones mid-session" invites an operator to
/// take it at the worst possible moment: installing replaces the running binary, and this app
/// holds WDA relays, XCTest runners and device leases that only its own shutdown releases.
///
/// **Never installs.** It reports, and the operator decides. Nor does it run at startup — a
/// farm machine is frequently offline and nobody asked it to phone home, so the network call
/// happens when somebody asks for it and not before.
///
/// Read-only with respect to devices, so no admission gate — but it *reads* the admission
/// state, which is the whole point.
#[tauri::command]
pub async fn update_check(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<UpdateStatus, String> {
    use tauri_plugin_updater::UpdaterExt;

    // Asked before the network call, so a busy fleet is reported even if GitHub is
    // unreachable: "do not update now" is the more urgent half of the answer.
    let busy = state.busy_reason();

    let update = app
        .updater()
        .map_err(|error| format!("không dựng được updater: {error}"))?
        .check()
        .await
        .map_err(|error| format!("không kiểm được bản mới: {error}"))?;

    Ok(match update {
        Some(update) => UpdateStatus {
            available: true,
            version: Some(update.version.clone()),
            current: update.current_version.clone(),
            busy_reason: busy,
        },
        None => UpdateStatus {
            available: false,
            version: None,
            current: app.package_info().version.to_string(),
            busy_reason: busy,
        },
    })
}

/// Download the published update and hand over to its installer.
///
/// Only ever reached by an explicit press, and it re-asks [`AppState::busy_reason`] rather
/// than trusting what the UI was showing: the check and the press are separated by however
/// long the operator took to read it, and a nurture session can start in between.
///
/// **The order is the whole point.** Download first, because a failed download must leave
/// the fleet exactly as it was. Only once the bytes are in hand does the app let go of the
/// phones, and only then does the installer run — on Windows `install` calls
/// `std::process::exit` itself, which skips `RunEvent::Exit` entirely. Waiting for the normal
/// exit path to release anything would leak a WDA relay, an XCTest runner and an adb
/// forward per device, and on a sixteen-phone farm that is sixteen of each.
///
/// **No admission gate, and not merely because it is read-only.** Holding a
/// [`CommandAdmission`](crate::state::AppState::ensure_accepting_work) would deadlock:
/// `graceful_shutdown` waits for in-flight mutating commands to drain, and this command
/// would be one of them, waiting for itself.
#[tauri::command]
pub async fn update_install(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;

    if let Some(reason) = state.busy_reason() {
        return Err(reason);
    }

    let update = app
        .updater()
        .map_err(|error| format!("không dựng được updater: {error}"))?
        .check()
        .await
        .map_err(|error| format!("không kiểm được bản mới: {error}"))?
        .ok_or_else(|| "không có bản mới để cài".to_string())?;

    let bytes = update
        .download(|_, _| {}, || {})
        .await
        .map_err(|error| format!("không tải được bản mới: {error}"))?;

    // A plain OS thread, not `spawn_blocking`: `graceful_shutdown` blocks on futures
    // through the global runtime, and doing that from inside a runtime-managed thread is
    // the one way to turn an orderly shutdown into a panic during shutdown.
    let handle = app.clone();
    let worker = std::thread::Builder::new()
        .name("riviu-updater-install".into())
        .spawn(move || {
            crate::graceful_shutdown(&handle);
            update.install(bytes)
        })
        .map_err(|error| format!("không tạo được luồng cài đặt: {error}"))?;

    // On Windows this join never returns — the installer exits the process from under it,
    // which is the success path. Elsewhere the archive is unpacked in place and the
    // operator reopens the app, which the fleet is already released for.
    match worker.join() {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(format!(
            "cài bản mới thất bại sau khi đã dừng phiên — mở lại app: {error}"
        )),
        Err(_) => Err("luồng cài đặt dừng bất thường — mở lại app".to_string()),
    }
}

/// What [`update_check`] found, and whether acting on it is safe right now.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub available: bool,
    /// The published version, when one is newer than this build.
    pub version: Option<String>,
    pub current: String,
    /// Why installing now would interrupt work, or `None` when the fleet is idle.
    ///
    /// The frontend must refuse to offer the install while this is `Some`. Carried as a
    /// sentence rather than a bool so the operator is told *what* is running.
    pub busy_reason: Option<String>,
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

        let error = match control
            .open_manual_session("fixture", DeviceWorkOwner::GroupSync)
            .await
        {
            Ok(_) => panic!("group sync must skip an interaction-owned device"),
            Err(error) => CommandError::from(error),
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
