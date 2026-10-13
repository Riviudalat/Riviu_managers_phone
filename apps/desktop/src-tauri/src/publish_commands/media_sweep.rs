//! Sweeping orphaned publish media off idle phones.
//!
//! `verified_cleanup.rs` fulfils the media obligation of a *verified* post. It never sees
//! an assignment that failed before Post, was cancelled or missed, and those keep their
//! `Pictures/riviu-<scope>-<hash>` album and `Pictures/.riviu-publish/<scope>` staging
//! directory forever. Measured 14/10/2026 on the 30-phone fleet after the a62c6b72 wave:
//! 12–30 such entries per phone, every campaign set to delete after verification, and
//! TikTok's picker timing out while reading an album list that long.
//!
//! This sweep lists what the backend itself created, keeps every name whose staging
//! scope still has a living assignment (anything before Post, posting, verifying,
//! uncertain, or succeeded without a cleanup proof) and deletes the rest, a bounded
//! number per visit, each with an intent row before and a receipt row after the effect.
//! It runs under the same rules as the popup sweeper: `IdleSweep` takes the exclusive
//! lease without waiting, keeps the preview stream, and skips a phone the moment any
//! publish, nurture, interaction, Flow or job work is pending on it. It never opens a
//! UI session and never touches a name it cannot parse as its own.
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use riviu_core::db::{
    publish_album_scope, Database, PublishMediaSweepClaim, PublishMediaSweepKind,
};
use riviu_core::{
    DeviceControlPlane, DevicePlatform, DeviceRegistry, DeviceStatus, DeviceWorkOwner,
};

use crate::state::CommandAdmissionState;

/// A phone is looked at again this long after its last visit. Each visit costs two `ls`
/// round trips plus one `rm -rf` and readback per deletion; ten minutes keeps that off
/// the USB bus the previews stream over while still clearing a night's leftovers before
/// the next one starts.
const VISIT_INTERVAL: Duration = Duration::from_secs(10 * 60);
/// How often the fleet is looked at for phones due a visit.
const TICK: Duration = Duration::from_secs(60);
/// Phones visited per tick. Two, not the fleet: a sweep that walked thirty phones at once
/// would compete with itself for adb the way AGENTS.md §9.67 measured.
const PHONES_PER_TICK: usize = 2;
/// Directories deleted per visit. A phone with thirty leftovers is clean after four
/// visits, and a visit never holds the lease for more than a few seconds.
pub(crate) const DELETIONS_PER_VISIT: usize = 8;
/// A name whose deletion failed, or whose claim never got a receipt, is left alone this
/// long before the sweep tries it again.
const FAILURE_BACKOFF: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct SweepOutcome {
    /// Why the phone was not touched this visit, if it was not.
    pub skipped: Option<String>,
    pub albums_listed: usize,
    pub stages_listed: usize,
    pub deleted: usize,
    pub failed: usize,
    /// Orphans left for a later visit because of the per-visit cap or backoff.
    pub remaining: usize,
}

/// The background worker. Spawned once from `AppState::bootstrap`.
pub(crate) struct OrphanMediaSweep {
    control: Arc<DeviceControlPlane>,
    db: Arc<Database>,
    registry: DeviceRegistry,
    admission: Arc<CommandAdmissionState>,
    stop: Arc<AtomicBool>,
    last_visit: HashMap<String, Instant>,
}

impl OrphanMediaSweep {
    pub(crate) fn spawn(
        control: Arc<DeviceControlPlane>,
        db: Arc<Database>,
        registry: DeviceRegistry,
        admission: Arc<CommandAdmissionState>,
        stop: Arc<AtomicBool>,
    ) {
        let mut sweep = Self {
            control,
            db,
            registry,
            admission,
            stop,
            last_visit: HashMap::new(),
        };
        tauri::async_runtime::spawn(async move {
            let mut ticker = tokio::time::interval(TICK);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                if sweep.stop.load(Ordering::Acquire) {
                    break;
                }
                sweep.tick().await;
            }
        });
    }

    async fn tick(&mut self) {
        if riviu_core::interaction_campaign::any_campaign_running() {
            return;
        }
        let now = Instant::now();
        let mut due: Vec<String> = self
            .registry
            .list()
            .into_iter()
            .filter(|device| {
                device.platform == DevicePlatform::Android
                    && matches!(device.status, DeviceStatus::Ready | DeviceStatus::Connected)
            })
            .map(|device| device.udid)
            .filter(|udid| {
                self.last_visit
                    .get(udid)
                    .is_none_or(|last| now.duration_since(*last) >= VISIT_INTERVAL)
            })
            .collect();
        self.last_visit
            .retain(|_, last| now.duration_since(*last) < 2 * VISIT_INTERVAL);
        due.sort();
        for udid in due.into_iter().take(PHONES_PER_TICK) {
            if self.stop.load(Ordering::Acquire) {
                return;
            }
            let Ok(_admitted) = self.admission.ensure_accepting_work() else {
                return;
            };
            self.last_visit.insert(udid.clone(), Instant::now());
            match sweep_orphan_publish_media(&self.control, &self.db, &udid, DELETIONS_PER_VISIT)
                .await
            {
                Ok(outcome) if outcome.skipped.is_some() => {}
                Ok(outcome) if outcome.deleted == 0 && outcome.failed == 0 => {
                    log::debug!(
                        "dọn media sót {udid}: không có thư mục thừa ({} album, {} staging còn chủ)",
                        outcome.albums_listed,
                        outcome.stages_listed
                    );
                }
                Ok(outcome) => log::info!(
                    "dọn media sót {udid}: đã xoá {} thư mục, lỗi {}, còn lại {} cho lượt sau",
                    outcome.deleted,
                    outcome.failed,
                    outcome.remaining
                ),
                Err(error) => log::warn!("dọn media sót {udid}: {error:#}"),
            }
        }
    }
}

