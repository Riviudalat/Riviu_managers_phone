use std::path::{Path, PathBuf};
use std::sync::Arc;

use riviu_core::{
    apply_offset, AutomationScript, DeviceControlPlane, DeviceExclusiveContext, DeviceInfo,
    DeviceWorkOwner, GroupSyncPolicy, HardwareKey, InteractionSessionKind, JobRecord,
    StreamSettings, SwipeGesture, TapPoint, UiSession, UiWithStreamContext,
};
use riviu_script_engine::{example_script_json, parse_script};
use serde::Serialize;
use tauri::State;

use crate::view_watchdog::PaintReport;

use crate::command_error::CommandError;
use crate::state::{AppState, LeaseStream};

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
    /// Why, when the code alone does not say. `DeviceBusy` explains itself through
    /// `current_owner`; an action that simply failed does not, and the operator cannot act on
    /// "one of your twenty phones did not work".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Everything about a group request that can be judged before any phone is touched.
///
/// Both checks used to sit in different places and one of them was in the wrong place
/// entirely: the missing-key case was inside the per-device loop, in the match arm that
/// needed the key. So a malformed request drove every phone up to that point and *then*
/// returned an error, leaving the fleet half-actioned and the operator told it had failed.
///
/// A precondition belongs before the loop. Pulled out as a function so that is testable
/// without an `AppState`.
fn check_group_input(kind: &str, has_key: bool) -> Result<(), CommandError> {
    if !matches!(kind, "tap" | "swipe" | "type" | "home" | "key") {
        return Err(CommandError::operation(format!(
            "unknown group input kind: {kind}"
        )));
    }
    if kind == "key" && !has_key {
        return Err(CommandError::operation(
            "group input kind key requires a hardware key",
        ));
    }
    Ok(())
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

/// A udid reduced to something that cannot steer a path.
///
/// Every artifact this app writes is named after a device, and a udid is not a safe filename:
/// a Wi-Fi serial carries a `:`, which Windows refuses outright. Worse, `Path::join` with an
/// **absolute** component *replaces* the path rather than extending it, so an unsanitised udid
/// was not merely a bad filename — `C:/Users/x/.../Startup/z` or `\\host\share\z` would be
/// written there instead, and `create_dir_all` would build the tree to meet it.
///
/// Same reduction `set_wallpaper_bytes` already applies, promoted to a shared helper so the
/// next artifact path gets it for free instead of re-deriving the reasoning.
pub(crate) fn safe_udid_stem(udid: &str) -> String {
    udid.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

pub(crate) async fn with_manual_session<F, Fut>(
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
pub async fn list_devices(state: State<'_, AppState>) -> Result<Vec<DeviceInfo>, CommandError> {
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
    // Two outcomes, and they used to be one. The confirming read is what turns "the
    // preparation ran" into "and here is the device it left behind"; when it failed, the
    // old code fell back to the *stale* registry record and then stamped `Ready` on it
    // anyway. A phone unplugged between the session closing and this read came back as a
    // Ready device with a healthy agent, described entirely from memory.
    //
    // What the preparation itself proves is kept, because it is proven:
    // `prepare_ui_with_control` installed the agent and opened and closed a UI session, so
    // `wda_ready` is earned on either path. What is not kept is a `Ready` status nobody
    // observed, and the silence about why.
    let refreshed = state.control.refresh_device(&udid).await;
    let observed = match &refreshed {
        Ok(device) => device.clone(),
        Err(_) => state
            .registry
            .get(&udid)
            .ok_or_else(|| CommandError::operation("device missing"))?,
    };
    let device = prepared_device(observed, refreshed.err().map(|error| error.to_string()));
    state.registry.upsert(device.clone());
    Ok(device)
}

/// Stamp a prepared device with what was proven, and only that.
///
/// `prepare_ui_with_control` installed the agent and opened and closed a UI session, so
/// `wda_ready` is earned whichever way the confirming read went — that is why it is set on
/// both paths. `Ready` is not: it describes the device as the refresh found it, and when
/// the refresh failed nobody found it.
///
/// The old code took the stale registry record on failure and stamped `Ready` on it anyway.
/// A phone unplugged between the session closing and the read came back as a Ready device
/// with a healthy agent, described entirely from memory, and said nothing about the read
/// that had just failed.
fn prepared_device(mut device: DeviceInfo, unconfirmed: Option<String>) -> DeviceInfo {
    device.wda_ready = true;
    device.stream_url = None;
    device.tile_stream_state = riviu_core::TileStreamState::Parked;
    match unconfirmed {
        None => {
            device.status = riviu_core::DeviceStatus::Ready;
            device.last_error = None;
        }
        Some(reason) => {
            device.last_error = Some(format!(
                "Đã chuẩn bị xong nhưng không đọc lại được trạng thái máy: {reason}"
            ));
        }
    }
    device
}

#[tauri::command]
pub async fn install_ipa(
    state: State<'_, AppState>,
    udid: String,
    path: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let context = state
        .device_lease(&udid, DeviceWorkOwner::Repair, LeaseStream::Park)
        .await?;
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
        .device_lease(&udid, DeviceWorkOwner::Repair, LeaseStream::Park)
        .await?;
    state
        .control
        .uninstall_app(&context, &bundle_id)
        .await
        .map_err(CommandError::from)
}

/// Every app one phone reports as present.
///
/// One device at a time, deliberately not a `udids: Vec<String>` batch. The batch shape
/// in this app belongs to `agent_list_statuses`, which touches no phone at all — copying
/// it here would turn opening a panel into one blocking call that paints nothing until
/// the slowest phone in the fleet answers. The frontend fans out instead, so each row
/// appears when its own phone replies.
///
/// Takes admission but **no exclusive lease**: this reads and mutates nothing, and a
/// lease would let a panel refresh evict a running session. Admission is still right,
/// because it touches a device and shutdown must drain it before releasing the fleet.
///
/// A backend that cannot enumerate **refuses**, and the refusal names
/// `listInstalledApps`. It must never be flattened into an empty list — an iPhone that
/// answers `[]` reads as a phone with nothing installed, which is a lie the panel would
/// render as fact.
#[tauri::command]
pub async fn list_installed_apps(
    state: State<'_, AppState>,
    udid: String,
) -> Result<Vec<riviu_core::InstalledApp>, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state
        .control
        .list_installed_apps(&udid)
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn screenshot(state: State<'_, AppState>, udid: String) -> Result<String, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let dest = state.artifacts_dir.join("screenshots").join(format!(
        "{}-{}.jpg",
        safe_udid_stem(&udid),
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
        .device_lease(&udid, DeviceWorkOwner::ManualControl, LeaseStream::Keep)
        .await?;
    let path = state
        .control
        .screenshot(&context, &dest)
        .await
        .map_err(CommandError::from)?;
    Ok(path.display().to_string())
}

/// Run one operator-typed shell command on a device and return its output.
///
/// An escape hatch for a person, deliberately not a seam for code: nothing in this app
/// may call it to get work done, because a string typed by a human is the one input no
/// contract can describe. `adb shell <script>` only — never `adb <subcommand>`, which
/// would put `install`, `reboot`, `root` and above all `kill-server` one typo away.
/// AGENTS.md records that `adb kill-server` tears down every other tool's connection on
/// the machine, so it must not be reachable from a text box.
///
/// Takes an exclusive lease like `syslog`, and for a stronger reason: an arbitrary script
/// can reboot the phone or kill the app a running session is driving, so firing one at a
/// device somebody else holds has to be refused rather than raced.
#[tauri::command]
pub async fn device_shell(
    state: State<'_, AppState>,
    udid: String,
    script: String,
) -> Result<riviu_core::ShellOutcome, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    // `_keeping_stream`, following `screenshot` and deliberately not `syslog`: the plain
    // acquire parks the live preview, so running a command would black the tile the
    // operator is watching for its effect.
    let context = state
        .device_lease(&udid, DeviceWorkOwner::ManualControl, LeaseStream::Keep)
        .await?;
    state
        .control
        .device_shell(&context, &script)
        .await
        .map_err(CommandError::from)
}

/// Put a picture or a video into the phone's gallery, where the operator can see it.
///
/// Stage, prepare, then import — all three, which is the difference between this and
/// `push_material`. That one stops after staging, and staging lands the file in a hidden
/// dot-directory that MediaStore does not index: a row labelled "Import" that puts a file
/// somewhere the operator cannot find it would be the same lying button the Rotate row was
/// written to avoid. The import step is what moves it into a visible directory and tells
/// MediaStore about it.
///
/// One file per call, staged as a single-file campaign because that is the shape the
/// measured pipeline takes. `_keeping_stream`, because the operator is watching the tile to
/// see the picture appear.
#[tauri::command]
pub async fn import_media(
    state: State<'_, AppState>,
    udid: String,
    path: String,
) -> Result<String, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    import_one_media(&state, &udid, &path).await
}

/// Push one local media file into a device's gallery via stage → prepare → import. Shared by
/// `import_media` (one device) and `distribute_files` (a different file per device), so the
/// two agree byte-for-byte on the staging/manifest pipeline.
async fn import_one_media(
    state: &AppState,
    udid: &str,
    path: &str,
) -> Result<String, CommandError> {
    let source = PathBuf::from(path);
    if !source.is_file() {
        return Err(CommandError::invalid_argument(format!(
            "không thấy file {path}"
        )));
    }
    let name = source
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "media".to_string());

    // A staging tree of exactly one file. The campaign id is per-call rather than per-file
    // so two imports of the same picture do not collide in the device's staging directory.
    let campaign_id = uuid::Uuid::new_v4().to_string();
    let staged = state
        .artifacts_dir
        .join("import-staging")
        .join(udid)
        .join(&campaign_id);
    std::fs::create_dir_all(&staged).map_err(CommandError::operation)?;
    std::fs::copy(&source, staged.join(&name)).map_err(CommandError::operation)?;

    let context = state
        .device_lease(udid, DeviceWorkOwner::ManualControl, LeaseStream::Keep)
        .await?;

    let staged_evidence = state
        .control
        .stage_publish_media(
            &context,
            &state.active_agent_bundle_id,
            &campaign_id,
            &staged,
        )
        .await
        .map_err(CommandError::from)?;
    // The manifest hash the phone computed, which prepare and import both key on. Reading it
    // back from the staging evidence rather than recomputing it here is the point: the two
    // sides have to agree about what landed.
    let manifest = staged_evidence
        .get("manifestSha256")
        .and_then(|value| value.as_str())
        .ok_or_else(|| {
            CommandError::operation("staging did not report a manifest hash to import against")
        })?
        .to_string();
    state
        .control
        .prepare_publish_media(&context, &campaign_id, &manifest)
        .await
        .map_err(CommandError::from)?;
    let imported = state
        .control
        .import_publish_media(&context, &campaign_id, &manifest)
        .await
        .map_err(CommandError::from)?;

    // Best effort: the file is on the phone either way, and failing the whole import because
    // a temporary directory survived would report a success as a failure.
    let _ = std::fs::remove_dir_all(&staged);
    Ok(format!("Đã đưa {name} vào thư viện máy ({imported})"))
}

