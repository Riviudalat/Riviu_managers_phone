#[cfg(test)]
use std::path::PathBuf;

#[cfg(test)]
use riviu_core::db::Database;
#[cfg(test)]
use riviu_core::DeviceDriver;
use riviu_core::{
    AgentSettings, AgentStatus, DeviceControlError, DeviceControlPlane, DeviceWorkOwner,
};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::command_error::CommandError;
use crate::state::AppState;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperMaintenancePlan {
    pub maintenance_id: String,
    pub udid: String,
    pub submitted: Vec<riviu_core::db::HelperMaintenanceSubmittedBinding>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum HelperMaintenancePhase {
    Prepared,
    ExecutionStarted,
    Settled,
}

impl HelperMaintenancePhase {
    fn restore_pre_dispatch_busy(&mut self, observation_only: bool, error: &DeviceControlError) -> bool {
        // The driver emits this exact category only before any maintenance fence
        // or external dispatch. An unknown error must never authorize replay.
        if !observation_only && *self == Self::ExecutionStarted
            && matches!(error, DeviceControlError::Driver {
                operation: "executeHelperMaintenance", message, ..
            } if message == "HelperMaintenanceBusyBeforeDispatch")
        {
            *self = Self::Prepared;
            return true;
        }
        false
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreparedHelperMaintenance {
    public: HelperMaintenancePlan,
    driver_plan: serde_json::Value,
    prepared_at_ms: i64,
    phase: HelperMaintenancePhase,
}

// Full snapshots and exact driver plans stay durable and off IPC. Retain settled
// entries too: supersession never erases the previous maintenance identity.
fn maintenance_key(udid: &str) -> String {
    format!("helper.maintenance.ledger:{udid}")
}

fn load_maintenance_ledger(
    db: &riviu_core::db::Database,
    udid: &str,
) -> Result<Vec<PreparedHelperMaintenance>, CommandError> {
    db.get_setting(&maintenance_key(udid))
        .map_err(|_| CommandError::operation("cannot read helper maintenance ledger"))?
        .map(|raw| serde_json::from_str(&raw)
            .map_err(|_| CommandError::operation("invalid helper maintenance ledger; retained unchanged")))
        .transpose()
        .map(|ledger| ledger.unwrap_or_default())
}

async fn save_maintenance_ledger(
    db: &std::sync::Arc<riviu_core::db::Database>,
    udid: &str,
    ledger: &[PreparedHelperMaintenance],
) -> Result<(), CommandError> {
    let key = maintenance_key(udid);
    let raw = serde_json::to_string(ledger).map_err(err)?;
    db.storage_write(move |db| db.set_setting(&key, &raw)).await
        .map_err(|_| CommandError::operation("cannot persist helper maintenance phase; do not redispatch"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HelperMaintenancePending {
    pub plan: HelperMaintenancePlan,
    pub observation_only: bool,
}

#[tauri::command]
pub fn agent_helper_maintenance_pending(
    state: State<'_, AppState>,
    udid: String,
) -> Result<Option<HelperMaintenancePending>, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    Ok(load_maintenance_ledger(&state.db, &udid)?.into_iter()
        .find(|entry| entry.phase != HelperMaintenancePhase::Settled)
        .map(|entry| HelperMaintenancePending {
            observation_only: entry.phase != HelperMaintenancePhase::Prepared,
            plan: entry.public,
        }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HelperMaintenanceReceipt {
    pub maintenance_id: String,
    pub udid: String,
    pub state: &'static str,
    pub records_retained: bool,
    pub exact_release_proved: bool,
    pub clipboard_restoration_proved: bool,
    pub old_obligations: &'static str,
    pub package_process_absent: bool,
}

#[tauri::command]
pub async fn agent_helper_maintenance_prepare(
    state: State<'_, AppState>,
    udid: String,
) -> Result<HelperMaintenancePlan, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    if udid.trim().is_empty() {
        return Err(CommandError::invalid_argument("a device serial is required"));
    }
    let context = state.control
        .try_acquire_exclusive(&udid, DeviceWorkOwner::Repair)
        .await?;
    prepare_helper_maintenance_admitted(&state.control, &state.db, &context, udid).await
}

async fn prepare_helper_maintenance_admitted(
    control: &DeviceControlPlane,
    db: &std::sync::Arc<riviu_core::db::Database>,
    context: &riviu_core::DeviceExclusiveContext,
    udid: String,
) -> Result<HelperMaintenancePlan, CommandError> {
    let mut ledger = load_maintenance_ledger(db, &udid)?;
    if ledger.iter().any(|entry| entry.phase != HelperMaintenancePhase::Settled) {
        return Err(CommandError::code("HelperMaintenancePending",
            "Kế hoạch cũ chưa giải quyết; đọc pending và chỉ quan sát cùng maintenanceId."));
    }
    let submitted = db.helper_maintenance_submitted_bindings(&udid)
        .map_err(|_| CommandError::operation("cannot freeze submitted publication bindings"))?;
    let plan = HelperMaintenancePlan {
        maintenance_id: uuid::Uuid::new_v4().to_string(),
        udid,
        submitted,
    };
    use sha2::{Digest, Sha256};
    let snapshot = serde_json::to_vec(&plan.submitted).map_err(err)?;
    let effect_intent = serde_json::json!({
        "maintenanceId": plan.maintenance_id,
        "udid": plan.udid,
        "submittedCount": plan.submitted.len(),
        "submittedSetSha256": format!("{:x}", Sha256::digest(&snapshot)),
    });
    let driver_plan = control
        .prepare_helper_maintenance(context, &plan.maintenance_id, effect_intent)
        .await
        .map_err(helper_maintenance_prepare_error)?;
    ledger.push(PreparedHelperMaintenance {
        public: plan.clone(),
        driver_plan,
        prepared_at_ms: chrono::Utc::now().timestamp_millis(),
        phase: HelperMaintenancePhase::Prepared,
    });
    save_maintenance_ledger(db, &plan.udid, &ledger).await?;
    Ok(plan)
}

#[tauri::command]
pub async fn agent_helper_maintenance_execute(
    state: State<'_, AppState>,
    plan: HelperMaintenancePlan,
    confirmed: bool,
    reconcile_only: Option<bool>,
) -> Result<HelperMaintenanceReceipt, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    if !confirmed {
        return Err(CommandError::code("HelperMaintenanceConfirmationRequired",
            "Cần xác nhận rõ ràng trước khi khôi phục helper."));
    }
    let context = state.control
        .try_acquire_exclusive(&plan.udid, DeviceWorkOwner::Repair)
        .await?;
    execute_helper_maintenance_admitted(&state.control, &state.db, &context, plan, confirmed, reconcile_only).await
}

async fn execute_helper_maintenance_admitted(
    control: &DeviceControlPlane,
    db: &std::sync::Arc<riviu_core::db::Database>,
    context: &riviu_core::DeviceExclusiveContext,
    plan: HelperMaintenancePlan,
    confirmed: bool,
    reconcile_only: Option<bool>,
) -> Result<HelperMaintenanceReceipt, CommandError> {
    let mut ledger = load_maintenance_ledger(db, &plan.udid)?;
    let index = ledger.iter().position(|entry| entry.public.maintenance_id == plan.maintenance_id)
        .ok_or_else(|| CommandError::code("HelperMaintenancePlanMissing",
            "Không có kế hoạch bền vững cho maintenanceId này; không thực thi."))?;
    if ledger[index].public != plan {
        return Err(CommandError::code("HelperMaintenancePlanChanged",
            "Kế hoạch không khớp bản đã lưu; không chấp nhận thay đổi danh tính."));
    }
    let current = db.helper_maintenance_submitted_bindings(&plan.udid)
        .map_err(|_| CommandError::operation("cannot recheck submitted publication bindings"))?;
    if current != ledger[index].public.submitted {
        // Only redacted DB identities/revisions/hashes, never raw intent or driver metadata.
        let expected = &ledger[index].public.submitted;
        let offset = expected.iter().zip(&current).position(|(a, b)| a != b)
            .unwrap_or(expected.len().min(current.len()));
        return Err(CommandError::code("HelperMaintenanceBindingsChanged", format!(
            "maintenanceId={} stale submitted binding at index {}: expected={}, current={}; giữ nguyên kế hoạch, không tạo effect mới",
            plan.maintenance_id, offset,
            serde_json::to_string(&expected.get(offset)).map_err(err)?,
            serde_json::to_string(&current.get(offset)).map_err(err)?,
        )));
    }
    let observation_only = reconcile_only.unwrap_or(false)
        || ledger[index].phase != HelperMaintenancePhase::Prepared;
    // Persist BEFORE awaiting the driver. A crash here may prevent the initial
    // effect, but can never authorize a second stop after an uncertain dispatch.
    if ledger[index].phase == HelperMaintenancePhase::Prepared {
        ledger[index].phase = HelperMaintenancePhase::ExecutionStarted;
        save_maintenance_ledger(db, &plan.udid, &ledger).await?;
    }
    let prepared = &ledger[index];
    let execution = control
        .execute_helper_maintenance(context, prepared.driver_plan.clone(), confirmed, observation_only)
        .await;
    let receipt = match execution {
        Ok(receipt) => receipt,
        Err(error) => {
            if ledger[index].phase.restore_pre_dispatch_busy(observation_only, &error) {
                save_maintenance_ledger(db, &plan.udid, &ledger).await?;
                return Err(CommandError::code("HelperMaintenanceBusyBeforeDispatch",
                    "Máy đang kết thúc thao tác helper; chưa thực hiện khôi phục. Đợi thao tác kết thúc rồi chọn Khôi phục helper để tiếp tục đúng kế hoạch."));
            }
            return Err(helper_maintenance_execution_error(&plan.udid, &error));
        }
    };
    let prepared = &ledger[index];
    // Validate the exact driver contract, then expose only nonsecret summary fields.
    // The receipt's plan contains owner/nonce/journal metadata and stays off IPC.
    if receipt["plan"] != prepared.driver_plan
        || receipt["state"] != "superseded"
        || receipt["exactReleaseProved"] != false
        || receipt["clipboardRestorationProved"] != false
        || receipt["oldObligations"] != "archivedUnresolved"
        || receipt["packageProcessAbsent"] != true
    {
        return Err(CommandError::code("HelperMaintenanceReceiptUnproved",
            "Biên nhận helper không khớp kế hoạch; không tự chạy lại hoặc báo đã khôi phục."));
    }
    ledger[index].phase = HelperMaintenancePhase::Settled;
    save_maintenance_ledger(db, &plan.udid, &ledger).await?;
    Ok(HelperMaintenanceReceipt {
        maintenance_id: plan.maintenance_id,
        udid: plan.udid,
        state: "superseded",
        records_retained: true,
        exact_release_proved: false,
        clipboard_restoration_proved: false,
        old_obligations: "archivedUnresolved",
        package_process_absent: true,
    })
}

pub(crate) fn helper_maintenance_is_pending(
    db: &riviu_core::db::Database,
    serial: &str,
) -> Result<bool, CommandError> {
    Ok(load_maintenance_ledger(db, serial)?.iter()
        .any(|entry| entry.phase != HelperMaintenancePhase::Settled))
}

/// User-authorized automatic policy, under the caller's exclusive Repair lease.
/// A pending identity is resumed, never replaced; executionStarted is observation-only.
pub(crate) async fn automatic_helper_maintenance(
    control: &DeviceControlPlane,
    db: &std::sync::Arc<riviu_core::db::Database>,
    context: &riviu_core::DeviceExclusiveContext,
    serial: &str,
) -> Result<(), CommandError> {
    let pending = load_maintenance_ledger(db, serial)?.into_iter()
        .find(|entry| entry.phase != HelperMaintenancePhase::Settled);
    let plan = match pending {
        Some(entry) => entry.public,
        None => prepare_helper_maintenance_admitted(control, db, context, serial.into()).await?,
    };
    execute_helper_maintenance_admitted(control, db, context, plan, true, None).await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentRuntimeView {
    pub settings: AgentSettings,
    pub token_configured: bool,
    pub active_artifact_id: String,
    pub active_artifact_version: String,
}

fn build_runtime_view(
    settings: AgentSettings,
    token_configured: bool,
    active_artifact_id: &str,
    active_artifact_version: &str,
) -> AgentRuntimeView {
    AgentRuntimeView {
        settings,
        token_configured,
        active_artifact_id: active_artifact_id.to_string(),
        active_artifact_version: active_artifact_version.to_string(),
    }
}

#[cfg(test)]
fn save_settings_with_driver(
    database: &Database,
    driver: &dyn DeviceDriver,
    settings: AgentSettings,
    token_configured: bool,
    active_artifact_id: &str,
    active_artifact_version: &str,
) -> anyhow::Result<AgentRuntimeView> {
    database.save_agent_settings(&settings)?;
    driver.set_agent_settings(settings.clone());
    Ok(build_runtime_view(
        settings,
        token_configured,
        active_artifact_id,
        active_artifact_version,
    ))
}

#[cfg(test)]
async fn repair_with_driver(driver: &dyn DeviceDriver, udid: &str) -> anyhow::Result<AgentStatus> {
    driver.repair_agent_install_only(udid).await?;
    Ok(driver.cached_agent_status(udid))
}

#[cfg(test)]
async fn bulk_repair_with_driver(
    driver: &dyn DeviceDriver,
    udids: Vec<String>,
) -> Vec<AgentStatus> {
    let mut statuses = Vec::with_capacity(udids.len());
    for udid in udids {
        let status = match repair_with_driver(driver, &udid).await {
            Ok(status) => status,
            Err(_) => {
                let mut status = driver.cached_agent_status(&udid);
                status.state = riviu_core::AgentState::Error;
                status.auth_ready = false;
                status.mjpeg_ready = false;
                status.session_ready = false;
                status
                    .message
                    .get_or_insert_with(|| "Agent repair failed".to_string());
                status
            }
        };
        statuses.push(status);
    }
    statuses
}

async fn preflight_with_control(
    control: &DeviceControlPlane,
    udid: &str,
) -> Result<AgentStatus, DeviceControlError> {
    let context = control
        .try_acquire_exclusive(udid, DeviceWorkOwner::Repair)
        .await?;
    control.preflight_agent(&context).await
}

async fn repair_with_control(
    control: &DeviceControlPlane,
    udid: &str,
) -> Result<AgentStatus, DeviceControlError> {
    let context = control
        .try_acquire_exclusive(udid, DeviceWorkOwner::Repair)
        .await?;
    control.repair_agent_install_only(&context).await?;
    Ok(control.cached_agent_status(udid))
}

async fn bulk_repair_with_control(
    control: &DeviceControlPlane,
    udids: Vec<String>,
) -> Vec<AgentStatus> {
    let mut statuses = Vec::with_capacity(udids.len());
    for udid in udids {
        let status = match repair_with_control(control, &udid).await {
            Ok(status) => status,
            Err(error) => {
                let mut status = control.cached_agent_status(&udid);
                status.state = riviu_core::AgentState::Error;
                status.auth_ready = false;
                status.mjpeg_ready = false;
                status.session_ready = false;
                status.message = Some(error.to_string());
                status
            }
        };
        statuses.push(status);
    }
    statuses
}

#[tauri::command]
pub fn agent_get_settings(state: State<'_, AppState>) -> AgentRuntimeView {
    build_runtime_view(
        state.control.agent_settings(),
        state.agent_token_configured,
        &state.active_agent_artifact_id,
        &state.active_agent_artifact_version,
    )
}

#[tauri::command]
pub fn agent_save_settings(
    state: State<'_, AppState>,
    settings: AgentSettings,
) -> Result<AgentRuntimeView, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state.db.save_agent_settings(&settings).map_err(err)?;
    state.control.set_agent_settings(settings.clone());
    Ok(build_runtime_view(
        settings,
        state.agent_token_configured,
        &state.active_agent_artifact_id,
        &state.active_agent_artifact_version,
    ))
}

#[tauri::command]
pub fn agent_list_statuses(state: State<'_, AppState>, udids: Vec<String>) -> Vec<AgentStatus> {
    udids
        .iter()
        .map(|udid| state.control.cached_agent_status(udid))
        .collect()
}

#[tauri::command]
pub async fn agent_preflight(
    state: State<'_, AppState>,
    udid: String,
) -> Result<AgentStatus, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    preflight_with_control(&state.control, &udid)
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn agent_repair(
    state: State<'_, AppState>,
    udid: String,
) -> Result<AgentStatus, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    repair_with_control(&state.control, &udid)
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn agent_bulk_repair(
    state: State<'_, AppState>,
    udids: Vec<String>,
) -> Result<Vec<AgentStatus>, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    Ok(bulk_repair_with_control(&state.control, udids).await)
}

fn helper_maintenance_execution_error(udid: &str, error: &DeviceControlError) -> CommandError {
    // The control-plane boundary has already erased the anyhow type. Accept only
    // complete existing driver literals, never substrings or arbitrary envelopes.
    let cause = match error {
        DeviceControlError::Driver { operation: "executeHelperMaintenance", message, .. } => message.as_str(),
        _ => "",
    };
    let (stage, category, reason) = match cause {
        "helper maintenance binding changed; no replay or retirement"
        | "maintenance active plan changed; no replay or retirement" =>
            ("maintenance_binding", "binding_changed", "Thông tin thiết bị đã khác kế hoạch đang giữ; cần đối chiếu trước khi khôi phục."),
        "maintenance effect unresolved; process present; no replay" =>
            ("maintenance_reconciliation", "process_present", "Tiến trình helper vẫn còn; tiếp tục đối soát kế hoạch cũ, chưa gửi lệnh dừng lần nữa."),
        "HelperRecoveryRequired" =>
            ("maintenance_execution", "recovery_required", "Cần đối soát phiên helper đang giữ; chưa nhận phiên thay thế."),
        _ => ("maintenance_execution", "unknown", "Kiểm tra chẩn đoán của đúng máy; nguyên nhân hiện chưa xác định."),
    };
    let device: String = udid.chars().take(128)
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':')).collect();
    log::warn!("helper maintenance unproved; device={device}; stage={stage}; category={category}; {reason}; no action replay");
    CommandError::code("HelperMaintenanceExecutionUnproved", format!(
        "Khôi phục helper chưa được chứng minh; giữ bản ghi cũ để đối soát, không gửi lại thao tác; no action replay; stage={stage}; category={category}; {reason}"))
}

fn helper_maintenance_prepare_error(error: DeviceControlError) -> CommandError {
    // Only allowlisted, complete driver messages become public categories.
    // Never serialize the raw chain: it may contain credentials or journal data.
    let cause = match &error {
        DeviceControlError::Driver { operation: "prepareHelperMaintenance", message, .. } => message.as_str(),
        _ => "",
    };
    let (code, message) = match cause {
        "live helper owner must settle normally" => ("HelperMaintenanceLiveOwner",
            "Phiên helper hiện chưa được xác định là phiên lỗi cần khôi phục. Kiểm tra trạng thái điều khiển của đúng máy; không đặt lại phiên đang hoạt động."),
        "helper inventory work still draining" | "helper clipboard work still draining" | "helper request still draining" => ("HelperMaintenanceDraining",
            "Máy đang kết thúc thao tác helper. Chưa chuẩn bị khôi phục; đợi thao tác kết thúc rồi thử lại trên đúng máy."),
        "helper clipboard/baseline cleanup unresolved" | "helper durable clipboard cleanup unresolved" => ("HelperMaintenanceCleanupPending",
            "Phiên cũ còn thao tác clipboard chưa đối soát xong. Cần xử lý kết quả thao tác cũ trước khi khôi phục helper; chưa đặt lại phiên."),
        "maintenance disabled in diagnostic mode" => ("HelperMaintenanceDisabled",
            "Chế độ chẩn đoán không cho phép chuẩn bị khôi phục helper; chưa thực hiện khôi phục."),
        "HelperMaintenanceRetainedOwnerMissing" => ("HelperMaintenanceRetainedOwnerMissing",
            "Không có hồ sơ chủ phiên helper được giữ trên PC này; chưa đủ bằng chứng để chuẩn bị khôi phục. Kiểm tra đúng PC và dữ liệu phiên; không tự giành quyền hoặc đặt lại helper."),
        "maintenance cannot guess or restore a lost boot IME" => ("HelperMaintenanceImeUnproved",
            "Chưa chứng minh được bàn phím ban đầu để khôi phục helper. Giữ nguyên phiên và kiểm tra bằng chứng bàn phím; không tự đổi IME."),
        _ => ("HelperMaintenancePrepareFailed",
            "Không chuẩn bị được helper; chưa xác định nguyên nhân và chưa thực hiện khôi phục. Mở Chẩn đoán của đúng thiết bị; giữ nguyên dữ liệu phiên, không tự đặt lại helper."),
    };
    CommandError::code(code, message)
}

fn err(error: impl std::fmt::Display) -> CommandError {
    CommandError::operation(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn helper_prepare_reports_only_safe_known_categories() {
        for (cause, expected) in [
            ("live helper owner must settle normally", "HelperMaintenanceLiveOwner"),
            ("helper request still draining", "HelperMaintenanceDraining"),
            ("helper clipboard/baseline cleanup unresolved", "HelperMaintenanceCleanupPending"),
            ("maintenance disabled in diagnostic mode", "HelperMaintenanceDisabled"),
            ("HelperMaintenanceRetainedOwnerMissing", "HelperMaintenanceRetainedOwnerMissing"),
            ("maintenance cannot guess or restore a lost boot IME", "HelperMaintenanceImeUnproved"),
            ("fixture-secret-token raw journal unknown", "HelperMaintenancePrepareFailed"),
        ] {
            let error = helper_maintenance_prepare_error(DeviceControlError::Driver {
                udid: "fixture-device".into(), operation: "prepareHelperMaintenance", message: cause.into(),
            });
            assert_eq!(error.code, expected, "{cause}");
            assert!(!error.message.contains("fixture-secret-token"));
            assert!(!error.message.contains("raw journal"));
        }
    }

    #[test]
    fn helper_execute_only_reopens_proven_pre_dispatch_busy() {
        for (observation_only, operation, message, allowed) in [
            (false, "executeHelperMaintenance", "HelperMaintenanceBusyBeforeDispatch", true),
            (true, "executeHelperMaintenance", "HelperMaintenanceBusyBeforeDispatch", false),
            (false, "prepareHelperMaintenance", "HelperMaintenanceBusyBeforeDispatch", false),
            (false, "executeHelperMaintenance", "transport timeout", false),
            (false, "executeHelperMaintenance", "context: HelperMaintenanceBusyBeforeDispatch", false),
        ] {
            let mut phase = HelperMaintenancePhase::ExecutionStarted;
            let error = DeviceControlError::Driver { udid: "fixture".into(), operation, message: message.into() };
            assert_eq!(phase.restore_pre_dispatch_busy(observation_only, &error), allowed);
            assert!(phase == if allowed { HelperMaintenancePhase::Prepared } else { HelperMaintenancePhase::ExecutionStarted });
        }
        // Public error projection shares the exact execution-failure boundary and
        // must never turn a distinguishable cause into permission to replay.
        for (message, category, stage) in [
            ("helper maintenance binding changed; no replay or retirement", "binding_changed", "maintenance_binding"),
            ("maintenance effect unresolved; process present; no replay", "process_present", "maintenance_reconciliation"),
            ("fixture-secret-token owner clipboard credential envelope", "unknown", "maintenance_execution"),
            ("helper maintenance binding changed; no replay or retirement: fixture-secret-token", "unknown", "maintenance_execution"),
        ] {
            let error = DeviceControlError::Driver {
                udid: "fixture-secret-owner".into(), operation: "executeHelperMaintenance", message: message.into(),
            };
            let mut phase = HelperMaintenancePhase::ExecutionStarted;
            assert!(!phase.restore_pre_dispatch_busy(true, &error));
            assert!(phase == HelperMaintenancePhase::ExecutionStarted);
            let public = helper_maintenance_execution_error("fixture", &error);
            assert_eq!(public.code, "HelperMaintenanceExecutionUnproved");
            assert!(public.message.contains(&format!("stage={stage}; category={category}")));
            assert!(public.message.contains("no action replay"));
            assert!(!serde_json::to_string(&public).unwrap().contains("fixture-secret"));
            assert!(!public.message.contains("owner clipboard credential envelope"));
        }
        let mut phase = HelperMaintenancePhase::Settled;
        let error = DeviceControlError::Driver { udid: "fixture".into(), operation: "executeHelperMaintenance", message: "HelperMaintenanceBusyBeforeDispatch".into() };
        assert!(!phase.restore_pre_dispatch_busy(false, &error));
        assert!(phase == HelperMaintenancePhase::Settled);
    }
    use riviu_core::{
        AgentState, DeviceControlPlane, DeviceDriver, DeviceWorkCoordinator, DeviceWorkOwner,
        StreamBudgetManager,
    };
    use riviu_ios_driver::MockIosDriver;
    use std::sync::Arc;

    fn database() -> (Database, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "riviu-agent-command-test-{}.db",
            uuid::Uuid::new_v4()
        ));
        (Database::open(&path).expect("open fixture database"), path)
    }

    #[tokio::test]
    async fn repair_command_returns_the_verified_status() {
        let mock = MockIosDriver::new();
        let driver: Arc<dyn DeviceDriver> = Arc::new(mock.clone());

        let status = repair_with_driver(driver.as_ref(), "MOCK-IPHONE-01")
            .await
            .expect("repair mock agent");

        assert_eq!(status.state, AgentState::Ready);
        assert!(status.auth_ready && status.mjpeg_ready && status.session_ready);
        assert_eq!(
            mock.agent_repair_calls(),
            0,
            "desktop repair must use the install-only lifecycle"
        );
        assert_eq!(mock.ordinary_session_calls(), 0);
        assert_eq!(mock.fresh_text_session_calls(), 0);
        assert_eq!(mock.stream_restart_calls(), 0);
    }

    #[tokio::test]
    async fn preflight_uses_install_only_readiness_without_session_or_stream() {
        let driver = MockIosDriver::new();
        let control = DeviceControlPlane::new(
            Arc::new(driver.clone()),
            Arc::new(DeviceWorkCoordinator::new()),
            Arc::new(StreamBudgetManager::default()),
        );

        let status = preflight_with_control(&control, "MOCK-IPHONE-01")
            .await
            .expect("install-only preflight");

        assert!(status.auth_ready);
        assert_eq!(driver.agent_preflight_calls(), 0);
        assert_eq!(driver.agent_repair_calls(), 0);
        assert_eq!(driver.ordinary_session_calls(), 0);
        assert_eq!(driver.fresh_text_session_calls(), 0);
        assert_eq!(driver.stream_restart_calls(), 0);
    }

    #[tokio::test]
    async fn shared_device_owner_repair_is_busy_while_nurture_owns_device() {
        let driver = MockIosDriver::new();
        let control = DeviceControlPlane::new(
            Arc::new(driver.clone()),
            Arc::new(DeviceWorkCoordinator::new()),
            Arc::new(StreamBudgetManager::default()),
        );
        let _nurture = control
            .try_acquire_exclusive("fixture", DeviceWorkOwner::Nurture)
            .await
            .expect("nurture lease");

        let error = repair_with_control(&control, "fixture")
            .await
            .expect_err("repair must fail fast while nurture owns the device");

        assert!(matches!(
            error,
            riviu_core::DeviceControlError::Busy(riviu_core::DeviceBusy {
                current_owner: DeviceWorkOwner::Nurture,
                requested_owner: DeviceWorkOwner::Repair,
                ..
            })
        ));
        assert_eq!(driver.agent_repair_calls(), 0);
    }

    #[test]
    fn runtime_view_never_serializes_a_token() {
        let view = build_runtime_view(
            AgentSettings::default(),
            true,
            "riviu-agent-fixture",
            "1.2.3",
        );

        let json = serde_json::to_value(view).expect("serialize runtime view");
        let text = serde_json::to_string(&json).expect("serialize runtime JSON");

        assert!(!text.contains("fixture-secret-token"));
        assert!(!json
            .as_object()
            .expect("object")
            .keys()
            .any(|key| matches!(key.as_str(), "token" | "agentToken" | "authToken")));
    }

    #[tokio::test]
    async fn saving_auto_repair_updates_db_and_live_driver_settings() {
        let (db, path) = database();
        let driver: Arc<dyn DeviceDriver> = Arc::new(MockIosDriver::new());
        let settings = AgentSettings { auto_repair: false };

        save_settings_with_driver(
            &db,
            driver.as_ref(),
            settings.clone(),
            true,
            "artifact",
            "version",
        )
        .expect("save settings");

        assert_eq!(
            db.get_agent_settings().expect("database settings"),
            settings
        );
        assert_eq!(driver.agent_settings(), settings);
        std::fs::remove_file(path).expect("remove fixture database");
    }

    #[tokio::test]
    async fn ready_and_error_agent_statuses_serialize_as_camel_case() {
        let driver = MockIosDriver::new();
        let ready = driver.cached_agent_status("mock-device");
        let mut error = ready.clone();
        error.state = AgentState::Error;
        error.message = Some("fixture error".to_string());

        let ready_json = serde_json::to_value(ready).expect("serialize ready status");
        let error_json = serde_json::to_value(error).expect("serialize error status");

        assert_eq!(ready_json["state"], "ready");
        assert_eq!(ready_json["authReady"], true);
        assert_eq!(error_json["state"], "error");
        assert_eq!(error_json["message"], "fixture error");
    }

    #[tokio::test]
    async fn bulk_repair_continues_after_one_device_fails() {
        let driver = MockIosDriver::new();
        driver.set_mock_repair_failure("MOCK-IPHONE-02", true);

        let statuses = bulk_repair_with_driver(
            &driver,
            vec![
                "MOCK-IPHONE-01".to_string(),
                "MOCK-IPHONE-02".to_string(),
                "MOCK-IPHONE-01".to_string(),
            ],
        )
        .await;

        assert_eq!(driver.agent_repair_calls(), 0);
        assert_eq!(statuses.len(), 3);
        assert_eq!(statuses[0].state, AgentState::Ready);
        assert_eq!(statuses[1].state, AgentState::Error);
        assert_eq!(statuses[2].state, AgentState::Ready);
        assert_eq!(statuses[1].udid, "MOCK-IPHONE-02");
    }
}
