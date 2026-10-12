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

use riviu_android_driver::riviu_agent::HelperShellIdentityRefused;

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
    qualify_replacement: Option<bool>,
) -> Result<HelperMaintenanceReceipt, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    if !confirmed {
        return Err(CommandError::code("HelperMaintenanceConfirmationRequired",
            "Cần xác nhận rõ ràng trước khi khôi phục helper."));
    }
    let context = state.control
        .try_acquire_exclusive(&plan.udid, DeviceWorkOwner::Repair)
        .await?;
    let receipt = execute_helper_maintenance_admitted(
        &state.control, &state.db, &context, plan, confirmed, reconcile_only,
    ).await?;
    if qualify_replacement.unwrap_or(false) {
        // Settlement is already durable. Qualification cannot undo its receipt or
        // authorize replay of the archived maintenance/claim/clipboard effects.
        let android = state.android.as_ref().ok_or_else(|| CommandError::code(
            "HelperReplacementQualificationUnavailable",
            "Đã lưu kết quả khôi phục; backend Android chưa sẵn sàng để xác minh phiên helper mới.",
        ))?;
        android.prepare_helper_runtime(&receipt.udid).await.map_err(|_| CommandError::code(
            "HelperReplacementQualificationFailed",
            "Đã lưu kết quả khôi phục; chưa xác minh được phiên helper mới. Kiểm tra chẩn đoán của đúng máy.",
        ))?;
    }
    // Keep the same exclusive Repair context and admission through qualification.
    drop(context);
    Ok(receipt)
}

