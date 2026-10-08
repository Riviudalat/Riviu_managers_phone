//! Uninstall, or uninstall-then-reinstall, one app-library artifact on operator-chosen phones.
//!
//! The app is always named by a library row, never by a package string from the UI: the
//! package/bundle id comes from the imported artifact's own metadata, so this path can only
//! remove something the operator once added to the library. Riviu's own helper and agent
//! packages are refused even if somebody imported them.
//!
//! Uninstall is destructive and not idempotent in effect (the app's data and logged-in
//! account go with it), so every phone gets exactly one attempt:
//!
//! - the phone is taken with `try_acquire_exclusive`, which refuses a phone that is busy;
//!   it never waits behind, or preempts, publish/nurture/interaction work;
//! - a phone with a post still uploading or waiting for verification is refused as busy,
//!   because removing TikTok there would destroy the evidence that post still needs;
//! - the app list is read before dispatch, and an unreadable list refuses before any effect;
//! - the intent is written to the op log before the uninstall is sent;
//! - `Done` needs a readback that no longer lists the app. Anything that happened after the
//!   uninstall was sent and is not proven is `UnknownAfterDispatch`, and nothing here retries.
//!
//! Reinstall holds the same lease from preparation through install, so no other owner can
//! pick the phone up between "app removed" and "library artifact installed".

use super::{
    collect_bounded, err, log, materialize_android_install_set_for_spec,
    snapshot_managed_app_artifact, validate_install_set_identity, BatchScratch,
    MaterializedInstallSet, MAX_INSTALL_CONCURRENCY,
};
use crate::command_error::CommandError;
use crate::state::AppState;
use riviu_core::db::Database;
use riviu_core::{
    AndroidInstallDeviceSpec, AppInstallResult, AppInstallStatus, AppLibraryItem,
    AppLibraryPlatform, AppPackageFormat, DeviceAppInstallRequest, DeviceControlError,
    DeviceControlPlane, DeviceExclusiveContext, DevicePlatform, DeviceWorkOwner, InstallEffectGate,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tauri::State;
use uuid::Uuid;

/// What the operator asked for. Reinstall is uninstall followed by the exact library artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppRemovalMode {
    Uninstall,
    Reinstall,
}

/// Per-phone terminal state.
///
/// `RefusedBusy` and `FailedBeforeEffect` both mean nothing was sent to the phone.
/// `UnknownAfterDispatch` means something was sent and the outcome is not proven; it is
/// reported for the operator to check and is never retried automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppRemovalOutcome {
    Done,
    RefusedBusy,
    FailedBeforeEffect,
    UnknownAfterDispatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppRemovalRequest {
    pub app_id: String,
    pub udids: Vec<String>,
    pub mode: AppRemovalMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppRemovalResult {
    pub udid: String,
    pub outcome: AppRemovalOutcome,
    /// True only when an app list read after the uninstall (or before it, when the app was
    /// already absent) did not contain the library package.
    pub removed_verified: bool,
    /// The install half of a reinstall, when it was reached.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install: Option<AppInstallResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppRemovalBatchResponse {
    pub app_id: String,
    pub mode: AppRemovalMode,
    pub results: Vec<AppRemovalResult>,
}

/// Packages that are Riviu's own control path on the phone. Removing one would cut the
/// controller off the phone it is supposed to be driving.
///
/// Sources: `com.riviu.agent` (crates/android-driver/src/riviu_agent.rs), the UiAutomator2
/// server and its `.test` runner (crates/android-driver/src/driver/mod.rs `AGENT_PACKAGE`),
/// and the iPhone agent `com.riviu.managersphone.agent.xctrunner` (crates/ios-driver/src/wda.rs)
/// plus the stock WebDriverAgent runner name.
const PROTECTED_PACKAGE_ROOTS: &[&str] = &[
    "com.riviu",
    "io.appium.uiautomator2",
    "com.facebook.WebDriverAgentRunner",
];

/// The package/bundle id this library row installs, checked before any phone is touched.
pub(crate) fn removal_package_id(item: &AppLibraryItem) -> Result<String, CommandError> {
    let (primary, fallback) = match item.platform {
        AppLibraryPlatform::Android => (&item.application_id, &item.bundle_id),
        AppLibraryPlatform::Ios => (&item.bundle_id, &item.application_id),
    };
    let id = if primary.trim().is_empty() {
        fallback.trim()
    } else {
        primary.trim()
    };
    if id.is_empty() {
        return Err(CommandError::code(
            "AppIdentityMissing",
            format!(
                "Gói {} trong thư viện chưa có mã ứng dụng; không gỡ được.",
                item.name
            ),
        ));
    }
    let well_formed = id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && id.chars().next().is_some_and(|c| c.is_ascii_alphabetic());
    if !well_formed {
        return Err(CommandError::code(
            "AppIdentityInvalid",
            format!(
                "Mã ứng dụng \"{id}\" của gói {} không hợp lệ; không gỡ.",
                item.name
            ),
        ));
    }
    let lowered = id.to_ascii_lowercase();
    if PROTECTED_PACKAGE_ROOTS.iter().any(|root| {
        let root = root.to_ascii_lowercase();
        lowered == root || lowered.starts_with(&format!("{root}."))
    }) {
        return Err(CommandError::code(
            "AppProtected",
            format!(
                "{id} là thành phần Riviu dùng để điều khiển máy; không gỡ hoặc cài lại từ thư viện."
            ),
        ));
    }
    Ok(id.to_string())
}

/// One phone's share of a removal batch. `item.path` must already be the batch snapshot of the
/// library artifact when `mode` is `Reinstall`.
pub(crate) struct RemovalPlan<'a> {
    pub item: &'a AppLibraryItem,
    pub package: &'a str,
    pub mode: AppRemovalMode,
    pub scratch: Option<&'a Path>,
}

