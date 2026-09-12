//! Install the bundled Android helper once for each observed connection.
//!
//! This is a child of `state` so preparation participates in the same command
//! admission and shutdown drain as operator work. Inventory only schedules it;
//! installation never runs on the roster/sampler task.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;

use riviu_android_driver::AndroidDriver;
use riviu_core::db::Database;
use riviu_core::{
    DeviceControlError, DeviceControlPlane, DeviceInfo, DevicePlatform, DeviceStatus,
    DeviceWorkOwner,
};
use tokio::task::{Id, JoinError, JoinSet};

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
        self.queue.tick(
            devices,
            |serial| self.control.current_work_owner(serial).is_none(),
            move |ticket| {
                let control = control.clone();
                let android = android.clone();
                let admission = admission.clone();
                let db = db.clone();
                async move {
                    run_admitted(&control, &admission, &ticket.serial, || async {
                        record(
                            &db,
                            "agent.helper.setup.started",
                            &ticket.serial,
                            "Đang kiểm tra gói Riviu Helper.",
                        );
                        match android.ensure_helper_installed(&ticket.serial).await {
                            Ok(()) => record(
                                &db,
                                "agent.helper.setup.succeeded",
                                &ticket.serial,
                                "Đã xác minh Riviu Helper và biểu tượng ứng dụng.",
                            ),
                            Err(error) => record(
                                &db,
                                "agent.helper.setup.failed",
                                &ticket.serial,
                                &format!("Chưa cài được Riviu Helper: {error:#}"),
                            ),
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

fn record(db: &Database, action: &str, serial: &str, message: &str) {
    if let Err(error) = db.log_op(action, &format!("Máy {serial}: {message}")) {
        log::error!("could not persist Android helper setup result for {serial}: {error:#}");
    }
}

async fn run_admitted<F: Future<Output = ()>>(
    control: &DeviceControlPlane,
    admission: &Arc<CommandAdmissionState>,
    serial: &str,
    install: impl FnOnce() -> F,
) -> AttemptOutcome {
    let Ok(_admission) = admission.ensure_accepting_work() else {
        return AttemptOutcome::Deferred;
    };
    // Do not borrow an overlay lease: a person using that phone wins outright.
    // Keeping the stream leaves scrcpy/minicap running throughout package setup.
    let _lease = match control
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
    install().await;
    // A failure is terminal too. Retrying package installation needs a new
    // observed connection; a three-second scan must never repeat a USB refusal.
    AttemptOutcome::Finished
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Ticket {
    serial: String,
    generation: u64,
}

struct Connection {
    generation: u64,
    finished: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttemptOutcome {
    Deferred,
    Finished,
}

#[derive(Default)]
struct SetupQueue {
    connections: HashMap<String, Connection>,
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
                    },
                );
            }
        }
        // Refresh connection identity before accepting completions: an old
        // worker must not acknowledge a phone that has since reconnected.
        while let Some(result) = self.tasks.try_join_next_with_id() {
            self.finish(result);
        }
        for device in devices {
            if self.tasks.len() >= MAX_CONCURRENT_INSTALLS {
                break;
            }
            if device.platform != DevicePlatform::Android
                || !matches!(device.status, DeviceStatus::Connected | DeviceStatus::Ready)
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
            if connection.finished {
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
            if connection.generation == ticket.generation && outcome == AttemptOutcome::Finished {
                connection.finished = true;
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
            run_admitted(&control, &admission, "fixture", || async {
                panic!("busy phone")
            })
            .await,
            AttemptOutcome::Deferred
        );
        drop(owned);
        let result = run_admitted(&control, &admission, "fixture", || async {
            assert_eq!(
                control.current_work_owner("fixture"),
                Some(DeviceWorkOwner::Repair)
            );
            assert_eq!(admission.in_flight.load(Ordering::Acquire), 1);
        })
        .await;
        assert_eq!(result, AttemptOutcome::Finished);
        assert_eq!(control.current_work_owner("fixture"), None);
        assert_eq!(admission.in_flight.load(Ordering::Acquire), 0);
        admission.reject_new_work();
        assert_eq!(
            run_admitted(&control, &admission, "fixture", || async {
                panic!("shutting down")
            })
            .await,
            AttemptOutcome::Deferred
        );
        control.shutdown_cleanup().await.unwrap();
    }
}
