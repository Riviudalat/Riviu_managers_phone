use std::sync::Arc;

use parking_lot::RwLock;

use crate::events::{AppEvent, EventBus};
use crate::types::{DeviceInfo, DeviceStatus};

#[derive(Clone)]
pub struct DeviceRegistry {
    devices: Arc<RwLock<Vec<DeviceInfo>>>,
    events: EventBus,
}

impl DeviceRegistry {
    pub fn new(events: EventBus) -> Self {
        Self {
            devices: Arc::new(RwLock::new(Vec::new())),
            events,
        }
    }

    pub fn list(&self) -> Vec<DeviceInfo> {
        self.devices.read().clone()
    }

    pub fn get(&self, udid: &str) -> Option<DeviceInfo> {
        self.devices.read().iter().find(|d| d.udid == udid).cloned()
    }

    pub fn upsert_many(&self, devices: Vec<DeviceInfo>) {
        {
            let mut guard = self.devices.write();
            *guard = devices.clone();
        }
        self.events.emit(AppEvent::DevicesUpdated { devices });
    }

    pub fn upsert(&self, device: DeviceInfo) {
        {
            let mut guard = self.devices.write();
            if let Some(existing) = guard.iter_mut().find(|d| d.udid == device.udid) {
                *existing = device.clone();
            } else {
                guard.push(device.clone());
            }
        }
        self.events.emit(AppEvent::DeviceUpdated {
            device: device.clone(),
        });
        self.events.emit(AppEvent::DevicesUpdated {
            devices: self.list(),
        });
    }

    /// Change one phone's status, and **say so when there is no such phone.**
    ///
    /// Two defects lived in the four lines this replaces, and both were silent.
    ///
    /// **It did nothing for a udid it could not find, with no `else`, no log and no return
    /// value.** That case is reachable on any busy fleet: `upsert_many` replaces the *whole*
    /// roster on every rescan, so a driver task that finishes just after a scan is holding a
    /// udid the roster no longer has. The update vanished, and the phone kept showing whatever
    /// it showed before -- `ready` after it had errored, or `error` after it had recovered.
    /// That is the shape of "the app lists the phone but cannot drive it", read from the wrong
    /// end.
    ///
    /// **And the read-modify-write was not atomic.** It called `get()` (read lock, released),
    /// mutated the clone, then `upsert()` (write lock). Anything happening in between -- another
    /// `set_status`, or a whole-roster swap -- was overwritten by a value computed before it.
    /// Now the find and the mutation happen under one write lock, and only the announcement
    /// happens outside it, because emitting under a lock is how a different bug starts.
    ///
    /// Returns whether it applied. Callers that ignore it are no worse off than before; callers
    /// that care can now ask.
    pub fn set_status(&self, udid: &str, status: DeviceStatus, error: Option<String>) -> bool {
        // Kept for the log line before the value moves into the mutation below.
        let wanted = format!("{status:?}");
        let updated = {
            let mut guard = self.devices.write();
            guard
                .iter_mut()
                .find(|device| device.udid == udid)
                .map(|device| {
                    device.status = status;
                    device.last_error = error;
                    device.clone()
                })
        };
        let Some(device) = updated else {
            tracing::warn!(
                "dropped a status update for {udid}: not in the roster (status {wanted}). \
                 A rescan replaces the whole roster, so a driver task finishing just after one \
                 can hold a udid that is briefly gone."
            );
            return false;
        };
        self.events.emit(AppEvent::DeviceUpdated {
            device: device.clone(),
        });
        self.events.emit(AppEvent::DevicesUpdated {
            devices: self.list(),
        });
        true
    }

    pub fn selected_ready(&self, udids: &[String]) -> Vec<DeviceInfo> {
        self.list()
            .into_iter()
            .filter(|d| udids.contains(&d.udid))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ConnectionKind, DeviceInfo, DevicePlatform, DeviceStatus, TileStreamState};

