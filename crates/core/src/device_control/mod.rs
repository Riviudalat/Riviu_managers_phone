use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use thiserror::Error;
use tokio::sync::{mpsc, oneshot, Notify};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::{
    AgentInstallProof, AgentSettings, AgentStatus, AppProcessState, BackgroundStreamLease,
    ClipboardAccessMode, DeviceBusy, DeviceCapabilityRegistry, DeviceCapabilitySnapshot,
    DeviceControllerCapabilities, DeviceDriver, DeviceInfo, DeviceWorkCoordinator, DeviceWorkLease,
    DeviceWorkOwner, ForegroundStreamReservation, GuardedClipboardOperation,
    GuardedClipboardOutput, GuardedClipboardProgress, InteractionSessionKind, ProcessAbsenceProof,
    StreamBudgetError, StreamBudgetManager, StreamStartProof, StreamStopProof, UiSession,
};

#[derive(Debug, Error)]
pub enum DeviceControlError {
    #[error(transparent)]
    Busy(#[from] DeviceBusy),
    #[error(transparent)]
    StreamBudget(#[from] StreamBudgetError),
    #[error("{operation} failed for device {udid}: {message}")]
    Driver {
        udid: String,
        operation: &'static str,
        message: String,
    },
    #[error("device context is not valid for this control plane: {reason}")]
    InvalidContext { reason: &'static str },
    #[error(
        "stream stop proof for device {udid} started at generation {actual}; expected {expected}"
    )]
    StopProofMismatch {
        udid: String,
        expected: u64,
        actual: u64,
    },
    #[error("stream generation for device {udid} was not observed before cleanup")]
    StopGenerationUnknown { udid: String },
    #[error("stream for device {udid} did not produce a decoded first frame")]
    FirstFrameMissing { udid: String },
    #[error("the device cleanup worker is closed")]
    CleanupWorkerClosed,
    #[error("the device control plane is shutting down")]
    ControlPlaneShuttingDown,
    #[error("the device control plane is stopped")]
    ControlPlaneStopped,
    #[error("background stream for device {udid} is blocked by {current_owner:?}")]
    BackgroundStreamBlocked {
        udid: String,
        current_owner: DeviceWorkOwner,
    },
    #[error("background stream capacity is changing while reserving device {udid}")]
    BackgroundStreamTransitionBusy { udid: String },
    #[error("{count} device cleanup ticket(s) remain quarantined")]
    CleanupQuarantined { count: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundAppProof {
    pub udid: String,
    pub bundle_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceReleaseProof {
    pub udid: String,
    pub owner: DeviceWorkOwner,
    pub stopped_generation: u64,
    pub next_generation: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextReleaseProof {
    pub udid: String,
    pub owner: DeviceWorkOwner,
    pub had_session: bool,
    pub had_stream: bool,
}

impl From<DeviceReleaseProof> for ContextReleaseProof {
    fn from(proof: DeviceReleaseProof) -> Self {
        Self {
            udid: proof.udid,
            owner: proof.owner,
            had_session: true,
            had_stream: true,
        }
    }
}

#[derive(Debug)]
pub enum InteractionAcquireResult {
    Acquired(DeviceExclusiveContext),
    SkippedUnavailable(DeviceBusy),
}

/// The sole high-level owner of device UI lifecycle and stream capacity.
///
/// The driver intentionally remains private. Callers must first obtain one of
/// the typed contexts below; each transition validates its plane, UDID, owner,
/// and current work token before touching the driver.
pub struct DeviceControlPlane {
    driver: Arc<dyn DeviceDriver>,
    work: Arc<DeviceWorkCoordinator>,
    streams: Arc<StreamBudgetManager>,
    capability_registry: Arc<DeviceCapabilityRegistry>,
    cleanup_tx: mpsc::UnboundedSender<WorkerCommand>,
    cleanup_handle: Mutex<Option<JoinHandle<()>>>,
    quarantined: Arc<QuarantineStore>,
    backgrounds: Arc<BackgroundStore>,
    capacity_gate: Arc<tokio::sync::Mutex<()>>,
    lifecycle: Arc<ControlPlaneLifecycle>,
    shutdown_gate: tokio::sync::Mutex<()>,
    background_gate: Mutex<()>,
    plane_id: Uuid,
}

mod apps;
mod leases;
mod roster;
mod sessions;
mod streams;

impl DeviceControlPlane {
    pub fn new(
        driver: Arc<dyn DeviceDriver>,
        work: Arc<DeviceWorkCoordinator>,
        streams: Arc<StreamBudgetManager>,
    ) -> Self {
        Self::new_with_capability_registry(
            driver,
            work,
            streams,
            Arc::new(DeviceCapabilityRegistry::empty()),
        )
    }

    pub fn new_with_capability_registry(
        driver: Arc<dyn DeviceDriver>,
        work: Arc<DeviceWorkCoordinator>,
        streams: Arc<StreamBudgetManager>,
        capability_registry: Arc<DeviceCapabilityRegistry>,
    ) -> Self {
        let (cleanup_tx, cleanup_rx) = mpsc::unbounded_channel();
        let quarantined = Arc::new(QuarantineStore::default());
        let backgrounds = Arc::new(BackgroundStore::default());
        let operation_locks = Arc::new(DeviceOperationLocks::default());
        let capacity_gate = Arc::new(tokio::sync::Mutex::new(()));
        let lifecycle = Arc::new(ControlPlaneLifecycle::default());
        let cleanup_handle = tokio::spawn(run_cleanup_worker(
            cleanup_rx,
            driver.clone(),
            work.clone(),
            streams.clone(),
            quarantined.clone(),
            backgrounds.clone(),
            operation_locks.clone(),
            capacity_gate.clone(),
            cleanup_tx.clone(),
        ));

        Self {
            driver,
            work,
            streams,
            capability_registry,
            cleanup_tx,
            cleanup_handle: Mutex::new(Some(cleanup_handle)),
            quarantined,
            backgrounds,
            capacity_gate,
            lifecycle,
            shutdown_gate: tokio::sync::Mutex::new(()),
            background_gate: Mutex::new(()),
            plane_id: Uuid::new_v4(),
        }
    }

    pub(crate) async fn try_reserve_ui_capacity(
        &self,
        context: DeviceExclusiveContext,
    ) -> Result<(DeviceExclusiveContext, UiCapacityReservation), CapacityContextUpgradeFailure>
    {
        if let Err(error) = self.validate_exclusive(&context) {
            return Err(CapacityContextUpgradeFailure {
                context: Some(context),
                error,
            });
        }
        let reserved = self.submit_reserve(context).await?;
        let (context, raw_reservation) = reserved.into_parts();
        let reservation = UiCapacityReservation {
            plane_id: self.plane_id,
            reservation: Some(raw_reservation),
            streams: self.streams.clone(),
        };
        Ok((context, reservation))
    }

    pub(crate) async fn read_active_app_bundle(
        &self,
        context: &DeviceExclusiveContext,
    ) -> Result<String, DeviceControlError> {
        let lease = self.validate_exclusive(context)?;
        self.driver
            .read_active_app_bundle(lease.udid())
            .await
            .map_err(|error| driver_error(lease.udid(), "readActiveAppBundle", error))
    }

    pub(crate) async fn try_start_interaction_session(
        &self,
        mut context: DeviceExclusiveContext,
        bundle_id: &str,
        kind: InteractionSessionKind,
    ) -> Result<UiSessionContext, SessionContextUpgradeFailure> {
        let capacity_token = match self.validate_interaction_capacity(&context) {
            Ok(token) => token,
            Err(error) => return Err(SessionContextUpgradeFailure { context, error }),
        };
        let udid = context.udid().to_string();
        let handoff = match self.driver.confirm_interaction_stream_stopped(&udid).await {
            Ok(proof) => proof,
            Err(error) => {
                return Err(SessionContextUpgradeFailure {
                    context,
                    error: driver_error(&udid, "confirmInteractionStreamStopped", error),
                });
            }
        };
        if let Err(error) = self.validate_interaction_capacity(&context) {
            return Err(SessionContextUpgradeFailure { context, error });
        }
        let session = match self
            .driver
            .start_interaction_session(&udid, bundle_id, kind)
            .await
        {
            Ok(session) => session,
            Err(error) => {
                return Err(SessionContextUpgradeFailure {
                    context,
                    error: driver_error(&udid, "startInteractionSession", error),
                });
            }
        };
        if let Err(error) = self.validate_interaction_capacity(&context) {
            self.driver.invalidate_ui_session(&udid);
            return Err(SessionContextUpgradeFailure { context, error });
        }

        Ok(UiSessionContext {
            plane_id: self.plane_id,
            lease: context.lease.take(),
            activity: context.activity.take(),
            session: Some(Arc::from(session)),
            ui_capacity_token: Some(capacity_token),
            stream_handoff_generation: Some(handoff.generation),
        })
    }

    pub(crate) async fn try_foreground_target_app_and_start_interaction_session(
        &self,
        mut context: DeviceExclusiveContext,
        bundle_id: &str,
        kind: InteractionSessionKind,
    ) -> Result<(UiSessionContext, ForegroundAppProof), SessionContextUpgradeFailure> {
        let capacity_token = match self.validate_interaction_capacity(&context) {
            Ok(token) => token,
            Err(error) => return Err(SessionContextUpgradeFailure { context, error }),
        };
        let udid = context.udid().to_string();
        let handoff = match self.driver.confirm_interaction_stream_stopped(&udid).await {
            Ok(proof) => proof,
            Err(error) => {
                return Err(SessionContextUpgradeFailure {
                    context,
                    error: driver_error(&udid, "confirmFlowStreamStopped", error),
                })
            }
        };
        if let Err(error) = self.validate_interaction_capacity(&context) {
            return Err(SessionContextUpgradeFailure { context, error });
        }
        let session = match self
            .driver
            .foreground_target_app_and_start_interaction_session(&udid, bundle_id, kind)
            .await
        {
            Ok(session) => session,
            Err(error) => {
                return Err(SessionContextUpgradeFailure {
                    context,
                    error: driver_error(&udid, "foregroundTargetAppAndStartSession", error),
                });
            }
        };
        if let Err(error) = self.validate_interaction_capacity(&context) {
            self.driver.invalidate_ui_session(&udid);
            return Err(SessionContextUpgradeFailure { context, error });
        }
        let proof = ForegroundAppProof {
            udid,
            bundle_id: bundle_id.to_string(),
        };
        Ok((
            UiSessionContext {
                plane_id: self.plane_id,
                lease: context.lease.take(),
                activity: context.activity.take(),
                session: Some(Arc::from(session)),
                ui_capacity_token: Some(capacity_token),
                stream_handoff_generation: Some(handoff.generation),
            },
            proof,
        ))
    }

    pub(crate) async fn try_start_reserved_stream(
        &self,
        context: UiSessionContext,
        capacity: UiCapacityReservation,
    ) -> Result<UiWithStreamContext, StreamContextUpgradeFailure> {
        self.start_reserved_stream_internal(context, capacity).await
    }

    pub(crate) fn failed_stream_session(
        &self,
        context: &FailedStreamStartContext,
    ) -> Result<Arc<dyn UiSession>, DeviceControlError> {
        if context.plane_id != self.plane_id {
            return Err(DeviceControlError::InvalidContext {
                reason: "failed stream context belongs to another control plane",
            });
        }
        Ok(context.session())
    }

    pub(crate) async fn close_failed_stream_start(
        &self,
        mut context: FailedStreamStartContext,
    ) -> Result<ContextReleaseProof, DeviceControlError> {
        if context.plane_id != self.plane_id {
            return Err(DeviceControlError::InvalidContext {
                reason: "failed stream context belongs to another control plane",
            });
        }
        let mut pending = context
            .pending
            .take()
            .expect("live failed stream context retains pending ownership");
        let ticket = pending
            .ticket
            .take()
            .expect("live failed stream context retains its cleanup ticket");
        self.close_cleanup_ticket(ticket).await.map(Into::into)
    }
}

fn driver_error(udid: &str, operation: &'static str, error: anyhow::Error) -> DeviceControlError {
    DeviceControlError::Driver {
        udid: udid.to_string(),
        operation,
        message: error.to_string(),
    }
}

fn validate_stop_generation(
    udid: &str,
    expected: u64,
    proof: StreamStopProof,
) -> Result<(), DeviceControlError> {
    if proof.old_generation != expected {
        return Err(DeviceControlError::StopProofMismatch {
            udid: udid.to_string(),
            expected,
            actual: proof.old_generation,
        });
    }
    if !proof.child_stopped || proof.new_generation <= proof.old_generation {
        return Err(DeviceControlError::StreamBudget(
            StreamBudgetError::StopNotConfirmed {
                udid: udid.to_string(),
            },
        ));
    }
    Ok(())
}

pub struct DeviceExclusiveContext {
    plane_id: Uuid,
    lease: Option<DeviceWorkLease>,
    activity: Option<ContextActivityPermit>,
    ui_capacity_token: Option<Uuid>,
}

/// A borrowed, still-live device lease — whichever kind of context is holding it.
///
/// **Why this exists.** Every device action below wants exactly one thing from the context
/// it is handed: a lease this plane can still validate. Which *kind* of context holds that
/// lease is the caller's business, not the action's.
///
/// Demanding a [`DeviceExclusiveContext`] specifically had a cost nobody intended. The
/// desktop's control overlay opens a [`UiSessionContext`] and holds it for as long as the
/// operator has the phone on screen, so ten of its own actions — rotate, install, adb,
/// import, export, reboot, backup, restore, screenshot, change keyboard — had to ask for a
/// *second* lease on a phone this process already owned. `try_acquire` refuses that, by
/// design, so every one of them failed `DeviceBusy` **whenever the overlay was open**, which
/// is the only time they can be reached. See AGENTS.md 9.82.
///
/// This lends the held lease instead. It is not shared ownership and it is not re-entrancy:
/// exactly one lease per device still exists, and a phone held by nurture, flow, a script or
/// a repair is still refused — the borrow is only ever offered by the UI that opened it.
// No `Debug`: `UiSessionContext` holds an `Arc<dyn UiSession>` and does not derive it.
#[derive(Clone, Copy)]
pub enum DeviceLeaseRef<'a> {
    Exclusive(&'a DeviceExclusiveContext),
    Session(&'a UiSessionContext),
}

impl<'a> From<&'a DeviceExclusiveContext> for DeviceLeaseRef<'a> {
    fn from(context: &'a DeviceExclusiveContext) -> Self {
        Self::Exclusive(context)
    }
}

impl<'a> From<&'a UiSessionContext> for DeviceLeaseRef<'a> {
    fn from(context: &'a UiSessionContext) -> Self {
        Self::Session(context)
    }
}

pub(crate) struct SessionContextUpgradeFailure {
    pub(crate) context: DeviceExclusiveContext,
    pub(crate) error: DeviceControlError,
}

pub(crate) struct CapacityContextUpgradeFailure {
    pub(crate) context: Option<DeviceExclusiveContext>,
    pub(crate) error: DeviceControlError,
}

impl DeviceExclusiveContext {
    pub fn udid(&self) -> &str {
        self.lease
            .as_ref()
            .expect("a live exclusive context has a lease")
            .udid()
    }

    pub fn owner(&self) -> DeviceWorkOwner {
        self.lease
            .as_ref()
            .expect("a live exclusive context has a lease")
            .owner()
    }
}

impl fmt::Debug for DeviceExclusiveContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceExclusiveContext")
            .field("plane_id", &self.plane_id)
            .field("lease", &self.lease)
            .field("ui_capacity_token", &self.ui_capacity_token)
            .finish()
    }
}

pub struct UiSessionContext {
    plane_id: Uuid,
    lease: Option<DeviceWorkLease>,
    activity: Option<ContextActivityPermit>,
    session: Option<Arc<dyn UiSession>>,
    ui_capacity_token: Option<Uuid>,
    stream_handoff_generation: Option<u64>,
}

pub(crate) struct StreamContextUpgradeFailure {
    pub(crate) context: Option<UiSessionContext>,
    pub(crate) failed_start: Option<FailedStreamStartContext>,
    pub(crate) error: DeviceControlError,
}

pub(crate) struct FailedStreamStartContext {
    plane_id: Uuid,
    pending: Option<PendingStreamStart>,
}

impl FailedStreamStartContext {
    fn session(&self) -> Arc<dyn UiSession> {
        self.pending
            .as_ref()
            .and_then(|pending| pending.ticket.as_ref())
            .expect("live failed stream start retains its cleanup ticket")
            .session
            .clone()
    }
}

/// Cancellation-safe ownership of a foreground stream slot before a producer
/// is started. Dropping it releases only a still-reserved slot; once consumed,
/// producer cleanup is owned by [`UiWithStreamContext`].
pub struct UiCapacityReservation {
    plane_id: Uuid,
    reservation: Option<ForegroundStreamReservation>,
    streams: Arc<StreamBudgetManager>,
}

impl UiCapacityReservation {
    pub fn udid(&self) -> &str {
        self.reservation
            .as_ref()
            .expect("a live capacity context has a reservation")
            .udid()
    }

    pub fn owner(&self) -> DeviceWorkOwner {
        self.reservation
            .as_ref()
            .expect("a live capacity context has a reservation")
            .owner()
    }
}

impl Drop for UiCapacityReservation {
    fn drop(&mut self) {
        if let Some(reservation) = self.reservation.take() {
            let _ = self.streams.release_reserved(reservation.token());
        }
    }
}

impl UiSessionContext {
    pub fn udid(&self) -> &str {
        self.lease
            .as_ref()
            .expect("a live session context has a lease")
            .udid()
    }

    pub fn owner(&self) -> DeviceWorkOwner {
        self.lease
            .as_ref()
            .expect("a live session context has a lease")
            .owner()
    }
}

pub struct UiWithStreamContext {
    plane_id: Uuid,
    lease: Option<DeviceWorkLease>,
    activity: Option<ContextActivityPermit>,
    reservation: Option<ForegroundStreamReservation>,
    session: Option<Arc<dyn UiSession>>,
    start_proof: StreamStartProof,
    cleanup: CleanupSink,
}

impl UiWithStreamContext {
    pub fn udid(&self) -> &str {
        self.lease
            .as_ref()
            .expect("a live stream context has a lease")
            .udid()
    }

    pub fn owner(&self) -> DeviceWorkOwner {
        self.lease
            .as_ref()
            .expect("a live stream context has a lease")
            .owner()
    }

    pub fn stream_proof(&self) -> &StreamStartProof {
        &self.start_proof
    }

    fn take_ticket(&mut self) -> Option<DeviceCleanupTicket> {
        Some(DeviceCleanupTicket {
            lease: self.lease.take()?,
            activity: self.activity.take()?,
            reservation: self.reservation.take()?,
            session: self.session.take()?,
            expected_generation: Some(self.start_proof.generation),
        })
    }
}

impl Drop for UiWithStreamContext {
    fn drop(&mut self) {
        if let Some(ticket) = self.take_ticket() {
            self.cleanup.enqueue(ticket);
        }
    }
}

struct DeviceCleanupTicket {
    lease: DeviceWorkLease,
    activity: ContextActivityPermit,
    reservation: ForegroundStreamReservation,
    session: Arc<dyn UiSession>,
    expected_generation: Option<u64>,
}

impl DeviceCleanupTicket {
    fn into_context(
        self,
        plane_id: Uuid,
        cleanup: CleanupSink,
        start_proof: StreamStartProof,
        replacement_session: Option<Arc<dyn UiSession>>,
    ) -> UiWithStreamContext {
        let Self {
            lease,
            activity,
            reservation,
            session,
            ..
        } = self;
        UiWithStreamContext {
            plane_id,
            lease: Some(lease),
            activity: Some(activity),
            reservation: Some(reservation),
            session: Some(replacement_session.unwrap_or(session)),
            start_proof,
            cleanup,
        }
    }
}

struct GuardedClipboardResponse {
    result: Result<GuardedClipboardOutput, DeviceControlError>,
    context: Option<UiWithStreamContext>,
}

struct RecoverStreamResponse {
    result: Result<Arc<dyn UiSession>, DeviceControlError>,
    context: Option<UiWithStreamContext>,
}

#[derive(Clone)]
struct BackgroundCleanupTicket {
    udid: String,
    token: Uuid,
}

impl BackgroundCleanupTicket {
    fn from_lease(lease: &BackgroundStreamLease) -> Self {
        Self {
            udid: lease.udid().to_string(),
            token: lease.token(),
        }
    }
}

#[derive(Default)]
struct BackgroundStore {
    entries: Mutex<HashMap<Uuid, BackgroundCleanupTicket>>,
}

impl BackgroundStore {
    fn insert(&self, udid: &str, token: Uuid) {
        self.entries.lock().insert(
            token,
            BackgroundCleanupTicket {
                udid: udid.to_string(),
                token,
            },
        );
    }

    fn contains(&self, udid: &str, token: Uuid) -> bool {
        self.entries
            .lock()
            .get(&token)
            .is_some_and(|ticket| ticket.udid == udid)
    }

    fn remove(&self, token: Uuid) -> Option<BackgroundCleanupTicket> {
        self.entries.lock().remove(&token)
    }

    fn remove_udid(&self, udid: &str) {
        self.entries.lock().retain(|_, ticket| ticket.udid != udid);
    }

    fn snapshot(&self) -> Vec<BackgroundCleanupTicket> {
        self.entries.lock().values().cloned().collect()
    }
}

#[derive(Default)]
struct DeviceOperationLocks {
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl DeviceOperationLocks {
    fn lock_for(&self, udid: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.locks
            .lock()
            .entry(udid.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    async fn lock_one(&self, udid: &str) -> tokio::sync::OwnedMutexGuard<()> {
        self.lock_for(udid).lock_owned().await
    }

    async fn lock_many(&self, udids: &[String]) -> Vec<tokio::sync::OwnedMutexGuard<()>> {
        let mut unique = udids.to_vec();
        unique.sort();
        unique.dedup();
        let mut guards = Vec::with_capacity(unique.len());
        for udid in unique {
            guards.push(self.lock_for(&udid).lock_owned().await);
        }
        guards
    }
}

struct ReservedUiCapacity {
    context: Option<DeviceExclusiveContext>,
    reservation: Option<ForegroundStreamReservation>,
    streams: Arc<StreamBudgetManager>,
    quarantined: Arc<QuarantineStore>,
}

struct StartedBackground {
    url: Option<String>,
    ticket: Option<BackgroundCleanupTicket>,
    cleanup: CleanupSink,
}

impl StartedBackground {
    fn into_url(mut self) -> String {
        let _ = self.ticket.take();
        self.url
            .take()
            .expect("live background start response has a URL")
    }
}

impl Drop for StartedBackground {
    fn drop(&mut self) {
        if let Some(ticket) = self.ticket.take() {
            self.cleanup.enqueue_background(ticket);
        }
    }
}

impl ReservedUiCapacity {
    fn into_parts(mut self) -> (DeviceExclusiveContext, ForegroundStreamReservation) {
        let context = self
            .context
            .take()
            .expect("live reserve response has a device context");
        let reservation = self
            .reservation
            .take()
            .expect("live reserve response has a stream reservation");
        (context, reservation)
    }
}

impl Drop for ReservedUiCapacity {
    fn drop(&mut self) {
        let Some(reservation) = self.reservation.take() else {
            return;
        };
        if self.streams.release_reserved(reservation.token()).is_err() {
            if let Some(context) = self.context.take() {
                self.quarantined.push_context(context);
            }
        }
    }
}

enum WorkerCommand {
    Park {
        context: DeviceExclusiveContext,
        response: oneshot::Sender<Result<DeviceExclusiveContext, DeviceControlError>>,
    },
    Reserve {
        context: DeviceExclusiveContext,
        response: oneshot::Sender<Result<ReservedUiCapacity, CapacityContextUpgradeFailure>>,
    },
    StartBackground {
        ticket: BackgroundCleanupTicket,
        response: oneshot::Sender<Result<StartedBackground, DeviceControlError>>,
    },
    StopBackground {
        ticket: BackgroundCleanupTicket,
        quarantine_on_error: bool,
        response: Option<oneshot::Sender<Result<StreamStopProof, DeviceControlError>>>,
    },
    Close {
        ticket: DeviceCleanupTicket,
        response: oneshot::Sender<Result<DeviceReleaseProof, DeviceControlError>>,
    },
    GuardedClipboard {
        plane_id: Uuid,
        ticket: DeviceCleanupTicket,
        cleanup: CleanupSink,
        original_stream: StreamStartProof,
        agent_bundle_id: String,
        target_bundle_id: String,
        final_session_kind: InteractionSessionKind,
        operation: GuardedClipboardOperation,
        response: oneshot::Sender<GuardedClipboardResponse>,
    },
    RecoverStream {
        plane_id: Uuid,
        ticket: DeviceCleanupTicket,
        cleanup: CleanupSink,
        original_stream: StreamStartProof,
        bundle_id: String,
        kind: InteractionSessionKind,
        recycle_transport: bool,
        response: oneshot::Sender<RecoverStreamResponse>,
    },
    Cleanup(DeviceCleanupTicket),
    DrainBackground {
        ack: oneshot::Sender<()>,
    },
    Shutdown {
        ack: oneshot::Sender<()>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlPlanePhase {
    Running,
    ShuttingDown,
    Stopped,
}

struct LifecycleState {
    phase: ControlPlanePhase,
    outstanding: usize,
}

struct ControlPlaneLifecycle {
    state: Mutex<LifecycleState>,
    changed: Notify,
}

impl Default for ControlPlaneLifecycle {
    fn default() -> Self {
        Self {
            state: Mutex::new(LifecycleState {
                phase: ControlPlanePhase::Running,
                outstanding: 0,
            }),
            changed: Notify::new(),
        }
    }
}

impl ControlPlaneLifecycle {
    pub(super) fn ensure_running(&self) -> Result<(), DeviceControlError> {
        match self.state.lock().phase {
            ControlPlanePhase::Running => Ok(()),
            ControlPlanePhase::ShuttingDown => Err(DeviceControlError::ControlPlaneShuttingDown),
            ControlPlanePhase::Stopped => Err(DeviceControlError::ControlPlaneStopped),
        }
    }

    fn register(self: &Arc<Self>) -> Result<ContextActivityPermit, DeviceControlError> {
        let mut state = self.state.lock();
        match state.phase {
            ControlPlanePhase::Running => {
                state.outstanding += 1;
                Ok(ContextActivityPermit {
                    lifecycle: self.clone(),
                    active: true,
                })
            }
            ControlPlanePhase::ShuttingDown => Err(DeviceControlError::ControlPlaneShuttingDown),
            ControlPlanePhase::Stopped => Err(DeviceControlError::ControlPlaneStopped),
        }
    }

    fn begin_shutdown(&self) {
        let mut state = self.state.lock();
        if state.phase == ControlPlanePhase::Running {
            state.phase = ControlPlanePhase::ShuttingDown;
            self.changed.notify_waiters();
        }
    }

    fn mark_stopped(&self) {
        self.state.lock().phase = ControlPlanePhase::Stopped;
        self.changed.notify_waiters();
    }

    fn phase(&self) -> ControlPlanePhase {
        self.state.lock().phase
    }

    fn outstanding(&self) -> usize {
        self.state.lock().outstanding
    }
}

struct ContextActivityPermit {
    lifecycle: Arc<ControlPlaneLifecycle>,
    active: bool,
}

impl Drop for ContextActivityPermit {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        self.active = false;
        let mut state = self.lifecycle.state.lock();
        state.outstanding = state
            .outstanding
            .checked_sub(1)
            .expect("context activity count cannot underflow");
        drop(state);
        self.lifecycle.changed.notify_waiters();
    }
}

#[derive(Default)]
struct QuarantineStore {
    cleanup: Mutex<Vec<DeviceCleanupTicket>>,
    contexts: Mutex<Vec<DeviceExclusiveContext>>,
    backgrounds: Mutex<Vec<BackgroundCleanupTicket>>,
    changed: Notify,
}

impl QuarantineStore {
    fn push_cleanup(&self, ticket: DeviceCleanupTicket) {
        self.cleanup.lock().push(ticket);
        self.changed.notify_waiters();
    }

    fn push_context(&self, context: DeviceExclusiveContext) {
        self.contexts.lock().push(context);
        self.changed.notify_waiters();
    }

    fn push_background(&self, ticket: BackgroundCleanupTicket) {
        self.backgrounds.lock().push(ticket);
        self.changed.notify_waiters();
    }

    fn context_activity_count(&self) -> usize {
        self.cleanup.lock().len() + self.contexts.lock().len()
    }

    fn count(&self) -> usize {
        self.context_activity_count() + self.backgrounds.lock().len()
    }
}

#[derive(Clone)]
struct CleanupSink {
    tx: mpsc::UnboundedSender<WorkerCommand>,
    quarantined: Arc<QuarantineStore>,
}

impl CleanupSink {
    fn enqueue(&self, ticket: DeviceCleanupTicket) {
        if let Err(error) = self.tx.send(WorkerCommand::Cleanup(ticket)) {
            if let WorkerCommand::Cleanup(ticket) = error.0 {
                self.quarantined.push_cleanup(ticket);
            }
        }
    }

    fn enqueue_background(&self, ticket: BackgroundCleanupTicket) {
        if let Err(error) = self.tx.send(WorkerCommand::StopBackground {
            ticket,
            quarantine_on_error: true,
            response: None,
        }) {
            if let WorkerCommand::StopBackground { ticket, .. } = error.0 {
                self.quarantined.push_background(ticket);
            }
        }
    }
}

struct GuardedClipboardCleanup<'a> {
    context: &'a mut UiWithStreamContext,
    progress: GuardedClipboardProgress,
    committed: bool,
}

impl<'a> GuardedClipboardCleanup<'a> {
    fn new(context: &'a mut UiWithStreamContext, progress: GuardedClipboardProgress) -> Self {
        Self {
            context,
            progress,
            committed: false,
        }
    }
}

impl Drop for GuardedClipboardCleanup<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let (stop, stream) = self.progress.snapshot();
        let Some(stop) = stop else {
            return;
        };
        self.context.start_proof = stream.unwrap_or(StreamStartProof {
            generation: stop.new_generation,
            first_frame_observed: false,
            stream_url: String::new(),
        });
        let cleanup = self.context.cleanup.clone();
        if let Some(ticket) = self.context.take_ticket() {
            cleanup.enqueue(ticket);
        }
    }
}

struct PendingStreamStart {
    ticket: Option<DeviceCleanupTicket>,
    cleanup: CleanupSink,
}

impl Drop for PendingStreamStart {
    fn drop(&mut self) {
        if let Some(ticket) = self.ticket.take() {
            self.cleanup.enqueue(ticket);
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_cleanup_worker(
    mut rx: mpsc::UnboundedReceiver<WorkerCommand>,
    driver: Arc<dyn DeviceDriver>,
    work: Arc<DeviceWorkCoordinator>,
    streams: Arc<StreamBudgetManager>,
    quarantined: Arc<QuarantineStore>,
    backgrounds: Arc<BackgroundStore>,
    operation_locks: Arc<DeviceOperationLocks>,
    capacity_gate: Arc<tokio::sync::Mutex<()>>,
    cleanup_tx: mpsc::UnboundedSender<WorkerCommand>,
) {
    let mut tasks = tokio::task::JoinSet::new();
    while let Some(command) = rx.recv().await {
        while tasks.try_join_next().is_some() {}
        match command {
            WorkerCommand::Park { context, response } => {
                let driver = driver.clone();
                let streams = streams.clone();
                let quarantined = quarantined.clone();
                let backgrounds = backgrounds.clone();
                let operation_locks = operation_locks.clone();
                tasks.spawn(async move {
                    let _operation = operation_locks.lock_one(context.udid()).await;
                    process_park(
                        &driver,
                        &streams,
                        &quarantined,
                        &backgrounds,
                        context,
                        response,
                    )
                    .await;
                });
            }
            WorkerCommand::Reserve { context, response } => {
                let driver = driver.clone();
                let streams = streams.clone();
                let quarantined = quarantined.clone();
                let backgrounds = backgrounds.clone();
                let operation_locks = operation_locks.clone();
                let capacity_gate = capacity_gate.clone();
                tasks.spawn(async move {
                    process_reserve(
                        &driver,
                        &streams,
                        &quarantined,
                        &backgrounds,
                        &operation_locks,
                        &capacity_gate,
                        context,
                        response,
                    )
                    .await;
                });
            }
            WorkerCommand::StartBackground { ticket, response } => {
                let dependencies = BackgroundWorkerDependencies {
                    driver: driver.clone(),
                    work: work.clone(),
                    streams: streams.clone(),
                    quarantined: quarantined.clone(),
                    backgrounds: backgrounds.clone(),
                    operation_locks: operation_locks.clone(),
                    cleanup_tx: cleanup_tx.clone(),
                };
                tasks.spawn(async move {
                    process_start_background(dependencies, ticket, response).await;
                });
            }
            WorkerCommand::StopBackground {
                ticket,
                quarantine_on_error,
                response,
            } => {
                let driver = driver.clone();
                let streams = streams.clone();
                let quarantined = quarantined.clone();
                let backgrounds = backgrounds.clone();
                let operation_locks = operation_locks.clone();
                tasks.spawn(async move {
                    let _operation = operation_locks.lock_one(&ticket.udid).await;
                    process_stop_background(
                        &driver,
                        &streams,
                        &quarantined,
                        &backgrounds,
                        ticket,
                        quarantine_on_error,
                        response,
                    )
                    .await;
                });
            }
            WorkerCommand::Close {
                mut ticket,
                response,
            } => {
                let driver = driver.clone();
                let streams = streams.clone();
                let quarantined = quarantined.clone();
                let operation_locks = operation_locks.clone();
                tasks.spawn(async move {
                    let _operation = operation_locks.lock_one(ticket.lease.udid()).await;
                    match clean_ticket(&driver, &streams, &mut ticket).await {
                        Ok(proof) => {
                            drop(ticket);
                            let _ = response.send(Ok(proof));
                        }
                        Err(error) => {
                            quarantined.push_cleanup(ticket);
                            let _ = response.send(Err(error));
                        }
                    }
                });
            }
            WorkerCommand::GuardedClipboard {
                plane_id,
                ticket,
                cleanup,
                original_stream,
                agent_bundle_id,
                target_bundle_id,
                final_session_kind,
                operation,
                response,
            } => {
                let driver = driver.clone();
                let streams = streams.clone();
                let quarantined = quarantined.clone();
                let operation_locks = operation_locks.clone();
                tasks.spawn(async move {
                    let _operation = operation_locks.lock_one(ticket.lease.udid()).await;
                    process_guarded_clipboard(
                        &driver,
                        &streams,
                        &quarantined,
                        plane_id,
                        ticket,
                        cleanup,
                        original_stream,
                        agent_bundle_id,
                        target_bundle_id,
                        final_session_kind,
                        operation,
                        response,
                    )
                    .await;
                });
            }
            WorkerCommand::RecoverStream {
                plane_id,
                ticket,
                cleanup,
                original_stream,
                bundle_id,
                kind,
                recycle_transport,
                response,
            } => {
                let driver = driver.clone();
                let operation_locks = operation_locks.clone();
                tasks.spawn(async move {
                    let _operation = operation_locks.lock_one(ticket.lease.udid()).await;
                    process_recover_stream(
                        &driver,
                        plane_id,
                        ticket,
                        cleanup,
                        original_stream,
                        bundle_id,
                        kind,
                        recycle_transport,
                        response,
                    )
                    .await;
                });
            }
            WorkerCommand::Cleanup(ticket) => {
                let driver = driver.clone();
                let streams = streams.clone();
                let quarantined = quarantined.clone();
                let operation_locks = operation_locks.clone();
                tasks.spawn(async move {
                    let _operation = operation_locks.lock_one(ticket.lease.udid()).await;
                    clean_or_quarantine(&driver, &streams, &quarantined, ticket).await;
                });
            }
            WorkerCommand::DrainBackground { ack } => {
                join_worker_tasks(&mut tasks).await;
                drain_backgrounds(
                    &driver,
                    &streams,
                    &quarantined,
                    &backgrounds,
                    &operation_locks,
                )
                .await;
                let _ = ack.send(());
            }
            WorkerCommand::Shutdown { ack } => {
                rx.close();
                join_worker_tasks(&mut tasks).await;
                let _ = ack.send(());
                break;
            }
        }
    }
}

async fn join_worker_tasks(tasks: &mut tokio::task::JoinSet<()>) {
    while tasks.join_next().await.is_some() {}
}

struct BackgroundWorkerDependencies {
    driver: Arc<dyn DeviceDriver>,
    work: Arc<DeviceWorkCoordinator>,
    streams: Arc<StreamBudgetManager>,
    quarantined: Arc<QuarantineStore>,
    backgrounds: Arc<BackgroundStore>,
    operation_locks: Arc<DeviceOperationLocks>,
    cleanup_tx: mpsc::UnboundedSender<WorkerCommand>,
}

struct ContextTransitionFailure {
    error: DeviceControlError,
    context: DeviceExclusiveContext,
    quarantine: bool,
}

async fn process_park(
    driver: &Arc<dyn DeviceDriver>,
    streams: &Arc<StreamBudgetManager>,
    quarantined: &Arc<QuarantineStore>,
    backgrounds: &Arc<BackgroundStore>,
    context: DeviceExclusiveContext,
    response: oneshot::Sender<Result<DeviceExclusiveContext, DeviceControlError>>,
) {
    match park_context(driver, streams, backgrounds, context).await {
        Ok(context) => {
            let _ = response.send(Ok(context));
        }
        Err(failure) => {
            if failure.quarantine {
                quarantined.push_context(failure.context);
            }
            let _ = response.send(Err(failure.error));
        }
    }
}

async fn park_context(
    driver: &Arc<dyn DeviceDriver>,
    streams: &Arc<StreamBudgetManager>,
    backgrounds: &Arc<BackgroundStore>,
    context: DeviceExclusiveContext,
) -> Result<DeviceExclusiveContext, ContextTransitionFailure> {
    let udid = context.udid().to_string();
    let Some(token) = streams.reservation_token(&udid) else {
        return Ok(context);
    };
    match streams.release_reserved(token) {
        Ok(()) => {
            backgrounds.remove(token);
            return Ok(context);
        }
        Err(StreamBudgetError::InvalidTransition { .. }) => {}
        Err(error) => {
            return Err(ContextTransitionFailure {
                error: error.into(),
                context,
                quarantine: true,
            })
        }
    }
    let stop = match streams.begin_stop(token) {
        Ok(stop) => stop,
        Err(error) => {
            return Err(ContextTransitionFailure {
                error: error.into(),
                context,
                quarantine: true,
            })
        }
    };
    let proof = match driver.stop_owned_stream(&udid).await {
        Ok(proof) => proof,
        Err(error) => {
            return Err(ContextTransitionFailure {
                error: driver_error(&udid, "parkOwnedStream", error),
                context,
                quarantine: true,
            })
        }
    };
    if let Err(error) = streams.complete_stop(stop, proof) {
        return Err(ContextTransitionFailure {
            error: error.into(),
            context,
            quarantine: true,
        });
    }
    backgrounds.remove(token);
    Ok(context)
}

#[allow(clippy::too_many_arguments)]
async fn process_reserve(
    driver: &Arc<dyn DeviceDriver>,
    streams: &Arc<StreamBudgetManager>,
    quarantined: &Arc<QuarantineStore>,
    backgrounds: &Arc<BackgroundStore>,
    operation_locks: &Arc<DeviceOperationLocks>,
    capacity_gate: &Arc<tokio::sync::Mutex<()>>,
    context: DeviceExclusiveContext,
    response: oneshot::Sender<Result<ReservedUiCapacity, CapacityContextUpgradeFailure>>,
) {
    match reserve_context(
        driver,
        streams,
        quarantined,
        backgrounds,
        operation_locks,
        capacity_gate,
        context,
    )
    .await
    {
        Ok(reserved) => {
            let _ = response.send(Ok(reserved));
        }
        Err(failure) => {
            if failure.quarantine {
                quarantined.push_context(failure.context);
                let _ = response.send(Err(CapacityContextUpgradeFailure {
                    context: None,
                    error: failure.error,
                }));
            } else {
                let _ = response.send(Err(CapacityContextUpgradeFailure {
                    context: Some(failure.context),
                    error: failure.error,
                }));
            }
        }
    }
}

async fn reserve_context(
    driver: &Arc<dyn DeviceDriver>,
    streams: &Arc<StreamBudgetManager>,
    quarantined: &Arc<QuarantineStore>,
    backgrounds: &Arc<BackgroundStore>,
    operation_locks: &Arc<DeviceOperationLocks>,
    capacity_gate: &Arc<tokio::sync::Mutex<()>>,
    mut context: DeviceExclusiveContext,
) -> Result<ReservedUiCapacity, ContextTransitionFailure> {
    let udid = context.udid().to_string();
    let owner = context.owner();
    let capacity_guard = capacity_gate.lock().await;
    let (operation_guards, expected_victim) = loop {
        let victim = match streams.preview_foreground_victim(&udid) {
            Ok(victim) => victim,
            Err(error) => {
                return Err(ContextTransitionFailure {
                    error: error.into(),
                    context,
                    quarantine: false,
                })
            }
        };
        let mut locked_udids = vec![udid.clone()];
        if let Some(victim_udid) = victim.as_ref() {
            locked_udids.push(victim_udid.clone());
        }
        let guards = operation_locks.lock_many(&locked_udids).await;
        match streams.preview_foreground_victim(&udid) {
            Ok(current) if current == victim => break (guards, victim),
            Ok(_) => drop(guards),
            Err(error) => {
                return Err(ContextTransitionFailure {
                    error: error.into(),
                    context,
                    quarantine: false,
                })
            }
        }
    };
    let transfer = match streams.begin_foreground_transfer(&udid, owner) {
        Ok(transfer) => transfer,
        Err(error) => {
            return Err(ContextTransitionFailure {
                error: error.into(),
                context,
                quarantine: false,
            })
        }
    };
    // The victim is now Revoking and the target ForegroundReserved — both occupy
    // capacity under the budget mutex — so a concurrent reserve for another
    // device fails fast (CapacityExhausted) or selects a different victim.
    // Release the global capacity gate before the (possibly slow) victim stop so
    // a stall on one device cannot block capacity reservation fleet-wide. The
    // per-UDID operation guards stay held across the stop.
    drop(capacity_guard);
    let proof = if transfer.requires_stop_proof() {
        let revoked_udid = transfer
            .revoked_udid()
            .expect("a transfer requiring proof always identifies its producer")
            .to_string();
        match driver.stop_owned_stream(&revoked_udid).await {
            Ok(proof) => proof,
            Err(error) => {
                // **Give the slot back before returning.** `ForegroundTransfer` has no
                // `Drop`, and both the victim's `Revoking` and a freshly reserved
                // `ForegroundReserved` occupy capacity -- so dropping it here left the slot
                // counted for the life of the process, and `release_reserved` could not
                // clean it up because `Revoking` is not releasable. Repeating this on
                // distinct producers walked the fleet's capacity down to nothing while the
                // quarantine count only said that *something* had failed.
                streams.abandon_transfer(transfer, std::time::Instant::now());
                return Err(ContextTransitionFailure {
                    error: driver_error(&revoked_udid, "stopOwnedStream", error),
                    context,
                    quarantine: true,
                });
            }
        }
    } else {
        StreamStopProof::not_required()
    };
    let reservation = match streams.complete_transfer(transfer, proof) {
        Ok(reservation) => reservation,
        Err(error) => {
            // Nothing to release here: `complete_transfer` walks its own refusals back now
            // (victim into backoff, fresh slot removed) for the same reason the stop path
            // above calls `abandon_transfer`. The release-by-udid that used to sit here was
            // dead on both of its branches — the wedged record lived under the *victim's*
            // udid, and a fresh reservation still carried `pending_transfer`, which
            // `release_reserved` refuses — so the slot leaked for the life of the process.
            return Err(ContextTransitionFailure {
                error: error.into(),
                context,
                quarantine: true,
            });
        }
    };
    context.ui_capacity_token = Some(reservation.token());
    if let Some(victim) = expected_victim {
        backgrounds.remove_udid(&victim);
    }
    drop(operation_guards);
    Ok(ReservedUiCapacity {
        context: Some(context),
        reservation: Some(reservation),
        streams: streams.clone(),
        quarantined: quarantined.clone(),
    })
}

async fn process_start_background(
    dependencies: BackgroundWorkerDependencies,
    ticket: BackgroundCleanupTicket,
    response: oneshot::Sender<Result<StartedBackground, DeviceControlError>>,
) {
    let _operation = dependencies.operation_locks.lock_one(&ticket.udid).await;
    let result = start_background(&dependencies, &ticket)
        .await
        .map(|url| StartedBackground {
            url: Some(url),
            ticket: Some(ticket.clone()),
            cleanup: CleanupSink {
                tx: dependencies.cleanup_tx.clone(),
                quarantined: dependencies.quarantined.clone(),
            },
        });
    let _ = response.send(result);
}

async fn start_background(
    dependencies: &BackgroundWorkerDependencies,
    ticket: &BackgroundCleanupTicket,
) -> Result<String, DeviceControlError> {
    if !dependencies
        .backgrounds
        .contains(&ticket.udid, ticket.token)
    {
        return Err(DeviceControlError::InvalidContext {
            reason: "background stream reservation is no longer current",
        });
    }
    if let Some(current_owner) = dependencies.work.current_owner(&ticket.udid) {
        clean_background(
            &dependencies.driver,
            &dependencies.streams,
            &dependencies.backgrounds,
            ticket,
        )
        .await?;
        return Err(DeviceControlError::BackgroundStreamBlocked {
            udid: ticket.udid.clone(),
            current_owner,
        });
    }

    let url = match dependencies.driver.ensure_stream(&ticket.udid).await {
        Ok(url) => url,
        Err(error) => {
            if dependencies
                .streams
                .mark_background_failed(ticket.token)
                .is_ok()
            {
                dependencies.backgrounds.remove(ticket.token);
            }
            return Err(driver_error(&ticket.udid, "startBackgroundStream", error));
        }
    };
    if let Err(mark_error) = dependencies.streams.mark_running(ticket.token) {
        reconcile_failed_background_start(
            &dependencies.driver,
            &dependencies.streams,
            &dependencies.backgrounds,
            ticket,
        )
        .await?;
        return Err(mark_error.into());
    }

    if let Some(current_owner) = dependencies.work.current_owner(&ticket.udid) {
        clean_background(
            &dependencies.driver,
            &dependencies.streams,
            &dependencies.backgrounds,
            ticket,
        )
        .await?;
        return Err(DeviceControlError::BackgroundStreamBlocked {
            udid: ticket.udid.clone(),
            current_owner,
        });
    }
    Ok(url)
}

async fn reconcile_failed_background_start(
    driver: &Arc<dyn DeviceDriver>,
    streams: &Arc<StreamBudgetManager>,
    backgrounds: &Arc<BackgroundStore>,
    ticket: &BackgroundCleanupTicket,
) -> Result<(), DeviceControlError> {
    let proof = driver
        .stop_owned_stream(&ticket.udid)
        .await
        .map_err(|error| driver_error(&ticket.udid, "reconcileBackgroundStart", error))?;
    if !proof.child_stopped || proof.new_generation <= proof.old_generation {
        return Err(StreamBudgetError::StopNotConfirmed {
            udid: ticket.udid.clone(),
        }
        .into());
    }
    match streams.begin_stop(ticket.token) {
        Ok(request) => streams.complete_stop(request, proof)?,
        Err(StreamBudgetError::InvalidTransition { .. }) => {
            streams.release_reserved(ticket.token)?;
        }
        Err(StreamBudgetError::StaleToken { .. }) => {}
        Err(error) => return Err(error.into()),
    }
    backgrounds.remove(ticket.token);
    Ok(())
}

async fn process_stop_background(
    driver: &Arc<dyn DeviceDriver>,
    streams: &Arc<StreamBudgetManager>,
    quarantined: &Arc<QuarantineStore>,
    backgrounds: &Arc<BackgroundStore>,
    ticket: BackgroundCleanupTicket,
    quarantine_on_error: bool,
    response: Option<oneshot::Sender<Result<StreamStopProof, DeviceControlError>>>,
) {
    let result = clean_background(driver, streams, backgrounds, &ticket).await;
    if result.is_err() && quarantine_on_error {
        if let Some(ticket) = backgrounds.remove(ticket.token) {
            quarantined.push_background(ticket);
        }
    }
    if let Some(response) = response {
        let _ = response.send(result);
    }
}

async fn clean_background(
    driver: &Arc<dyn DeviceDriver>,
    streams: &Arc<StreamBudgetManager>,
    backgrounds: &Arc<BackgroundStore>,
    ticket: &BackgroundCleanupTicket,
) -> Result<StreamStopProof, DeviceControlError> {
    if !backgrounds.contains(&ticket.udid, ticket.token) {
        return Err(DeviceControlError::InvalidContext {
            reason: "background stream reservation is no longer current",
        });
    }
    match streams.release_reserved(ticket.token) {
        Ok(()) => {
            backgrounds.remove(ticket.token);
            return Ok(StreamStopProof::not_required());
        }
        Err(StreamBudgetError::InvalidTransition { .. }) => {}
        Err(error) => return Err(error.into()),
    }
    let request = streams.begin_stop(ticket.token)?;
    let proof = driver
        .park_owned_stream(&ticket.udid)
        .await
        .map_err(|error| driver_error(&ticket.udid, "stopBackgroundStream", error))?;
    streams.complete_stop(request, proof)?;
    backgrounds.remove(ticket.token);
    Ok(proof)
}

async fn drain_backgrounds(
    driver: &Arc<dyn DeviceDriver>,
    streams: &Arc<StreamBudgetManager>,
    quarantined: &Arc<QuarantineStore>,
    backgrounds: &Arc<BackgroundStore>,
    operation_locks: &Arc<DeviceOperationLocks>,
) {
    for ticket in backgrounds.snapshot() {
        let _operation = operation_locks.lock_one(&ticket.udid).await;
        if clean_background(driver, streams, backgrounds, &ticket)
            .await
            .is_err()
        {
            if let Some(ticket) = backgrounds.remove(ticket.token) {
                quarantined.push_background(ticket);
            }
        }
    }
}

struct ValidatedClipboardTransition {
    output: GuardedClipboardOutput,
    final_session: Box<dyn UiSession>,
    stream: StreamStartProof,
}

fn validate_agent_foreground_clipboard_transition(
    udid: &str,
    expected_generation: u64,
    agent_bundle_id: &str,
    target_bundle_id: &str,
    transition: crate::GuardedClipboardTransition,
) -> Result<ValidatedClipboardTransition, DeviceControlError> {
    let crate::GuardedClipboardTransition {
        output,
        stop,
        agent,
        target,
        final_session,
        stream,
    } = transition;
    if target.bundle_id != target_bundle_id || target.pid == 0 {
        return Err(DeviceControlError::InvalidContext {
            reason: "guarded clipboard target identity proof is invalid",
        });
    }
    let stop = stop.ok_or(DeviceControlError::InvalidContext {
        reason: "Agent-foreground clipboard is missing its stream stop proof",
    })?;
    validate_stop_generation(udid, expected_generation, stop)?;
    let agent = agent.ok_or(DeviceControlError::InvalidContext {
        reason: "Agent-foreground clipboard is missing Agent identity proof",
    })?;
    if agent.bundle_id != agent_bundle_id || agent.pid == 0 {
        return Err(DeviceControlError::InvalidContext {
            reason: "Agent-foreground clipboard Agent identity proof is invalid",
        });
    }
    let final_session = final_session.ok_or(DeviceControlError::InvalidContext {
        reason: "Agent-foreground clipboard is missing its final session",
    })?;
    let stream = stream.ok_or(DeviceControlError::InvalidContext {
        reason: "Agent-foreground clipboard is missing its replacement stream",
    })?;
    if stream.generation != stop.new_generation {
        return Err(DeviceControlError::StopProofMismatch {
            udid: udid.to_string(),
            expected: stop.new_generation,
            actual: stream.generation,
        });
    }
    if !stream.first_frame_observed || stream.stream_url.trim().is_empty() {
        return Err(DeviceControlError::FirstFrameMissing {
            udid: udid.to_string(),
        });
    }
    Ok(ValidatedClipboardTransition {
        output,
        final_session,
        stream,
    })
}

#[allow(clippy::too_many_arguments)]
async fn process_guarded_clipboard(
    driver: &Arc<dyn DeviceDriver>,
    streams: &Arc<StreamBudgetManager>,
    quarantined: &Arc<QuarantineStore>,
    plane_id: Uuid,
    mut ticket: DeviceCleanupTicket,
    cleanup: CleanupSink,
    original_stream: StreamStartProof,
    agent_bundle_id: String,
    target_bundle_id: String,
    final_session_kind: InteractionSessionKind,
    operation: GuardedClipboardOperation,
    response: oneshot::Sender<GuardedClipboardResponse>,
) {
    let udid = ticket.lease.udid().to_string();
    let progress = GuardedClipboardProgress::default();
    let transition = driver
        .guarded_clipboard_transition(
            &udid,
            &agent_bundle_id,
            &target_bundle_id,
            final_session_kind,
            ClipboardAccessMode::AgentForegroundRequired,
            operation,
            progress.clone(),
        )
        .await
        .map_err(|error| driver_error(&udid, "guardedClipboardTransition", error))
        .and_then(|transition| {
            validate_agent_foreground_clipboard_transition(
                &udid,
                original_stream.generation,
                &agent_bundle_id,
                &target_bundle_id,
                transition,
            )
        });

    let completed = match transition {
        Ok(transition) => GuardedClipboardResponse {
            result: Ok(transition.output),
            context: Some(ticket.into_context(
                plane_id,
                cleanup,
                transition.stream,
                Some(Arc::from(transition.final_session)),
            )),
        },
        Err(error) => {
            let (stop, stream) = progress.snapshot();
            let destructive_generation = stream
                .as_ref()
                .map(|proof| proof.generation)
                .or_else(|| stop.map(|proof| proof.new_generation));
            let context = if let Some(generation) = destructive_generation {
                ticket.expected_generation = Some(generation);
                clean_or_quarantine(driver, streams, quarantined, ticket).await;
                None
            } else {
                Some(ticket.into_context(plane_id, cleanup, original_stream, None))
            };
            GuardedClipboardResponse {
                result: Err(error),
                context,
            }
        }
    };
    let _ = response.send(completed);
}

#[allow(clippy::too_many_arguments)]
async fn process_recover_stream(
    driver: &Arc<dyn DeviceDriver>,
    plane_id: Uuid,
    ticket: DeviceCleanupTicket,
    cleanup: CleanupSink,
    original_stream: StreamStartProof,
    bundle_id: String,
    kind: InteractionSessionKind,
    recycle_transport: bool,
    response: oneshot::Sender<RecoverStreamResponse>,
) {
    let udid = ticket.lease.udid().to_string();
    let expected = ticket.expected_generation;
    // Newest confirmed post-stop generation, so a failure rebuilds the context
    // with a generation a later close/drop can clean up against exactly.
    let mut destructive_generation: Option<u64> = None;

    let outcome: Result<(Arc<dyn UiSession>, StreamStartProof), DeviceControlError> = async {
        let expected = expected
            .ok_or_else(|| DeviceControlError::StopGenerationUnknown { udid: udid.clone() })?;
        let mut stopped = driver
            .stop_owned_stream(&udid)
            .await
            .map_err(|error| driver_error(&udid, "stopOwnedStreamForRecovery", error))?;
        validate_stop_generation(&udid, expected, stopped)?;
        destructive_generation = Some(stopped.new_generation);

        if recycle_transport {
            driver.recycle_ui_transport(&udid).await;
            let recycled_stop = driver
                .stop_owned_stream(&udid)
                .await
                .map_err(|error| driver_error(&udid, "resetLifecycleAfterRecycle", error))?;
            validate_stop_generation(&udid, stopped.new_generation, recycled_stop)?;
            stopped = recycled_stop;
            destructive_generation = Some(stopped.new_generation);
        }

        let session = driver
            .start_interaction_session(&udid, &bundle_id, kind)
            .await
            .map_err(|error| driver_error(&udid, "recoverInteractionSession", error))?;
        let session: Arc<dyn UiSession> = Arc::from(session);
        let stream = driver
            .start_stream_after_session(&udid)
            .await
            .map_err(|error| driver_error(&udid, "recoverStreamAfterSession", error))?;
        if !stream.first_frame_observed {
            return Err(DeviceControlError::FirstFrameMissing { udid: udid.clone() });
        }
        if stream.generation != stopped.new_generation {
            return Err(DeviceControlError::StopProofMismatch {
                udid: udid.clone(),
                expected: stopped.new_generation,
                actual: stream.generation,
            });
        }
        Ok((session, stream))
    }
    .await;

    let completed = match outcome {
        Ok((session, stream)) => RecoverStreamResponse {
            result: Ok(session.clone()),
            context: Some(ticket.into_context(plane_id, cleanup, stream, Some(session))),
        },
        Err(error) => {
            // Keep the caller's reservation/lease and rebuild the context,
            // exactly as the old in-caller path left it on failure — the caller
            // still owns cleanup via close_ui_context. The rebuilt proof carries
            // the post-stop generation so that if the caller was cancelled and
            // this context is dropped instead of delivered, its Drop enqueues a
            // cleanup against that same generation and nothing is quarantined.
            let mut proof = original_stream;
            if let Some(generation) = destructive_generation {
                proof.generation = generation;
                proof.first_frame_observed = false;
            }
            RecoverStreamResponse {
                result: Err(error),
                context: Some(ticket.into_context(plane_id, cleanup, proof, None)),
            }
        }
    };
    let _ = response.send(completed);
}

async fn clean_or_quarantine(
    driver: &Arc<dyn DeviceDriver>,
    streams: &Arc<StreamBudgetManager>,
    quarantined: &Arc<QuarantineStore>,
    mut ticket: DeviceCleanupTicket,
) {
    if clean_ticket(driver, streams, &mut ticket).await.is_err() {
        quarantined.push_cleanup(ticket);
    }
}

async fn clean_ticket(
    driver: &Arc<dyn DeviceDriver>,
    streams: &Arc<StreamBudgetManager>,
    ticket: &mut DeviceCleanupTicket,
) -> Result<DeviceReleaseProof, DeviceControlError> {
    let udid = ticket.lease.udid().to_string();
    let owner = ticket.lease.owner();
    let stop = streams.begin_stop(ticket.reservation.token())?;
    let proof = driver
        .stop_owned_stream(&udid)
        .await
        .map_err(|error| driver_error(&udid, "stopOwnedStream", error))?;
    let expected = ticket
        .expected_generation
        .ok_or_else(|| DeviceControlError::StopGenerationUnknown { udid: udid.clone() })?;
    if proof.old_generation != expected {
        return Err(DeviceControlError::StopProofMismatch {
            udid,
            expected,
            actual: proof.old_generation,
        });
    }
    streams.complete_stop(stop, proof)?;
    driver.invalidate_ui_session(&udid);

    Ok(DeviceReleaseProof {
        udid,
        owner,
        stopped_generation: proof.old_generation,
        next_generation: proof.new_generation,
    })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::Arc;

    use async_trait::async_trait;
    use tokio::sync::{Notify, Semaphore};
    use tokio::time::{timeout, Duration};

    use super::*;
    use crate::{
        ActiveTransport, ConnectionKind, DeviceCapabilityQualification, DeviceInfo,
        DeviceQualificationBase, DeviceStatus, InstalledAgentIdentity, InstalledTargetIdentity,
        InteractionSessionKind, OpenUrlCapability, ProtectedRouteContract, QualifiedGeometry,
        RouteMethod, RouteScope, ScreenOrientation, StreamStartProof, StreamStopProof,
        SwipeGesture, TapPoint, UiCapabilities,
    };

    const TEST_TIMEOUT: Duration = Duration::from_secs(1);
    const QUICK_WAIT: Duration = Duration::from_millis(25);

    struct TestDriver {
        session_starts: AtomicUsize,
        stream_starts: AtomicUsize,
        first_frame_observed: AtomicBool,
        launch_calls: AtomicUsize,
        guarded_clipboard_calls: AtomicUsize,
        guarded_clipboard_completions: AtomicUsize,
        guarded_clipboard_cleanup_started_early: AtomicBool,
        termination_calls: Mutex<Vec<String>>,
        inspection_snapshot: Mutex<Option<DeviceCapabilitySnapshot>>,
        inspected_targets: Mutex<Vec<String>>,
        inspection_fails: AtomicBool,
        configured_ui: Mutex<Vec<UiCapabilities>>,
        confirm_stopped_calls: AtomicUsize,
        stop_calls: AtomicUsize,
        ui_session_starts: AtomicUsize,
        invalidate_calls: AtomicUsize,
        stop_generation: AtomicU64,
        unconfirmed_stops: AtomicBool,
        block_sessions: AtomicBool,
        block_streams: AtomicBool,
        block_background_streams: AtomicBool,
        block_guarded_clipboard: AtomicBool,
        block_terminations: AtomicBool,
        session_started: Notify,
        allow_session: Semaphore,
        stream_started: Notify,
        allow_stream: Semaphore,
        background_stream_started: Notify,
        allow_background_stream: Semaphore,
        guarded_clipboard_stopped: Notify,
        allow_guarded_clipboard: Semaphore,
        allow_termination: Semaphore,
        stop_started: Notify,
        allow_stop: Semaphore,
    }

    impl Default for TestDriver {
        fn default() -> Self {
            Self {
                session_starts: AtomicUsize::new(0),
                stream_starts: AtomicUsize::new(0),
                first_frame_observed: AtomicBool::new(true),
                launch_calls: AtomicUsize::new(0),
                guarded_clipboard_calls: AtomicUsize::new(0),
                guarded_clipboard_completions: AtomicUsize::new(0),
                guarded_clipboard_cleanup_started_early: AtomicBool::new(false),
                termination_calls: Mutex::new(Vec::new()),
                inspection_snapshot: Mutex::new(None),
                inspected_targets: Mutex::new(Vec::new()),
                inspection_fails: AtomicBool::new(false),
                configured_ui: Mutex::new(Vec::new()),
                confirm_stopped_calls: AtomicUsize::new(0),
                stop_calls: AtomicUsize::new(0),
                ui_session_starts: AtomicUsize::new(0),
                invalidate_calls: AtomicUsize::new(0),
                stop_generation: AtomicU64::new(7),
                unconfirmed_stops: AtomicBool::new(false),
                block_sessions: AtomicBool::new(false),
                block_streams: AtomicBool::new(false),
                block_background_streams: AtomicBool::new(false),
                block_guarded_clipboard: AtomicBool::new(false),
                block_terminations: AtomicBool::new(false),
                session_started: Notify::new(),
                allow_session: Semaphore::new(0),
                stream_started: Notify::new(),
                allow_stream: Semaphore::new(0),
                background_stream_started: Notify::new(),
                allow_background_stream: Semaphore::new(0),
                guarded_clipboard_stopped: Notify::new(),
                allow_guarded_clipboard: Semaphore::new(0),
                allow_termination: Semaphore::new(0),
                stop_started: Notify::new(),
                allow_stop: Semaphore::new(0),
            }
        }
    }

    impl TestDriver {
        async fn wait_for_stop(&self) {
            timeout(TEST_TIMEOUT, self.stop_started.notified())
                .await
                .expect("cleanup worker should start the producer stop");
        }

        async fn wait_for_stop_count(&self, expected: usize) {
            timeout(TEST_TIMEOUT, async {
                while self.stop_calls.load(Ordering::SeqCst) < expected {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("expected producer stops should begin independently");
        }

        fn complete_stop(&self) {
            self.allow_stop.add_permits(1);
        }

        fn set_stop_generation(&self, generation: u64) {
            self.stop_generation.store(generation, Ordering::SeqCst);
        }

        fn return_unconfirmed_stops(&self) {
            self.unconfirmed_stops.store(true, Ordering::SeqCst);
        }

        fn block_session_start(&self) {
            self.block_sessions.store(true, Ordering::SeqCst);
        }

        fn block_stream_start(&self) {
            self.block_streams.store(true, Ordering::SeqCst);
        }

        fn block_background_stream_start(&self) {
            self.block_background_streams.store(true, Ordering::SeqCst);
        }

        fn block_guarded_clipboard_after_stop(&self) {
            self.block_guarded_clipboard.store(true, Ordering::SeqCst);
        }

        fn set_inspection_snapshot(&self, snapshot: DeviceCapabilitySnapshot) {
            *self.inspection_snapshot.lock() = Some(snapshot);
        }

        fn fail_inspection(&self) {
            self.inspection_fails.store(true, Ordering::SeqCst);
        }

        fn omit_first_frame(&self) {
            self.first_frame_observed.store(false, Ordering::SeqCst);
        }

        async fn wait_for_session_start(&self) {
            timeout(TEST_TIMEOUT, self.session_started.notified())
                .await
                .expect("session creation should begin");
        }

        async fn wait_for_stream_start(&self) {
            timeout(TEST_TIMEOUT, self.stream_started.notified())
                .await
                .expect("stream creation should begin");
        }

        async fn wait_for_background_stream_start(&self) {
            timeout(TEST_TIMEOUT, self.background_stream_started.notified())
                .await
                .expect("background stream creation should begin");
        }

        async fn wait_for_guarded_clipboard_stop(&self) {
            timeout(TEST_TIMEOUT, self.guarded_clipboard_stopped.notified())
                .await
                .expect("guarded clipboard should publish its destructive stop");
        }

        async fn wait_for_guarded_clipboard_completion(&self) {
            timeout(TEST_TIMEOUT, async {
                while self.guarded_clipboard_completions.load(Ordering::SeqCst) == 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("guarded clipboard should finish restoring the target and stream");
        }

        fn complete_guarded_clipboard(&self) {
            self.allow_guarded_clipboard.add_permits(1);
        }

        fn complete_background_stream_start(&self) {
            self.allow_background_stream.add_permits(1);
        }

        fn block_termination(&self) {
            self.block_terminations.store(true, Ordering::SeqCst);
        }

        async fn wait_for_termination_count(&self, expected: usize) {
            timeout(TEST_TIMEOUT, async {
                while self.termination_count() < expected {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("expected app terminations should begin independently");
        }

        fn termination_count(&self) -> usize {
            self.termination_calls.lock().len()
        }

        fn termination_count_for(&self, udid: &str) -> usize {
            self.termination_calls
                .lock()
                .iter()
                .filter(|called_udid| called_udid.as_str() == udid)
                .count()
        }

        fn complete_terminations(&self, count: usize) {
            self.allow_termination.add_permits(count);
        }
    }

    struct TestSession;

    #[async_trait]
    impl crate::UiSession for TestSession {
        async fn tap(&self, _point: TapPoint) -> anyhow::Result<()> {
            Ok(())
        }

        async fn swipe(&self, _gesture: SwipeGesture) -> anyhow::Result<()> {
            Ok(())
        }

        async fn type_text(&self, _text: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn home(&self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn find_and_tap(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn assert_visible(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        fn stream_url(&self) -> Option<String> {
            Some("http://fixture/stream".to_string())
        }
    }

    #[async_trait]
    impl crate::DeviceDriver for TestDriver {
        fn supports_verified_app_termination(&self, _udid: &str) -> bool {
            true
        }

        async fn inspect_app_process(
            &self,
            _udid: &str,
            bundle_id: &str,
        ) -> anyhow::Result<AppProcessState> {
            Ok(AppProcessState {
                bundle_id: bundle_id.to_string(),
                pid: Some(42),
                running: true,
            })
        }

        async fn inspect_interaction_device(
            &self,
            _udid: &str,
        ) -> anyhow::Result<DeviceCapabilitySnapshot> {
            if self.inspection_fails.load(Ordering::SeqCst) {
                anyhow::bail!("fixture inspection failure");
            }
            self.inspection_snapshot
                .lock()
                .clone()
                .ok_or_else(|| anyhow::anyhow!("fixture inspection snapshot missing"))
        }

        async fn inspect_device_for_target(
            &self,
            _udid: &str,
            target_bundle_id: &str,
        ) -> anyhow::Result<DeviceCapabilitySnapshot> {
            self.inspected_targets
                .lock()
                .push(target_bundle_id.to_string());
            let mut snapshot = self
                .inspection_snapshot
                .lock()
                .clone()
                .ok_or_else(|| anyhow::anyhow!("fixture inspection snapshot missing"))?;
            snapshot.target_app.bundle_id = target_bundle_id.to_string();
            Ok(snapshot)
        }

        async fn set_negotiated_interaction_capabilities(
            &self,
            _udid: &str,
            capabilities: UiCapabilities,
        ) -> anyhow::Result<()> {
            self.configured_ui.lock().push(capabilities);
            Ok(())
        }

        async fn stop_owned_stream(&self, _udid: &str) -> anyhow::Result<StreamStopProof> {
            if self.block_guarded_clipboard.load(Ordering::SeqCst)
                && self.guarded_clipboard_completions.load(Ordering::SeqCst) == 0
            {
                self.guarded_clipboard_cleanup_started_early
                    .store(true, Ordering::SeqCst);
            }
            self.stop_calls.fetch_add(1, Ordering::SeqCst);
            self.stop_started.notify_one();
            let permit = self
                .allow_stop
                .acquire()
                .await
                .expect("test stop semaphore remains open");
            permit.forget();
            let generation = self.stop_generation.load(Ordering::SeqCst);
            if self.unconfirmed_stops.load(Ordering::SeqCst) {
                return Ok(StreamStopProof {
                    old_generation: generation,
                    new_generation: generation,
                    child_stopped: false,
                });
            }
            Ok(StreamStopProof {
                old_generation: generation,
                new_generation: generation + 1,
                child_stopped: true,
            })
        }

        async fn confirm_interaction_stream_stopped(
            &self,
            _udid: &str,
        ) -> anyhow::Result<crate::StreamHandoffProof> {
            self.confirm_stopped_calls.fetch_add(1, Ordering::SeqCst);
            Ok(crate::StreamHandoffProof {
                generation: self.stop_generation.load(Ordering::SeqCst),
            })
        }

        async fn start_stream_after_session(
            &self,
            _udid: &str,
        ) -> anyhow::Result<StreamStartProof> {
            self.stream_starts.fetch_add(1, Ordering::SeqCst);
            if self.block_streams.load(Ordering::SeqCst) {
                self.stream_started.notify_one();
                let permit = self
                    .allow_stream
                    .acquire()
                    .await
                    .expect("test stream semaphore remains open");
                permit.forget();
            }
            Ok(StreamStartProof {
                generation: 7,
                first_frame_observed: self.first_frame_observed.load(Ordering::SeqCst),
                stream_url: "http://fixture/stream".to_string(),
            })
        }

        async fn start_interaction_session(
            &self,
            _udid: &str,
            _bundle_id: &str,
            _kind: InteractionSessionKind,
        ) -> anyhow::Result<Box<dyn crate::UiSession>> {
            self.session_starts.fetch_add(1, Ordering::SeqCst);
            if self.block_sessions.load(Ordering::SeqCst) {
                self.session_started.notify_one();
                let permit = self
                    .allow_session
                    .acquire()
                    .await
                    .expect("test session semaphore remains open");
                permit.forget();
            }
            Ok(Box::new(TestSession))
        }

        async fn guarded_clipboard_transition(
            &self,
            _udid: &str,
            agent_bundle_id: &str,
            target_bundle_id: &str,
            _final_session_kind: InteractionSessionKind,
            mode: ClipboardAccessMode,
            operation: GuardedClipboardOperation,
            progress: GuardedClipboardProgress,
        ) -> anyhow::Result<crate::GuardedClipboardTransition> {
            self.guarded_clipboard_calls.fetch_add(1, Ordering::SeqCst);
            let output = match operation {
                GuardedClipboardOperation::Set { .. } => GuardedClipboardOutput::Written,
                GuardedClipboardOperation::Get { .. } => GuardedClipboardOutput::Value {
                    content_type: "plaintext".to_string(),
                    bytes: b"fixture-clipboard".to_vec(),
                },
            };
            let target = crate::ActiveAppIdentity {
                bundle_id: target_bundle_id.to_string(),
                pid: 902,
            };
            if mode == ClipboardAccessMode::TargetBackgroundSafe {
                return Ok(crate::GuardedClipboardTransition {
                    output,
                    stop: None,
                    agent: None,
                    target,
                    final_session: None,
                    stream: None,
                });
            }
            self.stop_generation.store(8, Ordering::SeqCst);
            let stop = StreamStopProof {
                old_generation: 7,
                new_generation: 8,
                child_stopped: true,
            };
            progress.record_stop(stop);
            if self.block_guarded_clipboard.load(Ordering::SeqCst) {
                self.guarded_clipboard_stopped.notify_one();
                let permit = self
                    .allow_guarded_clipboard
                    .acquire()
                    .await
                    .expect("guarded clipboard semaphore remains open");
                permit.forget();
            }
            let stream = StreamStartProof {
                generation: 8,
                first_frame_observed: true,
                stream_url: "http://fixture/replacement".to_string(),
            };
            progress.record_stream(stream.clone());
            self.guarded_clipboard_completions
                .fetch_add(1, Ordering::SeqCst);
            Ok(crate::GuardedClipboardTransition {
                output,
                stop: Some(stop),
                agent: Some(crate::ActiveAppIdentity {
                    bundle_id: agent_bundle_id.to_string(),
                    pid: 701,
                }),
                target,
                final_session: Some(Box::new(TestSession)),
                stream: Some(stream),
            })
        }

        async fn list_devices(&self) -> anyhow::Result<Vec<DeviceInfo>> {
            Ok(Vec::new())
        }

        async fn refresh_device(&self, udid: &str) -> anyhow::Result<DeviceInfo> {
            Ok(DeviceInfo {
                udid: udid.to_string(),
                name: "fixture".to_string(),
                model: "fixture".to_string(),
                platform: crate::DevicePlatform::Ios,
                os_version: "fixture".to_string(),
                connection: ConnectionKind::Mock,
                status: DeviceStatus::Ready,
                battery: None,
                wda_ready: true,
                wda_expires_at: None,
                stream_url: None,
                tile_stream_state: crate::TileStreamState::Parked,
                last_error: None,
            })
        }

        async fn install_app(&self, _udid: &str, _path: &Path) -> anyhow::Result<()> {
            Ok(())
        }

        async fn uninstall_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn screenshot(&self, _udid: &str, dest: &Path) -> anyhow::Result<PathBuf> {
            Ok(dest.to_path_buf())
        }

        async fn syslog_tail(&self, _udid: &str, _lines: usize) -> anyhow::Result<String> {
            Ok(String::new())
        }

        async fn launch_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
            self.launch_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn terminate_app(
            &self,
            udid: &str,
            bundle_id: &str,
        ) -> anyhow::Result<ProcessAbsenceProof> {
            self.termination_calls.lock().push(udid.to_string());
            if self.block_terminations.load(Ordering::SeqCst) {
                let permit = self
                    .allow_termination
                    .acquire()
                    .await
                    .expect("test termination semaphore remains open");
                permit.forget();
            }
            Ok(ProcessAbsenceProof {
                bundle_id: bundle_id.to_string(),
                old_pid: Some(42),
            })
        }

        async fn reboot(&self, _udid: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn start_ui_session(&self, _udid: &str) -> anyhow::Result<Box<dyn crate::UiSession>> {
            self.ui_session_starts.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(TestSession))
        }

        fn invalidate_ui_session(&self, _udid: &str) {
            self.invalidate_calls.fetch_add(1, Ordering::SeqCst);
        }

        async fn ensure_stream(&self, _udid: &str) -> anyhow::Result<String> {
            if self.block_background_streams.load(Ordering::SeqCst) {
                self.background_stream_started.notify_one();
                let permit = self
                    .allow_background_stream
                    .acquire()
                    .await
                    .expect("test background stream semaphore remains open");
                permit.forget();
            }
            Ok("http://fixture/stream".to_string())
        }

        async fn prepare_device(&self, _udid: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn control_plane(driver: Arc<TestDriver>, limit: usize) -> DeviceControlPlane {
        DeviceControlPlane::new(
            driver,
            Arc::new(crate::DeviceWorkCoordinator::new()),
            Arc::new(crate::StreamBudgetManager::new(limit).expect("valid test stream limit")),
        )
    }

    fn capability_fixture() -> (DeviceCapabilitySnapshot, DeviceCapabilityQualification) {
        const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        const SHA_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        let geometry = QualifiedGeometry {
            logical_width: 375.0,
            logical_height: 667.0,
            pixel_width: 750,
            pixel_height: 1334,
            scale_x: 2.0,
            scale_y: 2.0,
            orientation: ScreenOrientation::Portrait,
        };
        let installed_agent = InstalledAgentIdentity {
            bundle_id: "com.fixture.agent".to_string(),
            version: "1.0".to_string(),
            build: "10".to_string(),
            executable_name: "FixtureRunner".to_string(),
            signer_identity_sha256: SHA_B.to_string(),
        };
        let target_app = InstalledTargetIdentity {
            bundle_id: "com.ss.iphone.ugc.Ame".to_string(),
            version: "35.0.0".to_string(),
            build: "350001".to_string(),
        };
        let snapshot = DeviceCapabilitySnapshot {
            installed_agent: installed_agent.clone(),
            selected_artifact_sha256: SHA_A.to_string(),
            agent_version: "fixture-agent-1".to_string(),
            protocol_version: 2,
            driver_adapter_version: "interaction-v1".to_string(),
            transport: ActiveTransport::LegacyUsbmuxTransport,
            product_type: "iPhone10,1".to_string(),
            os_version: "16.7.15".to_string(),
            target_app: target_app.clone(),
            protected_auth_ready: true,
            geometry: Some(geometry.clone()),
        };
        let qualification = DeviceCapabilityQualification {
            qualification_id: "fixture-g0".to_string(),
            environment: "FIXTURE_ONLY".to_string(),
            base: DeviceQualificationBase {
                installed_agent,
                selected_artifact_sha256: SHA_A.to_string(),
                agent_version: "fixture-agent-1".to_string(),
                protocol_version: 2,
                driver_adapter_version: "interaction-v1".to_string(),
                transport: ActiveTransport::LegacyUsbmuxTransport,
                product_type: "iPhone10,1".to_string(),
                ios_min_inclusive: "16.7.15".to_string(),
                ios_max_inclusive: "16.7.15".to_string(),
                target_app,
                geometry,
            },
            ui: UiCapabilities {
                open_url: Some(OpenUrlCapability {
                    route: ProtectedRouteContract {
                        contract_id: "open-url-v1".to_string(),
                        method: RouteMethod::Post,
                        scope: RouteScope::Sessionless,
                        path: "/wda/openUrl".to_string(),
                        auth_header_name: "X-Riviu-Token".to_string(),
                        body_schema_id: "open-url-body-v1".to_string(),
                        request_timeout_ms: 10_000,
                    },
                    target_bundle_id: "com.ss.iphone.ugc.Ame".to_string(),
                    live_report_sha256: SHA_C.to_string(),
                }),
                ..UiCapabilities::default()
            },
            clipboard_contract_id: None,
        };
        (snapshot, qualification)
    }

    #[tokio::test]
    async fn flow_inspection_is_target_qualified_without_mutating_interaction_capabilities() {
        let driver = Arc::new(TestDriver::default());
        let (snapshot, _) = capability_fixture();
        driver.set_inspection_snapshot(snapshot);
        let control = control_plane(driver.clone(), 1);
        let context = control
            .try_acquire_exclusive("iphone-a", DeviceWorkOwner::Script)
            .await
            .expect("Flow exclusive context");

        let inspected = control
            .inspect_flow_device(&context, "com.apple.Preferences")
            .await
            .expect("target-qualified inspection");

        assert_eq!(inspected.target_app.bundle_id, "com.apple.Preferences");
        assert_eq!(
            driver.inspected_targets.lock().as_slice(),
            &["com.apple.Preferences"]
        );
        assert!(driver.configured_ui.lock().is_empty());
        control
            .close_exclusive_context(context)
            .expect("close Flow exclusive context");
        control.shutdown_cleanup().await.expect("control shutdown");
    }

    #[tokio::test]
    async fn flow_inspection_rejects_blank_target_before_driver_io() {
        let driver = Arc::new(TestDriver::default());
        let control = control_plane(driver.clone(), 1);
        let context = control
            .try_acquire_exclusive("iphone-a", DeviceWorkOwner::Script)
            .await
            .expect("Flow exclusive context");

        control
            .inspect_flow_device(&context, "  ")
            .await
            .expect_err("blank target must fail closed");

        assert!(driver.inspected_targets.lock().is_empty());
        control
            .close_exclusive_context(context)
            .expect("close Flow exclusive context");
        control.shutdown_cleanup().await.expect("control shutdown");
    }

    #[tokio::test]
    async fn explicit_context_close_reports_the_highest_acquired_level() {
        let driver = Arc::new(TestDriver::default());
        let control = control_plane(driver, 1);
        let exclusive = control
            .try_acquire_exclusive("iphone-a", DeviceWorkOwner::Script)
            .await
            .expect("exclusive context");
        let exclusive_proof = control
            .close_exclusive_context(exclusive)
            .expect("close exclusive context");
        assert_eq!(
            exclusive_proof,
            ContextReleaseProof {
                udid: "iphone-a".to_string(),
                owner: DeviceWorkOwner::Script,
                had_session: false,
                had_stream: false,
            }
        );

        let exclusive = control
            .try_acquire_exclusive("iphone-a", DeviceWorkOwner::Script)
            .await
            .expect("replacement exclusive context");
        let session = control
            .start_owned_ui_session(exclusive)
            .await
            .expect("session context");
        let session_proof = control
            .close_session_context(session)
            .expect("close session context");
        assert_eq!(
            session_proof,
            ContextReleaseProof {
                udid: "iphone-a".to_string(),
                owner: DeviceWorkOwner::Script,
                had_session: true,
                had_stream: false,
            }
        );
        control.shutdown_cleanup().await.expect("control shutdown");
    }

    #[tokio::test]
    async fn interaction_session_records_a_stop_witness_without_a_prior_producer() {
        let driver = Arc::new(TestDriver::default());
        let control = control_plane(driver.clone(), 1);
        let exclusive = control
            .try_acquire_exclusive("fixture", DeviceWorkOwner::Interaction)
            .await
            .expect("exclusive context");
        let (exclusive, capacity) = control
            .reserve_ui_capacity(exclusive)
            .await
            .expect("reserved capacity");

        let session = control
            .start_interaction_session(
                exclusive,
                "com.ss.iphone.ugc.Ame",
                InteractionSessionKind::Ordinary,
            )
            .await
            .expect("interaction session");

        assert_eq!(driver.confirm_stopped_calls.load(Ordering::SeqCst), 1);
        assert_eq!(driver.stop_calls.load(Ordering::SeqCst), 0);
        assert_eq!(driver.session_starts.load(Ordering::SeqCst), 1);
        control
            .close_session_context(session)
            .expect("close session context");
        drop(capacity);
        control.shutdown_cleanup().await.expect("control shutdown");
    }

    #[tokio::test]
    async fn interaction_session_rejects_an_exclusive_context_without_reserved_capacity() {
        let driver = Arc::new(TestDriver::default());
        let control = control_plane(driver.clone(), 1);
        let exclusive = control
            .try_acquire_exclusive("fixture", DeviceWorkOwner::Interaction)
            .await
            .expect("exclusive context");

        let error = match control
            .start_interaction_session(
                exclusive,
                "com.ss.iphone.ugc.Ame",
                InteractionSessionKind::Ordinary,
            )
            .await
        {
            Ok(_) => panic!("session startup must require reserved stream capacity"),
            Err(error) => error,
        };

        assert!(matches!(error, DeviceControlError::InvalidContext { .. }));
        assert_eq!(driver.confirm_stopped_calls.load(Ordering::SeqCst), 0);
        assert_eq!(driver.session_starts.load(Ordering::SeqCst), 0);
        control.shutdown_cleanup().await.expect("control shutdown");
    }

    #[tokio::test]
    async fn interaction_session_rejects_a_released_capacity_reservation() {
        let driver = Arc::new(TestDriver::default());
        let control = control_plane(driver.clone(), 1);
        let exclusive = control
            .try_acquire_exclusive("fixture", DeviceWorkOwner::Interaction)
            .await
            .expect("exclusive context");
        let (exclusive, capacity) = control
            .reserve_ui_capacity(exclusive)
            .await
            .expect("reserved capacity");
        drop(capacity);

        let error = match control
            .start_interaction_session(
                exclusive,
                "com.ss.iphone.ugc.Ame",
                InteractionSessionKind::Ordinary,
            )
            .await
        {
            Ok(_) => panic!("released capacity must not authorize session startup"),
            Err(error) => error,
        };

        assert!(matches!(error, DeviceControlError::InvalidContext { .. }));
        assert_eq!(driver.confirm_stopped_calls.load(Ordering::SeqCst), 0);
        assert_eq!(driver.session_starts.load(Ordering::SeqCst), 0);
        control.shutdown_cleanup().await.expect("control shutdown");
    }

    #[tokio::test]
    async fn metadata_inspection_stays_denied_until_complete_runtime_snapshot_is_applied() {
        let driver = Arc::new(TestDriver::default());
        let (runtime_snapshot, qualification) = capability_fixture();
        let mut metadata_snapshot = runtime_snapshot.clone();
        metadata_snapshot.protected_auth_ready = false;
        metadata_snapshot.geometry = None;
        let expected_ui = qualification.ui.clone();
        driver.set_inspection_snapshot(metadata_snapshot.clone());
        let registry = Arc::new(
            DeviceCapabilityRegistry::try_new(vec![qualification])
                .expect("valid capability fixture"),
        );
        let control = DeviceControlPlane::new_with_capability_registry(
            driver.clone(),
            Arc::new(crate::DeviceWorkCoordinator::new()),
            Arc::new(crate::StreamBudgetManager::new(1).expect("stream budget")),
            registry,
        );
        let context = control
            .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::Interaction)
            .await
            .expect("interaction context");

        assert_eq!(
            control
                .inspect_interaction_device(&context)
                .await
                .expect("metadata inspection"),
            metadata_snapshot
        );
        assert_eq!(
            driver.configured_ui.lock().as_slice(),
            &[UiCapabilities::default()]
        );

        let metadata_capabilities = control
            .negotiate_interaction_capabilities(&context, &metadata_snapshot)
            .await
            .expect("metadata snapshot remains a valid denied result");
        assert_eq!(metadata_capabilities.ui, UiCapabilities::default());

        let runtime_capabilities = control
            .negotiate_interaction_capabilities(&context, &runtime_snapshot)
            .await
            .expect("complete runtime proof can apply the exact registry match");
        assert_eq!(runtime_capabilities.ui, expected_ui);
        assert_eq!(
            driver.configured_ui.lock().as_slice(),
            &[
                UiCapabilities::default(),
                UiCapabilities::default(),
                expected_ui
            ]
        );

        driver.fail_inspection();
        control
            .inspect_interaction_device(&context)
            .await
            .expect_err("failed reinspection must fail closed");
        assert_eq!(
            driver.configured_ui.lock().last(),
            Some(&UiCapabilities::default())
        );
    }

    async fn streaming_context(
        control: &DeviceControlPlane,
        udid: &str,
        owner: crate::DeviceWorkOwner,
    ) -> UiWithStreamContext {
        let exclusive = control
            .try_acquire_exclusive(udid, owner)
            .await
            .expect("exclusive device");
        let (exclusive, reservation) = control
            .reserve_ui_capacity(exclusive)
            .await
            .expect("foreground capacity");
        let session = control
            .start_interaction_session(
                exclusive,
                "com.ss.iphone.ugc.Ame",
                InteractionSessionKind::Ordinary,
            )
            .await
            .expect("interaction session");
        control
            .start_reserved_stream(session, reservation)
            .await
            .expect("stream after session")
    }

    #[tokio::test]
    async fn shared_device_owner_busy_does_not_create_a_session() {
        let driver = Arc::new(TestDriver::default());
        let control = control_plane(driver.clone(), 1);
        let _script = control
            .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::Script)
            .await
            .expect("script owns the device");

        let error = control
            .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::ManualControl)
            .await
            .expect_err("manual control must fail fast");

        assert!(matches!(
            error,
            DeviceControlError::Busy(crate::DeviceBusy {
                requested_owner: crate::DeviceWorkOwner::ManualControl,
                current_owner: crate::DeviceWorkOwner::Script,
                ..
            })
        ));
        assert_eq!(driver.session_starts.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_manual_session_does_not_park_the_live_preview() {
        let driver = Arc::new(TestDriver::default());
        let control = control_plane(driver.clone(), 1);
        let background = control
            .reserve_background_stream("iphone-a")
            .expect("background reservation");
        control
            .start_background_stream(&background)
            .await
            .expect("background producer");

        let session = control
            .open_manual_session("iphone-a", crate::DeviceWorkOwner::ManualControl)
            .await
            .expect("manual session beside the live tile");
        assert_eq!(driver.stop_calls.load(Ordering::SeqCst), 0);
        assert_eq!(driver.ui_session_starts.load(Ordering::SeqCst), 1);
        assert_eq!(driver.session_starts.load(Ordering::SeqCst), 0);
        assert_eq!(control.reserved_stream_capacity(), 1);

        control
            .close_manual_session(session)
            .expect("release the exclusive without dropping the WDA cache");
        assert_eq!(driver.invalidate_calls.load(Ordering::SeqCst), 0);
        assert_eq!(control.reserved_stream_capacity(), 1);

        driver.complete_stop();
        control.shutdown_cleanup().await.expect("control shutdown");
    }

    #[tokio::test]
    async fn a_manual_session_fails_fast_when_another_owner_holds_the_device() {
        let driver = Arc::new(TestDriver::default());
        let control = control_plane(driver.clone(), 1);
        let script = control
            .try_acquire_exclusive_keeping_stream("iphone-a", crate::DeviceWorkOwner::Script)
            .await
            .expect("script owns the device");

        let error = match control
            .open_manual_session("iphone-a", crate::DeviceWorkOwner::ManualControl)
            .await
        {
            Ok(_) => panic!("manual control must fail fast"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            DeviceControlError::Busy(crate::DeviceBusy {
                requested_owner: crate::DeviceWorkOwner::ManualControl,
                current_owner: crate::DeviceWorkOwner::Script,
                ..
            })
        ));
        assert_eq!(driver.ui_session_starts.load(Ordering::SeqCst), 0);
        drop(script);
        control.shutdown_cleanup().await.expect("control shutdown");
    }

    #[tokio::test]
    async fn two_manual_sessions_on_the_same_device_are_busy() {
        let driver = Arc::new(TestDriver::default());
        let control = control_plane(driver, 1);
        let first = control
            .open_manual_session("iphone-a", crate::DeviceWorkOwner::ManualControl)
            .await
            .expect("first manual session");
        let error = match control
            .open_manual_session("iphone-a", crate::DeviceWorkOwner::ManualControl)
            .await
        {
            Ok(_) => panic!("a second manual session must fail busy"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            DeviceControlError::Busy(crate::DeviceBusy {
                requested_owner: crate::DeviceWorkOwner::ManualControl,
                current_owner: crate::DeviceWorkOwner::ManualControl,
                ..
            })
        ));
        control
            .close_manual_session(first)
            .expect("release the first session");
        control.shutdown_cleanup().await.expect("control shutdown");
    }

    #[tokio::test]
    async fn a_held_session_lease_runs_a_device_action_without_a_second_acquire() {
        // The overlay opens one manual session and keeps it for as long as the operator has
        // the phone on screen. Ten of its own rows -- rotate, install, adb, import, export,
        // reboot, backup, restore, screenshot, change keyboard -- used to demand a *second*
        // lease and were refused Busy every time, which is to say whenever they were
        // reachable at all. They ride the held lease now.
        let driver = Arc::new(TestDriver::default());
        let control = control_plane(driver, 1);
        let session = control
            .open_manual_session("iphone-a", crate::DeviceWorkOwner::ManualControl)
            .await
            .expect("manual session");

        control
            .reboot(&session)
            .await
            .expect("reboot on the held lease");

        // And the session is still whole afterwards: borrowing must not consume it, or the
        // overlay would be dead for every gesture that follows.
        control
            .close_manual_session(session)
            .expect("the session survives a borrowed action");
        control.shutdown_cleanup().await.expect("control shutdown");
    }

    #[tokio::test]
    async fn an_open_overlay_lease_still_excludes_every_background_owner() {
        // The invariant AGENTS.md 9.58 and 4773 rest on, asserted directly rather than
        // assumed. Lending is only ever offered to the UI that opened the phone; nurture, a
        // flow, a script or a repair still finds it busy. If this ever passes for the wrong
        // reason, "ManualControl holds this phone" has stopped meaning one thing touches it.
        let driver = Arc::new(TestDriver::default());
        let control = control_plane(driver, 1);
        let session = control
            .open_manual_session("iphone-a", crate::DeviceWorkOwner::ManualControl)
            .await
            .expect("manual session");

        for owner in [
            crate::DeviceWorkOwner::Nurture,
            crate::DeviceWorkOwner::Script,
            crate::DeviceWorkOwner::Repair,
            crate::DeviceWorkOwner::Interaction,
        ] {
            match control.try_acquire_exclusive("iphone-a", owner).await {
                Ok(_) => panic!("{owner:?} must not take a phone the overlay is holding"),
                Err(DeviceControlError::Busy(busy)) => {
                    assert_eq!(busy.requested_owner, owner);
                    assert_eq!(busy.current_owner, crate::DeviceWorkOwner::ManualControl);
                }
                Err(other) => panic!("expected Busy for {owner:?}, got {other:?}"),
            }
        }

        control
            .close_manual_session(session)
            .expect("release the session");
        control.shutdown_cleanup().await.expect("control shutdown");
    }

    #[tokio::test]
    async fn a_session_lease_from_another_control_plane_is_refused() {
        // The borrow must not become a way around the plane check: a context is only ever
        // valid on the plane that minted it.
        let first = control_plane(Arc::new(TestDriver::default()), 1);
        let second = control_plane(Arc::new(TestDriver::default()), 1);
        let session = first
            .open_manual_session("iphone-a", crate::DeviceWorkOwner::ManualControl)
            .await
            .expect("manual session");

        match second.reboot(&session).await {
            Err(DeviceControlError::InvalidContext { .. }) => {}
            other => panic!("a foreign plane's context must be refused, got {other:?}"),
        }

        first
            .close_manual_session(session)
            .expect("release the session");
        first.shutdown_cleanup().await.expect("first shutdown");
        second.shutdown_cleanup().await.expect("second shutdown");
    }

    #[tokio::test]
    async fn a_held_manual_session_serves_two_gestures_without_parking() {
        let driver = Arc::new(TestDriver::default());
        let control = control_plane(driver.clone(), 1);
        let background = control
            .reserve_background_stream("iphone-a")
            .expect("background reservation");
        control
            .start_background_stream(&background)
            .await
            .expect("background producer");

        let session = control
            .open_manual_session("iphone-a", crate::DeviceWorkOwner::ManualControl)
            .await
            .expect("overlay session");
        let handle = control.session(&session).expect("session handle");
        handle
            .tap(TapPoint { x: 10.0, y: 10.0 })
            .await
            .expect("first tap");
        handle
            .tap(TapPoint { x: 20.0, y: 20.0 })
            .await
            .expect("second tap");
        assert_eq!(driver.stop_calls.load(Ordering::SeqCst), 0);
        assert_eq!(driver.invalidate_calls.load(Ordering::SeqCst), 0);
        assert_eq!(driver.ui_session_starts.load(Ordering::SeqCst), 1);

        control.close_manual_session(session).expect("end overlay");
        assert_eq!(driver.invalidate_calls.load(Ordering::SeqCst), 0);

        driver.complete_stop();
        control.shutdown_cleanup().await.expect("control shutdown");
    }

    #[tokio::test]
    async fn ending_a_manual_session_lets_another_owner_acquire() {
        let driver = Arc::new(TestDriver::default());
        let control = control_plane(driver, 1);
        let session = control
            .open_manual_session("iphone-a", crate::DeviceWorkOwner::ManualControl)
            .await
            .expect("overlay session");
        control.close_manual_session(session).expect("end overlay");
        let script = control
            .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::Script)
            .await
            .expect("script can acquire after overlay ends");
        drop(script);
        control.shutdown_cleanup().await.expect("control shutdown");
    }

    #[tokio::test]
    async fn screenshot_without_parking_keeps_the_live_preview() {
        let driver = Arc::new(TestDriver::default());
        let control = control_plane(driver.clone(), 1);
        let background = control
            .reserve_background_stream("iphone-a")
            .expect("background reservation");
        control
            .start_background_stream(&background)
            .await
            .expect("background producer");

        let exclusive = control
            .try_acquire_exclusive_keeping_stream("iphone-a", crate::DeviceWorkOwner::ManualControl)
            .await
            .expect("screenshot exclusive keeps the stream");
        let dest = std::env::temp_dir().join("riviu-fixture-manual-screenshot.jpg");
        control
            .screenshot(&exclusive, &dest)
            .await
            .expect("screenshot");
        assert_eq!(driver.stop_calls.load(Ordering::SeqCst), 0);
        drop(exclusive);

        driver.complete_stop();
        control.shutdown_cleanup().await.expect("control shutdown");
    }

    #[tokio::test]
    async fn shared_device_owner_manual_control_becomes_typed_interaction_skip() {
        let driver = Arc::new(TestDriver::default());
        let control = control_plane(driver, 1);
        let manual = control
            .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::ManualControl)
            .await
            .expect("manual control owns the device");

        match control
            .try_acquire_interaction("iphone-a")
            .await
            .expect("busy interaction is a typed scheduling result")
        {
            InteractionAcquireResult::SkippedUnavailable(busy) => {
                assert_eq!(busy.udid, "iphone-a");
                assert_eq!(busy.requested_owner, crate::DeviceWorkOwner::Interaction);
                assert_eq!(busy.current_owner, crate::DeviceWorkOwner::ManualControl);
            }
            InteractionAcquireResult::Acquired(_) => {
                panic!("interaction must not acquire a manually controlled device")
            }
        }
        drop(manual);
    }

    #[tokio::test]
    async fn shared_device_owner_different_udids_are_independent() {
        let driver = Arc::new(TestDriver::default());
        let control = control_plane(driver, 2);

        let first = control
            .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::Nurture)
            .await
            .expect("first device");
        let second = control
            .try_acquire_exclusive("iphone-b", crate::DeviceWorkOwner::Interaction)
            .await
            .expect("second device");

        assert_eq!(first.udid(), "iphone-a");
        assert_eq!(second.udid(), "iphone-b");
    }

    #[tokio::test]
    async fn shared_device_owner_cancelling_exclusive_context_releases_owner() {
        let driver = Arc::new(TestDriver::default());
        let control = control_plane(driver, 1);
        let context = control
            .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::Nurture)
            .await
            .expect("nurture device");

        drop(context);

        control
            .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::Repair)
            .await
            .expect("owner metadata should be released synchronously");
    }

    #[tokio::test]
    async fn shared_device_owner_stream_drop_retains_owner_and_capacity_until_stop_proof() {
        let driver = Arc::new(TestDriver::default());
        let control = control_plane(driver.clone(), 1);
        let context =
            streaming_context(&control, "iphone-a", crate::DeviceWorkOwner::Interaction).await;
        assert_eq!(control.reserved_stream_capacity(), 1);

        drop(context);
        driver.wait_for_stop().await;

        assert_eq!(control.reserved_stream_capacity(), 1);
        assert!(matches!(
            control
                .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::ManualControl)
                .await,
            Err(DeviceControlError::Busy(crate::DeviceBusy {
                current_owner: crate::DeviceWorkOwner::Interaction,
                ..
            }))
        ));

        driver.complete_stop();
        timeout(TEST_TIMEOUT, async {
            loop {
                match control
                    .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::ManualControl)
                    .await
                {
                    Ok(replacement) => break replacement,
                    Err(DeviceControlError::Busy(_)) => tokio::task::yield_now().await,
                    Err(error) => panic!("unexpected acquire error: {error}"),
                }
            }
        })
        .await
        .expect("confirmed stop should release the device");
        assert_eq!(control.reserved_stream_capacity(), 0);
    }

    #[tokio::test]
    async fn shared_device_owner_wrong_generation_proof_keeps_cleanup_quarantined() {
        let driver = Arc::new(TestDriver::default());
        let control = control_plane(driver.clone(), 1);
        let context =
            streaming_context(&control, "iphone-a", crate::DeviceWorkOwner::Interaction).await;
        driver.set_stop_generation(6);

        drop(context);
        driver.wait_for_stop().await;
        driver.complete_stop();
        timeout(TEST_TIMEOUT, async {
            while control.cleanup_quarantine_count() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("mismatched proof should be quarantined");

        assert_eq!(control.reserved_stream_capacity(), 1);
        assert!(matches!(
            control
                .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::Repair)
                .await,
            Err(DeviceControlError::Busy(crate::DeviceBusy {
                current_owner: crate::DeviceWorkOwner::Interaction,
                ..
            }))
        ));
    }

    #[tokio::test]
    async fn shared_device_owner_cancelling_session_start_releases_reserved_capacity() {
        let driver = Arc::new(TestDriver::default());
        driver.block_session_start();
        let control = Arc::new(control_plane(driver.clone(), 1));
        let task_control = control.clone();

        let task = tokio::spawn(async move {
            let exclusive = task_control
                .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::Interaction)
                .await
                .expect("interaction lease");
            let (exclusive, reservation) = task_control
                .reserve_ui_capacity(exclusive)
                .await
                .expect("foreground capacity");
            let _session = task_control
                .start_interaction_session(
                    exclusive,
                    "com.ss.iphone.ugc.Ame",
                    InteractionSessionKind::Ordinary,
                )
                .await
                .expect("blocked until the task is cancelled");
            drop(reservation);
        });
        driver.wait_for_session_start().await;
        assert_eq!(control.reserved_stream_capacity(), 1);

        task.abort();
        let _ = task.await;

        assert_eq!(control.reserved_stream_capacity(), 0);
        control
            .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::Repair)
            .await
            .expect("cancellation releases both ownership and reserved capacity");
    }

    #[tokio::test]
    async fn shared_device_owner_acquire_parks_same_udid_background_producer() {
        let driver = Arc::new(TestDriver::default());
        let work = Arc::new(crate::DeviceWorkCoordinator::new());
        let streams = Arc::new(crate::StreamBudgetManager::new(1).expect("stream budget"));
        let background = streams
            .reserve_background("iphone-a")
            .expect("background reservation");
        streams
            .mark_running(background.token())
            .expect("background producer running");
        driver.complete_stop();
        let control = DeviceControlPlane::new(driver.clone(), work, streams.clone());

        let context = control
            .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::ManualControl)
            .await
            .expect("acquire should park the prior producer");

        assert_eq!(context.owner(), crate::DeviceWorkOwner::ManualControl);
        assert_eq!(driver.stop_calls.load(Ordering::SeqCst), 1);
        assert_eq!(streams.reserved_capacity(), 0);
    }

    #[tokio::test]
    async fn shared_device_owner_cancelling_background_park_finishes_in_cleanup_worker() {
        let driver = Arc::new(TestDriver::default());
        let work = Arc::new(crate::DeviceWorkCoordinator::new());
        let streams = Arc::new(crate::StreamBudgetManager::new(1).expect("stream budget"));
        let background = streams
            .reserve_background("iphone-a")
            .expect("background reservation");
        streams
            .mark_running(background.token())
            .expect("background producer running");
        let control = Arc::new(DeviceControlPlane::new(
            driver.clone(),
            work.clone(),
            streams.clone(),
        ));
        let task_control = control.clone();

        let task = tokio::spawn(async move {
            task_control
                .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::Interaction)
                .await
        });
        driver.wait_for_stop().await;
        task.abort();
        let _ = task.await;
        driver.complete_stop();

        let replacement = timeout(TEST_TIMEOUT, async {
            loop {
                match work.try_acquire("iphone-a", crate::DeviceWorkOwner::Repair) {
                    Ok(lease) => break lease,
                    Err(_) => tokio::task::yield_now().await,
                }
            }
        })
        .await
        .expect("cleanup worker should finish the cancelled park");
        assert_eq!(replacement.owner(), crate::DeviceWorkOwner::Repair);
        assert_eq!(streams.reserved_capacity(), 0);
        assert_eq!(control.cleanup_quarantine_count(), 0);
    }

    #[tokio::test]
    async fn shared_device_owner_cancelling_capacity_preemption_finishes_in_cleanup_worker() {
        let driver = Arc::new(TestDriver::default());
        let work = Arc::new(crate::DeviceWorkCoordinator::new());
        let streams = Arc::new(crate::StreamBudgetManager::new(1).expect("stream budget"));
        let background = streams
            .reserve_background("iphone-b")
            .expect("background reservation");
        streams
            .mark_running(background.token())
            .expect("background producer running");
        let control = Arc::new(DeviceControlPlane::new(
            driver.clone(),
            work.clone(),
            streams.clone(),
        ));
        let exclusive = control
            .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::Interaction)
            .await
            .expect("target lease");
        let task_control = control.clone();

        let task = tokio::spawn(async move { task_control.reserve_ui_capacity(exclusive).await });
        driver.wait_for_stop().await;
        task.abort();
        let _ = task.await;
        driver.complete_stop();

        let replacement = timeout(TEST_TIMEOUT, async {
            loop {
                match work.try_acquire("iphone-a", crate::DeviceWorkOwner::Repair) {
                    Ok(lease) => break lease,
                    Err(_) => tokio::task::yield_now().await,
                }
            }
        })
        .await
        .expect("cleanup worker should finish the cancelled preemption");
        assert_eq!(replacement.owner(), crate::DeviceWorkOwner::Repair);
        assert_eq!(streams.reserved_capacity(), 0);
        assert_eq!(control.cleanup_quarantine_count(), 0);
    }

    #[tokio::test]
    async fn shared_device_owner_cross_udid_preemption_serializes_victim_acquisition() {
        let driver = Arc::new(TestDriver::default());
        let work = Arc::new(crate::DeviceWorkCoordinator::new());
        let streams = Arc::new(crate::StreamBudgetManager::new(1).expect("stream budget"));
        let background = streams
            .reserve_background("iphone-b")
            .expect("background reservation");
        streams
            .mark_running(background.token())
            .expect("background producer running");
        let control = Arc::new(DeviceControlPlane::new(
            driver.clone(),
            work,
            streams.clone(),
        ));
        let exclusive_a = control
            .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::Interaction)
            .await
            .expect("target lease");
        let reserve_control = control.clone();
        let reserve =
            tokio::spawn(async move { reserve_control.reserve_ui_capacity(exclusive_a).await });
        driver.wait_for_stop().await;

        let acquire_control = control.clone();
        let acquire_victim = tokio::spawn(async move {
            acquire_control
                .try_acquire_exclusive("iphone-b", crate::DeviceWorkOwner::ManualControl)
                .await
        });
        tokio::time::sleep(QUICK_WAIT).await;
        assert_eq!(
            driver.stop_calls.load(Ordering::SeqCst),
            1,
            "victim acquisition must queue behind the active preemption"
        );

        driver.complete_stop();
        let (context_a, capacity_a) = timeout(TEST_TIMEOUT, reserve)
            .await
            .expect("preemption completes")
            .expect("reserve task")
            .expect("foreground capacity");
        let context_b = timeout(TEST_TIMEOUT, acquire_victim)
            .await
            .expect("victim acquisition resumes")
            .expect("acquire task")
            .expect("victim context");
        assert_eq!(driver.stop_calls.load(Ordering::SeqCst), 1);
        assert_eq!(context_b.udid(), "iphone-b");
        drop(capacity_a);
        drop(context_a);
        drop(context_b);
        assert_eq!(streams.reserved_capacity(), 0);
    }

    /// A stalled victim stop on one device must not hold the global capacity
    /// gate: a foreground reserve for an unrelated device must resolve rather
    /// than block behind the stuck stop. Before the gate was released after
    /// begin_foreground_transfer, B deadlocked on `capacity_gate.lock()` and
    /// this timed out.
    #[tokio::test]
    async fn a_stalled_victim_stop_does_not_block_reservation_for_other_devices() {
        let driver = Arc::new(TestDriver::default());
        let streams = Arc::new(crate::StreamBudgetManager::new(1).expect("stream budget"));
        let background = streams
            .reserve_background("iphone-victim")
            .expect("background reservation");
        streams
            .mark_running(background.token())
            .expect("background producer running");
        let control = Arc::new(DeviceControlPlane::new(
            driver.clone(),
            Arc::new(crate::DeviceWorkCoordinator::new()),
            streams.clone(),
        ));

        // A reserves foreground capacity, revoking the victim; its stop blocks.
        let exclusive_a = control
            .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::Interaction)
            .await
            .expect("target A lease");
        let reserve_control = control.clone();
        let reserve_a =
            tokio::spawn(async move { reserve_control.reserve_ui_capacity(exclusive_a).await });
        driver.wait_for_stop().await;

        // B reserves for an unrelated device. Capacity is full during A's
        // transfer (A reserved + victim revoking at limit 1), so B must return
        // quickly with an error rather than block on the released gate.
        let exclusive_b = control
            .try_acquire_exclusive("iphone-b", crate::DeviceWorkOwner::ManualControl)
            .await
            .expect("target B lease");
        let outcome_b = timeout(TEST_TIMEOUT, control.reserve_ui_capacity(exclusive_b))
            .await
            .expect("a reserve for an unrelated device must not block on the stuck stop");
        assert!(
            outcome_b.is_err(),
            "capacity is full during the transfer, so B must fail fast"
        );
        assert_eq!(
            driver.stop_calls.load(Ordering::SeqCst),
            1,
            "B must not trigger a second stop"
        );

        driver.complete_stop();
        let (context_a, capacity_a) = timeout(TEST_TIMEOUT, reserve_a)
            .await
            .expect("A completes")
            .expect("reserve A task")
            .expect("A foreground capacity");
        drop(capacity_a);
        drop(context_a);
        assert_eq!(streams.reserved_capacity(), 0);
    }

    #[tokio::test]
    async fn shared_device_owner_unrelated_udid_stops_run_concurrently() {
        let driver = Arc::new(TestDriver::default());
        let streams = Arc::new(crate::StreamBudgetManager::new(2).expect("stream budget"));
        for udid in ["iphone-a", "iphone-b"] {
            let background = streams
                .reserve_background(udid)
                .expect("background reservation");
            streams
                .mark_running(background.token())
                .expect("background producer running");
        }
        let control = Arc::new(DeviceControlPlane::new(
            driver.clone(),
            Arc::new(crate::DeviceWorkCoordinator::new()),
            streams.clone(),
        ));

        let first_control = control.clone();
        let first = tokio::spawn(async move {
            first_control
                .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::ManualControl)
                .await
        });
        driver.wait_for_stop_count(1).await;
        let second_control = control.clone();
        let second = tokio::spawn(async move {
            second_control
                .try_acquire_exclusive("iphone-b", crate::DeviceWorkOwner::Interaction)
                .await
        });

        driver.wait_for_stop_count(2).await;
        driver.allow_stop.add_permits(2);
        let first = first
            .await
            .expect("first acquire task")
            .expect("first owner");
        let second = second
            .await
            .expect("second acquire task")
            .expect("second owner");
        assert_eq!(streams.reserved_capacity(), 0);
        drop(first);
        drop(second);
    }

    #[tokio::test]
    async fn shared_device_owner_missing_first_frame_still_requires_exact_stop_generation() {
        let driver = Arc::new(TestDriver::default());
        driver.omit_first_frame();
        let control = control_plane(driver.clone(), 1);
        let exclusive = control
            .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::Interaction)
            .await
            .expect("interaction lease");
        let (exclusive, capacity) = control
            .reserve_ui_capacity(exclusive)
            .await
            .expect("foreground capacity");
        let session = control
            .start_interaction_session(
                exclusive,
                "com.ss.iphone.ugc.Ame",
                InteractionSessionKind::Ordinary,
            )
            .await
            .expect("session");

        let error = match control.start_reserved_stream(session, capacity).await {
            Ok(_) => panic!("a stream without a decoded frame is not ready"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            DeviceControlError::FirstFrameMissing { .. }
        ));
        driver.wait_for_stop().await;
        driver.set_stop_generation(6);
        driver.complete_stop();
        timeout(TEST_TIMEOUT, async {
            while control.cleanup_quarantine_count() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("wrong generation must remain quarantined");

        assert_eq!(control.reserved_stream_capacity(), 1);
    }

    #[tokio::test]
    async fn hard_recovery_rejects_a_discontinuous_second_stop_before_session_restart() {
        let driver = Arc::new(TestDriver::default());
        let control = control_plane(driver.clone(), 1);
        let mut context =
            streaming_context(&control, "iphone-a", crate::DeviceWorkOwner::Nurture).await;
        driver.allow_stop.add_permits(2);

        let error = match control
            .recover_streaming_session(
                &mut context,
                "com.ss.iphone.ugc.Ame",
                InteractionSessionKind::Ordinary,
                true,
            )
            .await
        {
            Ok(_) => panic!("the second stop must continue from the first stop generation"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            DeviceControlError::StopProofMismatch {
                expected: 8,
                actual: 7,
                ..
            }
        ));
        assert_eq!(driver.session_starts.load(Ordering::SeqCst), 1);
        assert_eq!(driver.stream_starts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn agent_foreground_clipboard_replaces_the_session_and_stream_generation() {
        let driver = Arc::new(TestDriver::default());
        let control = control_plane(driver.clone(), 1);
        let mut context =
            streaming_context(&control, "iphone-a", crate::DeviceWorkOwner::Interaction).await;

        let value = control
            .get_streaming_clipboard(
                &mut context,
                "com.fixture.agent",
                "com.ss.iphone.ugc.Ame",
                InteractionSessionKind::Ordinary,
                ClipboardAccessMode::AgentForegroundRequired,
                65_536,
            )
            .await
            .expect("guarded clipboard transition");

        assert_eq!(
            value,
            ("plaintext".to_string(), b"fixture-clipboard".to_vec())
        );
        assert_eq!(context.start_proof.generation, 8);
        assert!(context.start_proof.first_frame_observed);
        assert_eq!(driver.guarded_clipboard_calls.load(Ordering::SeqCst), 1);
        driver.complete_stop();
        control
            .close_ui_context(context)
            .await
            .expect("replacement generation closes cleanly");
    }

    #[tokio::test]
    async fn background_safe_clipboard_keeps_the_current_session_generation() {
        let driver = Arc::new(TestDriver::default());
        let control = control_plane(driver.clone(), 1);
        let mut context =
            streaming_context(&control, "iphone-a", crate::DeviceWorkOwner::Interaction).await;

        control
            .set_streaming_clipboard(
                &mut context,
                "com.fixture.agent",
                "com.ss.iphone.ugc.Ame",
                InteractionSessionKind::Ordinary,
                ClipboardAccessMode::TargetBackgroundSafe,
                "plaintext".to_string(),
                b"fixture".to_vec(),
            )
            .await
            .expect("background-safe clipboard transition");

        assert_eq!(context.start_proof.generation, 7);
        assert_eq!(driver.guarded_clipboard_calls.load(Ordering::SeqCst), 1);
        driver.complete_stop();
        control
            .close_ui_context(context)
            .await
            .expect("unchanged generation closes cleanly");
    }

    #[tokio::test]
    async fn cancelling_agent_foreground_clipboard_finishes_transition_then_releases_capacity() {
        let driver = Arc::new(TestDriver::default());
        driver.block_guarded_clipboard_after_stop();
        let control = Arc::new(control_plane(driver.clone(), 1));
        let context =
            streaming_context(&control, "iphone-a", crate::DeviceWorkOwner::Interaction).await;
        let task_control = control.clone();

        let task = tokio::spawn(async move {
            let mut context = context;
            task_control
                .get_streaming_clipboard(
                    &mut context,
                    "com.fixture.agent",
                    "com.ss.iphone.ugc.Ame",
                    InteractionSessionKind::Ordinary,
                    ClipboardAccessMode::AgentForegroundRequired,
                    65_536,
                )
                .await
        });
        driver.wait_for_guarded_clipboard_stop().await;
        task.abort();
        let _ = task.await;

        driver.complete_guarded_clipboard();
        driver.wait_for_guarded_clipboard_completion().await;
        driver.wait_for_stop().await;
        assert!(!driver
            .guarded_clipboard_cleanup_started_early
            .load(Ordering::SeqCst));
        driver.complete_stop();
        let replacement = timeout(TEST_TIMEOUT, async {
            loop {
                match control
                    .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::Repair)
                    .await
                {
                    Ok(context) => break context,
                    Err(DeviceControlError::Busy(_)) => tokio::task::yield_now().await,
                    Err(error) => panic!("unexpected replacement acquire error: {error}"),
                }
            }
        })
        .await
        .expect("cancelled clipboard transition must release owner and capacity");

        assert_eq!(control.reserved_stream_capacity(), 0);
        assert_eq!(control.cleanup_quarantine_count(), 0);
        let (replacement, capacity) = control
            .reserve_ui_capacity(replacement)
            .await
            .expect("released stream capacity must be reservable by the next owner");
        assert_eq!(control.reserved_stream_capacity(), 1);
        drop(capacity);
        drop(replacement);
        assert_eq!(control.reserved_stream_capacity(), 0);
    }

    /// Cancelling the caller mid-recovery must not abort the in-flight stop.
    /// The stop runs on the cleanup worker, so after the caller is aborted the
    /// worker still finishes the stop and proceeds to start a fresh interaction
    /// session. The old in-caller stop ran in the caller's future, so aborting
    /// the caller dropped it and the session was never started.
    #[tokio::test]
    async fn cancelling_recovery_does_not_abort_the_worker_owned_stop() {
        let driver = Arc::new(TestDriver::default());
        let control = Arc::new(control_plane(driver.clone(), 1));
        let context =
            streaming_context(&control, "iphone-a", crate::DeviceWorkOwner::Nurture).await;
        let baseline_sessions = driver.session_starts.load(Ordering::SeqCst);
        let task_control = control.clone();

        let task = tokio::spawn(async move {
            let mut context = context;
            task_control
                .recover_streaming_session(
                    &mut context,
                    "com.ss.iphone.ugc.Ame",
                    InteractionSessionKind::Ordinary,
                    false,
                )
                .await
        });

        // The recovery's first stop runs in the worker and blocks (no permits).
        driver.wait_for_stop().await;
        // Cancel the caller while that stop is still in flight.
        task.abort();
        let _ = task.await;

        // Unblock the worker. It must finish the stop and go on to start a fresh
        // session — proving the destructive sequence outlived the caller.
        driver.allow_stop.add_permits(4);
        timeout(TEST_TIMEOUT, async {
            while driver.session_starts.load(Ordering::SeqCst) <= baseline_sessions {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the worker must finish the stop and start a fresh session after caller cancel");

        assert!(driver.session_starts.load(Ordering::SeqCst) > baseline_sessions);
    }

    #[tokio::test]
    async fn shared_device_owner_cancelling_explicit_close_retries_in_cleanup_worker() {
        let driver = Arc::new(TestDriver::default());
        let work = Arc::new(crate::DeviceWorkCoordinator::new());
        let streams = Arc::new(crate::StreamBudgetManager::new(1).expect("stream budget"));
        let control = Arc::new(DeviceControlPlane::new(
            driver.clone(),
            work.clone(),
            streams.clone(),
        ));
        let context =
            streaming_context(&control, "iphone-a", crate::DeviceWorkOwner::Interaction).await;
        let task_control = control.clone();

        let task = tokio::spawn(async move { task_control.close_ui_context(context).await });
        driver.wait_for_stop().await;
        task.abort();
        let _ = task.await;
        driver.complete_stop();

        let replacement = timeout(TEST_TIMEOUT, async {
            loop {
                match work.try_acquire("iphone-a", crate::DeviceWorkOwner::Repair) {
                    Ok(lease) => break lease,
                    Err(_) => tokio::task::yield_now().await,
                }
            }
        })
        .await
        .expect("cleanup worker should retry explicit close");
        assert_eq!(replacement.owner(), crate::DeviceWorkOwner::Repair);
        assert_eq!(
            driver.stop_calls.load(Ordering::SeqCst),
            1,
            "caller cancellation must not cancel and restart the owned stop"
        );
        assert_eq!(control.cleanup_quarantine_count(), 0);
        assert_eq!(streams.reserved_capacity(), 0);
    }

    #[tokio::test]
    async fn shared_device_owner_background_start_race_stops_before_foreground_handoff() {
        let driver = Arc::new(TestDriver::default());
        driver.block_background_stream_start();
        let control = Arc::new(control_plane(driver.clone(), 1));
        let background = control
            .reserve_background_stream("iphone-a")
            .expect("background reservation");
        let start_control = control.clone();
        let start =
            tokio::spawn(async move { start_control.start_background_stream(&background).await });
        driver.wait_for_background_stream_start().await;

        let acquire_control = control.clone();
        let mut acquire = tokio::spawn(async move {
            acquire_control
                .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::ManualControl)
                .await
        });
        assert!(
            timeout(QUICK_WAIT, &mut acquire).await.is_err(),
            "foreground handoff must queue behind an in-flight background start"
        );

        driver.complete_background_stream_start();
        driver.wait_for_stop_count(1).await;
        driver.complete_stop();

        let start_error = timeout(TEST_TIMEOUT, start)
            .await
            .expect("background start resolves")
            .expect("background start task")
            .expect_err("foreground owner won the raced handoff");
        assert!(matches!(
            start_error,
            DeviceControlError::BackgroundStreamBlocked {
                current_owner: crate::DeviceWorkOwner::ManualControl,
                ..
            }
        ));
        let foreground = timeout(TEST_TIMEOUT, acquire)
            .await
            .expect("foreground acquisition resolves after stop proof")
            .expect("foreground acquire task")
            .expect("foreground context");
        assert_eq!(driver.stop_calls.load(Ordering::SeqCst), 1);
        assert_eq!(control.reserved_stream_capacity(), 0);
        drop(foreground);
    }

    #[tokio::test]
    async fn shared_device_owner_shutdown_drains_in_flight_background_start() {
        let driver = Arc::new(TestDriver::default());
        driver.block_background_stream_start();
        let control = Arc::new(control_plane(driver.clone(), 1));
        let background = control
            .reserve_background_stream("iphone-a")
            .expect("background reservation");
        let start_control = control.clone();
        let start =
            tokio::spawn(async move { start_control.start_background_stream(&background).await });
        driver.wait_for_background_stream_start().await;

        let shutdown_control = control.clone();
        let mut shutdown = tokio::spawn(async move { shutdown_control.shutdown_cleanup().await });
        assert!(
            timeout(QUICK_WAIT, &mut shutdown).await.is_err(),
            "shutdown must wait for the in-flight start and its exact cleanup"
        );

        driver.complete_background_stream_start();
        driver.wait_for_stop_count(1).await;
        assert!(timeout(QUICK_WAIT, &mut shutdown).await.is_err());
        driver.complete_stop();
        start
            .await
            .expect("background start task")
            .expect("start completed before the shutdown barrier");
        timeout(TEST_TIMEOUT, shutdown)
            .await
            .expect("shutdown completes after background stop proof")
            .expect("shutdown task")
            .expect("clean shutdown");
        assert_eq!(control.reserved_stream_capacity(), 0);
    }

    #[tokio::test]
    async fn shared_device_owner_cancelled_background_start_still_stops_producer() {
        let driver = Arc::new(TestDriver::default());
        driver.block_background_stream_start();
        let control = Arc::new(control_plane(driver.clone(), 1));
        let background = control
            .reserve_background_stream("iphone-a")
            .expect("background reservation");
        let start_control = control.clone();
        let start =
            tokio::spawn(async move { start_control.start_background_stream(&background).await });
        driver.wait_for_background_stream_start().await;

        start.abort();
        let _ = start.await;
        driver.complete_background_stream_start();
        driver.wait_for_stop_count(1).await;
        assert_eq!(control.reserved_stream_capacity(), 1);
        driver.complete_stop();
        timeout(TEST_TIMEOUT, async {
            while control.reserved_stream_capacity() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("dropped start response queues exact producer cleanup");
        assert_eq!(control.cleanup_quarantine_count(), 0);
    }

    #[tokio::test]
    async fn shared_device_owner_shutdown_quarantines_unconfirmed_background_stop() {
        let driver = Arc::new(TestDriver::default());
        let control = Arc::new(control_plane(driver.clone(), 1));
        let background = control
            .reserve_background_stream("iphone-a")
            .expect("background reservation");
        control
            .start_background_stream(&background)
            .await
            .expect("background producer");
        driver.return_unconfirmed_stops();

        let shutdown_control = control.clone();
        let shutdown = tokio::spawn(async move { shutdown_control.shutdown_cleanup().await });
        driver.wait_for_stop_count(1).await;
        driver.complete_stop();
        let error = timeout(TEST_TIMEOUT, shutdown)
            .await
            .expect("shutdown remains bounded after failed proof")
            .expect("shutdown task")
            .expect_err("unconfirmed producer stop is fatal");

        assert!(matches!(
            error,
            DeviceControlError::CleanupQuarantined { count: 1 }
        ));
        assert_eq!(control.cleanup_quarantine_count(), 1);
        assert_eq!(control.reserved_stream_capacity(), 1);
    }

    #[tokio::test]
    async fn shared_device_owner_cancelling_stream_start_uses_exact_handoff_stop_proof() {
        let driver = Arc::new(TestDriver::default());
        driver.block_stream_start();
        let work = Arc::new(crate::DeviceWorkCoordinator::new());
        let streams = Arc::new(crate::StreamBudgetManager::new(1).expect("stream budget"));
        let control = Arc::new(DeviceControlPlane::new(
            driver.clone(),
            work.clone(),
            streams.clone(),
        ));
        let exclusive = control
            .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::Interaction)
            .await
            .expect("interaction lease");
        let (exclusive, capacity) = control
            .reserve_ui_capacity(exclusive)
            .await
            .expect("foreground capacity");
        let session = control
            .start_interaction_session(
                exclusive,
                "com.ss.iphone.ugc.Ame",
                InteractionSessionKind::Ordinary,
            )
            .await
            .expect("session");
        let task_control = control.clone();

        let task =
            tokio::spawn(
                async move { task_control.start_reserved_stream(session, capacity).await },
            );
        driver.wait_for_stream_start().await;
        task.abort();
        let _ = task.await;
        driver.wait_for_stop().await;
        driver.complete_stop();
        timeout(TEST_TIMEOUT, async {
            loop {
                match work.try_acquire("iphone-a", crate::DeviceWorkOwner::Repair) {
                    Ok(replacement) => break replacement,
                    Err(_) => tokio::task::yield_now().await,
                }
            }
        })
        .await
        .expect("handoff-qualified cleanup releases the cancelled stream start");

        assert_eq!(streams.reserved_capacity(), 0);
        assert_eq!(control.cleanup_quarantine_count(), 0);
    }

    #[tokio::test]
    async fn shared_device_owner_shutdown_blocks_old_context_and_waits_for_release() {
        let driver = Arc::new(TestDriver::default());
        let control = Arc::new(control_plane(driver.clone(), 1));
        let context = control
            .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::ManualControl)
            .await
            .expect("manual context");
        let shutdown_control = control.clone();
        let mut shutdown = tokio::spawn(async move { shutdown_control.shutdown_cleanup().await });

        assert!(timeout(QUICK_WAIT, &mut shutdown).await.is_err());
        let error = control
            .foreground_target_app(&context, "com.ss.iphone.ugc.Ame")
            .await
            .expect_err("old contexts cannot start transitions during shutdown");
        assert!(matches!(
            error,
            DeviceControlError::ControlPlaneShuttingDown
        ));
        assert_eq!(driver.launch_calls.load(Ordering::SeqCst), 0);

        drop(context);
        timeout(TEST_TIMEOUT, shutdown)
            .await
            .expect("shutdown waits for context release")
            .expect("shutdown task")
            .expect("clean shutdown");
    }

    #[tokio::test]
    async fn shared_device_owner_shutdown_waits_for_stream_stop_proof() {
        let driver = Arc::new(TestDriver::default());
        let streams = Arc::new(crate::StreamBudgetManager::new(1).expect("stream budget"));
        let control = Arc::new(DeviceControlPlane::new(
            driver.clone(),
            Arc::new(crate::DeviceWorkCoordinator::new()),
            streams.clone(),
        ));
        let context =
            streaming_context(&control, "iphone-a", crate::DeviceWorkOwner::Interaction).await;
        let shutdown_control = control.clone();
        let mut shutdown = tokio::spawn(async move { shutdown_control.shutdown_cleanup().await });
        assert!(timeout(QUICK_WAIT, &mut shutdown).await.is_err());

        drop(context);
        driver.wait_for_stop().await;
        assert!(timeout(QUICK_WAIT, &mut shutdown).await.is_err());
        assert_eq!(streams.reserved_capacity(), 1);

        driver.complete_stop();
        timeout(TEST_TIMEOUT, shutdown)
            .await
            .expect("shutdown waits for confirmed producer stop")
            .expect("shutdown task")
            .expect("clean shutdown");
        assert_eq!(streams.reserved_capacity(), 0);
        assert!(matches!(
            control
                .try_acquire_exclusive("iphone-a", crate::DeviceWorkOwner::Repair)
                .await,
            Err(DeviceControlError::ControlPlaneStopped)
        ));
    }

    #[tokio::test]
    async fn shared_device_owner_background_reserve_rejects_foreground_owners() {
        let driver = Arc::new(TestDriver::default());
        let control = control_plane(driver, 1);

        for owner in [
            crate::DeviceWorkOwner::ManualControl,
            crate::DeviceWorkOwner::Interaction,
        ] {
            let context = control
                .try_acquire_exclusive("iphone-a", owner)
                .await
                .expect("foreground context");
            let error = control
                .reserve_background_stream("iphone-a")
                .expect_err("sampler must not reserve under a foreground owner");
            assert!(matches!(
                error,
                DeviceControlError::BackgroundStreamBlocked {
                    current_owner,
                    ..
                } if current_owner == owner
            ));
            assert_eq!(control.reserved_stream_capacity(), 0);
            drop(context);
        }
    }

    #[tokio::test]
    async fn shared_device_owner_shutdown_waits_for_live_context_beside_quarantine() {
        let driver = Arc::new(TestDriver::default());
        let control = Arc::new(control_plane(driver.clone(), 1));
        let stream =
            streaming_context(&control, "iphone-a", crate::DeviceWorkOwner::Interaction).await;
        driver.set_stop_generation(6);
        drop(stream);
        driver.wait_for_stop().await;
        driver.complete_stop();
        timeout(TEST_TIMEOUT, async {
            while control.cleanup_quarantine_count() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("wrong generation enters quarantine");
        let live = control
            .try_acquire_exclusive("iphone-b", crate::DeviceWorkOwner::ManualControl)
            .await
            .expect("independent live context");
        let shutdown_control = control.clone();
        let mut shutdown = tokio::spawn(async move { shutdown_control.shutdown_cleanup().await });

        assert!(timeout(QUICK_WAIT, &mut shutdown).await.is_err());
        drop(live);
        let error = timeout(TEST_TIMEOUT, shutdown)
            .await
            .expect("shutdown finishes after live context drops")
            .expect("shutdown task")
            .expect_err("existing quarantine remains fatal");
        assert!(matches!(
            error,
            DeviceControlError::CleanupQuarantined { count: 1 }
        ));
    }

    #[tokio::test]
    async fn shared_device_owner_dropped_reserve_response_releases_capacity() {
        let streams = Arc::new(crate::StreamBudgetManager::new(1).expect("stream budget"));
        let work = Arc::new(crate::DeviceWorkCoordinator::new());
        let lifecycle = Arc::new(ControlPlaneLifecycle::default());
        let context = DeviceExclusiveContext {
            plane_id: Uuid::new_v4(),
            lease: Some(
                work.try_acquire("iphone-a", crate::DeviceWorkOwner::Interaction)
                    .expect("device lease"),
            ),
            activity: Some(lifecycle.register().expect("activity permit")),
            ui_capacity_token: None,
        };
        let transfer = streams
            .begin_foreground_transfer("iphone-a", crate::DeviceWorkOwner::Interaction)
            .expect("foreground transfer");
        let reservation = streams
            .complete_transfer(transfer, StreamStopProof::not_required())
            .expect("foreground reservation");

        drop(ReservedUiCapacity {
            context: Some(context),
            reservation: Some(reservation),
            streams: streams.clone(),
            quarantined: Arc::new(QuarantineStore::default()),
        });

        assert_eq!(streams.reserved_capacity(), 0);
        assert_eq!(lifecycle.outstanding(), 0);
    }

    #[tokio::test]
    async fn verified_termination_is_serialized_per_udid_but_not_across_devices() {
        let driver = Arc::new(TestDriver::default());
        driver.block_termination();
        let control = Arc::new(control_plane(driver.clone(), 1));
        assert_eq!(
            control.driver_contract_ids("fixture-udid"),
            std::collections::BTreeSet::from(["verifiedProcessControl".to_string()])
        );

        let legacy_exclusive = control
            .acquire_exclusive("iphone-a", DeviceWorkOwner::Script)
            .await
            .expect("legacy exclusive context");
        let legacy_session = control
            .start_owned_ui_session(legacy_exclusive)
            .await
            .expect("legacy session context");
        let legacy_control = control.clone();
        let legacy = tokio::spawn(async move {
            let proof = legacy_control
                .terminate_session_app(&legacy_session, "com.fixture.app")
                .await
                .expect("legacy terminate");
            legacy_control
                .close_session_context(legacy_session)
                .expect("close legacy session");
            proof
        });
        driver.wait_for_termination_count(1).await;

        let flow_control = control.clone();
        let same_device = tokio::spawn(async move {
            let context = flow_control
                .acquire_exclusive("iphone-a", DeviceWorkOwner::Script)
                .await
                .expect("queued Flow context");
            let state = flow_control
                .inspect_app_process(&context, "com.fixture.app")
                .await
                .expect("owned process inspection");
            assert_eq!(state.pid, Some(42));
            flow_control
                .terminate_app(&context, "com.fixture.app")
                .await
                .expect("Flow terminate")
        });

        let other_control = control.clone();
        let other_device = tokio::spawn(async move {
            let context = other_control
                .acquire_exclusive("iphone-b", DeviceWorkOwner::Script)
                .await
                .expect("independent Flow context");
            other_control
                .terminate_app(&context, "com.fixture.app")
                .await
                .expect("independent terminate")
        });

        driver.wait_for_termination_count(2).await;
        tokio::time::sleep(QUICK_WAIT).await;
        assert_eq!(driver.termination_count(), 2);
        assert_eq!(driver.termination_count_for("iphone-a"), 1);
        assert_eq!(driver.termination_count_for("iphone-b"), 1);

        driver.complete_terminations(2);
        driver.wait_for_termination_count(3).await;
        driver.complete_terminations(1);

        let legacy_proof = timeout(TEST_TIMEOUT, legacy)
            .await
            .expect("legacy terminate finishes")
            .expect("legacy task");
        assert_eq!(legacy_proof.old_pid, Some(42));
        timeout(TEST_TIMEOUT, same_device)
            .await
            .expect("same-device Flow terminate finishes")
            .expect("same-device task");
        timeout(TEST_TIMEOUT, other_device)
            .await
            .expect("other-device Flow terminate finishes")
            .expect("other-device task");

        control.shutdown_cleanup().await.expect("control cleanup");
    }
}
