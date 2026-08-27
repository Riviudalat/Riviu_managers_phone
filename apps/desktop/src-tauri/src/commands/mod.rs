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

mod device;
pub use device::*;
mod android_ops;
pub use android_ops::*;
mod view;
pub use view::*;
mod jobs;
pub use jobs::*;
mod system;
pub use system::*;

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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupInstallResult {
    pub udid: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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
/// One phone's share of a text-distribution run (feature A2, xiaowei `inputBatch`).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DistributeTextItem {
    pub udid: String,
    pub text: String,
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

/// One host discovered on the LAN via the ARP table (feature A9).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArpEntry {
    pub ip: String,
    pub mac: String,
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
        let panel = include_str!("../../../src/components/settings/StreamQualitySection.tsx");
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
