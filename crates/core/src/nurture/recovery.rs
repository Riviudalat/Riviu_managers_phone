//! Session outcome and the budgeted recovery ladder.
//!
//! The rule that matters: only a transport-class failure may touch the
//! transport. A rejected command means the agent is alive and just refused that
//! one call — recycling on it is what produced the 2–3 minute death spirals in
//! earlier live tests.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::driver::{ui_error_kind, UiErrorKind};
use crate::screen_watch::SessionHandle;
use crate::types::NurtureSessionStatus;

use super::NurtureEngine;

/// Soft recovery budget: drop the session and make a new one.
pub(super) const SOFT_RECOVERY_BUDGET: Duration = Duration::from_secs(15);
/// Hard recovery budget: tear down relay + device runner and rebuild. Killing
/// the device-side runner and waiting for iOS to reap it before a fresh XCTest
/// start measured ~70–90 s, so a tighter budget would abort a recovery that was
/// about to succeed.
pub(super) const HARD_RECOVERY_BUDGET: Duration = Duration::from_secs(150);
/// Hard recycles allowed per session before the device is declared failed.
pub(super) const MAX_HARD_RECOVERIES: u32 = 1;

/// How a session ended. `done` is reserved for a session that actually did the
/// work: a run that timed out having processed nothing is `failed`, not `done`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Partial,
    Failed,
    Stopped,
}

impl Outcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Outcome::Done => "done",
            Outcome::Partial => "partial",
            Outcome::Failed => "failed",
            Outcome::Stopped => "stopped",
        }
    }
}

/// Recovery spend for one session.
pub(super) struct Budget {
    pub(super) soft: u32,
    pub(super) hard: u32,
    pub(super) spent: Duration,
}

impl Budget {
    pub(super) fn new() -> Self {
        Self {
            soft: 0,
            hard: 0,
            spent: Duration::ZERO,
        }
    }

    pub(super) fn exhausted(&self) -> bool {
        self.hard > MAX_HARD_RECOVERIES
    }
}

impl NurtureEngine {
    /// Spend recovery budget on a failed gesture. Returns false when the
    /// session cannot continue.
    pub(super) async fn recover(
        &self,
        udid: &str,
        handle: &SessionHandle,
        budget: &mut Budget,
        err: &anyhow::Error,
        status: &mut NurtureSessionStatus,
        on_status: &(impl Fn(NurtureSessionStatus) + Send + Sync),
    ) -> bool {
        let kind = ui_error_kind(err);
        // A rejected command means the runner is alive and only this command
        // was wrong. Recycling on that is what produced the old death spirals.
        if matches!(kind, UiErrorKind::Http | UiErrorKind::Other) {
            return true;
        }

        let started = Instant::now();
        budget.soft += 1;
        status.last_message = format!(
            "soft recovery {}/{} ({}s ngân sách)",
            budget.soft,
            budget.soft,
            SOFT_RECOVERY_BUDGET.as_secs()
        );
        on_status(status.clone());
        self.driver.invalidate_ui_session(udid).await;
        let soft = tokio::time::timeout(SOFT_RECOVERY_BUDGET, self.driver.start_ui_session(udid))
            .await;
        budget.spent += started.elapsed();
        if let Ok(Ok(s)) = soft {
            handle.set(Arc::from(s));
            status.last_message = format!("soft recovery xong sau {:.0}s", started.elapsed().as_secs_f64());
            on_status(status.clone());
            return true;
        }

        if budget.hard >= MAX_HARD_RECOVERIES {
            status.last_message = "đã dùng hết hard recovery — dừng thiết bị".into();
            on_status(status.clone());
            budget.hard += 1;
            return false;
        }

        let hard_started = Instant::now();
        budget.hard += 1;
        status.last_message = format!(
            "hard recovery 1/{} ({}s ngân sách)",
            MAX_HARD_RECOVERIES,
            HARD_RECOVERY_BUDGET.as_secs()
        );
        on_status(status.clone());
        self.driver.recycle_ui_transport(udid).await;
        let hard = tokio::time::timeout(HARD_RECOVERY_BUDGET, self.driver.start_ui_session(udid))
            .await;
        budget.spent += hard_started.elapsed();
        match hard {
            Ok(Ok(s)) => {
                handle.set(Arc::from(s));
                status.last_message =
                    format!("hard recovery xong sau {:.0}s", hard_started.elapsed().as_secs_f64());
                on_status(status.clone());
                true
            }
            _ => {
                status.last_message = "hard recovery thất bại — thiết bị lỗi".into();
                on_status(status.clone());
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_recoveries_alone_never_stop_a_device() {
        let mut b = Budget::new();
        assert!(!b.exhausted());
        b.soft = 5;
        assert!(
            !b.exhausted(),
            "a device must not be given up on for soft recoveries — those are cheap"
        );
    }

    #[test]
    fn the_budget_runs_out_one_past_the_hard_recovery_cap() {
        let mut b = Budget::new();
        b.hard = MAX_HARD_RECOVERIES;
        assert!(!b.exhausted(), "the allowed hard recycle must still be usable");
        b.hard = MAX_HARD_RECOVERIES + 1;
        assert!(b.exhausted());
    }

    /// A rejected command means the agent is alive; recycling on it is the
    /// mistake that produced 2–3 minute stalls in earlier builds.
    #[test]
    fn only_transport_class_failures_justify_touching_the_transport() {
        assert!(!UiErrorKind::Http.is_safe_to_retry());
        assert!(UiErrorKind::Transport.is_safe_to_retry());
        assert!(UiErrorKind::Session.is_safe_to_retry());
    }
}
