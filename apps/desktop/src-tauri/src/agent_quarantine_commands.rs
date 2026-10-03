//! Explicit maintenance borrows an exact retained Interaction ticket.
use crate::{command_error::CommandError, state::AppState};
use riviu_core::device_control::{
    InteractionQuarantineRecoveryRequest, InteractionQuarantineRecoveryPlan,
    InteractionQuarantineRecoveryReceipt, InteractionQuarantineSnapshot,
};
use std::sync::Arc;
use tauri::State;

fn install_ledger_guard(state: &AppState) {
    let db = state.db.clone();
    state.control.set_interaction_recovery_ledger_guard(Arc::new(move |request| {
        let db = db.clone();
        Box::pin(async move {
            if riviu_core::interaction_campaign::any_campaign_running() {
                return Err("interaction workers must finish before retained-session maintenance".into());
            }
            let campaign = request.campaign_id.clone();
            let assignment = request.assignment_id.clone();
            let binding = db.storage_read(move |db| db.interaction_maintenance_binding(&campaign,&assignment))
                .await.map_err(|_| "cannot read retained interaction identity".to_owned())?;
            if riviu_core::interaction_campaign::any_campaign_running() {
                return Err("interaction workers are still active; retained lease stays owned".into());
            }
            if request.operation_id != format!("interaction:{}",binding.campaign_id)
                || request.campaign_id != binding.campaign_id || request.assignment_id != binding.assignment_id
                || request.udid != binding.udid || request.revision != binding.revision
                || request.intent_sha256 != binding.intent_sha256 {
                return Err("retained interaction revision or intent identity changed".into());
            }
            Ok(())
        })
    }));
}

#[tauri::command]
pub fn agent_quarantine_snapshot(
    state: State<'_,AppState>, udid: String,
) -> Result<Vec<InteractionQuarantineSnapshot>,CommandError> {
    let _admission=state.ensure_cleanup_maintenance()?;
    Ok(state.control.interaction_quarantine_snapshot(&udid))
}

#[tauri::command]
pub async fn agent_quarantine_prepare(
    state: State<'_,AppState>, udid: String,
) -> Result<InteractionQuarantineRecoveryPlan,CommandError> {
    let _admission=state.ensure_cleanup_maintenance()?;
    let snapshots=state.control.interaction_quarantine_snapshot(&udid);
    let [snapshot]=snapshots.as_slice() else {
        return Err(CommandError::code("QuarantineScopeAmbiguous","Không có đúng một phiên cần bảo trì trên máy này."));
    };
    let scope=snapshot.scope.as_ref().ok_or_else(||CommandError::code("QuarantineBindingMissing","Phiên cũ chưa có danh tính tác vụ; giữ nguyên."))?;
    let campaign=scope.run_id.clone();
    let assignment=scope.assignment_id.clone().ok_or_else(||CommandError::code("QuarantineBindingMissing","Phiên cũ chưa có danh tính lượt; giữ nguyên."))?;
    let binding=state.db.storage_read(move |db|db.interaction_maintenance_binding(&campaign,&assignment))
        .await.map_err(CommandError::operation)?;
    if binding.udid!=udid { return Err(CommandError::code("QuarantineBindingChanged","Tác vụ và máy không khớp; giữ nguyên phiên.")); }
    let request=InteractionQuarantineRecoveryRequest {
        maintenance_id:uuid::Uuid::new_v4(), plane_id:snapshot.plane_id, udid,
        owner:snapshot.owner, lease_token:snapshot.lease_token,
        stream_generation:snapshot.stream_generation.ok_or_else(||CommandError::code("QuarantineGenerationMissing","Chưa có thế hệ stream được giữ; chưa bảo trì."))?,
        session_epoch:snapshot.session_epoch.clone(), campaign_id:binding.campaign_id.clone(),
        assignment_id:binding.assignment_id, operation_id:format!("interaction:{}",binding.campaign_id),
        revision:binding.revision, intent_sha256:binding.intent_sha256,
        acknowledge_old_draft_unproved:true,
    };
    install_ledger_guard(&state);
    state.control.prepare_interaction_quarantine_recovery(request).await.map_err(CommandError::from)
}

#[tauri::command]
pub async fn agent_quarantine_execute(
    state: State<'_,AppState>, plan_id:String, confirmed:bool,
) -> Result<InteractionQuarantineRecoveryReceipt,CommandError> {
    let _admission=state.ensure_cleanup_maintenance()?;
    let id=uuid::Uuid::parse_str(&plan_id).map_err(|error|CommandError::invalid_argument(error.to_string()))?;
    install_ledger_guard(&state);
    let receipt=state.control.execute_interaction_quarantine_recovery(id,confirmed).await.map_err(CommandError::from)?;
    let key=format!("interaction.maintenance.receipt:{id}");
    let raw=serde_json::to_string(&receipt).map_err(CommandError::operation)?;
    state.db.storage_write(move |db|db.set_setting(&key,&raw)).await.map_err(CommandError::operation)?;
    Ok(receipt)
}

/// Receipt lookup is read-only and remains available after work admission closes.
#[tauri::command]
pub async fn agent_quarantine_receipt(
    state: State<'_,AppState>, plan_id:String,
) -> Result<Option<serde_json::Value>,CommandError> {
    let id=uuid::Uuid::parse_str(&plan_id).map_err(|error|CommandError::invalid_argument(error.to_string()))?;
    if let Some(receipt)=state.control.interaction_quarantine_recovery_receipt(id) {
        return serde_json::to_value(receipt).map(Some).map_err(CommandError::operation);
    }
    let key=format!("interaction.maintenance.receipt:{id}");
    let raw=state.db.storage_read(move |db|db.get_setting(&key)).await.map_err(CommandError::operation)?;
    raw.map(|value| {
        let receipt:serde_json::Value=serde_json::from_str(&value).map_err(CommandError::operation)?;
        if receipt.get("plan").and_then(|plan|plan.get("planId")).and_then(serde_json::Value::as_str)!=Some(id.to_string().as_str()) {
            return Err(CommandError::code("QuarantineReceiptIdentityChanged", "Retained receipt identity changed"));
        }
        Ok(receipt)
    }).transpose()
}