/// One phone's share of a file-distribution run (feature A2, xiaowei "文件分发").
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DistributeFileItem {
    pub udid: String,
    pub path: String,
}

/// Push a *different* file into each selected phone's gallery (xiaowei "File Distribution").
/// Same per-device batch shape as `group_input`/`distribute_text`: a phone that fails is
/// recorded and the run carries on, never aborting the batch.
#[tauri::command]
pub async fn distribute_files(
    state: State<'_, AppState>,
    assignments: Vec<DistributeFileItem>,
) -> Result<GroupInputReport, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let mut report = GroupInputReport {
        completed_udids: Vec::new(),
        skipped: Vec::new(),
    };
    for item in assignments {
        let DistributeFileItem { udid, path } = item;
        match import_one_media(&state, &udid, &path).await {
            Ok(_) => report.completed_udids.push(udid),
            Err(error) => report.skipped.push(open_failure_skip(udid, error)),
        }
    }
    Ok(report)
}

/// Copy the phone's photos and videos onto this machine.
///
/// The other direction, and a genuinely different operation: the import path above knows
/// about campaigns and manifests, this one knows only that the operator wants whatever is in
/// the camera roll right now.
/// What an export found and what of it landed.
///
/// The command used to return a bare count of files written, which cannot express the
/// failure it most needed to: a phone with five hundred photos of which twenty copied
/// reported `20`, and the toast said "Đã lấy 20 file" — the same words it says about a
/// phone that only ever had twenty. The per-file failures were logged where nobody was
/// looking.
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaExportReport {
    pub fetched: u32,
    pub found: u32,
    pub missed: u32,
}