/// Owner decision 2026-10-12: a root adbd is repaired automatically, never through a
/// confirm step. Called under the phone's exclusive Repair lease (so no job of this
/// controller runs on it) when the phone connects and at job admission. At most one
/// `adb unroot` per phone boot, also across app restarts (a durable marker), none while
/// the boot id is unproved, logged before the effect; the host adb server is never
/// touched and nothing is replayed. Returns the operator text when adbd still does not
/// run as shell; `None` lets the normal path continue, including when the identity is
/// unreadable (that path reports it).
pub(crate) async fn auto_unroot_adb(
    android: &riviu_android_driver::AndroidDriver,
    db: &riviu_core::db::Database,
    serial: &str,
) -> Option<String> {
    use riviu_android_driver::driver::{AdbRootRepair, AdbUnrootOutcome};
    let record =
        |action: &str, message: &str| db.log_op(action, &format!("Máy {serial}: {message}"));
    let repair = android
        .repair_root_adb_shell(serial, || {
            record(
                "agent.helper.adb_unroot.started",
                "adb đang chạy quyền root; tự tắt quyền root của adb (adb unroot, một lần cho lần khởi động này); máy mất kết nối vài giây",
            )
        })
        .await;
    let dispatched = matches!(repair, Ok(AdbRootRepair::Dispatched(_)));
    let failure = match repair {
        Ok(AdbRootRepair::NotRoot) => return None,
        Ok(AdbRootRepair::Dispatched(AdbUnrootOutcome::Shell)) => {
            if let Err(error) = record("agent.helper.adb_unroot.done", "adb đã chạy bằng tài khoản shell (uid 2000)") {
                log::error!("could not persist adb unroot result for {serial}: {error:#}");
            }
            return None;
        }
        Ok(AdbRootRepair::Dispatched(AdbUnrootOutcome::StillRoot) | AdbRootRepair::AlreadyTriedThisBoot) =>
            "adb trên máy vẫn chạy quyền root sau khi Riviu tự tắt một lần (thường do GenFarmer hoặc ROM bật lại); helper không chạy với root. Tắt GenFarmer trên PC này rồi khởi động lại máy; Riviu tự kiểm tra lại khi máy kết nối lại.",
        Ok(AdbRootRepair::Dispatched(AdbUnrootOutcome::UnexpectedUid)) =>
            "Riviu đã tự tắt quyền root của adb nhưng adb của máy không chạy bằng tài khoản shell (uid 2000). Tắt rồi bật lại Gỡ lỗi USB trên máy; Riviu tự kiểm tra lại khi máy kết nối lại.",
        Ok(AdbRootRepair::Dispatched(AdbUnrootOutcome::NotBack)) =>
            "Riviu đã tự tắt quyền root của adb nhưng máy chưa kết nối lại trong 45 giây. Kiểm tra cáp và box; Riviu tự kiểm tra lại khi máy hiện lại.",
        Ok(AdbRootRepair::BootUnproved) =>
            "adb trên máy đang chạy quyền root nhưng Riviu chưa đọc được mã lần khởi động của máy, nên không tự tắt quyền root (tránh tắt lặp lại); helper không chạy với root. Tắt GenFarmer trên PC này rồi khởi động lại máy; Riviu tự kiểm tra lại khi máy kết nối lại.",
        Err(error) => {
            log::warn!("automatic adb unroot not attempted on {serial}: {error:#}");
            return None;
        }
    };
    // A repeated check of an already-spent boot logs nothing new.
    if dispatched {
        if let Err(error) = record("agent.helper.adb_unroot.failed", failure) {
            log::error!("could not persist adb unroot result for {serial}: {error:#}");
        }
    }
    log::warn!(
        "{serial}: adb shell still not uid 2000 after automatic adb unroot; no action replay"
    );
    Some(failure.to_owned())
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

/// Where automatic maintenance stopped. Preparation is observation-only and persists
/// nothing before the driver plan exists; execution may have dispatched an effect.
#[derive(Debug)]
pub(crate) enum AutomaticMaintenanceError {
    /// Refused before any plan was executed; no device effect was attempted.
    Prepare(CommandError),
    /// Execution was reached (fresh or resumed plan). Never retried automatically.
    Execute(CommandError),
}

/// User-authorized automatic policy, under the caller's exclusive Repair lease.
/// A pending identity is resumed, never replaced; executionStarted is observation-only.
pub(crate) async fn automatic_helper_maintenance(
    control: &DeviceControlPlane,
    db: &std::sync::Arc<riviu_core::db::Database>,
    context: &riviu_core::DeviceExclusiveContext,
    serial: &str,
) -> Result<(), AutomaticMaintenanceError> {
    let pending = load_maintenance_ledger(db, serial).map_err(AutomaticMaintenanceError::Prepare)?
        .into_iter()
        .find(|entry| entry.phase != HelperMaintenancePhase::Settled);
    let plan = match pending {
        Some(entry) => entry.public,
        None => prepare_helper_maintenance_admitted(control, db, context, serial.into()).await
            .map_err(AutomaticMaintenanceError::Prepare)?,
    };
    execute_helper_maintenance_admitted(control, db, context, plan, true, None).await
        .map_err(AutomaticMaintenanceError::Execute)?;
    Ok(())
}

/// Operator line for an automatic reconcile, read before the supersession settles.
///
/// Owner decision 2026-10-12: a fenced helper owner is superseded automatically under the
/// phone's Repair lease (so no job of this controller is running on it), whatever the
/// ledger holds. The ledger no longer gates it; it only names what stays recorded for
/// reconciliation. Supersession replays nothing: uncertain posts, sends and Sheet writes
/// are still only observed.
pub(crate) fn helper_auto_reconcile_note(db: &riviu_core::db::Database, serial: &str) -> String {
    match helper_auto_reconcile_blocker(db, serial) {
        Ok(None) => "tự đối soát helper: không có thao tác chưa rõ — nhận phiên mới".into(),
        Ok(Some(record)) => format!(
            "tự đối soát helper: nhận phiên mới; {record} vẫn được giữ để đối soát, không lặp thao tác nào"
        ),
        Err(_) => "tự đối soát helper: nhận phiên mới; chưa đọc được sổ thao tác, bản ghi cũ vẫn được giữ".into(),
    }
}

/// The first record on this phone that is uncertain or in flight, or `None`. Records
/// already archived by this phone's last settled maintenance are not named again.
fn helper_auto_reconcile_blocker(
    db: &riviu_core::db::Database,
    serial: &str,
) -> Result<Option<String>, CommandError> {
    let since = load_maintenance_ledger(db, serial)?
        .iter()
        .filter(|entry| entry.phase == HelperMaintenancePhase::Settled)
        .map(|entry| entry.prepared_at_ms)
        .max()
        .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis);
    db.helper_auto_reconcile_blocker(serial, since)
        .map_err(|_| {
            CommandError::operation("cannot read the effect ledger for automatic helper reconcile")
        })
}

