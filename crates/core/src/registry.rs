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

    pub fn set_status(&self, udid: &str, status: DeviceStatus, error: Option<String>) {
        if let Some(mut device) = self.get(udid) {
            device.status = status;
            device.last_error = error;
            self.upsert(device);
        }
    }

    pub fn selected_ready(&self, udids: &[String]) -> Vec<DeviceInfo> {
        self.list()
            .into_iter()
            .filter(|d| udids.contains(&d.udid))
            .collect()
    }
}