#[tauri::command]
pub async fn export_media(
    state: State<'_, AppState>,
    udid: String,
    dest_dir: String,
) -> Result<MediaExportReport, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let dest = PathBuf::from(&dest_dir);
    if !dest.is_dir() {
        return Err(CommandError::invalid_argument(format!(
            "không thấy thư mục {dest_dir}"
        )));
    }
    // Per device, so exporting two phones into one folder does not interleave their camera
    // rolls into an unsortable pile.
    let into = dest.join(format!("riviu-{}", safe_udid_stem(&udid)));
    let context = state
        .device_lease(&udid, DeviceWorkOwner::ManualControl, LeaseStream::Keep)
        .await?;
    let pulled = state
        .control
        .pull_media(&context, &into)
        .await
        .map_err(CommandError::from)?;
    Ok(MediaExportReport {
        fetched: pulled.fetched.len() as u32,
        found: pulled.found as u32,
        missed: pulled.missed() as u32,
    })
}

/// Ask a device to rotate, and report the rotation it actually settled at.
///
/// Returns the observed rotation, not `()`, because measured on both fleet phones the
/// request is frequently ignored: a portrait-locked foreground app wins over every
/// mechanism tried. The caller compares what it asked for with what came back and tells
/// the operator the truth.
#[tauri::command]
pub async fn set_screen_rotation(
    state: State<'_, AppState>,
    udid: String,
    rotation: u8,
) -> Result<u8, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    // Keeps the stream for the same reason: the whole point is to watch the tile turn.
    let context = state
        .device_lease(&udid, DeviceWorkOwner::ManualControl, LeaseStream::Keep)
        .await?;
    state
        .control
        .set_screen_rotation(&context, rotation)
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn syslog(
    state: State<'_, AppState>,
    udid: String,
    lines: Option<usize>,
) -> Result<String, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let context = state
        .device_lease(&udid, DeviceWorkOwner::ManualControl, LeaseStream::Park)
        .await?;
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
        .device_lease(&udid, DeviceWorkOwner::Repair, LeaseStream::Park)
        .await?;
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
        .device_lease(&udid, DeviceWorkOwner::Repair, LeaseStream::Park)
        .await?;
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
        .device_lease(&udid, DeviceWorkOwner::Repair, LeaseStream::Park)
        .await?;
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