fn outcome(
    udid: &str,
    outcome: AppRemovalOutcome,
    removed_verified: bool,
    install: Option<AppInstallResult>,
    detail: impl Into<String>,
) -> AppRemovalResult {
    AppRemovalResult {
        udid: udid.to_string(),
        outcome,
        removed_verified,
        install,
        detail: Some(detail.into()),
    }
}

fn package_listed(apps: &[riviu_core::InstalledApp], package: &str) -> bool {
    apps.iter().any(|app| app.bundle_id == package)
}

enum PreparedInstall {
    Android(MaterializedInstallSet),
    Ios(PathBuf),
}

/// Everything the install half needs, built inside the lease and before the uninstall, so a
/// broken artifact or a phone whose spec cannot be read stops the reinstall with the app intact.
async fn prepare_reinstall(
    control: &DeviceControlPlane,
    context: &DeviceExclusiveContext,
    plan: &RemovalPlan<'_>,
) -> Result<PreparedInstall, String> {
    let item = plan.item;
    let source = PathBuf::from(&item.path);
    if item.platform == AppLibraryPlatform::Ios {
        return Ok(PreparedInstall::Ios(source));
    }
    let scratch = plan
        .scratch
        .ok_or_else(|| "chưa có thư mục tạm cho gói cài lại".to_string())?;
    let spec = if item.package_format == AppPackageFormat::Apk {
        AndroidInstallDeviceSpec {
            sdk_version: 0,
            supported_abis: Vec::new(),
            screen_density: 0,
            supported_locales: Vec::new(),
        }
    } else {
        control
            .android_install_device_spec(context)
            .await
            .map_err(|error| format!("chưa đọc được cấu hình máy: {error}"))?
    };
    let set = if item.package_format == AppPackageFormat::Apks {
        let root = scratch.join(format!("apks-{}", Uuid::new_v4()));
        match control
            .extract_app_container_for_spec(context, &source, &spec, &root)
            .await
        {
            Ok(paths) => MaterializedInstallSet {
                root: Some(root),
                paths,
            },
            Err(error) => {
                let _ = std::fs::remove_dir_all(&root);
                return Err(error.to_string());
            }
        }
    } else {
        materialize_android_install_set_for_spec(&source, item.package_format, scratch, &spec)
            .map_err(|error| error.message.to_string())?
    };
    validate_install_set_identity(&set.paths, item).map_err(|error| error.message.to_string())?;
    Ok(PreparedInstall::Android(set))
}

