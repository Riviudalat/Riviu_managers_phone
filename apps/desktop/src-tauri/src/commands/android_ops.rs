//! Commands that talk to the Android backend directly rather than through
//! `DeviceControlPlane`.
//!
//! Grouped because that is a real property of them and worth being able to see at a glance:
//! every one opens with `state.require_android()?`, and most are concepts that exist only on
//! Android — root, Magisk, `appops`, adb over Wi-Fi, `wm density`.

use super::*;

/// Put a USB Android phone into wireless adb and connect to it (A4). Returns `host:port`.
#[tauri::command]
pub async fn enable_wifi_adb(
    state: State<'_, AppState>,
    udid: String,
) -> Result<String, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let android = state.require_android()?;
    // On Android the udid is the adb serial.
    android
        .enable_wifi_adb(&udid)
        .await
        .map_err(|e| CommandError::operation(e.to_string()))
}

/// Put adbd back on USB, closing the `0.0.0.0:5555` port `enable_wifi_adb` opened (A4).
///
/// The counterpart that was missing: `wifi_adb_disconnect` only drops this host's client, so
/// before this the only way to close the port was to reboot the phone.
#[tauri::command]
pub async fn disable_wifi_adb(
    state: State<'_, AppState>,
    udid: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let android = state.require_android()?;
    android
        .disable_wifi_adb(&udid)
        .await
        .map_err(|e| CommandError::operation(e.to_string()))
}

/// `adb connect host:port` — manual wireless connect (A4).
#[tauri::command]
pub async fn wifi_adb_connect(
    state: State<'_, AppState>,
    host: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let android = state.require_android()?;
    android
        .wifi_connect(&host)
        .await
        .map_err(|e| CommandError::operation(e.to_string()))
}

/// `adb disconnect host:port` (A4).
#[tauri::command]
pub async fn wifi_adb_disconnect(
    state: State<'_, AppState>,
    host: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let android = state.require_android()?;
    android
        .wifi_disconnect(&host)
        .await
        .map_err(|e| CommandError::operation(e.to_string()))
}

/// Set an Android phone's wallpaper from a local image file (A3, "number as wallpaper").
#[tauri::command]
pub async fn set_wallpaper(
    state: State<'_, AppState>,
    udid: String,
    path: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let android = state.require_android()?;
    android
        .set_wallpaper(&udid, &path)
        .await
        .map_err(|e| CommandError::operation(e.to_string()))
}

/// Inject a mock GPS location on an Android phone (B, "虚拟定位").
#[tauri::command]
pub async fn set_mock_location(
    state: State<'_, AppState>,
    udid: String,
    lat: f64,
    lng: f64,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let android = state.require_android()?;
    android
        .set_mock_location(&udid, lat, lng)
        .await
        .map_err(|e| CommandError::operation(e.to_string()))
}

/// Stop mock location, returning the phone to real GPS (B).
#[tauri::command]
pub async fn stop_mock_location(
    state: State<'_, AppState>,
    udid: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let android = state.require_android()?;
    android
        .stop_mock_location(&udid)
        .await
        .map_err(|e| CommandError::operation(e.to_string()))
}

/// Set an Android wallpaper from PNG bytes the webview rendered (A3, "number as wallpaper").
/// The bytes are written to the app's own artifacts dir (always writable, unlike a
/// frontend temp path bound by the fs ACL) and handed to the driver to push + apply.
#[tauri::command]
pub async fn set_wallpaper_bytes(
    state: State<'_, AppState>,
    udid: String,
    png: Vec<u8>,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let android = state.require_android()?;
    let dir = state.artifacts_dir.join("wallpaper");
    std::fs::create_dir_all(&dir).map_err(CommandError::operation)?;
    let path = dir.join(format!("{}.png", safe_udid_stem(&udid)));
    std::fs::write(&path, &png).map_err(CommandError::operation)?;
    android
        .set_wallpaper(&udid, path.to_string_lossy().as_ref())
        .await
        .map_err(|e| CommandError::operation(e.to_string()))
}