/// A drag as the path the finger actually took, not as its two endpoints.
///
/// `device_swipe` sends one `pointerMove`, which the framework receives as a perfectly
/// straight line at a perfectly constant velocity between the same two points every time.
/// The overlay was deciding the whole gesture at release from exactly two samples, so that
/// is all it could ever produce -- which is what "not sticking to the finger" was.
///
/// The agent's `/actions` takes an arbitrary number of moves with individual durations in
/// ONE round trip, so the curve costs no more than the straight line did.
#[tauri::command]
pub async fn device_swipe_path(
    state: State<'_, AppState>,
    udid: String,
    path: riviu_core::types::SwipePath,
    image_w: Option<f64>,
    image_h: Option<f64>,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    // A path with no steps is not a drag; refuse it here rather than letting it reach the
    // device as a touch that never moves and never lifts.
    if path.steps.is_empty() {
        return Err(CommandError::operation(anyhow::anyhow!(
            "a swipe path needs at least one step"
        )));
    }
    with_manual_session(
        &state,
        &udid,
        DeviceWorkOwner::ManualControl,
        |session| async move {
            match (image_w, image_h) {
                (Some(w), Some(h)) if w > 0.0 && h > 0.0 => {
                    session.swipe_path_image(path, w, h).await
                }
                _ => session.swipe_path(path).await,
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

/// Lock (screen off) or unlock a phone — xiaowei "锁屏/解锁", batched by the UI over a group
/// (D, iOS `useIphoneLockScreen`; cross-platform via `UiSession::set_locked`).
#[tauri::command]
pub async fn set_screen_locked(
    state: State<'_, AppState>,
    udid: String,
    locked: bool,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    with_manual_session(
        &state,
        &udid,
        DeviceWorkOwner::ManualControl,
        move |session| async move { session.set_locked(locked).await },
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

/// The skip entry a phone gets when its session could not be opened.
///
/// A function rather than two arms inline, because the two arms were the defect:
/// `DeviceBusy` was recorded and the loop carried on, and **every other code aborted the
/// whole batch** — discarding `completed_udids` and telling the operator the fleet action
/// had failed when nineteen of twenty phones had already taken the input. A phone that is
/// unplugged, whose agent has died, or that answers with anything unexpected is exactly as
/// skippable as a busy one, and on a fleet this size it is the likelier of the two.
///
/// `message` is `None` only for Busy, which explains itself through `current_owner`.
/// Anything else carries its reason: "one of your twenty phones did not work" is not
/// something an operator can act on.
fn open_failure_skip(udid: String, error: CommandError) -> GroupInputSkip {
    let busy = error.code == "DeviceBusy";
    GroupInputSkip {
        udid,
        code: error.code,
        current_owner: error.current_owner,
        message: (!busy).then(|| error.message.to_string()),
    }
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
    sync: Option<GroupSyncPolicy>,
) -> Result<GroupInputReport, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    check_group_input(&kind, key.is_some())?;
    let scale = matches!((image_w, image_h), (Some(w), Some(h)) if w > 0.0 && h > 0.0);
    // Group-sync timing/offset (A1). Absent policy = the old lockstep behaviour, so callers
    // that never send `sync` are unchanged. One seed per operation keeps successive group
    // actions different while any single one stays reproducible (the policy is pure/tested).
    let sync = sync.unwrap_or_default();
    let group_count = udids.len();
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut report = GroupInputReport {
        completed_udids: Vec::new(),
        skipped: Vec::new(),
    };
    for (ordinal, udid) in udids.into_iter().enumerate() {
        // Compute this device's delay/offset before touching anything. Sleep *before*
        // opening the session so a staggered wait does not hold a GroupSync lease idle.
        let plan = sync.plan(ordinal, group_count, seed);
        if plan.delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(plan.delay_ms)).await;
        }
        let overlay_session = state.overlay_ui_session(&udid).await;
        let owned = if overlay_session.is_none() {
            match state
                .control
                .open_manual_session(&udid, DeviceWorkOwner::GroupSync)
                .await
            {
                Ok(context) => Some(context),
                Err(error) => {
                    report
                        .skipped
                        .push(open_failure_skip(udid, CommandError::from(error)));
                    continue;
                }
            }
        } else {
            None
        };
        // The same rule as the open above: a lookup that fails is this phone's problem,
        // not the batch's. `?` here threw away every phone already actioned.
        let session = match overlay_session {
            Some(session) => session,
            None => match state.control.session(
                owned
                    .as_ref()
                    .expect("group input opened a session when no overlay is held"),
            ) {
                Ok(session) => session,
                Err(error) => {
                    report
                        .skipped
                        .push(open_failure_skip(udid, CommandError::from(error)));
                    continue;
                }
            },
        };
        // In image mode the coordinates are pixels bounded by the frame, so jitter must be
        // clamped on-screen; in logical mode there is no upper bound (only floored at 0).
        let bound_w = scale.then(|| image_w.unwrap());
        let bound_h = scale.then(|| image_h.unwrap());
        let action = match kind.as_str() {
            "tap" => {
                let (x, y) = apply_offset(
                    x.unwrap_or(0.0),
                    y.unwrap_or(0.0),
                    plan.dx,
                    plan.dy,
                    bound_w,
                    bound_h,
                );
                if scale {
                    session
                        .tap_image(x, y, image_w.unwrap(), image_h.unwrap())
                        .await
                } else {
                    session.tap(TapPoint { x, y }).await
                }
            }
            "swipe" => {
                // Shift both endpoints by the same offset: the gesture's shape and length are
                // preserved, only its position on the screen jitters.
                let (fx, fy) = apply_offset(
                    x.unwrap_or(0.0),
                    y.unwrap_or(0.0),
                    plan.dx,
                    plan.dy,
                    bound_w,
                    bound_h,
                );
                let (tx, ty) = apply_offset(
                    to_x.unwrap_or(0.0),
                    to_y.unwrap_or(0.0),
                    plan.dx,
                    plan.dy,
                    bound_w,
                    bound_h,
                );
                let from = TapPoint { x: fx, y: fy };
                let to = TapPoint { x: tx, y: ty };
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
            // Validated before the loop, so this arm cannot be reached without a key.
            "key" => {
                session
                    .press_hardware_key(key.expect("key was validated"))
                    .await
            }
            _ => unreachable!("group input kind was validated"),
        };
        // The session is closed whatever the action did. Leaking a GroupSync lease because a
        // tap failed would take the phone out of the fleet until the app restarts.
        let cleanup = match owned {
            Some(context) => state.control.close_manual_session(context).err(),
            None => None,
        };
        // Both arms below consume `udid`, and the cleanup report after them needs it.
        let udid_for_cleanup = udid.clone();
        match action {
            Ok(()) => report.completed_udids.push(udid),
            // **Record and carry on, rather than abort.** This used to be `?`, so the first
            // phone that failed for any reason other than Busy threw away
            // `completed_udids` and told the operator the whole batch had failed — when in
            // a twenty-phone fleet nineteen of them may have worked. One fleet-batch shape
            // in this codebase, matching `install_ipa_to_group`.
            Err(error) => report.skipped.push(GroupInputSkip {
                udid,
                code: "ActionFailed".to_string(),
                current_owner: None,
                message: Some(error.to_string()),
            }),
        }
        // A cleanup that failed does not undo an input that landed, and it is not a
        // reason to abandon the phones after this one. The udid stays in
        // `completed_udids` because the tap really did happen; the failure is reported
        // beside it so the operator learns the session did not close cleanly. Appearing
        // in both lists is the accurate description of that, not a contradiction.
        if let Some(error) = cleanup {
            let error = CommandError::from(error);
            report.skipped.push(GroupInputSkip {
                udid: udid_for_cleanup,
                code: "CleanupFailed".to_string(),
                current_owner: None,
                message: Some(error.message.to_string()),
            });
        }
    }
    Ok(report)
}

/// One phone's share of a text-distribution run (feature A2, xiaowei `inputBatch`).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DistributeTextItem {
    pub udid: String,
    pub text: String,
}

/// Type a *different* string onto each selected phone (xiaowei "文字分发 / Text Distribution").
///
/// The frontend has already split the block and paired each piece to a phone in the operator's
/// chosen order, so here we only apply. Cross-platform: it goes through `UiSession::type_text`,
/// which on Android reaches `ACTION_SET_TEXT` (the one route that carries Vietnamese
/// diacritics) and on iOS reaches WDA — the same path `group_input`'s `type` uses. Same
/// per-device batch shape as `group_input`: a phone that fails is recorded and the run
/// carries on, never aborting the batch.
#[tauri::command]
pub async fn distribute_text(
    state: State<'_, AppState>,
    assignments: Vec<DistributeTextItem>,
) -> Result<GroupInputReport, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let mut report = GroupInputReport {
        completed_udids: Vec::new(),
        skipped: Vec::new(),
    };
    for item in assignments {
        let DistributeTextItem { udid, text } = item;
        let overlay_session = state.overlay_ui_session(&udid).await;
        let owned = if overlay_session.is_none() {
            match state
                .control
                .open_manual_session(&udid, DeviceWorkOwner::GroupSync)
                .await
            {
                Ok(context) => Some(context),
                Err(error) => {
                    report
                        .skipped
                        .push(open_failure_skip(udid, CommandError::from(error)));
                    continue;
                }
            }
        } else {
            None
        };
        let session = match overlay_session {
            Some(session) => session,
            None => match state.control.session(
                owned
                    .as_ref()
                    .expect("distribute_text opened a session when no overlay is held"),
            ) {
                Ok(session) => session,
                Err(error) => {
                    report
                        .skipped
                        .push(open_failure_skip(udid, CommandError::from(error)));
                    continue;
                }
            },
        };
        let action = session.type_text(&text).await;
        let cleanup = match owned {
            Some(context) => state.control.close_manual_session(context).err(),
            None => None,
        };
        let udid_for_cleanup = udid.clone();
        match action {
            Ok(()) => report.completed_udids.push(udid),
            Err(error) => report.skipped.push(GroupInputSkip {
                udid,
                code: "ActionFailed".to_string(),
                current_owner: None,
                message: Some(error.to_string()),
            }),
        }
        if let Some(error) = cleanup {
            let error = CommandError::from(error);
            report.skipped.push(GroupInputSkip {
                udid: udid_for_cleanup,
                code: "CleanupFailed".to_string(),
                current_owner: None,
                message: Some(error.message.to_string()),
            });
        }
    }
    Ok(report)
}

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
) -> Result<Vec<riviu_core::DeviceFileEntry>, CommandError> {
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

/// Bring one app to the front of one phone (xiaowei's App List, where a click launches).
///
/// Goes through the control plane rather than the Android driver directly, because unlike
/// everything else in this block it is *not* Android-only: foregrounding an app is a thing
/// both platforms do, and the lease keeps the live tile up while it happens.
#[tauri::command]
pub async fn launch_device_app(
    state: State<'_, AppState>,
    udid: String,
    bundle_id: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let context = state
        .device_lease(&udid, DeviceWorkOwner::ManualControl, LeaseStream::Keep)
        .await?;
    state
        .control
        .launch_app(&context, &bundle_id)
        .await
        .map_err(CommandError::from)
}

/// What the phone had on its clipboard.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardRead {
    /// The phone's own MIME description, e.g. `text/plain`. Reported rather than assumed:
    /// a clipboard holding an image is a real answer and the UI has to be able to say so
    /// instead of showing empty text.
    pub content_type: String,
    /// Decoded as UTF-8 when it is text. Non-text content leaves this empty and `bytes`
    /// carries the size.
    pub text: String,
    pub bytes: usize,
}

/// Read the phone's clipboard onto this machine (xiaowei "Export Clipboard").
///
/// The one row of the reference product's menu that this app had a *session method* for and
/// no command over it, which is why it went missing for so long: `UiSession::get_clipboard`
/// has existed since the interaction work and nothing could call it.
///
/// The ceiling is [`MAX_INTERACTION_CLIPBOARD_BYTES`] and not a number chosen here. Measured
/// 21/08/2026 on 23021RAAEG: asking for 256 KiB — which looked like a generous, harmless
/// choice — is refused outright with `clipboard read limit exceeds 65536 bytes`, because the
/// capability contract pins the value on both platforms rather than treating it as a maximum.
/// So the constant is the contract's, and a clipboard bigger than that is the phone's answer
/// to report, not something to raise a limit for.
#[tauri::command]
pub async fn device_get_clipboard(
    state: State<'_, AppState>,
    udid: String,
) -> Result<ClipboardRead, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let read = std::sync::Arc::new(parking_lot::Mutex::new(None));
    let sink = read.clone();
    with_manual_session(
        &state,
        &udid,
        DeviceWorkOwner::ManualControl,
        move |session| async move {
            let (content_type, bytes) = session
                .get_clipboard(riviu_core::MAX_INTERACTION_CLIPBOARD_BYTES)
                .await?;
            *sink.lock() = Some((content_type, bytes));
            Ok(())
        },
    )
    .await?;
    let (content_type, bytes) = read
        .lock()
        .take()
        .ok_or_else(|| CommandError::operation("máy không trả về nội dung clipboard"))?;
    Ok(ClipboardRead {
        // Lossy on purpose: a clipboard holding half a UTF-8 sequence is still worth
        // showing, and refusing the whole read over one bad byte would lose the rest.
        text: String::from_utf8_lossy(&bytes).to_string(),
        bytes: bytes.len(),
        content_type,
    })
}