/// Run one uninstall or reinstall on one phone. Never retries a dispatched effect.
pub(crate) async fn remove_on_device(
    control: &DeviceControlPlane,
    db: &Database,
    plan: &RemovalPlan<'_>,
    udid: String,
) -> AppRemovalResult {
    use AppRemovalOutcome::*;
    let package = plan.package;
    let context = match control
        .try_acquire_exclusive(&udid, DeviceWorkOwner::Repair)
        .await
    {
        Ok(context) => context,
        Err(DeviceControlError::Busy(busy)) => {
            return outcome(
                &udid,
                RefusedBusy,
                false,
                None,
                format!(
                    "Máy đang bận ({:?}); chưa gỡ, không chờ và không chen ngang việc đang chạy.",
                    busy.current_owner
                ),
            );
        }
        Err(DeviceControlError::PendingPublication { reason, .. }) => {
            return outcome(&udid, RefusedBusy, false, None, reason);
        }
        Err(error) => {
            return outcome(
                &udid,
                FailedBeforeEffect,
                false,
                None,
                format!("Chưa giữ được máy; chưa gỡ: {error}"),
            );
        }
    };
    match db.has_pending_publish_for_device(&udid) {
        Ok(false) => {}
        Ok(true) => {
            return outcome(
                &udid,
                RefusedBusy,
                false,
                None,
                "Máy còn bài đang tải hoặc chờ xác minh; chưa gỡ để không mất bằng chứng bài đăng.",
            );
        }
        Err(error) => {
            return outcome(
                &udid,
                FailedBeforeEffect,
                false,
                None,
                format!("Chưa đọc được trạng thái bài đang xử lý; chưa gỡ: {error}"),
            );
        }
    }
    let prepared = if plan.mode == AppRemovalMode::Reinstall {
        match prepare_reinstall(control, &context, plan).await {
            Ok(prepared) => Some(prepared),
            Err(detail) => {
                return outcome(
                    &udid,
                    FailedBeforeEffect,
                    false,
                    None,
                    format!("Chưa chuẩn bị được gói cài lại; chưa gỡ: {detail}"),
                );
            }
        }
    } else {
        None
    };
    let present = match control.list_installed_apps(&udid).await {
        Ok(apps) => package_listed(&apps, package),
        Err(error) => {
            return outcome(
                &udid,
                FailedBeforeEffect,
                false,
                None,
                format!("Chưa đọc được danh sách ứng dụng trên máy; chưa gỡ: {error}"),
            );
        }
    };
    let note = if present {
        if let Err(error) = db.log_op("app.uninstall.dispatch", &format!("{udid} {package}")) {
            return outcome(
                &udid,
                FailedBeforeEffect,
                false,
                None,
                format!("Chưa ghi được ý định gỡ; chưa gửi lệnh tới máy: {error}"),
            );
        }
        if let Err(error) = control.uninstall_app(&context, package).await {
            return outcome(
                &udid,
                UnknownAfterDispatch,
                false,
                None,
                format!("Đã gửi lệnh gỡ {package} nhưng không nhận được xác nhận; cần kiểm lại trên máy, không tự gỡ lại: {error}"),
            );
        }
        match control.list_installed_apps(&udid).await {
            Ok(apps) if !package_listed(&apps, package) => {}
            Ok(_) => {
                return outcome(
                    &udid,
                    UnknownAfterDispatch,
                    false,
                    None,
                    format!("Lệnh gỡ đã chạy nhưng {package} vẫn còn trong danh sách ứng dụng; cần kiểm lại trên máy."),
                );
            }
            Err(error) => {
                return outcome(
                    &udid,
                    UnknownAfterDispatch,
                    false,
                    None,
                    format!("Đã gửi lệnh gỡ nhưng chưa đọc lại được danh sách ứng dụng; cần kiểm lại: {error}"),
                );
            }
        }
        None
    } else {
        Some(format!("{package} không có trên máy; không cần gỡ."))
    };
    let Some(prepared) = prepared else {
        return AppRemovalResult {
            udid,
            outcome: Done,
            removed_verified: true,
            install: None,
            detail: note,
        };
    };
    if let Err(error) = db.log_op("app.reinstall.dispatch", &format!("{udid} {package}")) {
        return outcome(
            &udid,
            UnknownAfterDispatch,
            true,
            None,
            format!("Đã gỡ {package} nhưng chưa ghi được ý định cài lại nên chưa cài; cần cài lại thủ công: {error}"),
        );
    }
    let install = match prepared {
        PreparedInstall::Android(set) => {
            let gate = InstallEffectGate::new();
            let request = DeviceAppInstallRequest {
                apk_paths: set.paths.clone(),
                application_id: plan.item.application_id.clone(),
                version_name: plan.item.version_name.clone(),
                version_code: plan.item.version_code.clone(),
                allow_downgrade: false,
                effect_gate: Some(gate.clone()),
            };
            match control.install_app_set_checked(&context, &request).await {
                Ok(result) => result,
                Err(error) => AppInstallResult {
                    udid: udid.clone(),
                    status: if gate.effect_claimed() {
                        AppInstallStatus::Uncertain
                    } else {
                        AppInstallStatus::BeforeEffect
                    },
                    effect_started: gate.effect_claimed(),
                    observed_version_name: None,
                    observed_version_code: None,
                    detail: Some(error.to_string()),
                },
            }
        }
        PreparedInstall::Ios(path) => match control.install_app(&context, &path).await {
            Ok(()) => AppInstallResult {
                udid: udid.clone(),
                status: AppInstallStatus::Succeeded,
                effect_started: true,
                observed_version_name: None,
                observed_version_code: None,
                detail: None,
            },
            Err(error) => AppInstallResult {
                udid: udid.clone(),
                status: AppInstallStatus::Uncertain,
                effect_started: true,
                observed_version_name: None,
                observed_version_code: None,
                detail: Some(error.to_string()),
            },
        },
    };
    if install.status == AppInstallStatus::Succeeded {
        AppRemovalResult {
            udid,
            outcome: Done,
            removed_verified: true,
            install: Some(install),
            detail: note,
        }
    } else {
        let detail = format!(
            "Đã gỡ {package} nhưng cài lại chưa được xác nhận ({:?}); cần kiểm lại trên máy, không tự cài lại: {}",
            install.status,
            install.detail.as_deref().unwrap_or("không có chi tiết")
        );
        outcome(&udid, UnknownAfterDispatch, true, Some(install), detail)
    }
}

