//! Qualify the bundled Android helper and recover retained failed owners on connection.
//!
//! This is a child of `state` so preparation participates in the same command
//! admission and shutdown drain as operator work. Inventory only schedules it;
//! installation never runs on the roster/sampler task.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;

use riviu_android_driver::AndroidDriver;
use riviu_android_driver::driver::{HelperRecoveryGeneration, HELPER_SETUP_FAILURE};
use riviu_android_driver::riviu_agent::HelperShellIdentityRefused;
use crate::agent_commands::AutomaticMaintenanceError;
use riviu_core::db::Database;
use riviu_core::{
    DeviceControlError, DeviceControlPlane, DeviceInfo, DevicePlatform, DeviceStatus,
    DeviceWorkOwner,
};
use tokio::task::{Id, JoinError, JoinSet};
use tokio::time::{Duration, Instant};

use super::CommandAdmissionState;

const MAX_CONCURRENT_INSTALLS: usize = 2;
/// Finite backoff for observation-only preparation refusals on one connection: the
/// operator may fix the phone (e.g. `adb unroot`) without replugging it. Then terminal.
const PREPARE_RETRY_DELAYS: [Duration; 3] =
    [Duration::from_secs(30), Duration::from_secs(120), Duration::from_secs(300)];

pub(super) struct AndroidHelperSetup {
    control: Arc<DeviceControlPlane>,
    android: Arc<AndroidDriver>,
    admission: Arc<CommandAdmissionState>,
    db: Arc<Database>,
    queue: SetupQueue,
}

impl AndroidHelperSetup {
    pub(super) fn new(
        control: Arc<DeviceControlPlane>,
        android: Arc<AndroidDriver>,
        admission: Arc<CommandAdmissionState>,
        db: Arc<Database>,
    ) -> Self {
        Self {
            control,
            android,
            admission,
            db,
            queue: SetupQueue::default(),
        }
    }

    /// Only the Android backend's own stable inventory can prove reconnection.
    pub(super) fn tick_latest(&mut self) {
        if let Some(devices) = self.android.helper_connection_snapshot() {
            self.tick(&devices);
        }
    }

