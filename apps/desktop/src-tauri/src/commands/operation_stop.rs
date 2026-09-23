use super::*;
use std::collections::HashSet;
use tauri::Manager;
static ACTIVE_STOPS: std::sync::LazyLock<parking_lot::Mutex<HashSet<String>>> =
    std::sync::LazyLock::new(|| parking_lot::Mutex::new(HashSet::new()));
struct StopGuard(String);
impl Drop for StopGuard {
    fn drop(&mut self) {
        ACTIVE_STOPS.lock().remove(&self.0);
    }
}

pub use riviu_core::ipc_contract::{OperationStopResult, StopDeviceResult};

fn key(id: &str) -> String {
    format!("operation.stop.result:{id}")
}

/// Stop can close both installed Android TikTok variants without selecting an
/// account or granting either package permission to publish. Launch still needs
/// an explicit binding when the foreground app cannot resolve the ambiguity.
pub(super) async fn packages_to_stop(
    control: &riviu_core::DeviceControlPlane,
    udid: &str,
) -> anyhow::Result<Vec<String>> {
    match control.resolve_tiktok_package(udid).await {
        Ok(package) => Ok(vec![package]),
        Err(error)
            if error
                .to_string()
                .contains("more than one measured TikTok build is installed") =>
        {
            let mut packages: Vec<_> = control
                .list_installed_apps(udid)
                .await?
                .into_iter()
                .map(|app| app.bundle_id)
                .filter(|package| riviu_core::tiktok_target::is_measured_android_tiktok(package))
                .collect();
            packages.sort();
            packages.dedup();
            anyhow::ensure!(
                packages.len() == 2,
                "TikTok package inventory changed during Stop"
            );
            Ok(packages)
        }
        Err(error) => Err(error.into()),
    }
}

pub(super) fn source_devices(
    db: &riviu_core::db::Database,
    detail: &riviu_core::OperationRunDetail,
) -> anyhow::Result<Vec<String>> {
    use anyhow::Context;
    use riviu_core::OperationRunKind;
    let mut devices = match detail.summary.kind {
        OperationRunKind::Script => {
            db.get_job(uuid::Uuid::parse_str(&detail.summary.source_id)?)?
                .context("Script source snapshot missing")?
                .udids
        }
        OperationRunKind::Orchestration => {
            let source = db
                .get_orchestration_run(uuid::Uuid::parse_str(&detail.summary.source_id)?)?
                .context("Orchestration source snapshot missing")?;
            std::iter::once(&source.run.target)
                .chain(source.run.node_targets.values())
                .chain(source.attempts.iter().map(|a| &a.snapshot.target))
                .flat_map(|target| target.included.iter().map(|d| d.udid.clone()))
                .collect()
        }
        _ => detail
            .items
            .iter()
            .filter_map(|item| item.udid.clone())
            .collect(),
    };
    devices.sort();
    devices.dedup();
    Ok(devices)
}

fn all_devices_closed(devices: &[StopDeviceResult]) -> bool {
    !devices.is_empty() && devices.iter().all(|d| d.closed)
}

#[cfg(test)]
fn cached_stop_result(
    db: &riviu_core::db::Database,
    kind: riviu_core::OperationRunKind,
    source: &str,
    operation: &str,
) -> anyhow::Result<Option<OperationStopResult>> {
    // Revocation precedes the cached close result: a later resume must not inherit
    // an earlier stop's authority, but closing devices remains idempotent.
    if kind == riviu_core::OperationRunKind::Publish {
        db.begin_publish_operation_stop(source)?;
    }
    db.get_setting(&key(operation))?
        .map(|raw| serde_json::from_str(&raw).map_err(Into::into))
        .transpose()
}

