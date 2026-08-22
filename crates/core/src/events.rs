use serde::Serialize;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::flow::FlowId;
use crate::types::{DeviceInfo, JobRecord};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AppEvent {
    DevicesUpdated { devices: Vec<DeviceInfo> },
    DeviceUpdated { device: DeviceInfo },
    JobUpdated { job: JobRecord },
    FlowUpdated { flow_id: FlowId, revision: u64 },
    FlowRunUpdated { run_id: Uuid, revision: u64 },
    InteractionUpdated { campaign_id: String, revision: u64 },
    WdaExpiryWarning { udid: String, days_remaining: i64 },
}

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<AppEvent>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.tx.subscribe()
    }

    pub fn emit(&self, event: AppEvent) {
        let _ = self.tx.send(event);
    }
}