/// Write text onto the phone's clipboard, so the operator can paste it there by hand.
#[tauri::command]
pub async fn device_set_clipboard(
    state: State<'_, AppState>,
    udid: String,
    text: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    with_manual_session(
        &state,
        &udid,
        DeviceWorkOwner::ManualControl,
        move |session| async move { session.set_clipboard("text/plain", text.as_bytes()).await },
    )
    .await
}

/// One host discovered on the LAN via the ARP table (feature A9).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArpEntry {
    pub ip: String,
    pub mac: String,
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

#[tauri::command]
pub fn get_stream_settings(state: State<'_, AppState>) -> StreamSettings {
    state.stream_settings.read().clone()
}

/// Save the stream settings, and make the two that used to be ignored take effect.
///
/// `fps` was **overwritten** with the compiled-in constant here, so an operator could
/// move the frame-rate control and the value never left this function. `grid_quality`
/// had no reader anywhere in the tree. Both are now clamped rather than discarded and
/// pushed into the Android view path.
///
/// And the whole row now **survives a restart**. It did not: the value lived only in an
/// `Arc<RwLock<_>>` built from `Default` at bootstrap, so an operator's choice lasted until
/// they closed the app — a save that quietly forgets is not much better than the no-op this
/// already replaced once.
///
/// Running tiles are restarted so the change is visible: a settings row that only
/// applies to phones started later is the same silent no-op this replaces. The restart
/// is the path the watchdog already takes several times an hour, so it costs a second
/// of black tile and nothing else.
#[tauri::command]
pub async fn set_stream_settings(
    state: State<'_, AppState>,
    settings: StreamSettings,
) -> Result<StreamSettings, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let mut s = settings;
    s.fps = clamp_stream_fps(s.fps);
    // Persist before applying. A save that takes effect for this session and vanishes on
    // restart is worse than one that reports it could not be written.
    state
        .db
        .save_stream_settings(&s)
        .map_err(CommandError::operation)?;
    *state.stream_settings.write() = s.clone();

    if let Some(android) = &state.android {
        android.set_view_tuning(s.grid_quality.clone(), s.focus_quality.clone(), s.fps);
        for device in state.registry.list() {
            if device.platform != riviu_core::DevicePlatform::Android {
                continue;
            }
            if !android
                .view_is_running(&device.udid, riviu_android_driver::ViewPreset::Tile)
                .await
            {
                continue;
            }
            // Retune restarts the same producer rather than spawning a second one.
            // A failure here is logged and skipped: one phone that will not retune must
            // not stop the setting from reaching the rest of the fleet.
            //
            // Under the same ceiling as every other producer restart. The loop is already
            // sequential, so on an idle fleet it never waits; the permit is what stops it
            // stacking on top of recoveries the keeper started, which is how a settings
            // change used to become a twenty-first concurrent scrcpy spawn.
            let frames = state
                .view_paint
                .sample(&device.udid)
                .map(|report| report.frames)
                .unwrap_or(0);
            let permit = match state
                .view_recovery
                .admit_operator(&device.udid, frames)
                .await
            {
                Ok(permit) => permit,
                Err(error) => {
                    log::warn!(
                        "could not retune {} after a settings change: {}",
                        device.udid,
                        error.message
                    );
                    continue;
                }
            };
            if let Err(error) = android
                .set_view_preset(&device.udid, riviu_android_driver::ViewPreset::Tile)
                .await
            {
                log::warn!(
                    "could not retune {} after a settings change: {error:#}",
                    device.udid
                );
            }
            drop(permit);
            state.view_paint.clear(&device.udid);
        }
    }
    Ok(s)
}

