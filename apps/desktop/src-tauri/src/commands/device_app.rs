//! Persisted per-device social-app selection. Reads never acquire a UI session;
//! writes briefly own the device so a run cannot start between validation and CAS.

use super::*;
use riviu_core::ipc_contract::DeviceAppChoices;

const TIKTOK: &str = "tiktok";

fn require_supported_app_key(app_key: &str) -> Result<(), CommandError> {
    if app_key != TIKTOK {
        return Err(CommandError::invalid_argument(format!(
            "Ứng dụng xã hội chưa được hỗ trợ: {app_key}"
        )));
    }
    Ok(())
}

fn is_tiktok_package(platform: riviu_core::DevicePlatform, package: &str) -> bool {
    match platform {
        riviu_core::DevicePlatform::Android => {
            riviu_core::tiktok_target::is_measured_android_tiktok(package)
        }
        riviu_core::DevicePlatform::Ios => package == riviu_core::tiktok_target::IOS_TIKTOK_BUNDLE,
    }
}

async fn read_choices(
    state: &AppState,
    udid: &str,
    app_key: &str,
) -> Result<DeviceAppChoices, CommandError> {
    require_supported_app_key(app_key)?;
    let platform = state
        .registry
        .list()
        .into_iter()
        .find(|device| device.udid == udid)
        .map(|device| device.platform)
        .ok_or_else(|| CommandError::code("DeviceUnavailable", "Thiết bị không còn kết nối"))?;
    let mut installed_packages: Vec<String> = state
        .control
        .list_installed_apps(udid)
        .await
        .map_err(CommandError::from)?
        .into_iter()
        .map(|app| app.bundle_id)
        .filter(|package| is_tiktok_package(platform, package))
        .collect();
    installed_packages.sort();
    installed_packages.dedup();
    let binding_udid = udid.to_string();
    let binding_app_key = app_key.to_string();
    let binding = state
        .db
        .storage_read(move |db| db.device_app_binding(&binding_udid, &binding_app_key))
        .await
        .map_err(CommandError::operation)?;
    let revision = binding.as_ref().map_or(0, |row| row.revision);
    let stored = binding.map(|row| row.package);
    let selection_valid = stored
        .as_ref()
        .is_some_and(|package| installed_packages.contains(package));
    let selected_package = stored
        .clone()
        .or_else(|| (installed_packages.len() == 1).then(|| installed_packages[0].clone()));
    let suggested_package = if stored.is_none() && installed_packages.len() > 1 {
        state.control.resolve_tiktok_package(udid).await.ok()
    } else {
        None
    };
    let reason = if let Some(package) = stored.as_ref().filter(|_| !selection_valid) {
        Some(format!(
            "Gói đã chọn {package} không còn được cài trên thiết bị"
        ))
    } else if installed_packages.is_empty() {
        Some("Không có bản ứng dụng TikTok đã đo trên thiết bị".into())
    } else if stored.is_none() && installed_packages.len() > 1 {
        Some("Thiết bị có nhiều bản TikTok; hãy chọn ứng dụng cần dùng".into())
    } else {
        None
    };
    let selection_valid = selection_valid || stored.is_none() && installed_packages.len() == 1;
    Ok(DeviceAppChoices {
        udid: udid.into(),
        app_key: app_key.into(),
        installed_packages,
        selected_package,
        suggested_package,
        revision,
        selection_valid,
        reason,
    })
}

#[tauri::command]
pub async fn device_app_candidates(
    state: State<'_, AppState>,
    udid: String,
    app_key: String,
) -> Result<DeviceAppChoices, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    read_choices(&state, &udid, &app_key).await
}

#[tauri::command]
pub async fn device_app_select(
    state: State<'_, AppState>,
    udid: String,
    app_key: String,
    package: String,
    expected_revision: i64,
) -> Result<DeviceAppChoices, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    require_supported_app_key(&app_key)?;
    if let Some(owner) = state.control.current_work_owner(&udid) {
        return Err(CommandError::code(
            "DeviceAppSelectionBusy",
            format!("Thiết bị đang được {owner:?} giữ; chưa đổi ứng dụng"),
        ));
    }
    let guard_db = state.db.clone();
    let guard_udid = udid.clone();
    if let Some(reason) = guard_db
        .storage_read(move |db| db.device_app_selection_block_reason(&guard_udid))
        .await
        .map_err(CommandError::operation)?
    {
        return Err(CommandError::code("DeviceAppSelectionBusy", reason));
    }
    let context = state
        .control
        .try_acquire_exclusive_keeping_stream(&udid, DeviceWorkOwner::ManualControl)
        .await
        .map_err(CommandError::from)?;
    let selected = async {
        let guarded_db = state.db.clone();
        let guarded_udid = udid.clone();
        if let Some(reason) = guarded_db
            .storage_read(move |db| db.device_app_selection_block_reason(&guarded_udid))
            .await
            .map_err(CommandError::operation)?
        {
            return Err(CommandError::code("DeviceAppSelectionBusy", reason));
        }
        let choices = read_choices(&state, &udid, &app_key).await?;
        if !choices.installed_packages.contains(&package) {
            return Err(CommandError::code(
                "DeviceAppPackageUnavailable",
                format!("Gói {package} không được cài trên thiết bị {udid}"),
            ));
        }
        let db = state.db.clone();
        let selected_udid = udid.clone();
        let selected_app_key = app_key.clone();
        let selected_package = package.clone();
        db.storage_write(move |db| {
            db.select_device_app(
                &selected_udid,
                &selected_app_key,
                &selected_package,
                expected_revision,
            )
        })
        .await
        .map_err(|error| {
            if error
                .downcast_ref::<riviu_core::db::DeviceAppBindingConflict>()
                .is_some()
            {
                CommandError::code("DeviceAppBindingConflict", error.to_string())
            } else {
                CommandError::operation(error)
            }
        })?;
        read_choices(&state, &udid, &app_key).await
    }
    .await;
    let released = state
        .control
        .close_exclusive_context(context)
        .map_err(CommandError::from);
    let selected = selected?;
    released?;
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_are_scoped_to_the_devices_platform() {
        assert!(is_tiktok_package(
            riviu_core::DevicePlatform::Android,
            "com.zhiliaoapp.musically"
        ));
        assert!(!is_tiktok_package(
            riviu_core::DevicePlatform::Android,
            riviu_core::tiktok_target::IOS_TIKTOK_BUNDLE
        ));
        assert!(is_tiktok_package(
            riviu_core::DevicePlatform::Ios,
            riviu_core::tiktok_target::IOS_TIKTOK_BUNDLE
        ));
        assert!(!is_tiktok_package(
            riviu_core::DevicePlatform::Ios,
            "com.ss.android.ugc.trill"
        ));
    }
}
