//! Deferred phone-app closure. One durable row per phone/package; no UI sessions or streams.
use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use crate::state::CommandAdmissionState;
use riviu_core::{
    db::{AppCompletionRecord, Database},
    DeviceControlPlane, DeviceWorkOwner,
};

#[cfg(test)]
#[path = "phone_app_completion_tests.rs"]
mod tests;

pub(crate) fn spawn(
    control: Arc<DeviceControlPlane>,
    db: Arc<Database>,
    admission: Arc<CommandAdmissionState>,
    stop: Arc<AtomicBool>,
) {
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut tasks = tokio::task::JoinSet::new();
        let mut active = HashSet::new();
        loop {
            tokio::select! {
                _ = tick.tick() => {},
                Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                    match result {
                        Ok(udid) => { active.remove(&udid); },
                        Err(error) => { log::error!("phone app completion worker: {error}"); }
                    }
                }
            }
            if stop.load(Ordering::Acquire) {
                while tasks.join_next().await.is_some() {}
                break;
            }
            // A panicked child cannot leave a permanent in-memory claim.
            if tasks.is_empty() {
                active.clear();
            }
            let rows = match db.list_due_app_completions(32) {
                Ok(rows) => rows,
                Err(error) => {
                    log::warn!("phone app completion queue: {error}");
                    continue;
                }
            };
            for row in rows {
                if tasks.len() >= 4 {
                    break;
                }
                if active.contains(&row.udid) {
                    continue;
                }
                let Ok(admitted) = admission.ensure_accepting_work() else {
                    break;
                };
                active.insert(row.udid.clone());
                let (db, control) = (db.clone(), control.clone());
                tasks.spawn(async move {
                    let _admitted = admitted;
                    if let Err(error) = close_if_finished(&control, &db, &row).await {
                        let reason = format!("Chưa đóng được TikTok: {error}");
                        let waiting = matches!(
                            error.downcast_ref::<riviu_core::DeviceControlError>(),
                            Some(
                                riviu_core::DeviceControlError::Busy(_)
                                    | riviu_core::DeviceControlError::PendingPublication { .. }
                            )
                        );
                        let saved = if waiting {
                            db.wait_app_completion(&row, &reason)
                        } else {
                            db.defer_app_completion(&row, &reason)
                        };
                        if let Err(save) = saved {
                            log::warn!("phone app completion {}: {reason}; save: {save}", row.udid);
                        }
                    }
                    row.udid
                });
            }
        }
    });
}

async fn close_if_finished(
    control: &DeviceControlPlane,
    db: &Database,
    row: &AppCompletionRecord,
) -> anyhow::Result<()> {
    if let Some(reason) = db.app_completion_block_reason(&row.udid)? {
        db.wait_app_completion(row, &reason)?;
        return Ok(());
    }
    let Some(_capacity) = db.try_publish_work(
        &row.udid,
        "appCompletion",
        &format!("app-close:{}", row.revision),
    )?
    else {
        db.wait_app_completion(row, "Chờ lượt đóng TikTok")?;
        return Ok(());
    };
    let context = control
        .try_acquire_exclusive_keeping_stream(&row.udid, DeviceWorkOwner::IdleSweep)
        .await?;
    let result: anyhow::Result<()> = async {
        if !db.app_completion_is_current(row)? {
            return Ok(());
        }
        if let Some(reason) = db.app_completion_block_reason(&row.udid)? {
            db.wait_app_completion(row, &reason)?;
            return Ok(());
        }
        // Only this lease can act on the phone until process absence and release are observed.
        let proof = control.terminate_app(&context, &row.bundle_id).await?;
        anyhow::ensure!(
            proof.bundle_id == row.bundle_id,
            "termination proof belongs to another package"
        );
        db.finish_app_completion(row, &serde_json::to_string(&proof)?)?;
        Ok(())
    }
    .await;
    let release = control.close_exclusive_context(context);
    result?;
    release?;
    Ok(())
}