#[tauri::command]
pub fn latest_frame(
    state: State<'_, AppState>,
    udid: String,
) -> Result<Option<String>, CommandError> {
    Ok(state.streams.latest(&udid).map(|bytes| {
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes.as_slice())
    }))
}

#[tauri::command]
pub fn view_endpoint(state: State<'_, AppState>) -> Result<Option<String>, CommandError> {
    Ok(state.view_hub.endpoint())
}

/// What the frontend painted, for every device it is tracking, as of its last tick.
///
/// The frontend is the only thing that can see whether a frame came out of the decoder, and
/// AGENTS.md 9.66 is why it cannot simply log it: vite forwards the page's console but not a
/// Web Worker's, so counters that stay inside the worker are invisible by construction. This
/// carries them to the one place that can act on them.
///
/// Deliberately **not** on the mutating-command inventory and deliberately without
/// `ensure_accepting_work`: it writes a ledger, touches no device, and must keep working
/// while the app is draining so the last reports before a quit are not lost.
///
/// One call per tick for the whole fleet rather than one per device per beat — at twenty
/// phones and a 1 s beat the per-device shape would be twenty IPC round trips a second to
/// say "still fine".
#[tauri::command]
pub fn view_report_paint(state: State<'_, AppState>, reports: Vec<PaintReport>) {
    let now = std::time::Instant::now();
    for report in &reports {
        // The hub's generation is the authority on which producer is current. A report from
        // before a restart is not evidence about the producer that replaced it.
        let current = state.view_hub.current_generation(&report.udid);
        state.view_paint.record(report, current, now);
        if report.generation == current {
            state
                .view_recovery
                .note_painted(&report.udid, report.frames);
        }
    }
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
    // Do not tear down a producer somebody else is already replacing. This stops before it
    // claims, so losing that race used to leave the device with NO producer at all until the
    // keeper's next tick -- a stream the operator was watching went away because a recovery
    // for it started somewhere else. A start already in flight satisfies "ensure".
    if android.view_start_in_flight(&udid) {
        return Ok(());
    }
    // "Ensure" means two different operations depending on what is there, and only one of
    // them is rationed. With no producer running this is a first start: nothing is torn down,
    // so it takes no permit -- which is what keeps `startFleetPreview`'s twenty-way fan-out
    // (startPreview.ts) as fast as the bench says it can be. With one running, ensuring it
    // means replacing a picture that may be working, and that is precisely what the ceiling
    // is for.
    if !android.view_is_active(&udid).await {
        crate::view_watchdog::start_android_view(android, &state.registry, &udid)
            .await
            .map_err(CommandError::operation)?;
        return Ok(());
    }
    let frames = state
        .view_paint
        .sample(&udid)
        .map(|report| report.frames)
        .unwrap_or(0);
    let permit = state.view_recovery.admit_operator(&udid, frames).await?;
    // Stop-then-start, the preset the operator last asked for, and the ledger cleared -- all
    // of it in one place now, so the keeper and this command cannot drift apart.
    crate::view_watchdog::restart_android_view(
        android,
        &state.registry,
        &state.view_paint,
        &udid,
        permit,
    )
    .await
    .map_err(CommandError::operation)?;
    Ok(())
}

/// Ask the phone for a fresh keyframe. The cheap half of "the picture is stuck".
///
/// Takes **no recovery permit**, deliberately: it tears nothing down, so the ceiling that
/// bounds how much of the fleet can go dark at once has nothing to bound here. It is also
/// the operator-facing half of what the watchdog now tries first — one byte and a fresh IDR,
/// against ~11.5 s of black tile for a restart.
#[tauri::command]
pub async fn view_request_keyframe(
    state: State<'_, AppState>,
    udid: String,
) -> Result<bool, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let Some(android) = &state.android else {
        return Ok(false);
    };
    android
        .request_keyframe(&udid)
        .await
        .map_err(CommandError::operation)
}

