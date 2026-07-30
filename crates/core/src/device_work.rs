use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceWorkOwner {
    Nurture,
    Interaction,
    Script,
    Repair,
    ManualControl,
    GroupSync,
}

impl DeviceWorkOwner {
    fn may_wait(self) -> bool {
        matches!(self, Self::Nurture | Self::Script)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
#[serde(rename_all = "camelCase")]
#[error("device {udid} is busy with {current_owner:?}; {requested_owner:?} cannot acquire it")]
pub struct DeviceBusy {
    pub udid: String,
    pub requested_owner: DeviceWorkOwner,
    pub current_owner: DeviceWorkOwner,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DeviceWorkAcquireError {
    #[error("{requested_owner:?} cannot wait for device {udid}")]
    WaitNotAllowed {
        udid: String,
        #[serde(rename = "requestedOwner")]
        requested_owner: DeviceWorkOwner,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DeviceWorkTokenError {
    #[error("token {token} is not the current work token for device {udid}")]
    NotCurrent {
        udid: String,
        token: Uuid,
        #[serde(rename = "currentOwner")]
        current_owner: Option<DeviceWorkOwner>,
    },
}

#[derive(Clone, Default)]
pub struct DeviceWorkCoordinator {
    state: Arc<CoordinatorState>,
}

impl DeviceWorkCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn try_acquire(
        &self,
        udid: &str,
        owner: DeviceWorkOwner,
    ) -> Result<DeviceWorkLease, DeviceBusy> {
        let udid = udid.to_string();
        let device = self.state.device(&udid);
        let mut metadata = device.metadata.lock();

        if let Some(current_owner) = metadata.busy_owner() {
            return Err(DeviceBusy {
                udid,
                requested_owner: owner,
                current_owner,
            });
        }

        let permit = device
            .semaphore
            .clone()
            .try_acquire_owned()
            .expect("device semaphore and ownership metadata must agree");
        let token = Uuid::new_v4();
        metadata.current = Some(CurrentWork { owner, token });
        drop(metadata);

        Ok(DeviceWorkLease {
            udid,
            owner,
            token,
            _permit: Some(permit),
            state: self.state.clone(),
        })
    }

    pub async fn acquire(
        &self,
        udid: &str,
        owner: DeviceWorkOwner,
    ) -> Result<DeviceWorkLease, DeviceWorkAcquireError> {
        if !owner.may_wait() {
            return Err(DeviceWorkAcquireError::WaitNotAllowed {
                udid: udid.to_string(),
                requested_owner: owner,
            });
        }

        let udid = udid.to_string();
        let device = self.state.device(&udid);
        let waiter_id = Uuid::new_v4();

        {
            let mut metadata = device.metadata.lock();
            if metadata.current.is_none() && metadata.waiters.is_empty() {
                if let Ok(permit) = device.semaphore.clone().try_acquire_owned() {
                    let token = Uuid::new_v4();
                    metadata.current = Some(CurrentWork { owner, token });
                    drop(metadata);
                    return Ok(DeviceWorkLease {
                        udid,
                        owner,
                        token,
                        _permit: Some(permit),
                        state: self.state.clone(),
                    });
                }
            }
            metadata.waiters.push_back(WaitingWork {
                id: waiter_id,
                owner,
            });
        }

        let mut registration = WaitRegistration {
            udid: udid.clone(),
            waiter_id,
            state: self.state.clone(),
            armed: true,
        };
        let permit = {
            // This inner scope ensures semaphore cancellation completes before
            // the registration guard removes the waiter metadata.
            let acquire = device.semaphore.clone().acquire_owned();
            acquire
                .await
                .expect("device work semaphores are never closed")
        };

        let token = Uuid::new_v4();
        let mut metadata = device.metadata.lock();
        metadata.remove_waiter(waiter_id);
        metadata.current = Some(CurrentWork { owner, token });
        registration.armed = false;
        drop(metadata);

        Ok(DeviceWorkLease {
            udid,
            owner,
            token,
            _permit: Some(permit),
            state: self.state.clone(),
        })
    }

    pub fn validate_token(
        &self,
        udid: &str,
        token: Uuid,
    ) -> Result<DeviceWorkOwner, DeviceWorkTokenError> {
        let current = self
            .state
            .existing_device(udid)
            .and_then(|device| device.metadata.lock().current);

        match current {
            Some(work) if work.token == token => Ok(work.owner),
            _ => Err(DeviceWorkTokenError::NotCurrent {
                udid: udid.to_string(),
                token,
                current_owner: current.map(|work| work.owner),
            }),
        }
    }

    pub fn current_owner(&self, udid: &str) -> Option<DeviceWorkOwner> {
        self.state
            .existing_device(udid)
            .and_then(|device| device.metadata.lock().busy_owner())
    }

    pub(crate) fn with_idle_device<T>(
        &self,
        udid: &str,
        operation: impl FnOnce() -> T,
    ) -> Result<T, DeviceWorkOwner> {
        let device = self.state.device(udid);
        let metadata = device.metadata.lock();
        if let Some(owner) = metadata.busy_owner() {
            return Err(owner);
        }
        let result = operation();
        drop(metadata);
        Ok(result)
    }
}

pub struct DeviceWorkLease {
    udid: String,
    owner: DeviceWorkOwner,
    token: Uuid,
    _permit: Option<OwnedSemaphorePermit>,
    state: Arc<CoordinatorState>,
}

impl DeviceWorkLease {
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

impl fmt::Debug for DeviceWorkLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceWorkLease")
            .field("udid", &self.udid)
            .field("owner", &self.owner)
            .field("token", &self.token)
            .finish_non_exhaustive()
    }
}

impl Drop for DeviceWorkLease {
    fn drop(&mut self) {
        let Some(permit) = self._permit.take() else {
            return;
        };
        let Some(device) = self.state.existing_device(&self.udid) else {
            drop(permit);
            return;
        };

        let mut metadata = device.metadata.lock();
        if metadata.current.map(|work| work.token) == Some(self.token) {
            metadata.current = None;
        }
        drop(permit);
    }
}

#[derive(Default)]
struct CoordinatorState {
    devices: Mutex<HashMap<String, Arc<PerDeviceState>>>,
}

impl CoordinatorState {
    fn device(&self, udid: &str) -> Arc<PerDeviceState> {
        let mut devices = self.devices.lock();
        devices
            .entry(udid.to_string())
            .or_insert_with(|| Arc::new(PerDeviceState::new()))
            .clone()
    }

    fn existing_device(&self, udid: &str) -> Option<Arc<PerDeviceState>> {
        self.devices.lock().get(udid).cloned()
    }
}

struct PerDeviceState {
    semaphore: Arc<Semaphore>,
    metadata: Mutex<DeviceMetadata>,
}

impl PerDeviceState {
    fn new() -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(1)),
            metadata: Mutex::new(DeviceMetadata::default()),
        }
    }
}

