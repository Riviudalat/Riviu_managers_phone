use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use thiserror::Error;
use uuid::Uuid;

use crate::DeviceWorkOwner;

const DEFAULT_STREAM_LIMIT: usize = 1;
/// Hard ceiling on concurrent MJPEG producers on one desktop (AGENTS.md §3.5/
/// §3.12: default 1, hard max 2). The managed fleet may hold 20-100 phones, but
/// only this many ever stream at once — the desktop shows at most two tiles.
/// Accepting a larger configured limit would silently drop that guarantee.
const MAXIMUM_STREAM_LIMIT: usize = 2;
const BACKGROUND_TURN_TIMEOUT: Duration = Duration::from_secs(5);
const BACKGROUND_FAILURE_BACKOFF: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProducerState {
    BackgroundReserved,
    BackgroundRunning,
    Revoking,
    ForegroundReserved,
    ForegroundRunning,
    Stopping,
    FailedBackoff { until: Instant },
}

impl ProducerState {
    fn occupies_capacity(self) -> bool {
        !matches!(self, Self::FailedBackoff { .. })
    }

    fn name(self) -> &'static str {
        match self {
            Self::BackgroundReserved => "backgroundReserved",
            Self::BackgroundRunning => "backgroundRunning",
            Self::Revoking => "revoking",
            Self::ForegroundReserved => "foregroundReserved",
            Self::ForegroundRunning => "foregroundRunning",
            Self::Stopping => "stopping",
            Self::FailedBackoff { .. } => "failedBackoff",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StreamBudgetError {
    #[error("stream limit {requested} is invalid; supported range is 1..={maximum}")]
    InvalidLimit { requested: usize, maximum: usize },
    #[error("stream capacity is exhausted at configured limit {limit}")]
    CapacityExhausted { limit: usize },
    #[error("device {udid} already has stream state {state}")]
    AlreadyReserved { udid: String, state: &'static str },
    #[error("device {udid} is in stream failure backoff for {remaining:?}")]
    FailedBackoff { udid: String, remaining: Duration },
    #[error("stream token {token} is stale")]
    StaleToken { token: Uuid },
    #[error("cannot {operation} stream for {udid} while state is {state}")]
    InvalidTransition {
        udid: String,
        operation: &'static str,
        state: &'static str,
    },
    #[error("stream stop for {udid} was not confirmed")]
    StopNotConfirmed { udid: String },
    #[error("stream stop proof was supplied where no producer stop was required")]
    UnexpectedStopProof,
    #[error("foreground transfer {transfer_id} is stale")]
    StaleTransfer { transfer_id: Uuid },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamStopProof {
    pub old_generation: u64,
    pub new_generation: u64,
    pub child_stopped: bool,
}

impl StreamStopProof {
    pub fn confirmed() -> Self {
        Self {
            old_generation: 0,
            new_generation: 1,
            child_stopped: true,
        }
    }

    pub fn unconfirmed() -> Self {
        Self {
            old_generation: 0,
            new_generation: 0,
            child_stopped: false,
        }
    }

    pub fn not_required() -> Self {
        Self::unconfirmed()
    }

    fn confirms_stop(self) -> bool {
        self.child_stopped && self.new_generation > self.old_generation
    }

    fn is_not_required(self) -> bool {
        !self.child_stopped && self.new_generation == self.old_generation
    }
}

#[derive(Clone)]
pub struct StreamBudgetManager {
    inner: Arc<Mutex<BudgetState>>,
    configured_limit: usize,
    turn_timeout: Duration,
    failure_backoff: Duration,
}

impl Default for StreamBudgetManager {
    fn default() -> Self {
        Self::new(DEFAULT_STREAM_LIMIT).expect("the default stream limit is valid")
    }
}

impl StreamBudgetManager {
    pub fn new(configured_limit: usize) -> Result<Self, StreamBudgetError> {
        if !(1..=MAXIMUM_STREAM_LIMIT).contains(&configured_limit) {
            return Err(StreamBudgetError::InvalidLimit {
                requested: configured_limit,
                maximum: MAXIMUM_STREAM_LIMIT,
            });
        }
        Ok(Self {
            inner: Arc::new(Mutex::new(BudgetState::default())),
            configured_limit,
            turn_timeout: BACKGROUND_TURN_TIMEOUT,
            failure_backoff: BACKGROUND_FAILURE_BACKOFF,
        })
    }

    pub fn configured_limit(&self) -> usize {
        self.configured_limit
    }

    pub fn turn_timeout(&self) -> Duration {
        self.turn_timeout
    }

    pub fn failure_backoff(&self) -> Duration {
        self.failure_backoff
    }

    pub fn reserved_capacity(&self) -> usize {
        self.inner.lock().reserved_capacity()
    }

    pub fn running_producer_count(&self) -> usize {
        self.inner.lock().running_producer_count()
    }

    pub fn background_turn_due(&self, token: Uuid) -> Result<bool, StreamBudgetError> {
        self.background_turn_due_at(token, Instant::now())
    }

    pub fn mark_background_failed(&self, token: Uuid) -> Result<(), StreamBudgetError> {
        self.fail_start_at(token, Instant::now())
    }

    pub fn reserve_background(
        &self,
        udid: impl Into<String>,
    ) -> Result<BackgroundStreamLease, StreamBudgetError> {
        self.reserve_background_at(udid, Instant::now())
    }

    fn reserve_background_at(
        &self,
        udid: impl Into<String>,
        now: Instant,
    ) -> Result<BackgroundStreamLease, StreamBudgetError> {
        let udid = udid.into();
        let mut state = self.inner.lock();
        state.remove_expired_backoff(&udid, now);
        if let Some(record) = state.record_for_udid(&udid) {
            if let ProducerState::FailedBackoff { until } = record.state {
                return Err(StreamBudgetError::FailedBackoff {
                    udid,
                    remaining: until.saturating_duration_since(now),
                });
            }
            return Err(StreamBudgetError::AlreadyReserved {
                udid,
                state: record.state.name(),
            });
        }
        if state.reserved_capacity() >= self.configured_limit {
            return Err(StreamBudgetError::CapacityExhausted {
                limit: self.configured_limit,
            });
        }

        let token = Uuid::new_v4();
        let turn_deadline = now + self.turn_timeout;
        let sequence = state.next_sequence();
        state.insert(ProducerRecord {
            token,
            udid: udid.clone(),
            state: ProducerState::BackgroundReserved,
            producer_running: false,
            turn_deadline: Some(turn_deadline),
            sequence,
            pending_transfer: None,
        });
        Ok(BackgroundStreamLease {
            udid,
            token,
            turn_deadline,
        })
    }

    pub fn mark_running(&self, token: Uuid) -> Result<(), StreamBudgetError> {
        self.mark_running_at(token, Instant::now())
    }

    fn mark_running_at(&self, token: Uuid, now: Instant) -> Result<(), StreamBudgetError> {
        let mut state = self.inner.lock();
        let record = state.record_mut(token)?;
        record.state = match record.state {
            ProducerState::BackgroundReserved => {
                // Reservation may cover slow agent/session bootstrap. Start the
                // bounded background turn when the producer is actually live.
                record.turn_deadline = Some(now + self.turn_timeout);
                ProducerState::BackgroundRunning
            }
            ProducerState::ForegroundReserved => ProducerState::ForegroundRunning,
            current => {
                return Err(StreamBudgetError::InvalidTransition {
                    udid: record.udid.clone(),
                    operation: "start",
                    state: current.name(),
                })
            }
        };
        record.producer_running = true;
        Ok(())
    }

    pub fn begin_foreground_transfer(
        &self,
        udid: impl Into<String>,
        owner: DeviceWorkOwner,
    ) -> Result<ForegroundTransfer, StreamBudgetError> {
        let udid = udid.into();
        let mut state = self.inner.lock();
        state.remove_expired_backoff(&udid, Instant::now());
        let background_backoff = state.by_udid.get(&udid).copied().filter(|token| {
            state
                .records
                .get(token)
                .is_some_and(|record| matches!(record.state, ProducerState::FailedBackoff { .. }))
        });
        if let Some(token) = background_backoff {
            state.remove(token);
        }

        let target_record = state
            .by_udid
            .get(&udid)
            .and_then(|token| state.records.get(token));
        if let Some(record) = target_record {
            if !matches!(
                record.state,
                ProducerState::BackgroundReserved | ProducerState::BackgroundRunning
            ) {
                return Err(StreamBudgetError::AlreadyReserved {
                    udid,
                    state: record.state.name(),
                });
            }
        }

        let target_background = state.by_udid.get(&udid).copied().filter(|token| {
            state.records.get(token).is_some_and(|record| {
                matches!(
                    record.state,
                    ProducerState::BackgroundReserved | ProducerState::BackgroundRunning
                )
            })
        });
        let victim_token = if target_background.is_some() {
            target_background
        } else if state.reserved_capacity() >= self.configured_limit {
            state.oldest_background_token()
        } else {
            None
        };

        if victim_token.is_none() && state.reserved_capacity() >= self.configured_limit {
            return Err(StreamBudgetError::CapacityExhausted {
                limit: self.configured_limit,
            });
        }

        let transfer_id = Uuid::new_v4();
        if let Some(token) = victim_token {
            let victim = state
                .records
                .get_mut(&token)
                .expect("victim token came from the current state");
            let revoked_udid = victim.udid.clone();
            let stop_required = victim.producer_running;
            victim.state = ProducerState::Revoking;
            victim.pending_transfer = Some(PendingTransfer {
                transfer_id,
                target_udid: udid.clone(),
                target_owner: owner,
            });
            return Ok(ForegroundTransfer {
                transfer_id,
                slot_token: token,
                target_udid: udid,
                target_owner: owner,
                revoked_udid: Some(revoked_udid),
                stop_required,
            });
        }

        let token = Uuid::new_v4();
        let sequence = state.next_sequence();
        state.insert(ProducerRecord {
            token,
            udid: udid.clone(),
            state: ProducerState::ForegroundReserved,
            producer_running: false,
            turn_deadline: None,
            sequence,
            pending_transfer: Some(PendingTransfer {
                transfer_id,
                target_udid: udid.clone(),
                target_owner: owner,
            }),
        });
        Ok(ForegroundTransfer {
            transfer_id,
            slot_token: token,
            target_udid: udid,
            target_owner: owner,
            revoked_udid: None,
            stop_required: false,
        })
    }

    pub(crate) fn preview_foreground_victim(
        &self,
        udid: &str,
    ) -> Result<Option<String>, StreamBudgetError> {
        let state = self.inner.lock();
        let target_record = state
            .by_udid
            .get(udid)
            .and_then(|token| state.records.get(token));
        if let Some(record) = target_record {
            if matches!(
                record.state,
                ProducerState::BackgroundReserved | ProducerState::BackgroundRunning
            ) {
                return Ok(Some(record.udid.clone()));
            }
            if !matches!(record.state, ProducerState::FailedBackoff { .. }) {
                return Err(StreamBudgetError::AlreadyReserved {
                    udid: udid.to_string(),
                    state: record.state.name(),
                });
            }
        }

        if state.reserved_capacity() < self.configured_limit {
            return Ok(None);
        }
        let victim = state
            .oldest_background_token()
            .and_then(|token| state.records.get(&token))
            .map(|record| record.udid.clone());
        victim
            .ok_or(StreamBudgetError::CapacityExhausted {
                limit: self.configured_limit,
            })
            .map(Some)
    }

    pub fn complete_transfer(
        &self,
        transfer: ForegroundTransfer,
        proof: StreamStopProof,
    ) -> Result<ForegroundStreamReservation, StreamBudgetError> {
        let mut state = self.inner.lock();
        let record =
            state
                .records
                .get(&transfer.slot_token)
                .ok_or(StreamBudgetError::StaleTransfer {
                    transfer_id: transfer.transfer_id,
                })?;
        let pending = record
            .pending_transfer
            .as_ref()
            .filter(|pending| pending.transfer_id == transfer.transfer_id)
            .ok_or(StreamBudgetError::StaleTransfer {
                transfer_id: transfer.transfer_id,
            })?;
        if pending.target_udid != transfer.target_udid
            || pending.target_owner != transfer.target_owner
        {
            return Err(StreamBudgetError::StaleTransfer {
                transfer_id: transfer.transfer_id,
            });
        }
        if transfer.stop_required && !proof.confirms_stop() {
            return Err(StreamBudgetError::StopNotConfirmed {
                udid: transfer
                    .revoked_udid
                    .clone()
                    .unwrap_or_else(|| transfer.target_udid.clone()),
            });
        }
        if !transfer.stop_required && !proof.is_not_required() {
            return Err(StreamBudgetError::UnexpectedStopProof);
        }

        let old = state
            .remove(transfer.slot_token)
            .expect("the transfer record was validated above");
        let new_token = Uuid::new_v4();
        state.insert(ProducerRecord {
            token: new_token,
            udid: transfer.target_udid.clone(),
            state: ProducerState::ForegroundReserved,
            producer_running: false,
            turn_deadline: None,
            sequence: old.sequence,
            pending_transfer: None,
        });
        Ok(ForegroundStreamReservation {
            udid: transfer.target_udid,
            owner: transfer.target_owner,
            token: new_token,
        })
    }

    pub fn begin_stop(&self, token: Uuid) -> Result<StreamStopRequest, StreamBudgetError> {
        let mut state = self.inner.lock();
        let record = state.record_mut(token)?;
        if record.state == ProducerState::Stopping && record.producer_running {
            return Ok(StreamStopRequest {
                udid: record.udid.clone(),
                token,
            });
        }
        if !matches!(
            record.state,
            ProducerState::BackgroundRunning
                | ProducerState::ForegroundRunning
                | ProducerState::Revoking
        ) || !record.producer_running
        {
            return Err(StreamBudgetError::InvalidTransition {
                udid: record.udid.clone(),
                operation: "stop",
                state: record.state.name(),
            });
        }
        record.state = ProducerState::Stopping;
        record.pending_transfer = None;
        Ok(StreamStopRequest {
            udid: record.udid.clone(),
            token,
        })
    }

    pub fn complete_stop(
        &self,
        request: StreamStopRequest,
        proof: StreamStopProof,
    ) -> Result<(), StreamBudgetError> {
        let mut state = self.inner.lock();
        let record = state.record(request.token)?;
        if record.udid != request.udid || record.state != ProducerState::Stopping {
            return Err(StreamBudgetError::InvalidTransition {
                udid: record.udid.clone(),
                operation: "completeStop",
                state: record.state.name(),
            });
        }
        if !proof.confirms_stop() {
            return Err(StreamBudgetError::StopNotConfirmed {
                udid: record.udid.clone(),
            });
        }
        state.remove(request.token);
        Ok(())
    }

    pub fn release_reserved(&self, token: Uuid) -> Result<(), StreamBudgetError> {
        let mut state = self.inner.lock();
        let record = state.record(token)?;
        if !matches!(
            record.state,
            ProducerState::BackgroundReserved | ProducerState::ForegroundReserved
        ) || record.pending_transfer.is_some()
        {
            return Err(StreamBudgetError::InvalidTransition {
                udid: record.udid.clone(),
                operation: "release",
                state: record.state.name(),
            });
        }
        state.remove(token);
        Ok(())
    }

    pub fn reservation_udid(&self, token: Uuid) -> Option<String> {
        self.inner
            .lock()
            .records
            .get(&token)
            .filter(|record| record.state.occupies_capacity())
            .map(|record| record.udid.clone())
    }

    fn background_turn_due_at(&self, token: Uuid, now: Instant) -> Result<bool, StreamBudgetError> {
        let state = self.inner.lock();
        let record = state.record(token)?;
        if record.state != ProducerState::BackgroundRunning {
            return Err(StreamBudgetError::InvalidTransition {
                udid: record.udid.clone(),
                operation: "checkTurnDeadline",
                state: record.state.name(),
            });
        }
        Ok(record.turn_deadline.is_some_and(|deadline| now >= deadline))
    }

    fn fail_start_at(&self, token: Uuid, now: Instant) -> Result<(), StreamBudgetError> {
        let mut state = self.inner.lock();
        let record = state.record_mut(token)?;
        if !matches!(record.state, ProducerState::BackgroundReserved) {
            return Err(StreamBudgetError::InvalidTransition {
                udid: record.udid.clone(),
                operation: "failStart",
                state: record.state.name(),
            });
        }
        record.state = ProducerState::FailedBackoff {
            until: now + self.failure_backoff,
        };
        record.producer_running = false;
        record.turn_deadline = None;
        Ok(())
    }

    pub fn release_reserved_by_udid(&self, udid: &str) -> Result<(), StreamBudgetError> {
        let token = self
            .inner
            .lock()
            .by_udid
            .get(udid)
            .copied()
            .ok_or(StreamBudgetError::StaleToken { token: Uuid::nil() })?;
        self.release_reserved(token)
    }

    pub fn reservation_token(&self, udid: &str) -> Option<Uuid> {
        let state = self.inner.lock();
        state.by_udid.get(udid).copied().filter(|token| {
            state
                .records
                .get(token)
                .is_some_and(|record| record.state.occupies_capacity())
        })
    }

    #[cfg(test)]
    fn invariant_snapshot(&self) -> StreamBudgetInvariantSnapshot {
        let state = self.inner.lock();
        let active: Vec<_> = state
            .records
            .values()
            .filter(|record| record.state.occupies_capacity())
            .collect();
        StreamBudgetInvariantSnapshot {
            configured_limit: self.configured_limit,
            reserved_capacity: active.len(),
            running_producers: active
                .iter()
                .filter(|record| record.producer_running)
                .count(),
            active_udids: active.iter().map(|record| record.udid.clone()).collect(),
            active_tokens: active.iter().map(|record| record.token).collect(),
        }
    }
}

#[derive(Debug)]
pub struct BackgroundStreamLease {
    udid: String,
    token: Uuid,
    turn_deadline: Instant,
}

impl BackgroundStreamLease {
    pub fn udid(&self) -> &str {
        &self.udid
    }

    pub fn token(&self) -> Uuid {
        self.token
    }

    pub fn turn_deadline(&self) -> Instant {
        self.turn_deadline
    }
}

#[derive(Debug)]
pub struct ForegroundTransfer {
    transfer_id: Uuid,
    slot_token: Uuid,
    target_udid: String,
    target_owner: DeviceWorkOwner,
    revoked_udid: Option<String>,
    stop_required: bool,
}

impl ForegroundTransfer {
    pub fn revoked_udid(&self) -> Option<&str> {
        self.revoked_udid.as_deref()
    }

    pub fn requires_stop_proof(&self) -> bool {
        self.stop_required
    }
}

#[derive(Debug)]
pub struct ForegroundStreamReservation {
    udid: String,
    owner: DeviceWorkOwner,
    token: Uuid,
}

impl ForegroundStreamReservation {
    pub fn udid(&self) -> &str {
        &self.udid
    }

    pub fn owner(&self) -> DeviceWorkOwner {
        self.owner
    }

    pub fn token(&self) -> Uuid {
        self.token
    }
}

#[derive(Debug)]
pub struct StreamStopRequest {
    udid: String,
    token: Uuid,
}

impl StreamStopRequest {
    pub fn udid(&self) -> &str {
        &self.udid
    }

    pub fn token(&self) -> Uuid {
        self.token
    }
}

#[derive(Default)]
struct BudgetState {
    records: HashMap<Uuid, ProducerRecord>,
    by_udid: HashMap<String, Uuid>,
    sequence: u64,
}

impl BudgetState {
    fn next_sequence(&mut self) -> u64 {
        let sequence = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);
        sequence
    }

    fn insert(&mut self, record: ProducerRecord) {
        self.by_udid.insert(record.udid.clone(), record.token);
        self.records.insert(record.token, record);
    }

    fn remove(&mut self, token: Uuid) -> Option<ProducerRecord> {
        let record = self.records.remove(&token)?;
        if self.by_udid.get(&record.udid) == Some(&token) {
            self.by_udid.remove(&record.udid);
        }
        Some(record)
    }

    fn record(&self, token: Uuid) -> Result<&ProducerRecord, StreamBudgetError> {
        self.records
            .get(&token)
            .ok_or(StreamBudgetError::StaleToken { token })
    }

    fn record_mut(&mut self, token: Uuid) -> Result<&mut ProducerRecord, StreamBudgetError> {
        self.records
            .get_mut(&token)
            .ok_or(StreamBudgetError::StaleToken { token })
    }

    fn record_for_udid(&self, udid: &str) -> Option<&ProducerRecord> {
        self.by_udid
            .get(udid)
            .and_then(|token| self.records.get(token))
    }

    fn reserved_capacity(&self) -> usize {
        self.records
            .values()
            .filter(|record| record.state.occupies_capacity())
            .count()
    }

    fn running_producer_count(&self) -> usize {
        self.records
            .values()
            .filter(|record| record.producer_running)
            .count()
    }

    fn oldest_background_token(&self) -> Option<Uuid> {
        self.records
            .values()
            .filter(|record| {
                matches!(
                    record.state,
                    ProducerState::BackgroundReserved | ProducerState::BackgroundRunning
                )
            })
            .min_by_key(|record| record.sequence)
            .map(|record| record.token)
    }

    fn remove_expired_backoff(&mut self, udid: &str, now: Instant) {
        let expired = self.by_udid.get(udid).copied().filter(|token| {
            self.records.get(token).is_some_and(|record| {
                matches!(record.state, ProducerState::FailedBackoff { until } if now >= until)
            })
        });
        if let Some(token) = expired {
            self.remove(token);
        }
    }
}

struct ProducerRecord {
    token: Uuid,
    udid: String,
    state: ProducerState,
    producer_running: bool,
    turn_deadline: Option<Instant>,
    sequence: u64,
    pending_transfer: Option<PendingTransfer>,
}

struct PendingTransfer {
    transfer_id: Uuid,
    target_udid: String,
    target_owner: DeviceWorkOwner,
}

#[cfg(test)]
struct StreamBudgetInvariantSnapshot {
    configured_limit: usize,
    reserved_capacity: usize,
    running_producers: usize,
    active_udids: Vec<String>,
    active_tokens: Vec<Uuid>,
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::time::{Duration, Instant};

    use rand::{rngs::StdRng, Rng, SeedableRng};

    use super::*;
    use crate::DeviceWorkOwner;

    #[test]
    fn defaults_to_one_and_rejects_limits_above_the_hard_max_of_two() {
        let default_budget = StreamBudgetManager::default();
        assert_eq!(default_budget.configured_limit(), 1);
        assert_eq!(default_budget.turn_timeout(), Duration::from_secs(5));
        assert_eq!(default_budget.failure_backoff(), Duration::from_secs(30));
        assert_eq!(StreamBudgetManager::new(2).unwrap().configured_limit(), 2);
        assert!(matches!(
            StreamBudgetManager::new(0),
            Err(StreamBudgetError::InvalidLimit {
                requested: 0,
                maximum: 2
            })
        ));
        // The hard max is 2 (AGENTS.md §3.5/§3.12): three concurrent producers
        // must be rejected, not silently accepted.
        assert!(matches!(
            StreamBudgetManager::new(3),
            Err(StreamBudgetError::InvalidLimit {
                requested: 3,
                maximum: 2
            })
        ));
    }

    #[tokio::test]
    async fn foreground_retags_budget_one_background_without_double_producer() {
        let budget = StreamBudgetManager::new(1).unwrap();
        let bg = budget.reserve_background("tile-a").expect("background");
        budget.mark_running(bg.token()).unwrap();

        let transfer = budget
            .begin_foreground_transfer("tile-b", DeviceWorkOwner::Interaction)
            .expect("revocation decision");
        assert_eq!(budget.running_producer_count(), 1);
        assert_eq!(transfer.revoked_udid(), Some("tile-a"));
        assert!(matches!(
            budget.mark_running(bg.token()),
            Err(StreamBudgetError::InvalidTransition { .. })
        ));

        let fg = budget
            .complete_transfer(transfer, StreamStopProof::confirmed())
            .unwrap();
        assert_ne!(fg.token(), bg.token());
        assert_eq!(budget.reserved_capacity(), 1);
        assert_eq!(budget.running_producer_count(), 0);
        budget.mark_running(fg.token()).unwrap();
        assert_eq!(budget.running_producer_count(), 1);
    }

    #[test]
    fn failed_transfer_stop_keeps_capacity_occupied_and_fails_closed() {
        let budget = StreamBudgetManager::new(1).unwrap();
        let bg = budget.reserve_background("tile-a").unwrap();
        budget.mark_running(bg.token()).unwrap();
        let transfer = budget
            .begin_foreground_transfer("tile-b", DeviceWorkOwner::Interaction)
            .unwrap();

        assert!(matches!(
            budget.complete_transfer(transfer, StreamStopProof::unconfirmed()),
            Err(StreamBudgetError::StopNotConfirmed { .. })
        ));
        assert_eq!(budget.reserved_capacity(), 1);
        assert_eq!(budget.running_producer_count(), 1);
        assert!(matches!(
            budget.reserve_background("tile-c"),
            Err(StreamBudgetError::CapacityExhausted { limit: 1 })
        ));
    }

    #[test]
    fn background_turn_rotates_after_five_seconds() {
        let budget = StreamBudgetManager::new(1).unwrap();
        let start = Instant::now();
        let first = budget.reserve_background_at("tile-a", start).unwrap();
        assert_eq!(first.turn_deadline(), start + Duration::from_secs(5));
        let running_at = start + Duration::from_secs(10);
        budget.mark_running_at(first.token(), running_at).unwrap();
        assert!(!budget
            .background_turn_due_at(first.token(), running_at + Duration::from_secs(4))
            .unwrap());
        assert!(budget
            .background_turn_due_at(first.token(), running_at + Duration::from_secs(5))
            .unwrap());

        let stop = budget.begin_stop(first.token()).unwrap();
        budget
            .complete_stop(stop, StreamStopProof::confirmed())
            .unwrap();
        assert_eq!(budget.reserved_capacity(), 0);
        assert!(budget
            .reserve_background_at("tile-b", start + Duration::from_secs(5))
            .is_ok());
    }

    #[test]
    fn failed_background_has_thirty_second_backoff_without_hoarding_capacity() {
        let budget = StreamBudgetManager::new(1).unwrap();
        let start = Instant::now();
        let failed = budget.reserve_background_at("tile-a", start).unwrap();
        budget.fail_start_at(failed.token(), start).unwrap();
        assert_eq!(budget.reserved_capacity(), 0);
        assert!(matches!(
            budget.reserve_background_at("tile-a", start + Duration::from_secs(29)),
            Err(StreamBudgetError::FailedBackoff { udid, .. }) if udid == "tile-a"
        ));
        assert!(budget
            .reserve_background_at("tile-b", start + Duration::from_secs(1))
            .is_ok());
        budget.release_reserved_by_udid("tile-b").unwrap();
        assert!(budget
            .reserve_background_at("tile-a", start + Duration::from_secs(30))
            .is_ok());
    }

    #[test]
    fn foreground_preempts_only_background_and_never_another_foreground() {
        let budget = StreamBudgetManager::new(2).unwrap();
        let background = budget.reserve_background("tile-a").unwrap();
        budget.mark_running(background.token()).unwrap();
        let free = budget
            .begin_foreground_transfer("tile-b", DeviceWorkOwner::Nurture)
            .unwrap();
        assert_eq!(free.revoked_udid(), None);
        let first_foreground = budget
            .complete_transfer(free, StreamStopProof::not_required())
            .unwrap();
        budget.mark_running(first_foreground.token()).unwrap();

        let preempt = budget
            .begin_foreground_transfer("tile-c", DeviceWorkOwner::Script)
            .unwrap();
        assert_eq!(preempt.revoked_udid(), Some("tile-a"));
        let second_foreground = budget
            .complete_transfer(preempt, StreamStopProof::confirmed())
            .unwrap();
        budget.mark_running(second_foreground.token()).unwrap();
        assert!(matches!(
            budget.begin_foreground_transfer("tile-d", DeviceWorkOwner::Interaction),
            Err(StreamBudgetError::CapacityExhausted { limit: 2 })
        ));
    }

    #[test]
    fn foreground_demand_supersedes_background_failure_backoff() {
        let budget = StreamBudgetManager::new(1).unwrap();
        let start = Instant::now();
        let failed = budget.reserve_background_at("tile-a", start).unwrap();
        budget.fail_start_at(failed.token(), start).unwrap();

        let transfer = budget
            .begin_foreground_transfer("tile-a", DeviceWorkOwner::Interaction)
            .expect("background backoff must not block foreground work");
        assert_eq!(transfer.revoked_udid(), None);
        let foreground = budget
            .complete_transfer(transfer, StreamStopProof::not_required())
            .unwrap();
        assert_eq!(foreground.udid(), "tile-a");
    }

    #[test]
    fn stale_tokens_cannot_mutate_current_capacity() {
        let budget = StreamBudgetManager::new(1).unwrap();
        let background = budget.reserve_background("tile-a").unwrap();
        budget.mark_running(background.token()).unwrap();
        let transfer = budget
            .begin_foreground_transfer("tile-b", DeviceWorkOwner::Interaction)
            .unwrap();
        let foreground = budget
            .complete_transfer(transfer, StreamStopProof::confirmed())
            .unwrap();

        assert!(matches!(
            budget.mark_running(background.token()),
            Err(StreamBudgetError::StaleToken { .. })
        ));
        assert!(matches!(
            budget.begin_stop(background.token()),
            Err(StreamBudgetError::StaleToken { .. })
        ));
        assert!(matches!(
            budget.release_reserved(background.token()),
            Err(StreamBudgetError::StaleToken { .. })
        ));
        assert_eq!(
            budget.reservation_udid(foreground.token()),
            Some("tile-b".into())
        );
    }

    #[test]
    fn cleanup_requires_stop_proof_for_running_producers() {
        let budget = StreamBudgetManager::new(1).unwrap();
        let reserved = budget.reserve_background("tile-a").unwrap();
        budget.release_reserved(reserved.token()).unwrap();
        assert_eq!(budget.reserved_capacity(), 0);

        let running = budget.reserve_background("tile-b").unwrap();
        budget.mark_running(running.token()).unwrap();
        assert!(matches!(
            budget.release_reserved(running.token()),
            Err(StreamBudgetError::InvalidTransition { .. })
        ));
        let stop = budget.begin_stop(running.token()).unwrap();
        assert!(matches!(
            budget.complete_stop(stop, StreamStopProof::unconfirmed()),
            Err(StreamBudgetError::StopNotConfirmed { .. })
        ));
        assert_eq!(budget.reserved_capacity(), 1);

        let retry = budget
            .begin_stop(running.token())
            .expect("failed proof must leave cleanup retryable");
        budget
            .complete_stop(retry, StreamStopProof::confirmed())
            .unwrap();
        assert_eq!(budget.reserved_capacity(), 0);
    }

    #[test]
    fn generated_operation_sequences_preserve_invariants() {
        for seed in 0..64_u64 {
            let budget = StreamBudgetManager::new(2).unwrap();
            let mut rng = StdRng::seed_from_u64(seed);
            for step in 0..256 {
                let udid = format!("tile-{}", rng.gen_range(0..5));
                match rng.gen_range(0..5) {
                    0 => {
                        let _ = budget.reserve_background(&udid);
                    }
                    1 => {
                        if let Some(token) = budget.reservation_token(&udid) {
                            let _ = budget.mark_running(token);
                        }
                    }
                    2 => {
                        if let Some(token) = budget.reservation_token(&udid) {
                            if let Ok(stop) = budget.begin_stop(token) {
                                let _ = budget.complete_stop(stop, StreamStopProof::confirmed());
                            }
                        }
                    }
                    3 => {
                        let _ = budget.release_reserved_by_udid(&udid);
                    }
                    _ => {
                        if let Ok(transfer) = budget.begin_foreground_transfer(
                            format!("foreground-{seed}-{step}"),
                            DeviceWorkOwner::Interaction,
                        ) {
                            let proof = if transfer.revoked_udid().is_some() {
                                StreamStopProof::confirmed()
                            } else {
                                StreamStopProof::not_required()
                            };
                            let _ = budget.complete_transfer(transfer, proof);
                        }
                    }
                }

                let snapshot = budget.invariant_snapshot();
                assert!(snapshot.running_producers <= snapshot.reserved_capacity);
                assert!(snapshot.reserved_capacity <= snapshot.configured_limit);
                assert_eq!(
                    snapshot.active_udids.iter().collect::<HashSet<_>>().len(),
                    snapshot.active_udids.len()
                );
                assert_eq!(
                    snapshot.active_tokens.iter().collect::<HashSet<_>>().len(),
                    snapshot.active_tokens.len()
                );
            }
        }
    }
}