/// Job admission: one automatic reconcile per phone whose retained helper owner fences
/// it and that no job of this controller holds. Returns the refusal to show for each
/// phone it could not repair; every other phone is left to its normal preflight.
pub(crate) async fn reconcile_helpers_for_admission(
    state: &AppState,
    udids: &[String],
) -> std::collections::HashMap<String, String> {
    let Some(android) = state.android.clone() else {
        return Default::default();
    };
    let attempts = udids
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|udid| {
            let android = android.clone();
            async move {
                reconcile_helper_for_admission(&state.control, &state.db, &android, udid)
                    .await
                    .map(|refusal| (udid.clone(), refusal))
            }
        });
    futures_util::future::join_all(attempts)
        .await
        .into_iter()
        .flatten()
        .collect()
}

async fn reconcile_helper_for_admission(
    control: &DeviceControlPlane,
    db: &std::sync::Arc<riviu_core::db::Database>,
    android: &riviu_android_driver::AndroidDriver,
    udid: &str,
) -> Option<String> {
    // Only a phone fenced by a retained owner, and only while no other owner holds it.
    if !control.reports_element_bounds(udid) || control.current_work_owner(udid).is_some() {
        return None;
    }
    match control.verify_automation_readiness(udid).await {
        Err(error) if is_helper_recovery_required(&error.to_string()) => {}
        _ => return None,
    }
    let context = control
        .try_acquire_exclusive_keeping_stream(udid, DeviceWorkOwner::Repair)
        .await
        .ok()?;
    let record = |action: &str, message: &str| {
        if let Err(error) = db.log_op(action, &format!("Máy {udid}: {message}")) {
            log::error!("could not persist automatic helper reconcile for {udid}: {error:#}");
        }
    };
    // A root adbd is repaired first, whatever the ledger holds: `adb unroot` restarts only
    // the phone's adbd, touches no TikTok state and replays nothing.
    if let Some(refusal) = auto_unroot_adb(android, db, udid).await {
        return Some(refusal);
    }
    // A resumable owner needs no supersession; only a refused resume reaches maintenance.
    match android.prepare_helper_runtime(udid).await {
        Ok(_) => return None,
        Err(error)
            if error.is::<riviu_android_driver::riviu_agent::HelperRecoveryRequired>()
                && !error.is::<HelperShellIdentityRefused>() => {}
        Err(_) => return None,
    }
    let note = helper_auto_reconcile_note(db, udid);
    if let Err(error) = automatic_helper_maintenance(control, db, &context, udid).await {
        let error = match error {
            AutomaticMaintenanceError::Prepare(error)
            | AutomaticMaintenanceError::Execute(error) => error,
        };
        record(
            "agent.helper.auto_reconcile.failed",
            &format!("{}: {}", error.code, error.message),
        );
        return Some(error.message.into());
    }
    // Same as the button's qualifyReplacement: settlement is durable and never undone here.
    if android.prepare_helper_runtime(udid).await.is_err() {
        let message = "Đã tự đối soát helper; chưa xác minh được phiên helper mới. Kiểm tra chẩn đoán của đúng máy.";
        record("agent.helper.auto_reconcile.failed", message);
        return Some(message.into());
    }
    log::info!("{udid}: {note}");
    record("agent.helper.auto_reconcile", &note);
    drop(context);
    None
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
) -> Vec<AgentRepairOutcome> {
    let mut outcomes = Vec::with_capacity(udids.len());
    for udid in udids {
        let result = repair_with_driver(driver, &udid).await.map_err(|error| error.to_string());
        outcomes.push(repair_outcome(result, || driver.cached_agent_status(&udid)));
    }
    outcomes
}