/// One touch event, live, on the scrcpy control socket.
///
/// **Not a replacement for `device_tap`.** Taps, keys and text stay on uiautomator2, which
/// handles Vietnamese diacritics and does not care what size the video is. This exists for
/// the continuous middle of a drag, which until now reached the phone only after the operator
/// let go — `FocusStream` buffered the samples and posted a single swipe on release, so the
/// picture stood still under a moving finger. See AGENTS.md 9.77.
///
/// Deliberately outside `with_manual_session`. That helper claims device ownership and opens
/// a uiautomator2 session, neither of which this path needs — and a pointer at 60 Hz would be
/// claiming and releasing ownership sixty times a second. The control socket already belongs
/// to the producer that is drawing the picture being touched.
///
/// `Ok(false)` means the phone is not streaming, so the caller should fall back to the agent
/// rather than report a failure. A refusal to admit work still throws, because a drag during
/// shutdown should stop like everything else.
#[tauri::command]
pub async fn view_inject_touch(
    state: State<'_, AppState>,
    udid: String,
    action: String,
    x: f64,
    y: f64,
    image_w: f64,
    image_h: f64,
) -> Result<bool, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let Some(android) = &state.android else {
        return Ok(false);
    };
    let action =
        riviu_android_driver::TouchAction::parse(&action).map_err(CommandError::operation)?;
    android
        .inject_touch(&udid, action, x, y, image_w, image_h)
        .await
        .map_err(CommandError::operation)
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
    // A retune restarts the same producer, so it costs the adb server exactly what a
    // recovery costs and it belongs under the same ceiling. It uses the operator lane
    // because it *is* an operator action — opening or closing an overlay — and that lane has
    // no per-device backoff: refusing a person's second click because their first was 40 s
    // ago would read as the app being broken.
    //
    // A refusal here is not the end of it. The keeper reconciles toward
    // `desired_view_preset` on its own tick, which is what makes this safe to refuse at all;
    // `set_view_preset` records the desire before it does any work.
    let frames = state
        .view_paint
        .sample(&udid)
        .map(|report| report.frames)
        .unwrap_or(0);
    let permit = state.view_recovery.admit_operator(&udid, frames).await?;
    let outcome = android
        .set_view_preset(&udid, preset)
        .await
        .map_err(CommandError::operation)
        .map(|_| ());
    // Held across the retune for the same reason the recovery path holds it: a producer
    // that has spawned and not yet published is still using the resource being rationed.
    drop(permit);
    // The producer was replaced, so evidence about the old one is not evidence about this
    // one -- the same rule `restart_android_view` applies.
    state.view_paint.clear(&udid);
    outcome
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
        "{}-{}.jpg",
        safe_udid_stem(&udid),
        chrono::Utc::now().timestamp_millis()
    ));
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(CommandError::operation)?;
    }
    std::fs::write(&dest, jpeg).map_err(CommandError::operation)?;
    Ok(dest.display().to_string())
}

#[tauri::command]
pub fn list_jobs(state: State<'_, AppState>) -> Result<Vec<JobRecord>, CommandError> {
    state.jobs.list_jobs(100).map_err(err)
}

#[tauri::command]
pub async fn run_script(
    state: State<'_, AppState>,
    script_json: String,
    udids: Vec<String>,
) -> Result<JobRecord, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let script: AutomationScript = parse_script(&script_json).map_err(err)?;
    state.jobs.enqueue(script, udids).await.map_err(err)
}

#[tauri::command]
pub fn cancel_job(state: State<'_, AppState>, job_id: String) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let id = uuid::Uuid::parse_str(&job_id).map_err(err)?;
    state.jobs.cancel(id);
    Ok(())
}

#[tauri::command]
pub fn list_scripts(state: State<'_, AppState>) -> Result<Vec<(String, String)>, CommandError> {
    state.db.list_scripts().map_err(err)
}

#[tauri::command]
pub fn save_script(
    state: State<'_, AppState>,
    name: String,
    body_json: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    parse_script(&body_json).map_err(err)?;
    state.db.save_script(&name, &body_json).map_err(err)
}

#[tauri::command]
pub fn example_script() -> String {
    example_script_json().to_string()
}

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

/// The highest frame rate a settings row may ask for.
///
/// The scrcpy presets top out at 30 and the fleet measured 24–29 FPS, so a row offering
/// 60 would promise a rate no phone here delivers. Clamped rather than validated so a
/// stored value from an older build cannot refuse the whole save.
const MAX_SETTABLE_VIEW_FPS: u32 = 30;

/// The one place the settable frame rate is bounded.
///
/// Two callers now that the value is persisted — the save, and the load at startup — and
/// they must not drift: `get_stream_settings` reporting one number while the encoder runs
/// another is precisely the silent disagreement AGENTS.md 9.59 records as already fixed once.
pub(crate) fn clamp_stream_fps(fps: u32) -> u32 {
    fps.clamp(riviu_android_driver::MIN_VIEW_FPS, MAX_SETTABLE_VIEW_FPS)
}

