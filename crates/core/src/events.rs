use serde::Serialize;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::flow::FlowId;
use crate::types::{DeviceInfo, JobRecord, NurtureSessionStatus};

/// Everything the desktop pushes to the webview on `riviu://event`.
///
/// **`rename_all_fields` is load-bearing, and its absence was a live bug.** `rename_all` on an
/// enum renames the *variants* only; the fields of a struct variant keep their Rust spelling
/// unless `rename_all_fields` says otherwise. Every other payload in this app reaches the
/// frontend camelCase, because every other payload is a struct whose own `rename_all` does
/// cover its fields. So this enum was the one place on the wire sending `run_id`, `flow_id`
/// and `campaign_id` — and all three subscribers had been written against the camelCase the
/// rest of the app taught them to expect. They type-checked, compiled, and never fired.
///
/// The failure had no symptom of its own: `FlowRunMonitor` polls every 750 ms and looked
/// merely sluggish, `FlowWorkspace` refreshes on other paths, and `InteractionPopup` reloads
/// the whole list before it reads the field it never got. Which is why this is pinned by
/// `the_event_union_matches_the_variants_this_enum_sends` rather than left to a comment.
#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AppEvent {
    DevicesUpdated {
        devices: Vec<DeviceInfo>,
    },
    DeviceUpdated {
        device: DeviceInfo,
    },
    JobUpdated {
        job: JobRecord,
    },
    FlowUpdated {
        flow_id: FlowId,
        revision: u64,
    },
    FlowRunUpdated {
        run_id: Uuid,
        revision: u64,
    },
    InteractionUpdated {
        campaign_id: String,
        revision: u64,
    },
    WdaExpiryWarning {
        udid: String,
        days_remaining: i64,
    },
    /// Progress of one nurture session, once per status change.
    ///
    /// Was hand-rolled `serde_json::json!` in `nurture_commands.rs` and so invisible to
    /// anything that reasoned about this enum -- including the frontend's own event union.
    NurtureStatus {
        status: NurtureSessionStatus,
    },
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