    fn tick(&mut self, devices: &[DeviceInfo]) {
        let control = self.control.clone();
        let android = self.android.clone();
        let admission = self.admission.clone();
        let db = self.db.clone();
        self.queue.recovery = self.android.helper_recovery_snapshot();
        self.queue.tick(
            devices,
            |serial| self.control.current_work_owner(serial).is_none(),
            move |ticket| {
                let control = control.clone();
                let android = android.clone();
                let admission = admission.clone();
                let db = db.clone();
                async move {
                    run_admitted(&control, &admission, &ticket.serial, |context| {
                        let control = control.clone();
                        let android = android.clone();
                        let db = db.clone();
                        let serial = ticket.serial.clone();
                        async move {
                            auto_apply_baseline(&android, &db, &serial).await;
                            record(&db, "agent.helper.setup.started", &serial,
                                "Đang chuẩn bị Riviu Helper để điều khiển thiết bị.");
                            let automatic_failure = |error: &anyhow::Error| crate::command_error::CommandError::code(
                                "HelperAutomaticPreparationFailed", automatic_preparation_message(&serial, error));
                            let result = async {
                                let pending = crate::agent_commands::helper_maintenance_is_pending(&db, &serial)
                                    .map_err(|error| (SetupStop::PreEffect, error))?;
                                if !pending {
                                    match android.prepare_helper_runtime(&serial).await {
                                        Ok(_) => return Ok(()),
                                        Err(error) if error.is::<riviu_android_driver::riviu_agent::HelperRecoveryRequired>() => {}
                                        // Proved before any install or claim on this phone.
                                        Err(error) if error.is::<HelperShellIdentityRefused>() =>
                                            return Err((SetupStop::PreEffect, automatic_failure(&error))),
                                        Err(error) => return Err((SetupStop::Terminal, automatic_failure(&error))),
                                    }
                                }
                                // Automatic only when the durable ledger proves nothing on this
                                // phone is uncertain or in flight; otherwise the operator decides.
                                if let Some(blocker) = crate::agent_commands::helper_auto_reconcile_blocker(&db, &serial)
                                    .map_err(|error| (SetupStop::PreEffect, error))?
                                {
                                    return Err((SetupStop::Terminal, crate::command_error::CommandError::code(
                                        "HelperAutoReconcileBlocked",
                                        crate::agent_commands::helper_auto_reconcile_blocked_message(&blocker))));
                                }
                                crate::agent_commands::automatic_helper_maintenance(&control, &db, &context, &serial).await
                                    .map_err(|error| match error {
                                        AutomaticMaintenanceError::Prepare(error) => (SetupStop::PreEffect, error),
                                        AutomaticMaintenanceError::Execute(error) => (SetupStop::Terminal, error),
                                    })?;
                                // After an execution the replacement claim is never retried here.
                                android.prepare_helper_runtime(&serial).await
                                    .map_err(|error| (SetupStop::Terminal, automatic_failure(&error)))?;
                                record(&db, "agent.helper.auto_reconcile", &serial,
                                    crate::agent_commands::HELPER_AUTO_RECONCILE_NOTE);
                                Ok(())
                            }.await;
                            match result {
                                Ok(()) => {
                                    record(&db, "agent.helper.setup.succeeded", &serial,
                                        "Đã xác minh Riviu Helper hoạt động.");
                                    AttemptOutcome::Finished
                                }
                                Err((stop, error)) => {
                                    let disposition = failure_disposition(stop, &error.code);
                                    let note = match disposition {
                                        FailureDisposition::RetryPrepare => " Sẽ tự kiểm tra lại sau ít phút (tối đa 3 lần); chưa thực hiện khôi phục.",
                                        FailureDisposition::Deferred | FailureDisposition::Terminal => "",
                                    };
                                    record(&db, "agent.helper.setup.failed", &serial,
                                        &format!("{}: {}{note}", error.code, error.message));
                                    match disposition {
                                        FailureDisposition::Deferred => AttemptOutcome::Deferred,
                                        disposition => {
                                            android.record_helper_preparation_failure(&serial);
                                            let generation = android.helper_recovery_snapshot().get(&serial).copied();
                                            if disposition == FailureDisposition::RetryPrepare {
                                                AttemptOutcome::RetryPrepare(generation)
                                            } else {
                                                AttemptOutcome::Failed(generation)
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    })
                    .await
                }
            },
        );
    }

    /// Let an admitted package install finish before shutting its driver down.
    pub(super) async fn drain(&mut self) {
        self.queue.drain().await;
    }
}

/// Where an automatic attempt stopped, as far as device effects are concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SetupStop {
    /// Refused before any install, claim or maintenance execution: observation only.
    PreEffect,
    /// Maintenance execution was reached, or a claim may have been dispatched.
    Terminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FailureDisposition {
    /// Admission or pre-dispatch contention: the short contention backoff.
    Deferred,
    /// Observation-only refusal (root adb shell, unreadable inventory, unsettled owner):
    /// retried on `PREPARE_RETRY_DELAYS`, then terminal for this connection.
    RetryPrepare,
    /// Uncertain dispatch, USB refusal and unproved ownership: never an automatic retry.
    Terminal,
}

fn failure_disposition(stop: SetupStop, code: &str) -> FailureDisposition {
    if matches!(code, "HelperMaintenanceDraining" | "HelperMaintenanceBusyBeforeDispatch") {
        FailureDisposition::Deferred
    } else if stop == SetupStop::PreEffect && code != "HelperMaintenanceDisabled" {
        FailureDisposition::RetryPrepare
    } else {
        FailureDisposition::Terminal
    }
}

fn automatic_preparation_message(serial: &str, error: &anyhow::Error) -> String {
    match error.downcast_ref::<HelperShellIdentityRefused>() {
        Some(refusal) => refusal.operator_message(serial),
        None => "Chưa xác minh được Riviu Helper hoạt động; đã giữ bản ghi để xử lý tiếp.".into(),
    }
}

/// "Tự áp dụng khi máy kết nối": bring a newly connected (or re-plugged) phone to the
/// "Cài đặt máy" baseline under this connection's Repair lease, so it runs before helper
/// preparation and before the phone is admitted to publish/nurture/interaction work.
///
/// Idempotent -- a phone already at baseline is only read -- and never fatal: a phone that
/// cannot be brought to baseline is still prepared, and the result goes to the operation log
/// for the operator to read on the "Cài đặt máy" page.
async fn auto_apply_baseline(android: &AndroidDriver, db: &Database, serial: &str) {
    let config = match db.get_device_baseline_config() {
        Ok(config) => config,
        Err(error) => {
            record(db, "device.baseline.auto.failed", serial,
                &format!("Không đọc được cấu hình Cài đặt máy: {error:#}"));
            return;
        }
    };
    if !config.auto_apply_on_connect || config.settings.is_empty() {
        return;
    }
    let result = match android.apply_baseline(serial, &config.settings).await {
        Ok(items) => riviu_core::device_control::baseline::DeviceBaselineResult::from_items(serial, items),
        Err(error) => riviu_core::device_control::baseline::DeviceBaselineResult::whole_device(
            serial,
            riviu_core::device_control::baseline::BaselineOutcome::Failed,
            format!("không đọc được máy, chưa thay đổi gì: {error:#}"),
        ),
    };
    record(db, "device.baseline.auto", serial,
        &riviu_core::device_control::baseline::summarize(&result));
}

fn record(db: &Database, action: &str, serial: &str, message: &str) {
    if let Err(error) = db.log_op(action, &format!("Máy {serial}: {message}")) {
        log::error!("could not persist Android helper setup result for {serial}: {error:#}");
    }
}

async fn run_admitted<F: Future<Output = AttemptOutcome>>(
    control: &DeviceControlPlane,
    admission: &Arc<CommandAdmissionState>,
    serial: &str,
    install: impl FnOnce(riviu_core::DeviceExclusiveContext) -> F,
) -> AttemptOutcome {
    let Ok(_admission) = admission.ensure_accepting_work() else {
        return AttemptOutcome::Deferred;
    };
    // Do not borrow an overlay lease: a person using that phone wins outright.
    // Keeping the stream leaves scrcpy/minicap running throughout package setup.
    let lease = match control
        .try_acquire_exclusive_keeping_stream(serial, DeviceWorkOwner::Repair)
        .await
    {
        Ok(lease) => lease,
        Err(DeviceControlError::Busy(_)) => return AttemptOutcome::Deferred,
        Err(error) => {
            log::debug!("Android helper setup deferred on {serial}: {error}");
            return AttemptOutcome::Deferred;
        }
    };
    install(lease).await
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Ticket {
    serial: String,
    generation: u64,
}

struct Connection {
    generation: u64,
    finished: bool,
    recovery_generation: Option<HelperRecoveryGeneration>,
    deferred: u32,
    /// Prepare-stage retries already scheduled for this connection and failure episode.
    prepare_retries: usize,
    retry_at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttemptOutcome {
    Deferred,
    Failed(Option<HelperRecoveryGeneration>),
    /// Refused before any effect; `PREPARE_RETRY_DELAYS` decides whether it runs again.
    RetryPrepare(Option<HelperRecoveryGeneration>),
    Finished,
}

#[derive(Default)]
struct SetupQueue {
    connections: HashMap<String, Connection>,
    recovery: HashMap<String, HelperRecoveryGeneration>,
    next_generation: u64,
    tasks: JoinSet<AttemptOutcome>,
    tickets: HashMap<Id, Ticket>,
    stopped: bool,
}

impl SetupQueue {
    fn tick<F: Future<Output = AttemptOutcome> + Send + 'static>(
        &mut self,
        devices: &[DeviceInfo],
        available: impl Fn(&str) -> bool,
        mut prepare: impl FnMut(Ticket) -> F,
    ) {
        if self.stopped {
            return;
        }
        // Busy/preparing still belong to the same connection. Error is retained
        // as well: an unsuccessful probe must not grant another install attempt.
        let connected: HashSet<&str> = devices
            .iter()
            .filter(|device| {
                device.platform == DevicePlatform::Android
                    && !matches!(
                        device.status,
                        DeviceStatus::Disconnected | DeviceStatus::Pairing
                    )
            })
            .map(|device| device.udid.as_str())
            .collect();
        self.connections
            .retain(|serial, _| connected.contains(serial.as_str()));
        for serial in connected {
            if !self.connections.contains_key(serial) {
                self.next_generation += 1;
                self.connections.insert(
                    serial.to_string(),
                    Connection {
                        generation: self.next_generation,
                        finished: false,
                        recovery_generation: self.recovery.get(serial).copied(),
                        deferred: 0,
                        prepare_retries: 0,
                        retry_at: Instant::now(),
                    },
                );
            }
        }
        // Refresh connection identity before accepting completions: an old
        // worker must not acknowledge a phone that has since reconnected.
        while let Some(result) = self.tasks.try_join_next_with_id() {
            self.finish(result);
        }
        // Consume each new observed failure episode once, never once per poll.
        for (serial, connection) in &mut self.connections {
            if let Some(generation) = self.recovery.get(serial).copied() {
                if connection.recovery_generation != Some(generation) {
                    connection.recovery_generation = Some(generation);
                    connection.finished = false;
                    connection.deferred = 0;
                    connection.prepare_retries = 0;
                    connection.retry_at = Instant::now();
                }
            }
        }
        for device in devices {
            if self.tasks.len() >= MAX_CONCURRENT_INSTALLS {
                break;
            }
            let Some(connection) = self.connections.get(&device.udid) else {
                continue;
            };
            // A scheduled prepare retry runs while the roster shows its own latched failure.
            if device.platform != DevicePlatform::Android
                || !(matches!(device.status, DeviceStatus::Connected | DeviceStatus::Ready)
                    || (device.status == DeviceStatus::Error
                        && (connection.prepare_retries > 0
                            || (self.recovery.contains_key(&device.udid)
                                && device.last_error.as_deref() == Some(HELPER_SETUP_FAILURE)))))
                || !available(&device.udid)
                || self
                    .tickets
                    .values()
                    .any(|ticket| ticket.serial == device.udid)
            {
                continue;
            }
            if connection.finished || Instant::now() < connection.retry_at {
                continue;
            }
            let ticket = Ticket {
                serial: device.udid.clone(),
                generation: connection.generation,
            };
            let id = self.tasks.spawn(prepare(ticket.clone())).id();
            self.tickets.insert(id, ticket);
        }
    }

    fn finish(&mut self, result: Result<(Id, AttemptOutcome), JoinError>) {
        let (id, outcome) = match result {
            Ok(result) => result,
            Err(error) => {
                log::error!("Android helper setup task failed: {error}");
                (error.id(), AttemptOutcome::Finished)
            }
        };
        let Some(ticket) = self.tickets.remove(&id) else {
            return;
        };
        if let Some(connection) = self.connections.get_mut(&ticket.serial) {
            if connection.generation == ticket.generation {
                match outcome {
                    AttemptOutcome::Finished => connection.finished = true,
                    AttemptOutcome::Failed(generation) => {
                        connection.finished = true;
                        // The attempt itself can first observe failure. Consume that
                        // episode here so its next roster publication cannot retry it.
                        if generation.is_some() { connection.recovery_generation = generation; }
                    }
                    AttemptOutcome::RetryPrepare(generation) => {
                        if generation.is_some() { connection.recovery_generation = generation; }
                        match PREPARE_RETRY_DELAYS.get(connection.prepare_retries) {
                            Some(delay) => {
                                connection.prepare_retries += 1;
                                connection.retry_at = Instant::now() + *delay;
                            }
                            None => connection.finished = true,
                        }
                    }
                    AttemptOutcome::Deferred => {
                        connection.deferred = connection.deferred.saturating_add(1);
                        // Only admission/pre-dispatch contention reaches here. Cap delay,
                        // not drain duration: a long local job must eventually get a turn.
                        let seconds = 3u64 << connection.deferred.saturating_sub(1).min(4);
                        connection.retry_at = Instant::now() + Duration::from_secs(seconds);
                    }
                }
            }
        }
    }

    async fn drain(&mut self) {
        self.stopped = true;
        while let Some(result) = self.tasks.join_next_with_id().await {
            self.finish(result);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use riviu_core::{ConnectionKind, DeviceWorkCoordinator, StreamBudgetManager, TileStreamState};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn phone(serial: &str, status: DeviceStatus) -> DeviceInfo {
        DeviceInfo {
            udid: serial.into(),
            name: serial.into(),
            model: "fixture".into(),
            platform: DevicePlatform::Android,
            os_version: "fixture".into(),
            connection: ConnectionKind::Mock,
            status,
            battery: None,
            wda_ready: false,
            wda_expires_at: None,
            stream_url: None,
            tile_stream_state: TileStreamState::Parked,
            last_error: None,
        }
    }

    async fn complete_ready(queue: &mut SetupQueue) {
        while let Some(result) = queue.tasks.join_next_with_id().await {
            queue.finish(result);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn draining_connection_backs_off_without_delaying_a_healthy_sibling() {
        let mut queue = SetupQueue::default();
        let devices = vec![phone("draining", DeviceStatus::Ready)];
        queue.tick(&devices, |_| true, |_| async { AttemptOutcome::Deferred });
        complete_ready(&mut queue).await;
        let mut with_sibling = devices.clone();
        with_sibling.push(phone("healthy", DeviceStatus::Ready));
        let mut scheduled = Vec::new();
        queue.tick(
            &with_sibling,
            |_| true,
            |ticket| {
                scheduled.push(ticket.serial);
                async { AttemptOutcome::Finished }
            },
        );
        assert_eq!(
            scheduled,
            ["healthy"],
            "drain contention must not retry every poll"
        );
        complete_ready(&mut queue).await;
        for delay in [3, 6, 12, 24, 48, 48] {
            tokio::time::advance(Duration::from_secs(delay - 1)).await;
            queue.tick(
                &with_sibling,
                |_| true,
                |_| async { panic!("backoff not due") },
            );
            assert!(queue.tasks.is_empty());
            tokio::time::advance(Duration::from_secs(1)).await;
            let mut attempts = Vec::new();
            queue.tick(
                &with_sibling,
                |_| true,
                |ticket| {
                    attempts.push(ticket.serial);
                    async { AttemptOutcome::Deferred }
                },
            );
            assert_eq!(attempts, ["draining"]);
            complete_ready(&mut queue).await;
        }
        queue.drain().await;
    }

    #[tokio::test(start_paused = true)]
    async fn prepare_refusals_retry_on_a_finite_backoff_while_the_roster_shows_the_latch() {
        let mut queue = SetupQueue::default();
        let mut devices = vec![phone("a", DeviceStatus::Ready)];
        let generation = Some(HelperRecoveryGeneration(1));
        queue.tick(&devices, |_| true, move |_| async move { AttemptOutcome::RetryPrepare(generation) });
        complete_ready(&mut queue).await;
        // The roster now publishes the categorized latch for this phone.
        queue.recovery.insert("a".into(), HelperRecoveryGeneration(1));
        devices[0].status = DeviceStatus::Error;
        devices[0].last_error = Some(format!("{HELPER_SETUP_FAILURE}; stage=runtime_acquisition; category=shell_root; fixture"));
        for delay in [30, 120, 300] {
            tokio::time::advance(Duration::from_secs(delay - 1)).await;
            queue.tick(&devices, |_| true, |_| async { panic!("backoff not due") });
            assert!(queue.tasks.is_empty());
            tokio::time::advance(Duration::from_secs(1)).await;
            let mut attempts = 0;
            queue.tick(&devices, |_| true, |_| {
                attempts += 1;
                async move { AttemptOutcome::RetryPrepare(generation) }
            });
            assert_eq!(attempts, 1, "retry due after {delay}s");
            complete_ready(&mut queue).await;
        }
        tokio::time::advance(Duration::from_secs(3600)).await;
        queue.tick(&devices, |_| true, |_| async { panic!("retries exhausted") });
        assert!(queue.tasks.is_empty());
        assert!(queue.connections["a"].finished);
    }

    #[test]
    fn only_observation_only_refusals_are_retried_and_never_after_execution() {
        for code in ["HelperMaintenanceShellRoot", "HelperMaintenanceAdbUnavailable",
            "HelperMaintenancePrepareFailed", "HelperAutomaticPreparationFailed"] {
            assert_eq!(failure_disposition(SetupStop::PreEffect, code), FailureDisposition::RetryPrepare, "{code}");
            // Execution started (or a claim may have been sent): never an automatic retry.
            assert_eq!(failure_disposition(SetupStop::Terminal, code), FailureDisposition::Terminal, "{code}");
        }
        assert_eq!(failure_disposition(SetupStop::Terminal, "HelperMaintenanceExecutionUnproved"), FailureDisposition::Terminal);
        assert_eq!(failure_disposition(SetupStop::PreEffect, "HelperMaintenanceDisabled"), FailureDisposition::Terminal);
        assert_eq!(failure_disposition(SetupStop::PreEffect, "HelperMaintenanceDraining"), FailureDisposition::Deferred);
        assert_eq!(failure_disposition(SetupStop::Terminal, "HelperMaintenanceBusyBeforeDispatch"), FailureDisposition::Deferred);
    }

    #[tokio::test]
    async fn only_idle_android_connections_are_scheduled_and_finished_ones_do_not_repeat() {
        let mut queue = SetupQueue::default();
        let mut ios = phone("ios", DeviceStatus::Ready);
        ios.platform = DevicePlatform::Ios;
        let devices = vec![
            ios,
            phone("offline", DeviceStatus::Disconnected),
            phone("untrusted", DeviceStatus::Pairing),
            phone("starting", DeviceStatus::Preparing),
            phone("busy", DeviceStatus::Busy),
            phone("error", DeviceStatus::Error),
            phone("ready", DeviceStatus::Ready),
        ];
        let mut scheduled = Vec::new();
        queue.tick(
            &devices,
            |_| true,
            |ticket| {
                scheduled.push(ticket.serial);
                async { AttemptOutcome::Finished }
            },
        );
        assert_eq!(scheduled, ["ready"]);
        complete_ready(&mut queue).await;
        queue.tick(
            &devices,
            |_| true,
            |_| async { panic!("finished connection must not retry") },
        );
        assert!(queue.tasks.is_empty());
    }

    #[tokio::test]
    async fn finished_connection_rearms_once_per_observed_recovery_episode() {
        let mut queue = SetupQueue::default();
        let mut devices = vec![phone("a", DeviceStatus::Ready)];
        queue.tick(&devices, |_| true, |_| async { AttemptOutcome::Finished });
        complete_ready(&mut queue).await;
        let connection = queue.connections["a"].generation;
        devices[0].status = DeviceStatus::Error;
        devices[0].last_error = Some(HELPER_SETUP_FAILURE.into());
        queue.recovery.insert("a".into(), HelperRecoveryGeneration(1));
        queue.tick(&devices, |_| false, |_| async { panic!("Repair admission busy") });
        assert!(queue.tasks.is_empty());
        queue.tick(&devices, |_| true, |_| async { AttemptOutcome::Failed(Some(HelperRecoveryGeneration(1))) });
        assert_eq!(queue.tasks.len(), 1);
        complete_ready(&mut queue).await;
        for _ in 0..3 {
            queue.tick(&devices, |_| true, |_| async { panic!("same failed episode") });
            assert!(queue.tasks.is_empty());
        }
        queue.recovery.insert("a".into(), HelperRecoveryGeneration(2));
        devices[0].last_error = Some("unrelated metadata failure".into());
        queue.tick(&devices, |_| true, |_| async { panic!("unrelated Error") });
        assert!(queue.tasks.is_empty());
        devices[0].last_error = Some(HELPER_SETUP_FAILURE.into());
        queue.tick(&devices, |_| true, |_| async { AttemptOutcome::Finished });
        assert_eq!(queue.tasks.len(), 1);
        complete_ready(&mut queue).await;
        assert_eq!(queue.connections["a"].generation, connection, "no USB reconnect required");
    }

    #[tokio::test]
    async fn first_attempt_failure_consumes_its_own_observed_generation() {
        let mut queue = SetupQueue::default();
        let devices = vec![phone("a", DeviceStatus::Ready)];
        queue.tick(&devices, |_| true, |_| async { AttemptOutcome::Failed(Some(HelperRecoveryGeneration(1))) });
        complete_ready(&mut queue).await;
        queue.recovery.insert("a".into(), HelperRecoveryGeneration(1));
        queue.tick(&devices, |_| true, |_| async { panic!("failure is not a new retry") });
        assert!(queue.tasks.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn busy_admission_is_pending_and_does_not_starve_an_idle_device() {
        let mut queue = SetupQueue::default();
        let devices = vec![
            phone("busy", DeviceStatus::Ready),
            phone("idle", DeviceStatus::Ready),
        ];
        let mut scheduled = Vec::new();
        queue.tick(
            &devices,
            |serial| serial != "busy",
            |ticket| {
                scheduled.push(ticket.serial);
                async { AttemptOutcome::Deferred }
            },
        );
        assert_eq!(scheduled, ["idle"]);
        complete_ready(&mut queue).await;
        scheduled.clear();
        tokio::time::advance(Duration::from_secs(3)).await;
        queue.tick(
            &devices,
            |_| true,
            |ticket| {
                scheduled.push(ticket.serial);
                async { AttemptOutcome::Finished }
            },
        );
        assert_eq!(scheduled, ["busy", "idle"]);
        complete_ready(&mut queue).await;
    }

    #[tokio::test(start_paused = true)]
    async fn two_slow_installs_do_not_block_tick_or_admit_a_third() {
        let mut queue = SetupQueue::default();
        let devices = vec![
            phone("a", DeviceStatus::Ready),
            phone("b", DeviceStatus::Ready),
            phone("c", DeviceStatus::Ready),
        ];
        let started = tokio::time::Instant::now();
        queue.tick(
            &devices,
            |_| true,
            |_| async {
                tokio::time::sleep(Duration::from_secs(300)).await;
                AttemptOutcome::Finished
            },
        );
        assert_eq!(queue.tasks.len(), 2);
        queue.tick(
            &devices,
            |_| true,
            |_| async { panic!("capacity exhausted") },
        );
        assert_eq!(started.elapsed(), Duration::ZERO);
        complete_ready(&mut queue).await;
        let mut scheduled = Vec::new();
        queue.tick(
            &devices,
            |_| true,
            |ticket| {
                scheduled.push(ticket.serial);
                async { AttemptOutcome::Finished }
            },
        );
        assert_eq!(scheduled, ["c"]);
        queue.drain().await;
    }

    #[tokio::test]
    async fn reconnect_waits_for_old_worker_and_rejects_its_completion() {
        let mut queue = SetupQueue::default();
        let online = vec![phone("a", DeviceStatus::Ready)];
        let (finish, wait) = tokio::sync::oneshot::channel();
        let mut wait = Some(wait);
        queue.tick(
            &online,
            |_| true,
            |_| {
                let wait = wait.take().unwrap();
                async {
                    wait.await.unwrap();
                    AttemptOutcome::Finished
                }
            },
        );
        let old_generation = queue.connections["a"].generation;
        queue.tick(
            &[phone("a", DeviceStatus::Disconnected)],
            |_| true,
            |_| async { panic!("offline") },
        );
        queue.tick(
            &online,
            |_| true,
            |_| async { panic!("old worker still owns serial") },
        );
        assert_ne!(queue.connections["a"].generation, old_generation);
        finish.send(()).unwrap();
        complete_ready(&mut queue).await;
        assert!(!queue.connections["a"].finished);
        queue.tick(&online, |_| true, |_| async { AttemptOutcome::Finished });
        complete_ready(&mut queue).await;
        assert!(queue.connections["a"].finished);
    }

    #[tokio::test]
    async fn failed_worker_is_terminal_until_observed_reconnect() {
        let mut queue = SetupQueue::default();
        let online = vec![phone("a", DeviceStatus::Ready)];
        queue.tick(
            &online,
            |_| true,
            |_| async { panic!("fixture package worker failure") },
        );
        complete_ready(&mut queue).await;
        queue.tick(&online, |_| true, |_| async { panic!("must not retry") });
        assert!(queue.tasks.is_empty());
        queue.tick(&[], |_| true, |_| async { panic!("no devices") });
        queue.tick(&online, |_| true, |_| async { AttemptOutcome::Finished });
        assert_eq!(queue.tasks.len(), 1);
        queue.drain().await;
    }

    #[tokio::test(start_paused = true)]
    async fn drain_finishes_admitted_work_and_disables_future_ticks() {
        let mut queue = SetupQueue::default();
        let completed = Arc::new(AtomicUsize::new(0));
        let counter = completed.clone();
        let online = vec![phone("a", DeviceStatus::Ready)];
        queue.tick(
            &online,
            |_| true,
            move |_| {
                let counter = counter.clone();
                async move {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    counter.fetch_add(1, Ordering::Relaxed);
                    AttemptOutcome::Finished
                }
            },
        );
        queue.drain().await;
        assert_eq!(completed.load(Ordering::Relaxed), 1);
        queue.tick(&online, |_| true, |_| async { panic!("stopped") });
        assert!(queue.tasks.is_empty());
    }

    #[tokio::test]
    async fn exclusive_owner_and_shutdown_guard_prevent_install_without_touching_streams() {
        let control = Arc::new(DeviceControlPlane::new(
            Arc::new(riviu_ios_driver::MockIosDriver::new()),
            Arc::new(DeviceWorkCoordinator::new()),
            Arc::new(StreamBudgetManager::new(2).unwrap()),
        ));
        let admission = Arc::new(CommandAdmissionState::new(true));
        let owned = control
            .try_acquire_exclusive_keeping_stream("fixture", DeviceWorkOwner::ManualControl)
            .await
            .unwrap();
        assert_eq!(
            run_admitted(&control, &admission, "fixture", |context| async move {
                let _context = context;
                panic!("busy phone")
            })
            .await,
            AttemptOutcome::Deferred
        );
        drop(owned);
        let observed_control = &control;
        let observed_admission = &admission;
        let result = run_admitted(&control, &admission, "fixture", |context| async move {
            let _context = context;
            assert_eq!(
                observed_control.current_work_owner("fixture"),
                Some(DeviceWorkOwner::Repair)
            );
            assert_eq!(observed_admission.in_flight.load(Ordering::Acquire), 1);
            AttemptOutcome::Finished
        })
        .await;
        assert_eq!(result, AttemptOutcome::Finished);
        assert_eq!(control.current_work_owner("fixture"), None);
        assert_eq!(admission.in_flight.load(Ordering::Acquire), 0);
        admission.reject_new_work();
        assert_eq!(
            run_admitted(&control, &admission, "fixture", |context| async move {
                let _context = context;
                panic!("shutting down")
            })
            .await,
            AttemptOutcome::Deferred
        );
        control.shutdown_cleanup().await.unwrap();
    }
}
