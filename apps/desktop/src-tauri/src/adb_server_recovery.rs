//! Bounded self-repair of a hung adb server (HX-ADB, owner decision 14/10/2026).
//!
//! The driver detects and budgets the restart; this worker supplies the two things only the
//! app knows: the effect ledger that decides whether now is a safe boundary, and the durable
//! activity log for the intent and receipt.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use riviu_android_driver::{AndroidDriver, ServerRecovery};
use riviu_core::db::Database;

use crate::state::CommandAdmissionState;

/// How often a suspicion is checked. The suspicion itself is a cheap in-memory read; the
/// probe and the restart only run when it holds.
const TICK: Duration = Duration::from_secs(5);

pub(crate) fn spawn(
    android: Arc<AndroidDriver>,
    db: Arc<Database>,
    admission: Arc<CommandAdmissionState>,
    stop: Arc<AtomicBool>,
) {
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // One activity-log line per wait or refusal, not one per tick.
        let mut last_hold: Option<String> = None;
        loop {
            tick.tick().await;
            if stop.load(Ordering::Acquire) {
                return;
            }
            if !android.adb_server_hang_suspected() {
                last_hold = None;
                continue;
            }
            let Ok(_admitted) = admission.ensure_accepting_work() else {
                return;
            };
            // Returns the write error: the driver refuses a restart whose intent did not land,
            // and logs a receipt that did not as an error.
            let journal = |action: &str, message: &str| -> anyhow::Result<()> {
                log::warn!("{message}");
                db.log_op(action, message)
            };
            let safe_boundary = || match db.adb_restart_blockers(chrono::Utc::now()) {
                Ok(blockers) if blockers.is_empty() => Ok(()),
                Ok(blockers) => Err(blockers.join(", ")),
                // Unknown is not "no effect in flight".
                Err(error) => Err(format!("chưa đọc được sổ thao tác: {error:#}")),
            };
            let hold = match android
                .recover_hung_adb_server(&journal, safe_boundary)
                .await
            {
                ServerRecovery::Deferred(blockers) => Some((
                    "adb.server.restart.deferred",
                    format!(
                        "ADB server không trả lời nhưng còn thao tác đang ở giữa Đăng/Gửi \
                         ({blockers}); Riviu chờ các thao tác này xong rồi mới khởi động lại ADB"
                    ),
                )),
                ServerRecovery::Refused(refusal) => Some((
                    "adb.server.restart.refused",
                    format!(
                        "ADB server không trả lời; chưa khởi động lại: {}",
                        refusal.message()
                    ),
                )),
                ServerRecovery::NotSuspected
                | ServerRecovery::Answered
                | ServerRecovery::Restarted { .. }
                | ServerRecovery::Failed(_) => None,
            };
            match hold {
                Some((action, message)) if last_hold.as_deref() != Some(action) => {
                    if let Err(error) = journal(action, &message) {
                        log::error!("không ghi được {action} vào nhật ký hoạt động: {error:#}");
                    }
                    last_hold = Some(action.to_string());
                }
                Some(_) => {}
                None => last_hold = None,
            }
        }
    });
}