#[derive(Default)]
struct DeviceMetadata {
    current: Option<CurrentWork>,
    waiters: VecDeque<WaitingWork>,
}

impl DeviceMetadata {
    fn busy_owner(&self) -> Option<DeviceWorkOwner> {
        self.current
            .map(|work| work.owner)
            .or_else(|| self.waiters.front().map(|work| work.owner))
    }

    fn remove_waiter(&mut self, waiter_id: Uuid) {
        if let Some(index) = self
            .waiters
            .iter()
            .position(|waiter| waiter.id == waiter_id)
        {
            self.waiters.remove(index);
        }
    }
}

#[derive(Clone, Copy)]
struct CurrentWork {
    owner: DeviceWorkOwner,
    token: Uuid,
}

struct WaitingWork {
    id: Uuid,
    owner: DeviceWorkOwner,
}

struct WaitRegistration {
    udid: String,
    waiter_id: Uuid,
    state: Arc<CoordinatorState>,
    armed: bool,
}

impl Drop for WaitRegistration {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Some(device) = self.state.existing_device(&self.udid) {
            device.metadata.lock().remove_waiter(self.waiter_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::time::{timeout, Duration};

    const QUICK_WAIT: Duration = Duration::from_millis(25);
    const TEST_TIMEOUT: Duration = Duration::from_secs(1);

    #[test]
    fn every_screen_changing_owner_can_own_a_device() {
        let coordinator = DeviceWorkCoordinator::new();

        for owner in [
            DeviceWorkOwner::Nurture,
            DeviceWorkOwner::Interaction,
            DeviceWorkOwner::Script,
            DeviceWorkOwner::Repair,
            DeviceWorkOwner::ManualControl,
            DeviceWorkOwner::GroupSync,
        ] {
            let lease = coordinator
                .try_acquire("iphone-a", owner)
                .expect("device should be available");
            assert_eq!(lease.udid(), "iphone-a");
            assert_eq!(lease.owner(), owner);
            assert_eq!(
                coordinator.validate_token("iphone-a", lease.token()),
                Ok(owner)
            );
            drop(lease);
        }
    }

    #[tokio::test]
    async fn interaction_excludes_every_screen_changing_owner() {
        let coordinator = DeviceWorkCoordinator::new();
        let lease = coordinator
            .try_acquire("iphone-a", DeviceWorkOwner::Interaction)
            .expect("interaction lease");

        for owner in [
            DeviceWorkOwner::Nurture,
            DeviceWorkOwner::Script,
            DeviceWorkOwner::Repair,
            DeviceWorkOwner::ManualControl,
            DeviceWorkOwner::GroupSync,
        ] {
            let busy = coordinator.try_acquire("iphone-a", owner).unwrap_err();
            assert_eq!(busy.current_owner, DeviceWorkOwner::Interaction);
        }
        assert!(coordinator
            .try_acquire("iphone-b", DeviceWorkOwner::ManualControl)
            .is_ok());
        drop(lease);
    }

    #[test]
    fn try_acquire_returns_typed_busy_without_waiting() {
        let coordinator = DeviceWorkCoordinator::new();
        let _lease = coordinator
            .try_acquire("iphone-a", DeviceWorkOwner::Repair)
            .expect("repair lease");

        let busy = coordinator
            .try_acquire("iphone-a", DeviceWorkOwner::Interaction)
            .unwrap_err();

        assert_eq!(
            busy,
            DeviceBusy {
                udid: "iphone-a".to_string(),
                requested_owner: DeviceWorkOwner::Interaction,
                current_owner: DeviceWorkOwner::Repair,
            }
        );
    }

    #[tokio::test]
    async fn nurture_and_script_wait_in_fifo_order() {
        let coordinator = DeviceWorkCoordinator::new();
        let blocker = coordinator
            .try_acquire("iphone-a", DeviceWorkOwner::Interaction)
            .expect("interaction lease");

        let mut script = Box::pin(coordinator.acquire("iphone-a", DeviceWorkOwner::Script));
        assert!(timeout(QUICK_WAIT, script.as_mut()).await.is_err());

        let mut nurture = Box::pin(coordinator.acquire("iphone-a", DeviceWorkOwner::Nurture));
        assert!(timeout(QUICK_WAIT, nurture.as_mut()).await.is_err());

        drop(blocker);
        let script_lease = timeout(TEST_TIMEOUT, script.as_mut())
            .await
            .expect("script should be first")
            .expect("script may queue");
        assert_eq!(script_lease.owner(), DeviceWorkOwner::Script);
        assert!(timeout(QUICK_WAIT, nurture.as_mut()).await.is_err());

        drop(script_lease);
        let nurture_lease = timeout(TEST_TIMEOUT, nurture.as_mut())
            .await
            .expect("nurture should be second")
            .expect("nurture may queue");
        assert_eq!(nurture_lease.owner(), DeviceWorkOwner::Nurture);
    }

    #[tokio::test]
    async fn wait_api_rejects_owners_that_must_be_non_blocking() {
        let coordinator = DeviceWorkCoordinator::new();

        for owner in [
            DeviceWorkOwner::Interaction,
            DeviceWorkOwner::Repair,
            DeviceWorkOwner::ManualControl,
            DeviceWorkOwner::GroupSync,
        ] {
            let error = coordinator
                .acquire("iphone-a", owner)
                .await
                .expect_err("owner must use try_acquire");
            assert_eq!(
                error,
                DeviceWorkAcquireError::WaitNotAllowed {
                    udid: "iphone-a".to_string(),
                    requested_owner: owner,
                }
            );
        }
    }

    #[test]
    fn tokens_are_unique_current_and_scoped_to_the_udid() {
        let coordinator = DeviceWorkCoordinator::new();
        let first = coordinator
            .try_acquire("iphone-a", DeviceWorkOwner::Interaction)
            .expect("first lease");
        let other = coordinator
            .try_acquire("iphone-b", DeviceWorkOwner::Script)
            .expect("independent lease");
        let first_token = first.token();

        assert_eq!(
            coordinator.validate_token("iphone-a", first_token),
            Ok(DeviceWorkOwner::Interaction)
        );
        assert!(matches!(
            coordinator.validate_token("iphone-a", other.token()),
            Err(DeviceWorkTokenError::NotCurrent {
                current_owner: Some(DeviceWorkOwner::Interaction),
                ..
            })
        ));
        assert!(matches!(
            coordinator.validate_token("iphone-b", first_token),
            Err(DeviceWorkTokenError::NotCurrent {
                current_owner: Some(DeviceWorkOwner::Script),
                ..
            })
        ));

        drop(first);
        assert!(matches!(
            coordinator.validate_token("iphone-a", first_token),
            Err(DeviceWorkTokenError::NotCurrent {
                current_owner: None,
                ..
            })
        ));
        let second = coordinator
            .try_acquire("iphone-a", DeviceWorkOwner::Nurture)
            .expect("replacement lease");
        assert_ne!(second.token(), first_token);
        assert!(matches!(
            coordinator.validate_token("iphone-a", first_token),
            Err(DeviceWorkTokenError::NotCurrent {
                current_owner: Some(DeviceWorkOwner::Nurture),
                ..
            })
        ));
        assert_eq!(
            coordinator.validate_token("iphone-a", second.token()),
            Ok(DeviceWorkOwner::Nurture)
        );
    }

    #[tokio::test]
    async fn cancelling_a_waiter_releases_its_place_for_the_next_waiter() {
        let coordinator = DeviceWorkCoordinator::new();
        let blocker = coordinator
            .try_acquire("iphone-a", DeviceWorkOwner::Interaction)
            .expect("interaction lease");

        let mut cancelled = Box::pin(coordinator.acquire("iphone-a", DeviceWorkOwner::Script));
        assert!(timeout(QUICK_WAIT, cancelled.as_mut()).await.is_err());
        drop(cancelled);

        let mut next = Box::pin(coordinator.acquire("iphone-a", DeviceWorkOwner::Nurture));
        assert!(timeout(QUICK_WAIT, next.as_mut()).await.is_err());
        drop(blocker);

        let lease = timeout(TEST_TIMEOUT, next.as_mut())
            .await
            .expect("next waiter must not remain blocked")
            .expect("nurture may queue");
        assert_eq!(lease.owner(), DeviceWorkOwner::Nurture);
        assert_eq!(
            coordinator.validate_token("iphone-a", lease.token()),
            Ok(DeviceWorkOwner::Nurture)
        );
    }

    #[tokio::test]
    async fn waiting_on_one_udid_does_not_block_another_udid() {
        let coordinator = DeviceWorkCoordinator::new();
        let _blocker = coordinator
            .try_acquire("iphone-a", DeviceWorkOwner::Interaction)
            .expect("interaction lease");
        let mut waiting = Box::pin(coordinator.acquire("iphone-a", DeviceWorkOwner::Script));
        assert!(timeout(QUICK_WAIT, waiting.as_mut()).await.is_err());

        let independent = timeout(
            TEST_TIMEOUT,
            coordinator.acquire("iphone-b", DeviceWorkOwner::Nurture),
        )
        .await
        .expect("other UDID must not wait")
        .expect("nurture may queue");
        assert_eq!(independent.udid(), "iphone-b");
    }

    #[test]
    fn public_serialized_names_are_camel_case() {
        assert_eq!(
            serde_json::to_value(DeviceWorkOwner::ManualControl).unwrap(),
            json!("manualControl")
        );
        assert_eq!(
            serde_json::to_value(DeviceBusy {
                udid: "iphone-a".to_string(),
                requested_owner: DeviceWorkOwner::GroupSync,
                current_owner: DeviceWorkOwner::ManualControl,
            })
            .unwrap(),
            json!({
                "udid": "iphone-a",
                "requestedOwner": "groupSync",
                "currentOwner": "manualControl",
            })
        );
        assert_eq!(
            serde_json::to_value(DeviceWorkAcquireError::WaitNotAllowed {
                udid: "iphone-a".to_string(),
                requested_owner: DeviceWorkOwner::GroupSync,
            })
            .unwrap(),
            json!({
                "kind": "waitNotAllowed",
                "udid": "iphone-a",
                "requestedOwner": "groupSync",
            })
        );
    }

    #[test]
    fn current_owner_reports_the_front_waiter_during_handoff() {
        let coordinator = DeviceWorkCoordinator::new();
        let device = coordinator.state.device("iphone-a");
        device.metadata.lock().waiters.push_back(WaitingWork {
            id: Uuid::new_v4(),
            owner: DeviceWorkOwner::Nurture,
        });

        assert_eq!(
            coordinator.current_owner("iphone-a"),
            Some(DeviceWorkOwner::Nurture)
        );
    }
}
