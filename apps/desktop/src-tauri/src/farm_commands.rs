use crate::command_error::CommandError;
use std::path::PathBuf;

use chrono::{Duration, Utc};
use riviu_core::{
    AnalyticsSummary, AppLibraryItem, DeviceGroup, DeviceMeta, DeviceWorkOwner, MaterialItem,
    OpLog, ScheduleItem,
};
use tauri::State;
use uuid::Uuid;

use crate::state::AppState;

fn err(e: impl std::fmt::Display) -> CommandError {
    CommandError::operation(e)
}

fn log(state: &AppState, action: &str, detail: &str) {
    let _ = state.db.log_op(action, detail);
}

#[tauri::command]
pub fn get_device_meta(
    state: State<'_, AppState>,
    udid: String,
) -> Result<DeviceMeta, CommandError> {
    state.db.get_device_meta(&udid).map_err(err)
}

/// Every phone this app has a record for, in one call.
///
/// The grid reads it per refresh to label and order twenty tiles (alias, number). Per-device
/// reads would be twenty IPC round trips to draw one frame, and `get_device_meta` stays for
/// the one-phone editors that already use it.
#[tauri::command]
pub fn list_device_metas(state: State<'_, AppState>) -> Result<Vec<DeviceMeta>, CommandError> {
    state.db.list_device_metas().map_err(err)
}

#[tauri::command]
pub fn save_device_meta(state: State<'_, AppState>, meta: DeviceMeta) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state.db.upsert_device_meta(&meta).map_err(err)?;
    log(&state, "device.meta", &meta.udid);
    Ok(())
}

#[tauri::command]
pub fn list_groups(state: State<'_, AppState>) -> Result<Vec<DeviceGroup>, CommandError> {
    state.db.list_groups().map_err(err)
}

#[tauri::command]
pub fn save_group(
    state: State<'_, AppState>,
    group: DeviceGroup,
) -> Result<DeviceGroup, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let mut g = group;
    if g.id.is_empty() {
        g.id = Uuid::new_v4().to_string();
        g.created_at = Utc::now().to_rfc3339();
    }
    if g.color.is_empty() {
        g.color = "#FF6A00".into();
    }
    state.db.upsert_group(&g).map_err(err)?;
    log(&state, "group.save", &g.name);
    Ok(g)
}

#[tauri::command]
pub fn delete_group(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state.db.delete_group(&id).map_err(err)?;
    log(&state, "group.delete", &id);
    Ok(())
}

#[tauri::command]
pub fn list_materials(state: State<'_, AppState>) -> Result<Vec<MaterialItem>, CommandError> {
    state.db.list_materials().map_err(err)
}

#[tauri::command]
pub fn add_material(
    state: State<'_, AppState>,
    source_path: String,
    name: Option<String>,
) -> Result<MaterialItem, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let src = PathBuf::from(&source_path);
    if !src.is_file() {
        return Err(err(format!("file not found: {source_path}")));
    }
    let file_name = name.unwrap_or_else(|| {
        src.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "material.bin".into())
    });
    let dest_dir = state.artifacts_dir.join("materials");
    std::fs::create_dir_all(&dest_dir).map_err(err)?;
    let id = Uuid::new_v4().to_string();
    let dest = dest_dir.join(format!("{id}-{file_name}"));
    std::fs::copy(&src, &dest).map_err(err)?;
    let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    let kind = match src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" => "image",
        "mp4" | "mov" | "m4v" => "video",
        _ => "file",
    }
    .to_string();
    let item = MaterialItem {
        id,
        name: file_name,
        path: dest.display().to_string(),
        kind,
        size,
        created_at: Utc::now().to_rfc3339(),
    };
    state.db.add_material(&item).map_err(err)?;
    log(&state, "material.add", &item.name);
    Ok(item)
}

#[tauri::command]
pub fn delete_material(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    if let Some(item) = state
        .db
        .list_materials()
        .map_err(err)?
        .into_iter()
        .find(|m| m.id == id)
    {
        let _ = std::fs::remove_file(&item.path);
    }
    state.db.delete_material(&id).map_err(err)?;
    log(&state, "material.delete", &id);
    Ok(())
}

#[tauri::command]
pub async fn push_material(
    state: State<'_, AppState>,
    udid: String,
    material_id: String,
) -> Result<String, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let item = state
        .db
        .list_materials()
        .map_err(err)?
        .into_iter()
        .find(|m| m.id == material_id)
        .ok_or_else(|| "material not found".to_string())?;
    // Media never goes through installd. Stage it as a one-file campaign and
    // let the driver perform HouseArrest/AFC size+hash readback.
    let context = state
        .control
        .try_acquire_exclusive(&udid, DeviceWorkOwner::Script)
        .await
        .map_err(err)?;
    let staged = state
        .artifacts_dir
        .join("push-staging")
        .join(&udid)
        .join(&material_id)
        .join("material");
    std::fs::create_dir_all(&staged).map_err(err)?;
    let dest = staged.join(&item.name);
    std::fs::copy(&item.path, &dest).map_err(err)?;
    let campaign_root = staged
        .parent()
        .ok_or_else(|| "material staging root missing".to_string())?;
    let evidence = state
        .control
        .stage_publish_media(
            &context,
            &state.active_agent_bundle_id,
            &material_id,
            campaign_root,
        )
        .await
        .map_err(err)?;
    let msg = format!(
        "Transferred {} to Agent sandbox on {udid}; readback={}",
        item.name, evidence
    );
    log(&state, "material.push", &format!("{udid}:{material_id}"));
    Ok(msg)
}