fn err(e: impl std::fmt::Display) -> CommandError {
    CommandError::operation(e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use riviu_core::{DeviceWorkCoordinator, StreamBudgetManager};
    use riviu_ios_driver::MockIosDriver;

    #[test]
    fn the_fps_field_offers_exactly_the_range_this_file_clamps_to() {
        // The panel comment says these are "kept in step with MIN_VIEW_FPS and
        // MAX_SETTABLE_VIEW_FPS on the Rust side", and until now nothing kept them. Drift
        // here is not an error the operator can see: they type 45, the field accepts it,
        // the encoder silently runs 30, and the settings row goes on displaying 45 as
        // though it took. The panel already has one pinned constant (TILE_FPS_CEILING in
        // scrcpy.rs); this is the same pin for the other two numbers in the same block.
        let panel = include_str!("../../src/components/settings/StreamQualitySection.tsx");
        let declared = |name: &str| -> u32 {
            let needle = format!("const {name} = ");
            panel
                .lines()
                .find_map(|line| line.trim().strip_prefix(&needle).map(str::to_owned))
                .and_then(|rest| rest.trim().trim_end_matches(';').parse().ok())
                .unwrap_or_else(|| panic!("StreamQualitySection.tsx declares {name}"))
        };
        assert_eq!(
            declared("MIN_STREAM_FPS"),
            riviu_android_driver::MIN_VIEW_FPS,
            "the field's floor is not the encoder's floor"
        );
        assert_eq!(
            declared("MAX_STREAM_FPS"),
            MAX_SETTABLE_VIEW_FPS,
            "the field would accept a rate the save clamps away without saying so"
        );
    }

    fn stale(status: riviu_core::DeviceStatus) -> DeviceInfo {
        DeviceInfo {
            udid: "ce06".into(),
            name: "Note 8".into(),
            model: "SM-N950F".into(),
            platform: riviu_core::DevicePlatform::Android,
            os_version: "8.0".into(),
            connection: riviu_core::ConnectionKind::Usb,
            status,
            battery: None,
            wda_ready: false,
            wda_expires_at: None,
            stream_url: None,
            tile_stream_state: riviu_core::TileStreamState::default(),
            last_error: None,
        }
    }

    #[test]
    fn a_prepare_whose_read_back_failed_does_not_claim_the_device_is_ready() {
        // The read is what turns "the preparation ran" into "and here is the device it
        // left behind". When it failed, the old code fell back to the stale registry
        // record and stamped `Ready` on it anyway -- so a phone unplugged between the
        // session closing and the read came back Ready with a healthy agent, described
        // entirely from memory, and said nothing about the failure.
        let device = prepared_device(
            stale(riviu_core::DeviceStatus::Connected),
            Some("device 'ce06' not found".into()),
        );

        assert_ne!(device.status, riviu_core::DeviceStatus::Ready);
        assert!(device
            .last_error
            .as_deref()
            .expect("a reason")
            .contains("device 'ce06' not found"));
        // Still earned: the session opened and closed, which is what this flag means.
        assert!(device.wda_ready);
    }

    #[test]
    fn a_prepare_that_was_read_back_is_ready_and_carries_no_reason() {
        let device = prepared_device(stale(riviu_core::DeviceStatus::Connected), None);

        assert_eq!(device.status, riviu_core::DeviceStatus::Ready);
        assert_eq!(device.last_error, None);
        assert!(device.wda_ready);
        assert_eq!(
            device.tile_stream_state,
            riviu_core::TileStreamState::Parked,
            "prepare leaves no producer running"
        );
    }
    #[test]
    fn a_group_request_is_judged_before_a_single_phone_is_touched() {
        // The missing-key check used to live inside the per-device loop, in the arm that
        // needed the key -- so a malformed request drove every phone up to that point and
        // only then failed, leaving the fleet half-actioned.
        assert!(check_group_input("key", false).is_err());
        assert!(check_group_input("key", true).is_ok());
        assert!(check_group_input("rotate", true).is_err());
        for kind in ["tap", "swipe", "type", "home"] {
            assert!(check_group_input(kind, false).is_ok(), "{kind}");
        }
    }

    #[test]
    fn a_phone_that_cannot_be_opened_is_skipped_whatever_the_reason() {
        // `DeviceBusy` was the only code that produced a skip. Everything else returned,
        // which discarded `completed_udids` and reported the whole fleet action as failed —
        // on twenty phones, nineteen of which had already taken the input. The phones that
        // hit this are the ordinary ones: a cable that dropped, an agent that died, a
        // serial adb stopped answering for.
        let unplugged = open_failure_skip(
            "ce07".into(),
            CommandError::code("DeviceUnavailable", "device 'ce07' not found"),
        );
        assert_eq!(unplugged.code, "DeviceUnavailable");
        assert_eq!(
            unplugged.message.as_deref(),
            Some("device 'ce07' not found"),
            "a code alone is not something an operator can act on"
        );

        // Busy keeps its own shape: the holder is the whole story, and repeating it as a
        // message would put the same sentence on screen twice.
        let busy = open_failure_skip(
            "ce06".into(),
            CommandError {
                current_owner: Some(DeviceWorkOwner::Nurture),
                ..CommandError::code("DeviceBusy", "device 'ce06' is held by Nurture")
            },
        );
        assert_eq!(busy.message, None);
        assert_eq!(busy.current_owner, Some(DeviceWorkOwner::Nurture));
    }

    #[test]
    fn a_skip_carries_something_the_operator_can_act_on() {
        // Two different silences, two different fields. Busy is explained by who holds the
        // phone; a failed action is explained by nothing at all unless the message is kept,
        // and "one of your twenty phones did not work" is not something anyone can act on.
        let busy = GroupInputSkip {
            udid: "ce06".into(),
            code: "DeviceBusy".into(),
            current_owner: Some(DeviceWorkOwner::Nurture),
            message: None,
        };
        let failed = GroupInputSkip {
            udid: "ce07".into(),
            code: "ActionFailed".into(),
            current_owner: None,
            message: Some("agent did not answer".into()),
        };
        let encoded = serde_json::to_string(&vec![busy, failed]).expect("serialize skips");
        assert!(encoded.contains("currentOwner"));
        assert!(encoded.contains("agent did not answer"));
        // Absent rather than null, so the frontend can tell "no message" from "empty message".
        assert!(!encoded.contains("\"message\":null"));
    }

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

    /// A udid cannot decide *where* an artifact is written, only what it is called.
    ///
    /// `Path::join` with an absolute component **replaces** the path, so before this a udid of
    /// `C:/…/Startup/z` did not produce a badly-named screenshot in the artifacts folder — it
    /// produced a file in the Startup folder, with `create_dir_all` building the tree to reach
    /// it. A UNC udid is the same primitive pointed at SMB, which also leaks an NTLM handshake.
    #[test]
    fn a_udid_can_never_steer_an_artifact_path_off_the_artifacts_dir() {
        let artifacts = Path::new("C:/riviu/artifacts");
        for hostile in [
            "C:/Users/x/AppData/Roaming/Microsoft/Windows/Start Menu/Programs/Startup/z",
            r"\\attacker\share\z",
            "../../../../Windows/System32/z",
            "/etc/cron.d/z",
        ] {
            let joined = artifacts.join(format!("{}.png", safe_udid_stem(hostile)));
            assert_eq!(
                joined.parent(),
                Some(artifacts),
                "escaped the artifacts dir: {hostile:?} -> {joined:?}"
            );
        }

        // And the ordinary cases still round-trip to something readable: a USB serial is
        // untouched, and a Wi-Fi serial keeps its digits with the illegal ':' neutralised.
        assert_eq!(safe_udid_stem("10969614"), "10969614");
        assert_eq!(safe_udid_stem("192.168.1.44:5555"), "192_168_1_44_5555");
    }
}
