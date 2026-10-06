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
use riviu_core::db::Database;
use riviu_core::{
    DeviceControlError, DeviceControlPlane, DeviceInfo, DevicePlatform, DeviceStatus,
    DeviceWorkOwner,
};
use tokio::task::{Id, JoinError, JoinSet};
use tokio::time::{Duration, Instant};

use super::CommandAdmissionState;

const MAX_CONCURRENT_INSTALLS: usize = 2;

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
                            record(&db, "agent.helper.setup.started", &serial,
                                "Đang chuẩn bị Riviu Helper để điều khiển thiết bị.");
                            let result = async {
                                if !crate::agent_commands::helper_maintenance_is_pending(&db, &serial)? {
                                match android.prepare_helper_runtime(&serial).await {
                                    Ok(_) => return Ok(()),
                                    Err(error) if error.is::<riviu_android_driver::riviu_agent::HelperRecoveryRequired>() => {}
                                    Err(error) => return Err(crate::command_error::CommandError::code(
                                        "HelperAutomaticPreparationFailed", automatic_preparation_message(&error))),
                                }
                                }
                                crate::agent_commands::automatic_helper_maintenance(
                                    &control, &db, &context, &serial).await?;
                                android.prepare_helper_runtime(&serial).await
                                    .map(|_| ())
                                    .map_err(|error| crate::command_error::CommandError::code(
                                        "HelperAutomaticPreparationFailed", automatic_preparation_message(&error)))
                            }.await;
                            match result {
                                Ok(()) => {
                                    record(&db, "agent.helper.setup.succeeded", &serial,
                                        "Đã xác minh Riviu Helper hoạt động.");
                                    AttemptOutcome::Finished
                                }
                                Err(error) => {
                                    record(&db, "agent.helper.setup.failed", &serial,
                                        &format!("{}: {}", error.code, error.message));
                                    if matches!(error.code.as_str(), "HelperMaintenanceDraining"
                                        | "HelperMaintenanceBusyBeforeDispatch") {
                                        AttemptOutcome::Deferred
                                    } else {
                                        // Uncertain dispatch, USB refusal and unproved ownership are
                                        // terminal for this connection, never automatic effect retries.
                                        android.record_helper_preparation_failure(&serial);
                                        AttemptOutcome::Failed(android.helper_recovery_snapshot().get(&serial).copied())
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

fn automatic_preparation_message(error: &anyhow::Error) -> String {
    let message = error.to_string();
    if message.starts_with("helper runtime requires shell UID2000; observed adbd UID ") {
        message
    } else {
        "Chưa xác minh được Riviu Helper hoạt động; đã giữ bản ghi để xử lý tiếp.".into()
    }
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
    retry_at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttemptOutcome {
    Deferred,
    Failed(Option<HelperRecoveryGeneration>),
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
                    connection.retry_at = Instant::now();
                }
            }
        }
        for device in devices {
            if self.tasks.len() >= MAX_CONCURRENT_INSTALLS {
                break;
            }
            if device.platform != DevicePlatform::Android
                || !(matches!(device.status, DeviceStatus::Connected | DeviceStatus::Ready)
                    || (device.status == DeviceStatus::Error
                        && self.recovery.contains_key(&device.udid)
                        && device.last_error.as_deref() == Some(HELPER_SETUP_FAILURE)))
                || !available(&device.udid)
                || self
                    .tickets
                    .values()
                    .any(|ticket| ticket.serial == device.udid)
            {
                continue;
            }
            let Some(connection) = self.connections.get(&device.udid) else {
                continue;
            };
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