fn claim_stop_result(
    db: &riviu_core::db::Database,
    kind: riviu_core::OperationRunKind,
    source: &str,
    mut initial: OperationStopResult,
    active_stops: &parking_lot::Mutex<HashSet<String>>,
) -> anyhow::Result<(OperationStopResult, bool)> {
    // One synchronous claim phase. Never use a cached 'stopping' read after the
    // old closer can drop its active claim; that would spawn a second closer.
    let mut active = active_stops.lock();
    if active.contains(&initial.operation_id) {
        let current = db
            .get_setting(&key(&initial.operation_id))?
            .map(|raw| serde_json::from_str(&raw))
            .transpose()?
            .unwrap_or(initial);
        return Ok((current, false));
    }
    let cached: Option<OperationStopResult> = db
        .get_setting(&key(&initial.operation_id))?
        .map(|raw| serde_json::from_str(&raw))
        .transpose()?;
    let mut reuse_stop_generation = false;
    if let Some(current) = cached {
        let released_current = kind != riviu_core::OperationRunKind::Publish
            || current
                .devices
                .iter()
                .map(|d| db.publish_device_guard(&d.udid))
                .collect::<anyhow::Result<Vec<_>>>()?
                .iter()
                .all(|guard| !guard.blocking.iter().any(|hold| hold.campaign_id == source));
        let closed_current = kind != riviu_core::OperationRunKind::Publish
            || (current.stop_marker.is_some()
                && current.stop_marker
                    == db.get_setting(&format!("operation.stop.publish:{source}"))?
                && db
                    .publish_verifications_for_campaign(source, 1000)?
                    .is_empty());
        if current.state == "closed"
            && all_devices_closed(&current.devices)
            && current
                .devices
                .iter()
                .map(|d| &d.udid)
                .eq(initial.devices.iter().map(|d| &d.udid))
            && closed_current
            && released_current
            && (kind != riviu_core::OperationRunKind::Publish
                || !db.publish_campaign_has_account_reservation(source)?)
        {
            return Ok((current, false));
        }
        // Retrying a failed closer in an already revoked publication must not
        // revoke valid device-release proofs for its other phones. A resumed
        // verifier or changed marker still requires a fresh Stop generation.
        if kind == riviu_core::OperationRunKind::Publish
            && closed_current
            && current
                .devices
                .iter()
                .map(|d| &d.udid)
                .eq(initial.devices.iter().map(|d| &d.udid))
        {
            initial.stop_marker = current.stop_marker;
            reuse_stop_generation = true;
        }
    }
    if kind == riviu_core::OperationRunKind::Publish && !reuse_stop_generation {
        db.begin_publish_operation_stop(source)?;
        initial.stop_marker = db.get_setting(&format!("operation.stop.publish:{source}"))?;
    }
    if active.contains(&initial.operation_id) {
        return Ok((initial, false));
    }
    db.set_setting(
        &key(&initial.operation_id),
        &serde_json::to_string(&initial)?,
    )?;
    active.insert(initial.operation_id.clone());
    Ok((initial, true))
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    #[test]
    fn retrying_failed_close_keeps_the_current_stop_generation() {
        let path =
            std::env::temp_dir().join(format!("stop-retry-generation-{}.db", uuid::Uuid::new_v4()));
        let db = riviu_core::db::Database::open(&path).unwrap();
        let raw = rusqlite::Connection::open(&path).unwrap();
        raw.execute("INSERT INTO publish_campaigns(id,request_id,source_root,request_json,state,created_at,updated_at) VALUES('c','r','fixture','{}','cancelled','now','now')",[]).unwrap();
        db.begin_publish_operation_stop("c").unwrap();
        let marker = db.get_setting("operation.stop.publish:c").unwrap();
        let cached = OperationStopResult {
            operation_id: "publish:c".into(),
            state: "needsAttention".into(),
            stop_marker: marker.clone(),
            devices: vec![StopDeviceResult {
                udid: "phone".into(),
                closed: false,
                message: "transport failed".into(),
            }],
        };
        db.set_setting(&key("publish:c"), &serde_json::to_string(&cached).unwrap())
            .unwrap();
        let mut initial = cached;
        initial.stop_marker = None;
        initial.state = "stopping".into();
        let (claimed, run) = claim_stop_result(
            &db,
            riviu_core::OperationRunKind::Publish,
            "c",
            initial,
            &parking_lot::Mutex::new(HashSet::new()),
        )
        .unwrap();
        assert!(run);
        assert_eq!(claimed.stop_marker, marker);
        assert_eq!(db.get_setting("operation.stop.publish:c").unwrap(), marker);
    }
    #[test]
    fn cached_closed_publish_without_device_release_proof_must_run_the_closer_again() {
        let path =
            std::env::temp_dir().join(format!("legacy-stop-proof-{}.db", uuid::Uuid::new_v4()));
        let db = riviu_core::db::Database::open(&path).unwrap();
        let raw = rusqlite::Connection::open(&path).unwrap();
        raw.execute("INSERT INTO publish_campaigns(id,request_id,source_root,request_json,state,created_at,updated_at) VALUES('c','r','fixture','{}','cancelled','now','now')", []).unwrap();
        raw.execute("INSERT INTO publish_bundles(id,campaign_id,ordinal,name,source_path,caption,caption_sha256,manifest_json,created_at) VALUES('b','c',0,'fixture','fixture','caption',?1,'{}','now')", ["a".repeat(64)]).unwrap();
        raw.execute("INSERT INTO publish_assignments(id,campaign_id,bundle_id,ordinal,udid,state,effect_intent,created_at,updated_at) VALUES('a','c','b',0,'phone','uncertain','{}','now','now')", []).unwrap();
        db.begin_publish_operation_stop("c").unwrap();
        let initial = OperationStopResult {
            operation_id: "publish:c".into(),
            state: "stopping".into(),
            stop_marker: db.get_setting("operation.stop.publish:c").unwrap(),
            devices: vec![StopDeviceResult {
                udid: "phone".into(),
                closed: false,
                message: "waiting".into(),
            }],
        };
        let mut cached = initial.clone();
        cached.state = "closed".into();
        cached.devices[0].closed = true;
        db.set_setting(&key("publish:c"), &serde_json::to_string(&cached).unwrap())
            .unwrap();
        assert!(db.has_pending_publish_for_device("phone").unwrap());
        let (_, claimed) = claim_stop_result(
            &db,
            riviu_core::OperationRunKind::Publish,
            "c",
            initial,
            &parking_lot::Mutex::new(HashSet::new()),
        )
        .unwrap();
        assert!(
            claimed,
            "old closed result cannot stand in for a device release proof"
        );
        assert!(
            db.has_pending_publish_for_device("phone").unwrap(),
            "only a real closer may release the hold"
        );
    }
    #[test]
    fn regression_script_stop_uses_source_roster_and_empty_is_not_closed() {
        let path = std::env::temp_dir().join(format!("stop-roster-{}.db", uuid::Uuid::new_v4()));
        let db = riviu_core::db::Database::open(&path).unwrap();
        let now = chrono::Utc::now();
        let job = riviu_core::JobRecord {
            id: uuid::Uuid::new_v4(),
            script_name: "fixture".into(),
            udids: vec!["b".into(), "a".into(), "a".into()],
            status: riviu_core::JobStatus::Running,
            created_at: now,
            updated_at: now,
            steps: vec![],
            error: None,
        };
        db.save_job(&job).unwrap();
        let detail = riviu_core::project_job(&job);
        assert!(detail.items.is_empty());
        assert_eq!(source_devices(&db, &detail).unwrap(), vec!["a", "b"]);
        assert!(!all_devices_closed(&[]));
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn stale_stopping_read_cannot_claim_another_closer_after_first_closes() {
        let path =
            std::env::temp_dir().join(format!("stop-interleave-{}.db", uuid::Uuid::new_v4()));
        let db = riviu_core::db::Database::open(&path).unwrap();
        let raw = rusqlite::Connection::open(&path).unwrap();
        raw.execute("INSERT INTO publish_campaigns(id,request_id,source_root,request_json,state,created_at,updated_at) VALUES('c','r','fixture','{}','cancelled','now','now')",[]).unwrap();
        let initial = OperationStopResult {
            operation_id: "publish:c".into(),
            state: "stopping".into(),
            devices: vec![StopDeviceResult {
                udid: "phone".into(),
                closed: false,
                message: "waiting".into(),
            }],
            stop_marker: None,
        };
        db.set_setting(&key("publish:c"), &serde_json::to_string(&initial).unwrap())
            .unwrap();
        let stale =
            cached_stop_result(&db, riviu_core::OperationRunKind::Publish, "c", "publish:c")
                .unwrap()
                .unwrap();
        assert_eq!(stale.state, "stopping");
        // Deterministic interleaving: A completes and drops its runtime claim after
        // B's old read; B's actual claim must re-read under the shared gate.
        db.set_setting(
            &key("publish:c"),
            r#"{"operationId":"publish:c","state":"closed","devices":[]}"#,
        )
        .unwrap();
        let marker = db.get_setting("operation.stop.publish:c").unwrap();
        db.set_setting(
            &key("publish:c"),
            &serde_json::to_string(&OperationStopResult {
                operation_id: "publish:c".into(),
                state: "closed".into(),
                devices: vec![StopDeviceResult {
                    udid: "phone".into(),
                    closed: true,
                    message: "closed".into(),
                }],
                stop_marker: marker,
            })
            .unwrap(),
        )
        .unwrap();
        let active = parking_lot::Mutex::new(HashSet::new());
        let (current, claimed) = claim_stop_result(
            &db,
            riviu_core::OperationRunKind::Publish,
            "c",
            initial,
            &active,
        )
        .unwrap();
        assert!(!claimed, "stale stopping cannot start a second closer");
        assert_eq!(current.state, "closed");
        assert!(active.lock().is_empty());
    }

    #[test]
    fn cached_closed_stop_revokes_observer_generation_without_reclosing_devices() {
        let path = std::env::temp_dir().join(format!("stop-recovery-{}.db", uuid::Uuid::new_v4()));
        let db = riviu_core::db::Database::open(&path).unwrap();
        let raw = rusqlite::Connection::open(&path).unwrap();
        raw.execute("INSERT INTO publish_campaigns(id,request_id,source_root,request_json,state,created_at,updated_at) VALUES('c','r','fixture','{}','cancelled','now','now')",[]).unwrap();
        db.set_setting(
            "operation.stop.publish:c",
            "{\"generation\":\"old-observer\"}",
        )
        .unwrap();
        db.set_setting(&key("publish:c"), r#"{"operationId":"publish:c","state":"closed","devices":[{"udid":"phone","closed":true,"message":"previous close"}]}"#).unwrap();
        let cached =
            cached_stop_result(&db, riviu_core::OperationRunKind::Publish, "c", "publish:c")
                .unwrap()
                .unwrap();
        assert_ne!(
            db.get_setting("operation.stop.publish:c").unwrap().unwrap(),
            "{\"generation\":\"old-observer\"}"
        );
        assert_eq!(cached.state, "closed");
        assert_eq!(cached.devices[0].message, "previous close");
    }
}

#[tauri::command]
pub fn operation_stop_status(
    state: State<'_, AppState>,
    operation_id: String,
) -> Result<Option<OperationStopResult>, CommandError> {
    let result: Option<OperationStopResult> = state
        .db
        .get_setting(&key(&operation_id))
        .map_err(err)?
        .map(|raw| serde_json::from_str(&raw).map_err(err))
        .transpose()?;
    Ok(result.map(|mut result| {
        if result.state == "stopping" && !ACTIVE_STOPS.lock().contains(&operation_id) {
            result.state = "needsAttention".into();
            for d in &mut result.devices {
                if !d.closed {
                    d.message = "Lần dừng trước bị gián đoạn; bấm Dừng để kiểm tra lại".into();
                }
            }
        }
        result
    }))
}

#[tauri::command]
pub async fn operation_stop(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    operation_id: String,
) -> Result<OperationStopResult, CommandError> {
    let admission = state.ensure_accepting_work()?;
    let detail = super::jobs::read_operation_run(&state, &operation_id)?
        .ok_or_else(|| err("Không tìm thấy tác vụ"))?;
    use riviu_core::OperationRunKind as Kind;
    let source = detail.summary.source_id.clone();
    let udids = source_devices(&state.db, &detail).map_err(err)?;
    let result = OperationStopResult {
        operation_id: operation_id.clone(),
        state: "stopping".into(),
        stop_marker: None,
        devices: udids
            .iter()
            .map(|udid| StopDeviceResult {
                udid: udid.clone(),
                closed: false,
                message: "Đang dừng và chờ nhả máy".into(),
            })
            .collect(),
    };
    // Claim before spawning. Repeated clicks share the current stop request.
    let (result, claimed) = claim_stop_result(
        &state.db,
        detail.summary.kind,
        &source,
        result,
        &ACTIVE_STOPS,
    )
    .map_err(err)?;
    if detail.summary.kind == Kind::Publish {
        state
            .events
            .emit(riviu_core::events::AppEvent::PublishUpdated {
                campaign_id: source.clone(),
                revision: state
                    .db
                    .publish_campaign_revision(&source)
                    .unwrap_or_default(),
            });
    }
    if !claimed {
        return Ok(result);
    }
    let guard = StopGuard(operation_id.clone());
    let initial = result.clone();
    tauri::async_runtime::spawn(async move {
        let _guard = guard;
        let _admission = admission;
        let state = app.state::<AppState>();
        let mut result = result;
        let cancelled: Result<(), CommandError> = async {
            match detail.summary.kind {
                Kind::Nurture => {
                    // Stop only the exact run captured by the monitor, never a newer session.
                    for status in state
                        .nurture
                        .list_status()
                        .into_iter()
                        .filter(|s| riviu_core::nurture_source_id(s) == source)
                    {
                        state.nurture.stop(&status.udid);
                    }
                }
                Kind::Publish => {} // Revoked synchronously before reading the cached result.
                Kind::Interaction => {
                    state.db.cancel_interaction_campaign(&source).map_err(err)?;
                }
                Kind::Script => {
                    state
                        .jobs
                        .cancel(uuid::Uuid::parse_str(&source).map_err(err)?);
                }
                Kind::Flow => {
                    state
                        .flows
                        .cancel_run(uuid::Uuid::parse_str(&source).map_err(err)?)
                        .map_err(err)?;
                }
                Kind::Orchestration => {
                    crate::orchestration_commands::orchestration_cancel_run(
                        app.clone(),
                        app.state(),
                        source.clone(),
                    )
                    .await?;
                }
                Kind::AppInstall | Kind::MaterialTransfer => {
                    crate::farm_commands::operation_cancel_batch(
                        app.state(),
                        operation_id.clone(),
                    )?;
                }
            }
            Ok(())
        }
        .await;
        if let Err(error) = cancelled {
            result.state = "failed".into();
            for device in &mut result.devices {
                device.message = error.message.to_string();
            }
        } else {
            let stop_marker = result.stop_marker.clone();
            let futures = udids.iter().map(|udid| {
                close_stopped_device(&state, &operation_id, udid, stop_marker.as_deref())
            });
            use futures_util::StreamExt;
            let mut pending: futures_util::stream::FuturesUnordered<_> = futures.collect();
            while let Some(device) = pending.next().await {
                if let Some(row) = result.devices.iter_mut().find(|r| r.udid == device.udid) {
                    *row = device;
                }
                if let Ok(raw) = serde_json::to_string(&result) {
                    let _ = state.db.set_setting(&key(&operation_id), &raw);
                }
            }
            result.state = if all_devices_closed(&result.devices) {
                "closed"
            } else {
                "needsAttention"
            }
            .into();
        }
        if let Ok(raw) = serde_json::to_string(&result) {
            let _ = state.db.set_setting(&key(&operation_id), &raw);
        }
    });
    Ok(initial)
}

async fn close_stopped_device(
    state: &AppState,
    operation_id: &str,
    udid: &str,
    expected_publish_marker: Option<&str>,
) -> StopDeviceResult {
    let control = &state.control;
    let db = &state.db;
    let result: Result<(), String> = async {
        let publish_marker = expected_publish_marker;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            match control
                .try_acquire_exclusive_keeping_stream(udid, DeviceWorkOwner::ManualControl)
                .await
            {
                Ok(context) => {
                    let closed: Result<(), String> = async {
                        if let Some(campaign) = operation_id.strip_prefix("publish:") {
                            if publish_marker.is_none()
                                || db
                                    .get_setting(&format!("operation.stop.publish:{campaign}"))
                                    .map_err(|e| e.to_string())?
                                    .as_deref()
                                    != publish_marker
                            {
                                return Err(
                                    "Lần dừng đã thay đổi; không đóng ứng dụng bằng yêu cầu cũ"
                                        .into(),
                                );
                            }
                        }
                        // A queued or newly started operation after cancellation still owns
                        // its target scope even during a gap between device leases.
                        for id in db
                            .operation_source_ids(None, None)
                            .map_err(|e| e.to_string())?
                        {
                            if id == operation_id {
                                continue;
                            }
                            if let Some(other) = super::jobs::read_operation_run(state, &id)
                                .map_err(|e| e.message.to_string())?
                            {
                                if !other.summary.state.is_terminal()
                                    && source_devices(db, &other)
                                        .map_err(|e| e.to_string())?
                                        .iter()
                                        .any(|id| id == udid)
                                {
                                    return Err(format!(
                                        "Máy còn tác vụ khác: {}",
                                        other.summary.title
                                    ));
                                }
                            }
                        }
                        // Other campaigns' unresolved uploads are never part of this cancellation.
                        let guard = db.publish_device_guard(udid).map_err(|e| e.to_string())?;
                        let own = operation_id.strip_prefix("publish:");
                        if guard
                            .blocking
                            .iter()
                            .any(|hold| Some(hold.campaign_id.as_str()) != own)
                        {
                            return Err("Máy còn bài đăng của tác vụ khác; chưa đóng TikTok".into());
                        }
                        for package in packages_to_stop(control, udid)
                            .await
                            .map_err(|e| e.to_string())?
                        {
                            control
                                .terminate_app(&context, &package)
                                .await
                                .map_err(|e| e.to_string())?;
                        }
                        if control.reports_element_bounds(udid) {
                            let home = control
                                .device_shell(&context, "input keyevent KEYCODE_HOME")
                                .await
                                .map_err(|e| e.to_string())?;
                            if home.exit_code != 0 {
                                return Err("TikTok đã tắt nhưng chưa về màn hình chính".into());
                            }
                        }
                        Ok(())
                    }
                    .await;
                    let released = control
                        .close_exclusive_context(context)
                        .map_err(|e| e.to_string());
                    closed?;
                    released?;
                    if let (Some(campaign), Some(marker)) =
                        (operation_id.strip_prefix("publish:"), publish_marker)
                    {
                        if !db
                            .record_publish_stopped_device_release(campaign, udid, marker)
                            .map_err(|e| e.to_string())?
                        {
                            return Err(
                                "Phiên đã thay đổi; không dùng kết quả dừng cũ để nhả máy".into()
                            );
                        }
                        state
                            .events
                            .emit(riviu_core::events::AppEvent::PublishUpdated {
                                campaign_id: campaign.into(),
                                revision: db
                                    .publish_campaign_revision(campaign)
                                    .unwrap_or_default(),
                            });
                    }
                    return Ok(());
                }
                Err(riviu_core::DeviceControlError::Busy(_))
                    if std::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await
                }
                Err(error) => return Err(error.to_string()),
            }
        }
    }
    .await;
    StopDeviceResult {
        udid: udid.into(),
        closed: result.is_ok(),
        message: result
            .err()
            .unwrap_or_else(|| "Đã dừng; TikTok đã tắt".into()),
    }
}