/// One device's "Sửa agent" result.
///
/// The install-only repair (UiAutomator2 on Android, Riviu Agent on iOS) and the Android
/// helper are separate layers. A latched helper failure overrides the cached status, so the
/// status alone made every Android repair read as failed even when the agent was repaired.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRepairOutcome {
    pub status: AgentStatus,
    /// The install-only repair itself was proved on this attempt.
    pub agent_repaired: bool,
    /// The helper is still latched as failed; only "Khôi phục helper" addresses that.
    pub helper_recovery_required: bool,
}

fn helper_setup_latched(status: &AgentStatus) -> bool {
    status.message.as_deref()
        .is_some_and(|message| message.starts_with(riviu_android_driver::driver::HELPER_SETUP_FAILURE))
}

fn repair_outcome(
    result: Result<AgentStatus, String>,
    cached: impl FnOnce() -> AgentStatus,
) -> AgentRepairOutcome {
    match result {
        Ok(status) => AgentRepairOutcome {
            helper_recovery_required: helper_setup_latched(&status),
            agent_repaired: true,
            status,
        },
        Err(message) => {
            let mut status = cached();
            let helper_recovery_required = helper_setup_latched(&status);
            status.state = riviu_core::AgentState::Error;
            status.auth_ready = false;
            status.mjpeg_ready = false;
            status.session_ready = false;
            status.message = Some(message);
            AgentRepairOutcome { status, agent_repaired: false, helper_recovery_required }
        }
    }
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
) -> Vec<AgentRepairOutcome> {
    let mut outcomes = Vec::with_capacity(udids.len());
    for udid in udids {
        let result = repair_with_control(control, &udid).await.map_err(|error| error.to_string());
        outcomes.push(repair_outcome(result, || control.cached_agent_status(&udid)));
    }
    outcomes
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
) -> Result<Vec<AgentRepairOutcome>, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    Ok(bulk_repair_with_control(&state.control, udids).await)
}

fn helper_device_label(udid: &str) -> String {
    udid.chars().take(128)
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':')).collect()
}

/// The driver's recovery fence, recognized anywhere in a stringified chain: the control
/// plane serializes `{error:#}`, so the fence often sits beneath a context prefix.
fn is_helper_recovery_required(cause: &str) -> bool {
    cause.contains(&riviu_android_driver::riviu_agent::HelperRecoveryRequired.to_string())
}

fn helper_maintenance_execution_error(udid: &str, error: &DeviceControlError) -> CommandError {
    // The control-plane boundary has already erased the anyhow type. Accept only
    // complete existing driver literals or the driver's own typed markers.
    let cause = match error {
        DeviceControlError::Driver { operation: "executeHelperMaintenance", message, .. } => message.as_str(),
        _ => "",
    };
    let device = helper_device_label(udid);
    let refusal = HelperShellIdentityRefused::from_chain_text(cause);
    let identity_reason = refusal.map(HelperShellIdentityRefused::operator_message);
    let (stage, category, reason) = match cause {
        "helper maintenance binding changed; no replay or retirement"
        | "maintenance active plan changed; no replay or retirement" =>
            ("maintenance_binding", "binding_changed", "Thông tin thiết bị đã khác kế hoạch đang giữ; cần đối chiếu trước khi khôi phục."),
        "maintenance effect unresolved; process present; no replay" =>
            ("maintenance_reconciliation", "process_present", "Tiến trình helper vẫn còn; tiếp tục đối soát kế hoạch cũ, chưa gửi lệnh dừng lần nữa."),
        _ if refusal.is_some() => (
            "maintenance_execution",
            refusal.map_or("unknown", HelperShellIdentityRefused::category),
            identity_reason.unwrap_or_default(),
        ),
        _ if is_helper_recovery_required(cause) =>
            ("maintenance_execution", "recovery_required", "Cần đối soát phiên helper đang giữ; chưa nhận phiên thay thế."),
        _ => ("maintenance_execution", "unknown", "Kiểm tra chẩn đoán của đúng máy; nguyên nhân hiện chưa xác định."),
    };
    log::warn!("helper maintenance unproved; device={device}; stage={stage}; category={category}; {reason}; no action replay");
    CommandError::code("HelperMaintenanceExecutionUnproved", format!(
        "Khôi phục helper chưa được chứng minh; giữ bản ghi cũ để đối soát, không gửi lại thao tác; no action replay; stage={stage}; category={category}; {reason}"))
}