#[tauri::command]
pub fn list_apps_library(state: State<'_, AppState>) -> Result<Vec<AppLibraryItem>, CommandError> {
    state.db.list_apps_library().map_err(err)
}

#[tauri::command]
pub fn add_app_library(
    state: State<'_, AppState>,
    source_path: String,
    name: Option<String>,
    bundle_id: Option<String>,
    version: Option<String>,
) -> Result<AppLibraryItem, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let src = PathBuf::from(&source_path);
    if !src.is_file() {
        return Err(err(format!("IPA not found: {source_path}")));
    }
    let file_name = name.unwrap_or_else(|| {
        src.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "app.ipa".into())
    });
    let dest_dir = state.artifacts_dir.join("apps");
    std::fs::create_dir_all(&dest_dir).map_err(err)?;
    let id = Uuid::new_v4().to_string();
    let dest = dest_dir.join(format!("{id}-{file_name}"));
    std::fs::copy(&src, &dest).map_err(err)?;
    let item = AppLibraryItem {
        id,
        name: file_name,
        path: dest.display().to_string(),
        bundle_id: bundle_id.unwrap_or_default(),
        version: version.unwrap_or_default(),
        created_at: Utc::now().to_rfc3339(),
    };
    state.db.add_app_library(&item).map_err(err)?;
    log(&state, "app.add", &item.name);
    Ok(item)
}

#[tauri::command]
pub fn delete_app_library(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    if let Some(item) = state
        .db
        .list_apps_library()
        .map_err(err)?
        .into_iter()
        .find(|a| a.id == id)
    {
        let _ = std::fs::remove_file(&item.path);
    }
    state.db.delete_app_library(&id).map_err(err)?;
    log(&state, "app.delete", &id);
    Ok(())
}

#[tauri::command]
pub async fn install_library_app(
    state: State<'_, AppState>,
    udid: String,
    app_id: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let item = state
        .db
        .list_apps_library()
        .map_err(err)?
        .into_iter()
        .find(|a| a.id == app_id)
        .ok_or_else(|| "app not found".to_string())?;
    let context = state
        .control
        .try_acquire_exclusive(&udid, DeviceWorkOwner::Repair)
        .await
        .map_err(err)?;
    state
        .control
        .install_app(&context, &PathBuf::from(&item.path))
        .await
        .map_err(err)?;
    log(&state, "app.install", &format!("{udid}:{}", item.name));
    Ok(())
}

#[tauri::command]
pub fn list_schedules(state: State<'_, AppState>) -> Result<Vec<ScheduleItem>, CommandError> {
    state.db.list_schedules().map_err(err)
}

#[tauri::command]
pub fn save_schedule(
    state: State<'_, AppState>,
    schedule: ScheduleItem,
) -> Result<ScheduleItem, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let mut s = schedule;
    if s.id.is_empty() {
        s.id = Uuid::new_v4().to_string();
    }
    if s.every_minutes == 0 {
        s.every_minutes = 60;
    }
    s.next_run_at = Some((Utc::now() + Duration::minutes(s.every_minutes as i64)).to_rfc3339());
    state.db.upsert_schedule(&s).map_err(err)?;
    log(&state, "schedule.save", &s.name);
    Ok(s)
}

#[tauri::command]
pub fn delete_schedule(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state.db.delete_schedule(&id).map_err(err)?;
    log(&state, "schedule.delete", &id);
    Ok(())
}

#[tauri::command]
pub fn list_op_logs(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<OpLog>, CommandError> {
    state.db.list_op_logs(limit.unwrap_or(100)).map_err(err)
}

#[tauri::command]
pub fn analytics_summary(state: State<'_, AppState>) -> Result<AnalyticsSummary, CommandError> {
    let devices = state.registry.list();
    let ready = devices
        .iter()
        .filter(|d| d.wda_ready || matches!(d.status, riviu_core::DeviceStatus::Ready))
        .count();
    state
        .db
        .analytics_summary(devices.len(), ready)
        .map_err(err)
}

#[tauri::command]
pub fn api_docs() -> String {
    r#"# Riviu Manager local API (Tauri invoke)

## Devices
- list_devices / refresh_devices / prepare_device / reboot_device
- device_tap / device_swipe / device_type_text / device_home / group_input
- resign_wda / bulk_resign_wda / screenshot / syslog

## Farm data
- list_groups / save_group / delete_group
- list_materials / add_material / delete_material / push_material
- list_apps_library / add_app_library / delete_app_library / install_library_app / uninstall_app
- list_schedules / save_schedule / delete_schedule
- publish_scan_folder / publish_create_campaign / publish_list / publish_get
- publish_prepare / publish_transfer / publish_post / publish_cancel
- list_op_logs / analytics_summary

## Sidecar
- python riviu_pmd.py list|install|uninstall|media-stage|stream|start-wda|...
"#
    .into()
}
