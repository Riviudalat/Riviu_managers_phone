//! Process-pinned diagnostic scope. It never grants campaign or public-action authority.
use crate::{command_error::CommandError, state::AppState};
use anyhow::{ensure, Context};
use serde::Deserialize;
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeviceScope {
    pub udid: String,
    pub package: String,
    pub expected_account: String,
    pub target_url: Option<String>,
    pub draft_text: Option<String>,
    pub source_root: Option<PathBuf>,
    pub bundle_id: Option<String>,
    #[serde(default)]
    pub publish_fingerprint: Option<String>,
    #[serde(default)]
    pub sound_policy: Option<riviu_core::PublishSoundPolicy>,
    #[serde(default)]
    pub helper_canary: bool,
    #[serde(default)]
    pub allow_warm_launch: bool,
    #[serde(default)]
    pub allow_installed_runner: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScopeFile {
    activation_id: String,
    device_scopes: Vec<DeviceScope>,
}
#[derive(Clone)]
pub(crate) struct Session {
    pub root: PathBuf,
    pub activation: String,
    pub devices: Arc<[DeviceScope]>,
    pub fence_root: PathBuf,
}
impl Session {
    pub fn from_process() -> anyhow::Result<Option<Self>> {
        let requested = std::env::var_os("RIVIU_NO_PUBLIC_REHEARSAL");
        let scope = std::env::var_os("RIVIU_NO_PUBLIC_SCOPE");
        let directory = std::env::var_os("RIVIU_NO_PUBLIC_DIR");
        if requested.is_none() && scope.is_none() && directory.is_none() {
            return Ok(None);
        }
        ensure!(
            cfg!(debug_assertions) && requested.as_deref() == Some(std::ffi::OsStr::new("1")),
            "no-public rehearsal is debug-only and requires explicit activation"
        );
        ensure!(
            std::env::var_os("RIVIU_MOCK_DEVICES").is_none()
                && std::env::var_os("RIVIU_UI_SMOKE").is_none(),
            "no-public device rehearsal cannot use UI smoke/mock"
        );
        let scope = PathBuf::from(scope.context("no-public scope required")?);
        let root = PathBuf::from(directory.context("fresh diagnostic directory required")?);
        ensure!(
            scope.is_absolute() && root.is_absolute() && !root.exists(),
            "scope and new diagnostic root must be absolute"
        );
        ensure!(
            !scope.is_symlink() && std::fs::metadata(&scope)?.len() <= 65536,
            "invalid scope file"
        );
        let file: ScopeFile = serde_json::from_slice(&std::fs::read(&scope)?)?;
        ensure!(
            uuid::Uuid::parse_str(&file.activation_id).is_ok()
                && !file.device_scopes.is_empty()
                && file.device_scopes.len() <= 100,
            "invalid activation/device scope"
        );
        let mut ids = std::collections::HashSet::new();
        for device in &file.device_scopes {
            if let Some(url) = &device.target_url {
                ensure!(
                    url.starts_with("https://www.tiktok.com/@") && url.len() <= 4096,
                    "direct target URL required"
                );
            }
            ensure!(
                device.source_root.is_some() == device.bundle_id.is_some(),
                "publish source and bundle must be paired"
            );
            if let Some(source) = &device.source_root {
                ensure!(source.is_absolute(), "publish source must be absolute");
            }
            ensure!(
                !device.udid.trim().is_empty()
                    && ids.insert(device.udid.clone())
                    && (!device.expected_account.trim().is_empty() || (device.target_url.is_none() && device.source_root.is_none() && device.draft_text.is_none())),
                "device/account must be explicit and unique"
            );
            ensure!(
                matches!(
                    device.package.as_str(),
                    "com.ss.android.ugc.trill" | "com.zhiliaoapp.musically"
                ),
                "only measured Android TikTok packages are supported"
            );
            ensure!(
                device
                    .draft_text
                    .as_ref()
                    .is_none_or(|text| !text.is_empty()
                        && !text.contains(['\r', '\n'])
                        && text.len() <= 4096),
                "invalid draft text"
            );
        }
        let parent = root.parent().context("diagnostic parent missing")?;
        for ancestor in parent.ancestors().chain(scope.ancestors()) {
            if ancestor.exists() {
                let metadata = std::fs::symlink_metadata(ancestor)?;
                ensure!(
                    !metadata.file_type().is_symlink(),
                    "scope/root cannot traverse symlinks"
                );
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    ensure!(
                        metadata.file_attributes() & 0x400 == 0,
                        "scope/root cannot traverse reparse points"
                    );
                }
            }
        }
        ensure!(
            parent.is_dir() && !parent.is_symlink(),
            "diagnostic parent must exist and be plain"
        );
        let fence_root = dirs::data_dir()
            .context("diagnostic data root unavailable")?
            .join("riviu-no-public-fences");
        if fence_root.exists() {
            ensure!(
                !fence_root.is_symlink(),
                "diagnostic fence root must be plain"
            );
        }
        std::fs::create_dir_all(&fence_root)?;
        for ancestor in fence_root.ancestors() {
            let metadata = std::fs::symlink_metadata(ancestor)?;
            ensure!(
                !metadata.file_type().is_symlink(),
                "diagnostic fences cannot traverse links"
            );
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                ensure!(
                    metadata.file_attributes() & 0x400 == 0,
                    "diagnostic fences cannot traverse reparse points"
                );
            }
        }
        std::fs::create_dir(&root)?;
        std::fs::write(
            root.join("scope.json"),
            serde_json::to_vec(
                &serde_json::json!({"activationId":file.activation_id,"deviceIds":ids,"publicEffectsAllowed":false}),
            )?,
        )?;
        Ok(Some(Self {
            root,
            activation: file.activation_id,
            devices: file.device_scopes.into(),
            fence_root,
        }))
    }
    pub fn device(&self, udid: &str) -> anyhow::Result<&DeviceScope> {
        self.devices
            .iter()
            .find(|entry| entry.udid == udid)
            .context("device is outside scope")
    }
    pub fn check_command(&self, command: &str) -> Result<(), CommandError> {
        if read_command(command) {
            return Ok(());
        }
        Err(CommandError::code(
            "NoPublicRehearsalDenied",
            "Diagnostic process refuses this operation; no public action is authorized",
        ))
    }
}
fn read_command(command: &str) -> bool {
    matches!(
        command,
        "startup_error"
            | "app_log_directory"
            | "log_frontend_error"
            | "list_devices"
            | "list_device_work_states"
            | "list_jobs"
            | "list_groups"
            | "list_device_metas"
            | "driver_mode"
            | "driver_degraded_reason"
            | "android_unavailable_reason"
            | "android_tool_problems"
            | "agent_list_statuses"
            | "get_stream_settings"
            | "no_public_status"
            | "no_public_metadata"
            | "no_public_shutdown"
            | "no_public_reconcile_setup"
            | "no_public_inspect"
            | "no_public_run_status"
            | "no_public_cancel"
            | "no_public_prepare_publish"
            | "no_public_prepare_interaction"
    )
}
#[tauri::command]
pub(crate) fn no_public_status(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, CommandError> {
    use tauri::Manager;
    let policy = app.state::<crate::ui_smoke::StartupPolicy>();
    let session = policy.rehearsal().ok_or_else(|| {
        CommandError::code(
            "NoPublicModeRequired",
            "No-public diagnostic mode is not active",
        )
    })?;
    Ok(
        serde_json::json!({"activationId":session.activation,"publicEffectsAllowed":false,"devices":state.registry.list(),"scopeDeviceIds":session.devices.iter().map(|d| &d.udid).collect::<Vec<_>>()}),
    )
}

#[tauri::command]
pub(crate) async fn no_public_reconcile_setup(
    app: tauri::AppHandle, state: tauri::State<'_, AppState>, udid: String,
    prior_activation: String, prior_request: String,
) -> Result<serde_json::Value, CommandError> {
    use tauri::Manager;
    let policy = app.state::<crate::ui_smoke::StartupPolicy>();
    let scope = policy.rehearsal().ok_or_else(||CommandError::code("NoPublicModeRequired", "diagnostic required"))?;
    scope.device(&udid).map_err(CommandError::operation)?;
    let result = async {
        uuid::Uuid::parse_str(&prior_activation)?; uuid::Uuid::parse_str(&prior_request)?;
        // Only these two reviewed, pre-session setup refusals are eligible. This is
        // not a general phase/string-based recovery policy for unknown operations.
        anyhow::ensure!(matches!((prior_activation.as_str(), prior_request.as_str(), udid.as_str()),
            ("025b7131-e623-4d72-9dc0-1972f04a2352", "93a38bdc-0327-4fc6-b78b-7c09743841e4", "ce021712aaf9533405") |
            ("3cb58329-3d81-496e-b200-ea8946a8cc3a", "a5b93c5b-bef0-4d8d-85b2-8812cd1052e7", "ce031713b0c610ab0c") |
            ("41f4e58c-8870-481b-9b09-339eeb1ca155", "fd6f1d78-1b98-497d-9f16-1acb05087cac", "ce031713b0c610ab0c")), "setup refusal is not independently reviewed; reconciliation refused");
        let old_root = scope.root.parent().context("diagnostic parent missing")?.join(format!("{prior_activation}.run"));
        let prior_dir = old_root.join(format!("run-{prior_request}"));
        let status: serde_json::Value = serde_json::from_slice(&std::fs::read(prior_dir.join("status.json"))?)?;
        anyhow::ensure!(status["activationId"].as_str()==Some(prior_activation.as_str()) && status["requestId"].as_str()==Some(prior_request.as_str()) && status["udid"].as_str()==Some(udid.as_str()) && status["kind"]=="inspect" && status["state"]=="needsAttention", "prior setup diagnostic does not match");
        anyhow::ensure!(matches!(status["phase"].as_str(),Some("deviceMetadata" | "warmForegroundIntent")), "prior diagnostic progressed beyond setup; no reconciliation");
        let process: serde_json::Value = serde_json::from_slice(&std::fs::read(scope.root.parent().expect("parent checked").join(format!("{prior_activation}.process.json")))?)?;
        let expected_exe = if prior_activation == "025b7131-e623-4d72-9dc0-1972f04a2352" { "be9843b19071f954208ec9ece000d4b7a502c564521980ce8bacb715e45cddb7" } else if prior_activation == "41f4e58c-8870-481b-9b09-339eeb1ca155" { "bec3a9a421a05cf3550ed809486e9ea466b62e1664b33d72a2358203c4f0fdae" } else { "d129055fa1e634714e0da860b9c5ed265723b06002fe858c6bebdf9ae6e7cdc8" };
        anyhow::ensure!(process["activationId"].as_str()==Some(prior_activation.as_str()) && process["exeSha256"].as_str()==Some(expected_exe), "prior executable provenance differs");
        anyhow::ensure!(state.control.current_work_owner(&udid).is_none() && state.control.cleanup_quarantine_count()==0, "current plane not idle");
        let fence_path = scope.fence_root.join(format!("{}.json",riviu_core::frame_sha256(udid.as_bytes())));
        let fence: serde_json::Value = serde_json::from_slice(&std::fs::read(&fence_path)?)?;
        anyhow::ensure!(fence["activationId"].as_str()==Some(prior_activation.as_str()) && fence["requestId"].as_str()==Some(prior_request.as_str()) && fence["udid"].as_str()==Some(udid.as_str()), "prior device fence identity changed");
        let _admission = state.ensure_accepting_work().map_err(|e|anyhow::anyhow!(e.message))?;
        let exclusive = state.control.try_acquire_exclusive_keeping_stream(&udid, riviu_core::DeviceWorkOwner::Repair).await?;
        let android = state.android.as_ref().context("Android required")?;
        if let Err(error) = android.prove_diagnostic_setup_idle(&udid).await {
            state.control.close_exclusive_context(exclusive)?;
            return Err(error);
        }
        let receipt = serde_json::json!({"udid":udid,"priorActivation":prior_activation,"priorRequest":prior_request,"state":"setupReconciled","reviewedExactRequest":true,"sessionAbsent":true,"helperNotOpenedByPriorRun":true,"streamAbsent":true,"publicEffectsAllowed":false});
        use std::io::Write;
        let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(prior_dir.join("setup-reconciliation.json"))?;
        file.write_all(&serde_json::to_vec_pretty(&receipt)?)?;file.sync_all()?;
        let unchanged: serde_json::Value = serde_json::from_slice(&std::fs::read(&fence_path)?)?;
        anyhow::ensure!(unchanged == fence, "prior fence changed during reconciliation");
        android.settle_diagnostic_setup_reservation(&udid);
        std::fs::rename(fence_path, prior_dir.join("reconciled-device-fence.json"))?;
        state.control.close_exclusive_context(exclusive)?;
        Ok::<_,anyhow::Error>(receipt)
    }.await;
    result.map_err(CommandError::operation)
}

#[tauri::command]
pub(crate) async fn no_public_shutdown(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    activation_id: String,
) -> Result<serde_json::Value, CommandError> {
    use tauri::Manager;
    let policy = app.state::<crate::ui_smoke::StartupPolicy>();
    let scope = policy.rehearsal().ok_or_else(||CommandError::code("NoPublicModeRequired", "diagnostic required"))?;
    if scope.activation != activation_id { return Err(CommandError::code("ActivationMismatch", "wrong diagnostic activation")); }
    state.shutdown_ui_smoke().await.map_err(CommandError::operation)?;
    let reply = serde_json::json!({"activationId":activation_id,"cleanupCompleted":true,"publicEffectsAllowed":false});
    let exit = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        exit.exit(0);
    });
    Ok(reply)
}