    fn phone(udid: &str) -> DeviceInfo {
        DeviceInfo {
            udid: udid.to_string(),
            name: format!("phone {udid}"),
            model: "SM-G955F".into(),
            platform: DevicePlatform::Android,
            os_version: "9".into(),
            connection: ConnectionKind::Usb,
            status: DeviceStatus::Ready,
            battery: None,
            wda_ready: false,
            wda_expires_at: None,
            stream_url: None,
            tile_stream_state: TileStreamState::Parked,
            last_error: None,
        }
    }

    fn registry() -> DeviceRegistry {
        DeviceRegistry::new(EventBus::new(64))
    }

    #[test]
    fn a_status_change_lands_and_is_announced_twice() {
        let registry = registry();
        registry.upsert_many(vec![phone("a"), phone("b")]);
        let mut events = registry.events.subscribe();

        assert!(registry.set_status("a", DeviceStatus::Error, Some("adb offline".into())));

        let stored = registry.get("a").expect("still in the roster");
        assert_eq!(stored.status, DeviceStatus::Error);
        assert_eq!(stored.last_error.as_deref(), Some("adb offline"));
        // One for the phone, one for the roster: the grid listens to the second.
        assert!(matches!(
            events.try_recv(),
            Ok(AppEvent::DeviceUpdated { .. })
        ));
        assert!(matches!(
            events.try_recv(),
            Ok(AppEvent::DevicesUpdated { .. })
        ));
    }

    /// **The update that used to vanish.**
    ///
    /// No `else`, no log, no return value -- so a status for a phone the roster no longer holds
    /// was simply gone, and the phone kept showing the status it had before.
    #[test]
    fn a_status_for_a_phone_that_is_not_in_the_roster_is_reported_as_a_miss() {
        let registry = registry();
        registry.upsert_many(vec![phone("a")]);

        assert!(
            !registry.set_status("gone", DeviceStatus::Error, None),
            "a udid that is not there has to answer no"
        );
        assert!(registry.get("gone").is_none(), "and must not be invented");
    }

    /// The reachable version of the same thing: a rescan drops a phone, and a driver task that
    /// was already running finishes afterwards holding its udid.
    #[test]
    fn a_rescan_that_drops_a_phone_makes_a_late_status_a_miss_rather_than_a_ghost() {
        let registry = registry();
        registry.upsert_many(vec![phone("a"), phone("b")]);
        // The scan sees only one phone this time -- a cable, or a blip.
        registry.upsert_many(vec![phone("a")]);

        assert!(!registry.set_status("b", DeviceStatus::Error, Some("late".into())));
        assert_eq!(registry.list().len(), 1, "the roster is what the scan said");
    }

    /// Both fields move together. They used to be written to a clone that a concurrent writer
    /// could overwrite whole; now they are written under the lock that finds the row.
    #[test]
    fn a_status_change_clears_a_stale_error_as_well_as_setting_one() {
        let registry = registry();
        registry.upsert_many(vec![phone("a")]);

        registry.set_status("a", DeviceStatus::Error, Some("unauthorized".into()));
        registry.set_status("a", DeviceStatus::Ready, None);

        let stored = registry.get("a").expect("present");
        assert_eq!(stored.status, DeviceStatus::Ready);
        assert_eq!(
            stored.last_error, None,
            "a recovered phone must not keep the sentence that explained its failure"
        );
    }

    #[test]
    fn upsert_replaces_a_known_phone_and_appends_an_unknown_one() {
        let registry = registry();
        registry.upsert_many(vec![phone("a")]);

        let mut renamed = phone("a");
        renamed.name = "renamed".into();
        registry.upsert(renamed);
        assert_eq!(registry.list().len(), 1);
        assert_eq!(registry.get("a").unwrap().name, "renamed");

        registry.upsert(phone("b"));
        assert_eq!(registry.list().len(), 2);
    }
}
