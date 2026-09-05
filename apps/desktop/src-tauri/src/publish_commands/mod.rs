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
mod preflight;
mod sheet;
// Unregistered stepwise entry points and their historical calibration.
#[allow(dead_code)]
mod legacy;

pub use execution::*;
pub use preflight::*;
pub use sheet::*;

#[cfg(test)]
const PRODUCTION_SOURCES: &str = concat!(
    include_str!("mod.rs"),
    include_str!("preflight.rs"),
    include_str!("execution.rs"),
    include_str!("sheet.rs"),
    include_str!("legacy.rs"),
);

#[cfg(test)]
mod tests;