async fn run_removal_batch(
    state: &AppState,
    request: AppRemovalRequest,
) -> Result<AppRemovalBatchResponse, CommandError> {
    if request.app_id.trim().is_empty() || request.udids.is_empty() {
        return Err(CommandError::invalid_argument(
            "Cần chọn một ứng dụng trong thư viện và ít nhất một máy.",
        ));
    }
    let unique = request.udids.iter().collect::<HashSet<_>>();
    if unique.len() != request.udids.len() {
        return Err(CommandError::invalid_argument(
            "Một lượt gỡ không được lặp lại cùng một máy.",
        ));
    }
    let item = state
        .db
        .list_apps_library()
        .map_err(err)?
        .into_iter()
        .find(|item| item.id == request.app_id)
        .ok_or_else(|| CommandError::code("AppNotFound", "Ứng dụng không còn trong thư viện."))?;
    let package = removal_package_id(&item)?;
    let batch_id = format!("app-{:?}-{}", request.mode, Uuid::new_v4()).to_ascii_lowercase();
    let (scratch, item) = if request.mode == AppRemovalMode::Reinstall {
        let scratch = BatchScratch::create(&state.artifacts_dir, &batch_id)?;
        let source = snapshot_managed_app_artifact(&item, scratch.path())?;
        let item = AppLibraryItem {
            path: source.display().to_string(),
            ..item
        };
        (Some(scratch), item)
    } else {
        (None, item)
    };
    let expected = match item.platform {
        AppLibraryPlatform::Ios => DevicePlatform::Ios,
        AppLibraryPlatform::Android => DevicePlatform::Android,
    };
    let (connected, roster_error) = match state.control.list_devices().await {
        Ok(devices) => (devices, None),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    let mut blocked = HashMap::<String, AppRemovalResult>::new();
    for udid in &request.udids {
        let detail = match roster_error.as_ref() {
            Some(error) => Some(format!("Chưa đọc được danh sách máy: {error}")),
            None => match connected.iter().find(|device| &device.udid == udid) {
                None => Some("Máy chưa kết nối; chưa gỡ.".to_string()),
                Some(device) if device.platform != expected => {
                    Some(format!("Máy khác nền tảng với gói {}; chưa gỡ.", item.name))
                }
                Some(_) => None,
            },
        };
        if let Some(detail) = detail {
            blocked.insert(
                udid.clone(),
                outcome(
                    udid,
                    AppRemovalOutcome::FailedBeforeEffect,
                    false,
                    None,
                    detail,
                ),
            );
        }
    }
    let plan = RemovalPlan {
        item: &item,
        package: &package,
        mode: request.mode,
        scratch: scratch.as_ref().map(BatchScratch::path),
    };
    let plan = &plan;
    let blocked = &blocked;
    let mut results = collect_bounded(
        request.udids.iter().cloned(),
        MAX_INSTALL_CONCURRENCY,
        |udid| async move {
            match blocked.get(&udid) {
                Some(result) => result.clone(),
                None => remove_on_device(&state.control, &state.db, plan, udid).await,
            }
        },
    )
    .await;
    let order = request
        .udids
        .iter()
        .enumerate()
        .map(|(index, udid)| (udid.as_str(), index))
        .collect::<HashMap<_, _>>();
    results.sort_by_key(|result| order.get(result.udid.as_str()).copied());
    let done = results
        .iter()
        .filter(|result| result.outcome == AppRemovalOutcome::Done)
        .count();
    log(
        state,
        match request.mode {
            AppRemovalMode::Uninstall => "app.uninstall.batch",
            AppRemovalMode::Reinstall => "app.reinstall.batch",
        },
        &format!("{batch_id} {package} {done}/{}", results.len()),
    );
    drop(scratch);
    Ok(AppRemovalBatchResponse {
        app_id: request.app_id,
        mode: request.mode,
        results,
    })
}

/// Uninstall (or reinstall) one library app on one phone.
#[tauri::command]
pub async fn uninstall_library_app(
    state: State<'_, AppState>,
    udid: String,
    app_id: String,
    mode: AppRemovalMode,
) -> Result<AppRemovalResult, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    run_removal_batch(
        &state,
        AppRemovalRequest {
            app_id,
            udids: vec![udid],
            mode,
        },
    )
    .await?
    .results
    .into_iter()
    .next()
    .ok_or_else(|| CommandError::operation("removal batch returned no device result"))
}

