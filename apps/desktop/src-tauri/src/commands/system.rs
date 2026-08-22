//! Things about this installation rather than about a phone: the Apple ID, the WDA
//! signing identity, which driver mode came up, and the updater.

use super::*;

#[tauri::command]
pub fn get_apple_id(state: State<'_, AppState>) -> Result<riviu_core::AppleIdConfig, CommandError> {
    state.signing.apple_id_config().map_err(err)
}

#[tauri::command]
pub fn set_apple_id(
    state: State<'_, AppState>,
    email: String,
    password: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state.signing.save_apple_id(&email, &password).map_err(err)
}

#[tauri::command]
pub fn clear_apple_id(state: State<'_, AppState>) -> Result<(), CommandError> {
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
) -> Result<UpdateStatus, CommandError> {
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
) -> Result<(), CommandError> {
    use tauri_plugin_updater::UpdaterExt;

    if let Some(reason) = state.busy_reason() {
        return Err(reason.into());
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
        Ok(Err(error)) => Err(err(format!(
            "cài bản mới thất bại sau khi đã dừng phiên — mở lại app: {error}"
        ))),
        Err(_) => Err(err("luồng cài đặt dừng bất thường — mở lại app")),
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