/// Whether an Android phone is rooted (Magisk `su`), for gating the root-tier UI (feature C).
#[tauri::command]
pub async fn is_rooted(state: State<'_, AppState>, udid: String) -> Result<bool, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let Some(android) = &state.android else {
        return Ok(false);
    };
    Ok(android.is_rooted(&udid).await)
}

/// Overwrite the app-visible device fingerprint (feature C, xiaowei 一键新机). android_id
/// applies without root; serialno/mac need root. Returns a summary of what changed.
#[tauri::command]
pub async fn set_device_identity(
    state: State<'_, AppState>,
    udid: String,
    android_id: Option<String>,
    serialno: Option<String>,
    mac: Option<String>,
) -> Result<String, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let android = state.require_android()?;
    android
        .set_device_identity(
            &udid,
            android_id.as_deref(),
            serialno.as_deref(),
            mac.as_deref(),
        )
        .await
        .map_err(|e| CommandError::operation(e.to_string()))
}

/// Factory-reset a rooted Android phone (feature C). Irreversible; UI confirms first.
#[tauri::command]
pub async fn factory_reset(state: State<'_, AppState>, udid: String) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let android = state.require_android()?;
    android
        .factory_reset(&udid)
        .await
        .map_err(|e| CommandError::operation(e.to_string()))
}

/// Run one root shell command on an Android phone (feature C, advanced). Errors if not rooted.
#[tauri::command]
pub async fn root_shell(
    state: State<'_, AppState>,
    udid: String,
    command: String,
) -> Result<String, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let android = state.require_android()?;
    android
        .root_shell(&udid, &command)
        .await
        .map_err(|e| CommandError::operation(e.to_string()))
}

// --- The per-phone function menu (xiaowei 功能). One command per row, and each one is the
// whole row: the frontend never assembles a shell string, because a menu item that pastes
// `rm -rf` into a device shell from TypeScript is a menu item with no validator in front of
// it. Every Android call below lives in `AndroidDriver` where the path and package
// validators are. ---

/// Read one directory on the phone, for the file browser (xiaowei "Preview Mobile Files").
///
/// Lease-free, deliberately, and following `list_installed_apps`: it reads nothing but a
/// directory listing, and taking an exclusive lease to open a folder would let a browser
/// click evict a running nurture session.
#[tauri::command]
pub async fn device_list_dir(
    state: State<'_, AppState>,
    udid: String,
    path: String,
) -> Result<riviu_core::DeviceDirListing, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let android = state.require_android()?;
    android
        .list_device_dir(&udid, &path)
        .await
        .map_err(|e| CommandError::operation(e.to_string()))
}

/// Copy one file or folder from the phone to this machine (xiaowei "Export File").
/// Returns the local path it landed at.
#[tauri::command]
pub async fn device_pull_path(
    state: State<'_, AppState>,
    udid: String,
    remote: String,
    dest_dir: String,
) -> Result<String, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let android = state.require_android()?;
    let dest = PathBuf::from(&dest_dir);
    if !dest.is_dir() {
        return Err(CommandError::invalid_argument(format!(
            "không thấy thư mục {dest_dir}"
        )));
    }
    android
        .pull_device_path(&udid, &remote, &dest)
        .await
        .map(|path| path.display().to_string())
        .map_err(|e| CommandError::operation(e.to_string()))
}

/// Put one local file onto the phone (xiaowei "Import File"). Returns the device path.
#[tauri::command]
pub async fn device_push_file(
    state: State<'_, AppState>,
    udid: String,
    local: String,
    remote_dir: String,
) -> Result<String, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let android = state.require_android()?;
    android
        .push_device_file(&udid, Path::new(&local), &remote_dir)
        .await
        .map_err(|e| CommandError::operation(e.to_string()))
}

/// Delete a file or folder on the phone. The driver refuses storage roots; the UI confirms.
#[tauri::command]
pub async fn device_delete_path(
    state: State<'_, AppState>,
    udid: String,
    path: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let android = state.require_android()?;
    android
        .delete_device_path(&udid, &path)
        .await
        .map_err(|e| CommandError::operation(e.to_string()))
}