/// Uninstall (or reinstall) one library app across a group, one lease and one outcome per phone.
#[tauri::command]
pub async fn uninstall_library_app_to_group(
    state: State<'_, AppState>,
    group_id: String,
    app_id: String,
    mode: AppRemovalMode,
) -> Result<AppRemovalBatchResponse, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let group = state
        .db
        .list_groups()
        .map_err(err)?
        .into_iter()
        .find(|group| group.id == group_id)
        .ok_or_else(|| {
            CommandError::code("GroupNotFound", format!("Không tìm thấy nhóm {group_id}."))
        })?;
    let mut seen = HashSet::new();
    let udids = group
        .udids
        .into_iter()
        .filter(|udid| seen.insert(udid.clone()))
        .collect::<Vec<_>>();
    if udids.is_empty() {
        return Err(CommandError::invalid_argument("Nhóm chưa có máy nào."));
    }
    run_removal_batch(
        &state,
        AppRemovalRequest {
            app_id,
            udids,
            mode,
        },
    )
    .await
}

/// Uninstall (or reinstall) one library app on an explicit list of phones.
#[tauri::command]
pub async fn uninstall_library_app_batch(
    state: State<'_, AppState>,
    request: AppRemovalRequest,
) -> Result<AppRemovalBatchResponse, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    run_removal_batch(&state, request).await
}

#[cfg(test)]
#[path = "app_removal_tests.rs"]
mod tests;
