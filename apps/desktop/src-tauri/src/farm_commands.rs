use std::path::PathBuf;

use chrono::{Duration, Utc};
use riviu_core::{
    AnalyticsSummary, AppLibraryItem, AuthSession, DeviceGroup, DeviceMeta, DeviceWorkOwner,
    LocalUser, MaterialItem, OpLog, ProxyConfig, PublishTask, ScheduleItem,
};
use riviu_script_engine::parse_script;
use tauri::State;
use uuid::Uuid;

use crate::state::AppState;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn log(state: &AppState, action: &str, detail: &str) {
    let _ = state.db.log_op(action, detail);
}

#[tauri::command]
pub fn auth_session(state: State<'_, AppState>) -> Result<AuthSession, String> {
    let _admission = state.ensure_accepting_work()?;
    let show = std::env::var("RIVIU_SHOW_AUTH")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let user = state.db.guest_user().map_err(err)?;
    Ok(AuthSession {
        show_auth_ui: show,
        bypassed: !show,
        user: Some(user),
    })
}

#[tauri::command]
pub fn auth_login(
    state: State<'_, AppState>,
    email: String,
    password: String,
) -> Result<LocalUser, String> {
    let _admission = state.ensure_accepting_work()?;
    let user = state
        .db
        .login_user(&email, &password)
        .map_err(err)?
        .ok_or_else(|| "Sai email hoặc mật khẩu".to_string())?;
    log(&state, "auth.login", &email);
    Ok(user)
}

#[tauri::command]
pub fn auth_register(
    state: State<'_, AppState>,
    email: String,
    password: String,
) -> Result<LocalUser, String> {
    let _admission = state.ensure_accepting_work()?;
    let user = state
        .db
        .register_user(&email, &password, "operator")
        .map_err(err)?;
    log(&state, "auth.register", &email);
    Ok(user)
}

#[tauri::command]
pub fn get_device_meta(state: State<'_, AppState>, udid: String) -> Result<DeviceMeta, String> {
    state.db.get_device_meta(&udid).map_err(err)
}

#[tauri::command]
pub fn save_device_meta(state: State<'_, AppState>, meta: DeviceMeta) -> Result<(), String> {
    let _admission = state.ensure_accepting_work()?;
    state.db.upsert_device_meta(&meta).map_err(err)?;
    log(&state, "device.meta", &meta.udid);
    Ok(())
}

#[tauri::command]
pub fn list_groups(state: State<'_, AppState>) -> Result<Vec<DeviceGroup>, String> {
    state.db.list_groups().map_err(err)
}