/// Turn the phone's Wi-Fi radio on or off, returning the state it settled at (xiaowei ADB
/// submenu). Note this is the *phone's* Wi-Fi, not this app's wireless-adb link — a phone
/// reached over Wi-Fi disconnects itself by obeying, which the UI warns about first.
#[tauri::command]
pub async fn set_wifi_radio(
    state: State<'_, AppState>,
    udid: String,
    on: bool,
) -> Result<bool, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let android = state.require_android()?;
    android
        .set_wifi_radio(&udid, on)
        .await
        .map_err(|e| CommandError::operation(e.to_string()))
}

/// Put the display back to its factory density and/or resolution (xiaowei "Reset DPI" /
/// "Reset resolution"). Returns what the phone reads as afterwards.
#[tauri::command]
pub async fn reset_display_metrics(
    state: State<'_, AppState>,
    udid: String,
    density: bool,
    size: bool,
) -> Result<String, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let android = state.require_android()?;
    android
        .reset_display_metrics(&udid, density, size)
        .await
        .map_err(|e| CommandError::operation(e.to_string()))
}

/// Power the phone off (xiaowei "Shutdown"). Irreversible from here — only a human at the
/// phone can turn it back on — so the UI confirms with that said plainly.
#[tauri::command]
pub async fn power_off_device(
    state: State<'_, AppState>,
    udid: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let android = state.require_android()?;
    android
        .power_off(&udid)
        .await
        .map_err(|e| CommandError::operation(e.to_string()))
}

/// Open the phone's own Settings app (xiaowei "Phone Settings").
#[tauri::command]
pub async fn open_system_settings(
    state: State<'_, AppState>,
    udid: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let android = state.require_android()?;
    android
        .open_system_settings(&udid)
        .await
        .map_err(|e| CommandError::operation(e.to_string()))
}

/// Wake the screen (xiaowei "Turn On Screen"). KEYCODE_WAKEUP, so calling it on an awake
/// phone does nothing — unlike the power key, which would put it to sleep.
#[tauri::command]
pub async fn wake_screen(state: State<'_, AppState>, udid: String) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let android = state.require_android()?;
    android
        .wake_screen(&udid)
        .await
        .map_err(|e| CommandError::operation(e.to_string()))
}

/// Screenshot into the phone's own gallery (xiaowei "Screenshot to phone"). Returns the
/// device path; the companion `screenshot` command is the one that copies to this machine.
#[tauri::command]
pub async fn screenshot_to_device(
    state: State<'_, AppState>,
    udid: String,
) -> Result<String, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let android = state.require_android()?;
    android
        .screenshot_to_device(&udid)
        .await
        .map_err(|e| CommandError::operation(e.to_string()))
}

/// Switch the phone's keyboard (xiaowei "Switch Input Method"). The picker only ever offers
/// ids the phone itself printed, and the driver refuses the Riviu helper's own IME.
#[tauri::command]
pub async fn set_input_method(
    state: State<'_, AppState>,
    udid: String,
    ime_id: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let android = state.require_android()?;
    android
        .set_input_method(&udid, &ime_id)
        .await
        .map_err(|e| CommandError::operation(e.to_string()))
}

/// Scan the host's ARP table for LAN devices, so the operator can pick one and `adb connect`
/// to it wirelessly (A9, xiaowei ARP list). Reads the OS `arp -a`; does not touch any phone.
#[tauri::command]
pub async fn arp_scan(state: State<'_, AppState>) -> Result<Vec<ArpEntry>, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let output = tokio::process::Command::new("arp")
        .arg("-a")
        .output()
        .await
        .map_err(|e| CommandError::operation(format!("arp -a lỗi: {e}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(riviu_android_driver::adb::parse_arp_table(&stdout)
        .into_iter()
        .map(|(ip, mac)| ArpEntry { ip, mac })
        .collect())
}