#[tauri::command]
pub(crate) async fn no_public_metadata(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    udid: String,
) -> Result<serde_json::Value, CommandError> {
    use tauri::Manager;
    let policy = app.state::<crate::ui_smoke::StartupPolicy>();
    let scope = policy.rehearsal().ok_or_else(||CommandError::code("NoPublicModeRequired", "diagnostic required"))?;
    let device = scope.device(&udid).map_err(CommandError::operation)?;
    let (package, version, locale) = state.control.tiktok_build(&udid).await.map_err(CommandError::from)?;
    Ok(serde_json::json!({"deviceId":udid,"expectedPackage":device.package,"package":package,"version":version,"locale":locale,"scopeMatch":package==device.package,"publicEffectsAllowed":false,"deviceInputSent":false}))
}

#[tauri::command]
pub(crate) fn no_public_run_status(
    state: tauri::State<'_, AppState>,
    request_id: String,
) -> Option<serde_json::Value> {
    state.rehearsal_runs.status(&request_id)
}
#[tauri::command]
pub(crate) fn no_public_cancel(
    state: tauri::State<'_, AppState>,
    request_id: String,
) -> Result<serde_json::Value, CommandError> {
    state
        .rehearsal_runs
        .cancel(&request_id)
        .map_err(CommandError::operation)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_never_implicitly_authorizes_helper_setup() {
        let device: DeviceScope = serde_json::from_value(serde_json::json!({"udid":"fixture","package":"com.zhiliaoapp.musically","expectedAccount":"","targetUrl":null,"draftText":null,"sourceRoot":null,"bundleId":null})).unwrap();
        assert!(!device.helper_canary, "discovery must not inherit helper preparation authority");
    }
    #[test]
    fn public_legacy_and_plugin_operations_are_denied() {
        for command in [
            "publish_start",
            "publish_execute",
            "publish_retry_assignment",
            "interaction_start_thread",
            "nurture_start",
            "flow_run",
            "device_type_text",
            "inspector_v2",
            "agent_repair",
            "google_sheets_connect",
            "plugin:updater|install",
        ] {
            assert!(!read_command(command), "{command}");
        }
        assert!(read_command("no_public_prepare_publish"));
        assert!(read_command("no_public_prepare_interaction"));
    }
}