/// One visit to one phone. Returns `skipped` without touching the phone when it is not
/// idle or its lease is held; errors only when the inventory itself could not be read.
pub(crate) async fn sweep_orphan_publish_media(
    control: &DeviceControlPlane,
    db: &Database,
    udid: &str,
    cap: usize,
) -> anyhow::Result<SweepOutcome> {
    let skipped = |reason: &str| SweepOutcome {
        skipped: Some(reason.to_owned()),
        ..SweepOutcome::default()
    };
    if control.idle_work_deferred(udid) {
        return Ok(skipped("máy đang được bàn giao"));
    }
    if let Some(reason) = db.app_completion_block_reason(udid)? {
        return Ok(skipped(&reason));
    }
    let Some(_permit) = db.try_publish_work(udid, "cleanup", "orphan-media-sweep")? else {
        return Ok(skipped("máy còn lượt đăng đang giữ chỗ"));
    };
    let context = match control
        .try_acquire_exclusive_keeping_stream(udid, DeviceWorkOwner::IdleSweep)
        .await
    {
        Ok(context) => context,
        Err(error) => return Ok(skipped(&format!("máy đang bận: {error}"))),
    };
    let outcome: anyhow::Result<SweepOutcome> = async {
        // The lease makes the pending check, the listing and every deletion one device
        // operation: no publishing worker can stage a new scope in between.
        if db.has_pending_publish_for_device(udid)? {
            return Ok(skipped("máy còn bài đang tải hoặc chờ xác minh"));
        }
        let leftovers = control.list_publish_media_leftovers(&context).await?;
        let live = db.live_publish_media_scopes(udid)?;
        let since = chrono::Utc::now().timestamp_millis()
            - i64::try_from(FAILURE_BACKOFF.as_millis()).unwrap_or(i64::MAX);
        let backoff = db.publish_media_sweep_backoff(udid, since)?;
        let orphans = orphan_names(&leftovers, &live, &backoff);
        let mut outcome = SweepOutcome {
            albums_listed: leftovers.albums.len(),
            stages_listed: leftovers.stages.len(),
            remaining: orphans.len(),
            ..SweepOutcome::default()
        };
        for (kind, name) in orphans.into_iter().take(cap) {
            outcome.remaining -= 1;
            let claim = db.claim_publish_media_sweep(udid, kind, &name)?;
            let result = match kind {
                PublishMediaSweepKind::Album => {
                    control.cleanup_publish_media(&context, &name).await
                }
                PublishMediaSweepKind::Stage => {
                    control.cleanup_publish_stage(&context, &name).await
                }
            };
            let (cleaned, proof) = sweep_receipt(&claim, result.map_err(|error| error.to_string()));
            db.finish_publish_media_sweep(&claim, cleaned, &proof)?;
            if cleaned {
                outcome.deleted += 1;
            } else {
                outcome.failed += 1;
                log::warn!(
                    "dọn media sót {udid}: chưa xoá được {} {name}: {}",
                    kind.as_str(),
                    proof["message"].as_str().unwrap_or("không rõ")
                );
            }
        }
        Ok(outcome)
    }
    .await;
    // Preview and warm app are left exactly as found; only the lease is released.
    let released = control.close_exclusive_context(context);
    match (outcome, released) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error.into()),
        (Ok(outcome), Ok(_)) => Ok(outcome),
    }
}

/// Which listed names may go: every album and stage whose scope has no living
/// assignment and is not backed off. Albums first, because they are what the picker
/// reads; the order within a kind is the backend's sorted listing.
pub(crate) fn orphan_names(
    leftovers: &riviu_core::PublishMediaLeftovers,
    live: &BTreeSet<String>,
    backoff: &BTreeSet<String>,
) -> Vec<(PublishMediaSweepKind, String)> {
    let mut seen = HashSet::new();
    let mut orphans = Vec::new();
    for album in &leftovers.albums {
        let Some(scope) = publish_album_scope(album) else {
            continue;
        };
        if live.contains(scope) || backoff.contains(album) || !seen.insert(album.clone()) {
            continue;
        }
        orphans.push((PublishMediaSweepKind::Album, album.clone()));
    }
    for stage in &leftovers.stages {
        if live.contains(stage) || backoff.contains(stage) || !seen.insert(stage.clone()) {
            continue;
        }
        orphans.push((PublishMediaSweepKind::Stage, stage.clone()));
    }
    orphans
}

/// Only a readback naming this exact directory as cleaned counts; anything else is a
/// failure with the backend's message kept (bounded) for the receipt row.
fn sweep_receipt(
    claim: &PublishMediaSweepClaim,
    result: Result<serde_json::Value, String>,
) -> (bool, serde_json::Value) {
    match result {
        Ok(value) => {
            let native = value.get("value").unwrap_or(&value);
            let key = match claim.kind {
                PublishMediaSweepKind::Album => "importId",
                PublishMediaSweepKind::Stage => "stageScope",
            };
            let cleaned = native.get("state").and_then(serde_json::Value::as_str)
                == Some("cleaned")
                && native.get(key).and_then(serde_json::Value::as_str) == Some(claim.name.as_str());
            if cleaned {
                (
                    true,
                    serde_json::json!({"state":"cleaned","nativeProof":value}),
                )
            } else {
                (
                    false,
                    serde_json::json!({"state":"not_cleaned","message":"native cleanup did not prove this directory was removed","nativeProof":value}),
                )
            }
        }
        Err(error) => (
            false,
            serde_json::json!({"state":"not_cleaned","message":error.chars().take(512).collect::<String>()}),
        ),
    }
}

#[cfg(test)]
#[path = "media_sweep_tests.rs"]
mod tests;