#[tauri::command]
pub fn save_group(state: State<'_, AppState>, group: DeviceGroup) -> Result<DeviceGroup, String> {
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
pub fn delete_group(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let _admission = state.ensure_accepting_work()?;
    state.db.delete_group(&id).map_err(err)?;
    log(&state, "group.delete", &id);
    Ok(())
}

#[tauri::command]
pub fn list_proxies(state: State<'_, AppState>) -> Result<Vec<ProxyConfig>, String> {
    state.db.list_proxies().map_err(err)
}

#[tauri::command]
pub fn save_proxy(state: State<'_, AppState>, proxy: ProxyConfig) -> Result<ProxyConfig, String> {
    let _admission = state.ensure_accepting_work()?;
    let mut p = proxy;
    if p.id.is_empty() {
        p.id = Uuid::new_v4().to_string();
    }
    state.db.upsert_proxy(&p).map_err(err)?;
    log(&state, "proxy.save", &p.name);
    Ok(p)
}

#[tauri::command]
pub fn delete_proxy(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let _admission = state.ensure_accepting_work()?;
    state.db.delete_proxy(&id).map_err(err)?;
    log(&state, "proxy.delete", &id);
    Ok(())
}

#[tauri::command]
pub fn export_proxy_config(state: State<'_, AppState>, id: String) -> Result<String, String> {
    let proxies = state.db.list_proxies().map_err(err)?;
    let p = proxies
        .into_iter()
        .find(|x| x.id == id)
        .ok_or_else(|| "proxy not found".to_string())?;
    let text = format!(
        "type={}\nhost={}\nport={}\nusername={}\npassword={}\n# Apply manually on device Wi‑Fi / VPN\n",
        p.proxy_type, p.host, p.port, p.username, p.password
    );
    Ok(text)
}

#[tauri::command]
pub fn list_materials(state: State<'_, AppState>) -> Result<Vec<MaterialItem>, String> {
    state.db.list_materials().map_err(err)
}

#[tauri::command]
pub fn add_material(
    state: State<'_, AppState>,
    source_path: String,
    name: Option<String>,
) -> Result<MaterialItem, String> {
    let _admission = state.ensure_accepting_work()?;
    let src = PathBuf::from(&source_path);
    if !src.is_file() {
        return Err(format!("file not found: {source_path}"));
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
pub fn delete_material(state: State<'_, AppState>, id: String) -> Result<(), String> {
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
) -> Result<String, String> {
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
pub fn list_apps_library(state: State<'_, AppState>) -> Result<Vec<AppLibraryItem>, String> {
    state.db.list_apps_library().map_err(err)
}

#[tauri::command]
pub fn add_app_library(
    state: State<'_, AppState>,
    source_path: String,
    name: Option<String>,
    bundle_id: Option<String>,
    version: Option<String>,
) -> Result<AppLibraryItem, String> {
    let _admission = state.ensure_accepting_work()?;
    let src = PathBuf::from(&source_path);
    if !src.is_file() {
        return Err(format!("IPA not found: {source_path}"));
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
pub fn delete_app_library(state: State<'_, AppState>, id: String) -> Result<(), String> {
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
) -> Result<(), String> {
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
pub fn list_schedules(state: State<'_, AppState>) -> Result<Vec<ScheduleItem>, String> {
    state.db.list_schedules().map_err(err)
}

#[tauri::command]
pub fn save_schedule(
    state: State<'_, AppState>,
    schedule: ScheduleItem,
) -> Result<ScheduleItem, String> {
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
pub fn delete_schedule(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let _admission = state.ensure_accepting_work()?;
    state.db.delete_schedule(&id).map_err(err)?;
    log(&state, "schedule.delete", &id);
    Ok(())
}

#[tauri::command]
pub fn list_publish_tasks(state: State<'_, AppState>) -> Result<Vec<PublishTask>, String> {
    state.db.list_publish_tasks().map_err(err)
}

#[tauri::command]
pub async fn create_publish_task(
    state: State<'_, AppState>,
    name: String,
    script_name: String,
    material_ids: Vec<String>,
    udids: Vec<String>,
) -> Result<PublishTask, String> {
    let _admission = state.ensure_accepting_work()?;
    let task = PublishTask {
        id: Uuid::new_v4().to_string(),
        name,
        script_name: script_name.clone(),
        material_ids,
        udids: udids.clone(),
        status: "queued".into(),
        created_at: Utc::now().to_rfc3339(),
    };
    state.db.add_publish_task(&task).map_err(err)?;
    if let Some(body) = state.db.get_script(&script_name).map_err(err)? {
        let script = parse_script(&body).map_err(err)?;
        let _ = state.jobs.enqueue(script, udids).await.map_err(err)?;
        state
            .db
            .update_publish_status(&task.id, "running")
            .map_err(err)?;
    } else {
        state
            .db
            .update_publish_status(&task.id, "missing_script")
            .map_err(err)?;
    }
    log(&state, "publish.create", &task.name);
    Ok(task)
}

#[tauri::command]
pub fn list_op_logs(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<OpLog>, String> {
    state.db.list_op_logs(limit.unwrap_or(100)).map_err(err)
}

#[tauri::command]
pub fn list_users(state: State<'_, AppState>) -> Result<Vec<LocalUser>, String> {
    state.db.list_users().map_err(err)
}

#[tauri::command]
pub fn analytics_summary(state: State<'_, AppState>) -> Result<AnalyticsSummary, String> {
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
- list_proxies / save_proxy / delete_proxy / export_proxy_config
- list_materials / add_material / delete_material / push_material
- list_apps_library / add_app_library / delete_app_library / install_library_app / uninstall_app
- list_schedules / save_schedule / delete_schedule
- publish_scan_folder / publish_create_campaign / publish_list / publish_get
- publish_prepare / publish_transfer / publish_post / publish_cancel
- list_publish_tasks / create_publish_task (legacy script compatibility)
- list_op_logs / analytics_summary / list_users

## Auth (hidden by default)
- auth_session / auth_login / auth_register
- Set RIVIU_SHOW_AUTH=1 to show login UI

## Sidecar
- python riviu_pmd.py list|install|uninstall|media-stage|stream|start-wda|...
"#
    .into()
}