/// Public code and category for a refused maintenance preparation.
///
/// Only allowlisted complete driver messages and the driver's typed markers become
/// public categories. Never serialize the raw chain: it may contain credentials or
/// journal data. Preparation is observation-only, so every category is pre-effect.
fn helper_maintenance_prepare_error(error: DeviceControlError) -> CommandError {
    let (udid, cause) = match &error {
        DeviceControlError::Driver { operation: "prepareHelperMaintenance", udid, message } => (udid.as_str(), message.as_str()),
        DeviceControlError::Driver { udid, .. } => (udid.as_str(), ""),
        // Admission and lease errors already have their own public form.
        _ => return CommandError::from(error),
    };
    let device = helper_device_label(udid);
    let (code, category, message): (&str, &str, String) = match cause {
        "live helper owner must settle normally" => ("HelperMaintenanceLiveOwner", "live_owner",
            "Phiên helper hiện chưa được xác định là phiên lỗi cần khôi phục. Kiểm tra trạng thái điều khiển của đúng máy; không đặt lại phiên đang hoạt động.".into()),
        "helper inventory work still draining" | "helper clipboard work still draining" | "helper request still draining" => ("HelperMaintenanceDraining", "draining",
            "Máy đang kết thúc thao tác helper. Chưa chuẩn bị khôi phục; đợi thao tác kết thúc rồi thử lại trên đúng máy.".into()),
        "helper clipboard/baseline cleanup unresolved" | "helper durable clipboard cleanup unresolved" => ("HelperMaintenanceCleanupPending", "cleanup_pending",
            "Phiên cũ còn thao tác clipboard chưa đối soát xong. Cần xử lý kết quả thao tác cũ trước khi khôi phục helper; chưa đặt lại phiên.".into()),
        "maintenance disabled in diagnostic mode" => ("HelperMaintenanceDisabled", "disabled",
            "Chế độ chẩn đoán không cho phép chuẩn bị khôi phục helper; chưa thực hiện khôi phục.".into()),
        "HelperMaintenanceRetainedOwnerMissing" => ("HelperMaintenanceRetainedOwnerMissing", "retained_owner_missing",
            "Không có hồ sơ chủ phiên helper được giữ trên PC này; chưa đủ bằng chứng để chuẩn bị khôi phục. Kiểm tra đúng PC và dữ liệu phiên; không tự giành quyền hoặc đặt lại helper.".into()),
        "maintenance cannot guess or restore a lost boot IME" => ("HelperMaintenanceImeUnproved", "ime_unproved",
            "Bàn phím mặc định của máy vẫn là bàn phím Riviu Helper từ phiên cũ, nên Riviu không biết bàn phím gốc để trả lại và không tự đổi. Trên máy: Cài đặt → Quản lý chung → Danh sách bàn phím và mặc định → Bàn phím mặc định → chọn Bàn phím Samsung (hoặc bàn phím thường dùng). Riviu tự kiểm tra lại trong vài phút; nếu đã quá 7 phút thì rút cáp cắm lại máy.".into()),
        // Typed refusals are checked before the recovery fence they may also carry.
        _ => match HelperShellIdentityRefused::from_chain_text(cause) {
            Some(refusal) => (match refusal {
                HelperShellIdentityRefused::Root => "HelperMaintenanceShellRoot",
                HelperShellIdentityRefused::UnexpectedUid => "HelperMaintenanceShellUid",
                HelperShellIdentityRefused::AndroidUser => "HelperMaintenanceAndroidUser",
            }, refusal.category(), refusal.operator_message().to_owned()),
            None if is_helper_recovery_required(cause) => ("HelperMaintenanceRecoveryRequired", "recovery_required",
                "Hồ sơ helper trên PC này còn kế hoạch khôi phục dở hoặc không khớp đúng máy; chưa chuẩn bị kế hoạch mới. Mở Chẩn đoán của đúng thiết bị; giữ nguyên dữ liệu phiên, không tự đặt lại helper.".into()),
            None if cause == "helper runtime inventory unavailable" || cause.starts_with("adb shell timed out after ") => ("HelperMaintenanceAdbUnavailable", "adb_unavailable",
                "Không đọc được trạng thái máy qua ADB (lệnh lỗi hoặc quá thời gian); chưa thực hiện khôi phục. Kiểm tra cáp và kết nối ADB của đúng máy rồi thử lại.".into()),
            None => ("HelperMaintenancePrepareFailed", "unknown",
                "Không chuẩn bị được helper; chưa xác định nguyên nhân và chưa thực hiện khôi phục. Mở Chẩn đoán của đúng thiết bị; giữ nguyên dữ liệu phiên, không tự đặt lại helper.".into()),
        },
    };
    log::warn!("helper maintenance prepare refused; device={device}; stage=maintenance_prepare; category={category}; no action replay");
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
    fn helper_prepare_names_real_driver_chains_instead_of_unknown() {
        use riviu_android_driver::riviu_agent::HelperRecoveryRequired;
        // The control plane serializes the whole `{error:#}` chain, so the driver's
        // typed fences arrive behind context prefixes, never as bare literals.
        let fenced = |refusal| format!("{:#}", anyhow::Error::new(refusal).context(HelperRecoveryRequired));
        for (cause, expected, hint) in [
            (fenced(HelperShellIdentityRefused::Root), "HelperMaintenanceShellRoot", Some("tự tắt quyền root của adb")),
            (format!("{}", HelperShellIdentityRefused::Root), "HelperMaintenanceShellRoot", Some("uid 0")),
            (fenced(HelperShellIdentityRefused::UnexpectedUid), "HelperMaintenanceShellUid", Some("uid 2000")),
            (fenced(HelperShellIdentityRefused::AndroidUser), "HelperMaintenanceAndroidUser", Some("người dùng chính")),
            (HelperRecoveryRequired.to_string(), "HelperMaintenanceRecoveryRequired", None),
            (format!("{:#}", anyhow::Error::new(HelperRecoveryRequired)
                .context("fixture-secret-token raw journal context")), "HelperMaintenanceRecoveryRequired", None),
            ("helper runtime inventory unavailable".into(), "HelperMaintenanceAdbUnavailable", None),
        ] {
            let error = helper_maintenance_prepare_error(DeviceControlError::Driver {
                udid: "fixture-device".into(), operation: "prepareHelperMaintenance", message: cause.clone(),
            });
            assert_eq!(error.code, expected, "{cause}");
            if let Some(hint) = hint { assert!(error.message.contains(hint), "{}", error.message); }
            assert!(!error.message.contains("fixture-secret-token"));
            assert!(!error.message.contains("HelperRecoveryRequired"));
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
            // The driver fence is the full Display text; the bare token never arrives.
            ("HelperRecoveryRequired: prior owner cleanup unresolved; no claim or action replay", "recovery_required", "maintenance_execution"),
            ("HelperRecoveryRequired: prior owner cleanup unresolved; no claim or action replay: HelperShellRootRefused: adb shell runs as root (uid 0); helper requires shell uid 2000; no root fallback",
                "shell_root", "maintenance_execution"),
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
    async fn automatic_maintenance_names_the_stage_it_stopped_in() {
        let control = DeviceControlPlane::new(
            Arc::new(MockIosDriver::new()),
            Arc::new(DeviceWorkCoordinator::new()),
            Arc::new(StreamBudgetManager::default()),
        );
        let (database, path) = database();
        let db = Arc::new(database);
        let context = control.try_acquire_exclusive("fixture", DeviceWorkOwner::Repair).await
            .expect("repair lease");
        // No plan yet: the driver refuses preparation, before anything is persisted.
        let error = automatic_helper_maintenance(&control, &db, &context, "fixture").await
            .expect_err("fixture driver cannot prepare");
        assert!(matches!(error, AutomaticMaintenanceError::Prepare(_)), "{error:?}");
        assert!(!helper_maintenance_is_pending(&db, "fixture").unwrap());
        // A pending plan resumes into execution; that failure is never a prepare failure.
        let plan = HelperMaintenancePlan {
            maintenance_id: "fixture_maintenance_1234".into(), udid: "fixture".into(), submitted: Vec::new(),
        };
        save_maintenance_ledger(&db, "fixture", &[PreparedHelperMaintenance {
            public: plan, driver_plan: serde_json::json!({"fixture": true}),
            prepared_at_ms: 0, phase: HelperMaintenancePhase::Prepared,
        }]).await.unwrap();
        let error = automatic_helper_maintenance(&control, &db, &context, "fixture").await
            .expect_err("fixture driver cannot execute");
        assert!(matches!(error, AutomaticMaintenanceError::Execute(_)), "{error:?}");
        assert!(helper_maintenance_is_pending(&db, "fixture").unwrap(), "execution start stays recorded");
        drop(context);
        drop(db);
        let _ = std::fs::remove_file(path);
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
        assert_eq!(statuses[0].status.state, AgentState::Ready);
        assert_eq!(statuses[1].status.state, AgentState::Error);
        assert_eq!(statuses[2].status.state, AgentState::Ready);
        assert_eq!(statuses[1].status.udid, "MOCK-IPHONE-02");
        assert_eq!(statuses.iter().map(|outcome| outcome.agent_repaired).collect::<Vec<_>>(), [true, false, true]);
        assert!(statuses.iter().all(|outcome| !outcome.helper_recovery_required));
    }

    #[test]
    fn bulk_repair_reports_agent_repair_separately_from_a_latched_helper() {
        let latched = format!("{}; stage=runtime_acquisition; category=recovery_required; fixture",
            riviu_android_driver::driver::HELPER_SETUP_FAILURE);
        let mut cached = AgentStatus::unknown("fixture-android");
        cached.state = AgentState::RepairRequired;
        cached.message = Some(latched.clone());
        // UiAutomator2 repaired, helper still latched: not a repair failure.
        let repaired = repair_outcome(Ok(cached.clone()), || panic!("success keeps its status"));
        assert!(repaired.agent_repaired && repaired.helper_recovery_required);
        assert_eq!(repaired.status.message.as_deref(), Some(latched.as_str()));
        // A failed repair reports its own error and still names the helper latch.
        let failed = repair_outcome(Err("fixture repair error".into()), || cached.clone());
        assert!(!failed.agent_repaired && failed.helper_recovery_required);
        assert_eq!(failed.status.state, AgentState::Error);
        assert_eq!(failed.status.message.as_deref(), Some("fixture repair error"));
        let json = serde_json::to_value(&repaired).unwrap();
        assert_eq!(json["agentRepaired"], true);
        assert_eq!(json["helperRecoveryRequired"], true);
        assert_eq!(json["status"]["udid"], "fixture-android");
    }
}
