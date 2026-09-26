use crate::command_error::CommandError;
use anyhow::Context;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{Local, NaiveDateTime};
use riviu_core::db::Database;
use riviu_core::DeviceControlPlane;
use riviu_core::{
    copy_bundle_to_managed, scan_publish_folder, DeviceWorkOwner, PublishCampaignDetail,
    PublishCampaignRecord, PublishCampaignRequest, PublishCleanupPolicy, PublishFolderManifest,
    PublishScanOptions, PublishVisibility,
};
use riviu_core::{FrameSource, InteractionSessionKind, TapPoint};
use tauri::State;
use uuid::Uuid;

use crate::state::AppState;

mod execution;
pub(crate) mod pipeline;
mod verification;
mod verification_queue;
mod verification_restart;
mod verification_session;
pub(crate) use verification::verify_pending_assignment;
pub use verification::*;
pub(crate) use verification_queue::VerificationQueue;
mod verified_cleanup;
/// Batched read for the device picker. No phone access and no state mutation.
#[tauri::command]
pub async fn publish_device_guards(
    state: State<'_, AppState>,
    udids: Vec<String>,
) -> Result<HashMap<String, riviu_core::db::PublishDeviceGuard>, CommandError> {
    if udids.len() > 500 || udids.iter().any(|id| id.is_empty() || id.len() > 256) {
        return Err(CommandError::from(
            "Danh sách kiểm tra máy không hợp lệ".to_owned(),
        ));
    }
    let udids = udids.into_iter().collect::<std::collections::BTreeSet<_>>();
    state
        .db
        .storage_read(move |db| {
            udids
                .into_iter()
                .map(|udid| db.publish_device_guard(&udid).map(|guard| (udid, guard)))
                .collect()
        })
        .await
        .map_err(preflight::err)
}

pub use verified_cleanup::*;
mod progress;
mod retry;
mod schedule;
pub use retry::*;
pub use schedule::*;
pub(crate) mod preflight;
mod preview;
mod sheet;
// Unregistered stepwise entry points and their historical calibration.
#[allow(dead_code)]
mod legacy;

pub use execution::*;
pub use preflight::*;
pub use preview::*;
pub use sheet::*;

#[cfg(test)]
const PRODUCTION_SOURCES: &str = concat!(
    include_str!("mod.rs"),
    include_str!("preflight.rs"),
    include_str!("preview.rs"),
    include_str!("execution.rs"),
    include_str!("schedule.rs"),
    include_str!("sheet.rs"),
    include_str!("legacy.rs"),
);

#[cfg(test)]
mod tests;

#[cfg(test)]
mod live_canary;
