//! Manual replacement of old runs uses their existing Stop owners and receipts.
use super::operation_stop::StopTiming;
use super::*;
use futures_util::StreamExt;
use std::collections::HashSet;
use tauri::Manager;

static HANDOFF: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) fn lock_manual_handoff() -> Result<tokio::sync::MutexGuard<'static, ()>, CommandError> {
    HANDOFF
        .try_lock()
        .map_err(|_| CommandError::operation("Đang nhả thiết bị cho yêu cầu trước; chờ hoàn tất"))
}

#[derive(Default)]
struct HandoffStops {
    operations: Vec<String>,
    operation_devices: std::collections::HashMap<String, HashSet<String>>,
    publish_markers: std::collections::HashMap<String, String>,
    // assignment -> (device, exclusion request); never authorizes sibling assignments.
    assignments: std::collections::HashMap<String, (String, String)>,
    scoped: bool,
}

fn intersects(selected: &HashSet<String>, source: &[String]) -> bool {
    source.iter().any(|id| selected.contains(id))
}

fn selected_devices_closed(result: &OperationStopResult, selected: &HashSet<String>) -> bool {
    let devices: Vec<_> = result
        .devices
        .iter()
        .filter(|d| selected.contains(&d.udid))
        .collect();
    !devices.is_empty() && devices.iter().all(|d| d.closed)
}

fn selected_operation_scope(
    selected: &HashSet<String>,
    source: Option<&HashSet<String>>,
) -> Vec<String> {
    source
        .into_iter()
        .flat_map(|source| selected.intersection(source).cloned())
        .collect()
}

/// A selected device another owner still holds, observed without stopping anything.
#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct NeedsRelease {
    pub udid: String,
    pub owner: String,
    pub title: String,
    /// False when releasing this device would stop devices outside the selection, or
    /// when an old post may already be public and needs review instead of a stop.
    pub releasable: bool,
    pub message: String,
}

/// Read-only: what a confirmed handoff would have to release for exactly `udids`.
/// Publish runs split per assignment and Nurture per device; any other run is
/// releasable only when all of its devices are inside the selection.
pub(crate) fn observe_release_needs(
    state: &AppState,
    udids: &[String],
) -> Result<Vec<NeedsRelease>, CommandError> {
    let selected: HashSet<_> = udids.iter().cloned().collect();
    let mut needs = Vec::new();
    let mut covered = HashSet::new();
    let mut ids: HashSet<String> = state
        .db
        .active_operation_source_ids()
        .map_err(CommandError::operation)?
        .into_iter()
        .collect();
    for run in state
        .nurture
        .list_status()
        .into_iter()
        .filter(|s| s.running)
    {
        ids.insert(format!("nurture:{}", riviu_core::nurture_source_id(&run)));
    }
    let mut ids: Vec<_> = ids.into_iter().collect();
    ids.sort();
    for id in ids {
        let Some(detail) = super::jobs::read_operation_run(state, &id)? else {
            continue;
        };
        if detail.summary.state.is_terminal() {
            continue;
        }
        if detail.summary.kind == riviu_core::OperationRunKind::Publish
            && state
                .db
                .publish_campaign_state(&detail.summary.source_id)
                .map_err(CommandError::operation)?
                == Some(riviu_core::PublishCampaignState::Scheduled)
        {
            continue;
        }
        let devices = super::operation_stop::source_devices(&state.db, &detail)
            .map_err(CommandError::operation)?;
        if !intersects(&selected, &devices) {
            continue;
        }
        let outside = devices.iter().filter(|d| !selected.contains(*d)).count();
        let splittable = matches!(
            detail.summary.kind,
            riviu_core::OperationRunKind::Publish | riviu_core::OperationRunKind::Nurture
        );
        let releasable = splittable || outside == 0;
        // Keyed per (device, owner): a publish run matches its own guard hold only.
        let key = if detail.summary.kind == riviu_core::OperationRunKind::Publish {
            format!("publish:{}", detail.summary.source_id)
        } else {
            detail.summary.kind.as_key().to_owned()
        };
        for udid in devices.iter().filter(|d| selected.contains(*d)) {
            covered.insert((udid.clone(), key.clone()));
            needs.push(NeedsRelease {
                udid: udid.clone(),
                owner: detail.summary.kind.as_key().into(),
                title: detail.summary.title.clone(),
                releasable,
                message: if releasable {
                    "Xác nhận sẽ chỉ dừng tác vụ cũ trên máy này".into()
                } else {
                    format!(
                        "Tác vụ này còn chạy trên {outside} máy ngoài lựa chọn; dừng nó ở Theo dõi hoặc chọn thêm các máy đó"
                    )
                },
            });
        }
    }
    // Every independent blocker gets its own row; one covering operation never hides a
    // second publication or owner on the same phone. Consumers treat any false row as
    // blocking the whole device.
    for udid in udids {
        for hold in state
            .db
            .publish_device_guard(udid)
            .map_err(CommandError::operation)?
            .blocking
        {
            if !covered.insert((udid.clone(), format!("publish:{}", hold.campaign_id))) {
                continue;
            }
            // A new post may interrupt link verification: confirmation excludes only this
            // assignment (stopping its verifier) and closes the phone. A possibly public
            // post becomes needsReview, so the verification debt is kept, not cleared.
            needs.push(NeedsRelease {
                udid: udid.clone(),
                owner: "publish".into(),
                title: hold.campaign_id.clone(),
                releasable: true,
                message: format!(
                    "Lượt {} còn giữ máy ({}). Xác nhận sẽ dừng xác minh trên máy này và giữ việc kiểm tra link trong Theo dõi",
                    hold.campaign_id, hold.reason
                ),
            });
        }
        if let Some(owner) = state.control.current_work_owner(udid) {
            let owner_kinds: &[&str] = match owner {
                DeviceWorkOwner::Nurture => &["nurture"],
                DeviceWorkOwner::Interaction => &["interaction"],
                DeviceWorkOwner::Script => &["script", "flow", "orchestration"],
                _ => &[],
            };
            // The owner of an operation already listed is released by stopping it.
            if owner_kinds
                .iter()
                .any(|kind| covered.contains(&(udid.clone(), (*kind).to_owned())))
                || (owner == DeviceWorkOwner::Script
                    && covered.iter().any(|(device, kind)| device == udid && kind.starts_with("publish:")))
            {
                continue;
            }
            let releasable = matches!(
                owner,
                DeviceWorkOwner::ManualControl | DeviceWorkOwner::IdleSweep | DeviceWorkOwner::Nurture
            );
            needs.push(NeedsRelease {
                udid: udid.clone(),
                owner: format!("{owner:?}"),
                title: String::new(),
                releasable,
                message: if releasable {
                    "Xác nhận sẽ nhả phiên đang giữ máy này".into()
                } else {
                    "Máy đang có tác vụ khác giữ quyền điều khiển; chờ tác vụ đó xong".into()
                },
            });
        }
    }
    needs.sort_by(|a, b| a.udid.cmp(&b.udid));
    Ok(needs)
}

#[tauri::command]
pub async fn operation_prepare_devices(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    udids: Vec<String>,
) -> Result<OperationStopResult, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let handoff = lock_manual_handoff()?;
    prepare_manual_devices(&app, &state, udids, &handoff).await
}

/// Keep the same handoff lock through creation so lost create acknowledgements
/// replay their receipt before any old-run cancellation is requested again.
pub(crate) async fn prepare_manual_devices(
    app: &tauri::AppHandle,
    state: &AppState,
    udids: Vec<String>,
    _handoff: &tokio::sync::MutexGuard<'static, ()>,
) -> Result<OperationStopResult, CommandError> {
    let timing = StopTiming::new("handoff");
    if udids.is_empty() || udids.len() > 500 {
        return Err(CommandError::invalid_argument("Chọn từ 1 đến 500 thiết bị"));
    }
    let selected: HashSet<_> = udids.into_iter().collect();
    let source_timing = timing.phase("source", None);
    let roster = state
        .control
        .list_devices()
        .await
        .map_err(CommandError::from)?;
    let disconnected: HashSet<_> = selected
        .iter()
        .filter(|id| {
            !roster
                .iter()
                .any(|d| &d.udid == *id && d.status != riviu_core::DeviceStatus::Disconnected)
        })
        .cloned()
        .collect();
    let active: HashSet<_> = selected.difference(&disconnected).cloned().collect();
    let sources_started = std::time::Instant::now();
    let mut ids: HashSet<String> = state
        .db
        .active_operation_source_ids()
        .map_err(CommandError::operation)?
        .into_iter()
        .collect();
    for run in state
        .nurture
        .list_status()
        .into_iter()
        .filter(|s| s.running)
    {
        ids.insert(format!("nurture:{}", riviu_core::nurture_source_id(&run)));
    }
    let mut held = HashSet::new();
    for id in &selected {
        let guards = state
            .db
            .publish_device_guard(id)
            .map_err(CommandError::operation)?;
        for hold in guards
            .blocking
            .into_iter()
            .chain(guards.link_review.into_iter().filter(|h| {
                state
                    .db
                    .publish_campaign_has_account_reservation(&h.campaign_id)
                    .unwrap_or(true)
            }))
        {
            held.insert(format!("publish:{}", hold.campaign_id));
        }
    }
    let active_source_count = ids.len();
    let held_count = held.len();
    ids.extend(held.iter().cloned());
    let candidate_count = ids.len();
    let mut operations = Vec::new();
    let mut operation_devices = std::collections::HashMap::new();
    for id in ids {
        let Some(detail) = super::jobs::read_operation_run(state, &id)? else {
            continue;
        };
        if detail.summary.kind == riviu_core::OperationRunKind::Publish
            && state
                .db
                .publish_campaign_state(&detail.summary.source_id)
                .map_err(CommandError::operation)?
                == Some(riviu_core::PublishCampaignState::Scheduled)
        {
            continue;
        }
        let devices = super::operation_stop::source_devices(&state.db, &detail)
            .map_err(CommandError::operation)?;
        if intersects(&active, &devices)
            && (!detail.summary.state.is_terminal() || held.contains(&id))
        {
            operations.push(id.clone());
            operation_devices.insert(id, devices.into_iter().collect());
        }
    }
    log::info!("handoff source scan: active_sources={} held_publish={} candidates={} selected_operations={} elapsed_ms={}", active_source_count, held_count, candidate_count, operations.len(), sources_started.elapsed().as_millis());
    drop(source_timing);
    operations.sort();
    // Request every relevant cancellation before waiting for any closer. Runs
    // with interdependent steps are cancelled as a unit by their original owner.
    let mut stopped = HandoffStops {
        operations,
        operation_devices,
        ..Default::default()
    };
    let revoke_timing = timing.phase("revoke", None);
    for id in &stopped.operations {
        // A handoff owns all selected old runs. Reuse their still-current
        // revocation even when individual closers blocked one another; starting
        // those same closers again would repeat the circular wait indefinitely.
        // Physical closure is still proved below under the exclusive lease.
        let revoked = if let Some(campaign) = id.strip_prefix("publish:") {
            let marker = state
                .db
                .get_setting(&format!("operation.stop.publish:{campaign}"))
                .map_err(CommandError::operation)?;
            let current = super::operation_stop_status(app.state(), id.clone())?;
            current
                .filter(|result| marker.is_some() && result.stop_marker == marker)
                .filter(|_| {
                    state
                        .db
                        .publish_verifications_for_campaign(campaign, 1)
                        .is_ok_and(|rows| rows.is_empty())
                })
        } else {
            None
        };
        let result = match revoked {
            Some(result) => result,
            None => super::operation_stop(app.clone(), app.state(), id.clone()).await?,
        };
        if let Some(campaign) = id.strip_prefix("publish:") {
            let marker = result.stop_marker.ok_or_else(|| {
                CommandError::operation("Chưa xác định được phiên Dừng của bài đăng")
            })?;
            stopped.publish_markers.insert(campaign.into(), marker);
        }
    }
    for id in &active {
        state.nurture.stop(id);
        state.end_overlay_session(id).await?;
    }
    drop(revoke_timing);
    let wait_timing = timing.phase("wait", None);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(125);
    let mut not_released = HashSet::new();
    loop {
        not_released.clear();
        for id in &stopped.operations {
            let result = super::operation_stop_status(app.state(), id.clone())?;
            match result {
                Some(result) if selected_devices_closed(&result, &active) => {}
                Some(result) if result.state != "stopping" => {}
                _ => {
                    if let Some(scope) = stopped.operation_devices.get(id) {
                        not_released.extend(selected_operation_scope(&active, Some(scope)));
                    }
                }
            }
        }
        for id in &active {
            if state.control.current_work_owner(id).is_some()
                || state
                    .nurture
                    .list_status()
                    .iter()
                    .any(|s| &s.udid == id && s.running)
            {
                not_released.insert(id.clone());
            }
        }
        if not_released.is_empty() {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    drop(wait_timing);
    let close_timing = timing.phase("close", None);
    // Only settled devices enter this stream. A unique device has one closer,
    // and all results are drained before handing any device to new work.
    // Ordinals index the full sorted, deduplicated request, including blocked
    // and disconnected devices, so filtering readiness never renumbers them.
    let mut device_order: Vec<_> = selected.iter().cloned().collect();
    device_order.sort();
    let ready: Vec<_> = device_order
        .into_iter()
        .enumerate()
        .filter(|(_, id)| active.contains(id) && !not_released.contains(id))
        .collect();
    let mut pending = futures_util::stream::iter(ready.into_iter().map(|(slot, id)| {
        let stopped = &stopped;
        let device_timing = timing.phase("total", Some(slot));
        async move {
            let result = close_handoff_device(state, &id, stopped, deadline, device_timing).await;
            (id.clone(), result)
        }
    }))
    .buffer_unordered(2);
    let mut close_results = std::collections::HashMap::new();
    while let Some((id, result)) = pending.next().await {
        close_results.insert(id, result);
    }
    drop(pending);
    drop(close_timing);
    let mut devices: Vec<_> = selected
        .into_iter()
        .map(|udid| {
            let (closed, message) = if disconnected.contains(&udid) {
                (false, "Thiết bị không còn kết nối".into())
            } else if not_released.contains(&udid) {
                (
                    false,
                    "Tác vụ cũ chưa nhả thiết bị; chưa bắt đầu tác vụ mới".into(),
                )
            } else {
                match close_results.remove(&udid) {
                    Some(Ok(())) => (true, "Đã nhả tác vụ cũ; có thể kiểm tra tác vụ mới".into()),
                    Some(Err(error)) => (false, error.message.to_string()),
                    None => (false, "Không có kết quả nhả thiết bị".into()),
                }
            };
            StopDeviceResult {
                udid,
                closed,
                message,
            }
        })
        .collect();
    devices.sort_by(|a, b| a.udid.cmp(&b.udid));
    let state = if devices.iter().all(|row| row.closed) {
        "closed"
    } else {
        "needsAttention"
    };
    Ok(OperationStopResult {
        operation_id: format!("handoff:{}", uuid::Uuid::new_v4()),
        state: state.into(),
        devices,
        stop_marker: None,
    })
}

/// Replacement after confirmation. Publish excludes selected assignments, Nurture stops
/// selected devices; an indivisible operation is refused before any revocation if it
/// includes a device outside the selection.
pub(crate) async fn prepare_publish_devices(
    app: &tauri::AppHandle,
    state: &AppState,
    udids: Vec<String>,
    _handoff: &tokio::sync::MutexGuard<'static, ()>,
) -> Result<OperationStopResult, CommandError> {
    let timing = StopTiming::new("publishHandoff");
    if udids.is_empty() || udids.len() > 500 {
        return Err(CommandError::invalid_argument("Chọn từ 1 đến 500 thiết bị"));
    }
    let selected: HashSet<_> = udids.into_iter().collect();
    let mut devices: Vec<_> = selected.iter().cloned().collect();
    devices.sort();
    let needs = observe_release_needs(state, &devices)?;
    if let Some(blocked) = needs.iter().find(|row| !row.releasable) {
        return Err(CommandError::operation(format!("{}: {}", blocked.udid, blocked.message)));
    }
    let mut stopped = HandoffStops { scoped: true, ..Default::default() };
    // Complete the scope check before stopping even one operation.
    for id in state.db.active_operation_source_ids().map_err(CommandError::operation)? {
        let Some(detail) = super::jobs::read_operation_run(state, &id)? else { continue };
        if detail.summary.state.is_terminal()
            || matches!(detail.summary.kind, riviu_core::OperationRunKind::Publish | riviu_core::OperationRunKind::Nurture) {
            continue;
        }
        let scope = super::operation_stop::source_devices(&state.db, &detail).map_err(CommandError::operation)?;
        if !intersects(&selected, &scope) { continue; }
        if scope.iter().any(|id| !selected.contains(id)) {
            return Err(CommandError::operation("Tác vụ không thể tách còn giữ máy ngoài lựa chọn; dừng ở Theo dõi hoặc chọn đủ máy"));
        }
        stopped.operation_devices.insert(id.clone(), scope.into_iter().collect());
        stopped.operations.push(id);
    }
    for udid in &devices {
        let mut assignments = state.db.publish_handoff_assignments(udid).map_err(CommandError::operation)?;
        // Guard holds include succeeded rows whose link is still being recovered.
        for hold in state.db.publish_device_guard(udid).map_err(CommandError::operation)?.blocking {
            if !assignments.iter().any(|(id, _)| id == &hold.assignment_id) {
                let campaign = state.db.get_publish_campaign(&hold.campaign_id).map_err(CommandError::operation)?
                    .ok_or_else(|| CommandError::operation("Không tìm thấy lượt đang giữ máy"))?;
                let assignment = campaign.assignments.iter().find(|a| a.id == hold.assignment_id)
                    .ok_or_else(|| CommandError::operation("Không tìm thấy bài đang giữ máy"))?;
                assignments.push((assignment.id.clone(), assignment.revision));
            }
        }
        for (assignment, revision) in assignments {
            let request = uuid::Uuid::new_v4().to_string();
            // This inserts the exclusion used by observer_authorized, fencing the selected
            // verifier as well as the composer. No campaign-wide stop marker is written.
            state.db.request_publish_handoff_exclusion(&assignment, revision, &request)
                .map_err(CommandError::operation)?;
            stopped.assignments.insert(assignment, (udid.clone(), request));
        }
    }
    for id in &stopped.operations {
        super::operation_stop(app.clone(), app.state(), id.clone()).await?;
    }
    for udid in &devices {
        state.nurture.stop(udid);
        state.end_overlay_session(udid).await?;
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(125);
    let mut waiting = HashSet::new();
    loop {
        waiting.clear();
        for (assignment, (udid, _)) in &stopped.assignments {
            if state.db.publish_handoff_worker_running(assignment).map_err(CommandError::operation)? {
                waiting.insert(udid.clone());
            }
        }
        for id in &stopped.operations {
            if super::operation_stop_status(app.state(), id.clone())?.is_none_or(|r| r.state == "stopping") {
                waiting.extend(selected_operation_scope(&selected, stopped.operation_devices.get(id)));
            }
        }
        for udid in &devices {
            if state.control.current_work_owner(udid).is_some()
                || state.nurture.list_status().iter().any(|s| s.udid == *udid && s.running) {
                waiting.insert(udid.clone());
            }
        }
        if waiting.is_empty() || tokio::time::Instant::now() >= deadline { break; }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    let mut closes = futures_util::stream::iter(devices.into_iter().enumerate().map(|(slot, udid)| {
        let stopped = &stopped;
        let waiting = &waiting;
        let timing = timing.phase("total", Some(slot));
        async move {
            let result = if waiting.contains(&udid) {
                Err(CommandError::operation("Tác vụ cũ chưa nhả thiết bị; chưa bắt đầu lượt mới"))
            } else {
                close_handoff_device(state, &udid, stopped, deadline, timing).await
            };
            StopDeviceResult { udid, closed: result.is_ok(), message: match result {
                Ok(()) => "Đã nhả tác vụ cũ trên máy được chọn".into(),
                Err(error) => error.message.to_string(),
            }}
        }
    })).buffer_unordered(2);
    let mut devices = Vec::new();
    while let Some(result) = closes.next().await { devices.push(result); }
    devices.sort_by(|a, b| a.udid.cmp(&b.udid));
    Ok(OperationStopResult { operation_id: format!("handoff:{}", uuid::Uuid::new_v4()),
        state: if devices.iter().all(|r| r.closed) { "closed" } else { "needsAttention" }.into(),
        devices, stop_marker: None })
}

async fn close_handoff_device(
    state: &AppState,
    udid: &str,
    authorized: &HandoffStops,
    deadline: tokio::time::Instant,
    timing: StopTiming,
) -> Result<(), CommandError> {
    let wait_timing = timing.phase("wait", None);
    let context = state
        .control
        .try_acquire_exclusive_keeping_stream(udid, DeviceWorkOwner::ManualControl)
        .await
        .map_err(CommandError::from)?;
    drop(wait_timing);
    let result = async {
        let queue_timing = timing.phase("closeQueue", None);
        let _permit = super::operation_stop::acquire_physical_close(deadline)
            .await
            .map_err(CommandError::operation)?;
        drop(queue_timing);
        let _close_timing = timing.phase("close", None);
        // Recheck every owner/marker after queueing with the lease still held.
        if state
            .nurture
            .list_status()
            .iter()
            .any(|run| run.udid == udid && run.running)
        {
            return Err(CommandError::operation(
                "Có phiên nuôi mới xuất hiện trong lúc nhả máy",
            ));
        }
        let source_timing = timing.phase("source", None);
        let source_ids = state
            .db
            .active_operation_source_ids()
            .map_err(CommandError::operation)?;
        let scan_timing =
            super::operation_stop::SourceScanTiming::new("handoffClose", source_ids.len());
        for id in source_ids {
            if let Some(other) = super::jobs::read_operation_run(state, &id)? {
                if other.summary.kind == riviu_core::OperationRunKind::Publish
                    && state
                        .db
                        .publish_campaign_state(&other.summary.source_id)
                        .map_err(CommandError::operation)?
                        == Some(riviu_core::PublishCampaignState::Scheduled)
                {
                    continue;
                }
                if authorized.scoped && other.summary.kind == riviu_core::OperationRunKind::Nurture {
                    // The aggregate run may still own unselected siblings. The selected
                    // device's running flag was checked above and its lease is ours.
                    continue;
                }
                if authorized.scoped && other.summary.kind == riviu_core::OperationRunKind::Publish {
                    let rows = state.db.publish_handoff_assignments(udid).map_err(CommandError::operation)?;
                    if rows.iter().all(|(id, _)| authorized.assignments.contains_key(id)) {
                        continue;
                    }
                }
                if !other.summary.state.is_terminal()
                    && super::operation_stop::source_devices(&state.db, &other)
                        .map_err(CommandError::operation)?
                        .iter()
                        .any(|d| d == udid)
                {
                    return Err(CommandError::operation(
                        "Có tác vụ mới xuất hiện trong lúc nhả máy; chưa đóng ứng dụng",
                    ));
                }
            }
        }
        drop(scan_timing);
        drop(source_timing);
        let mut assignment_proofs = Vec::new();
        for (assignment, (device, request)) in &authorized.assignments {
            if device == udid {
                let fence = state.db.publish_handoff_fence(assignment, request).map_err(CommandError::operation)?;
                assignment_proofs.push((assignment.clone(), request.clone(), fence));
            }
        }
        let mut proofs = Vec::new();
        for hold in state
            .db
            .publish_device_guard(udid)
            .map_err(CommandError::operation)?
            .blocking
        {
            if authorized.scoped && assignment_proofs.iter().any(|(id, _, _)| id == &hold.assignment_id) {
                continue;
            }
            let id = format!("publish:{}", hold.campaign_id);
            if !authorized.operations.contains(&id) {
                return Err(CommandError::operation(
                    "Bài đăng đã thay đổi trong lúc chuyển giao; giữ phiên để kiểm tra",
                ));
            }
            let marker = state
                .db
                .get_setting(&format!("operation.stop.publish:{}", hold.campaign_id))
                .map_err(CommandError::operation)?
                .ok_or_else(|| CommandError::operation("Chưa dừng phiên đăng cũ"))?;
            if authorized.publish_markers.get(&hold.campaign_id) != Some(&marker)
                || !state
                    .db
                    .publish_verifications_for_campaign(&hold.campaign_id, 1)
                    .map_err(CommandError::operation)?
                    .is_empty()
            {
                return Err(CommandError::operation(
                    "Bài cũ đã được tiếp tục xác minh; không dùng quyền Dừng cũ để đóng ứng dụng",
                ));
            }
            proofs.push((hold.campaign_id, marker));
        }
        // A finished Nurture run can still defer app cleanup. With no active
        // operation its old search stack survived the previous handoff and
        // broke account preflight. The exclusive context and guards above also
        // authorize closing that idle TikTok process before new manual work.
        let completion_rows = state
            .db
            .pending_app_completions_for_device(udid)
            .map_err(CommandError::operation)?;
        let mut packages = completion_rows
            .iter()
            .map(|record| record.bundle_id.clone())
            .collect::<Vec<_>>();
        if packages.is_empty() {
            packages.extend(
                super::operation_stop::packages_to_stop(&state.control, udid)
                    .await
                    .map_err(CommandError::operation)?,
            );
        }
        packages.sort();
        packages.dedup();
        for package in packages {
            let proof = state
                .control
                .terminate_app(&context, &package)
                .await
                .map_err(CommandError::from)?;
            if let Some(record) = completion_rows
                .iter()
                .find(|record| record.bundle_id == package)
            {
                if !state
                    .db
                    .finish_app_completion(
                        record,
                        &serde_json::to_string(&proof).map_err(CommandError::operation)?,
                    )
                    .map_err(CommandError::operation)?
                {
                    return Err(CommandError::operation(
                        "Yêu cầu đóng ứng dụng đã đổi trong lúc chuyển giao; chưa xác nhận nhả máy",
                    ));
                }
            }
        }
        if state.control.reports_element_bounds(udid) {
            let home = state
                .control
                .device_shell(&context, "input keyevent KEYCODE_HOME")
                .await
                .map_err(CommandError::from)?;
            if home.exit_code != 0 {
                return Err(CommandError::operation("Chưa xác nhận về màn hình chính"));
            }
        }
        Ok((proofs, assignment_proofs))
    }
    .await;
    state
        .control
        .close_exclusive_context(context)
        .map_err(CommandError::from)?;
    let (proofs, assignment_proofs) = result?;
    for (assignment, request, fence) in assignment_proofs {
        state.db.record_publish_handoff_release(&assignment, &request, &fence)
            .map_err(CommandError::operation)?;
    }
    for (campaign, marker) in proofs {
        if !state
            .db
            .record_publish_stopped_device_release(&campaign, udid, &marker)
            .map_err(CommandError::operation)?
        {
            return Err(CommandError::operation(
                "Phiên đã đổi trước khi ghi nhận nhả máy",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_or_unrelated_scope_never_cancels_a_source() {
        let selected = HashSet::from(["chosen".to_owned()]);
        assert!(!intersects(&selected, &[]));
        assert!(!intersects(&selected, &["other".into()]));
        assert!(intersects(&selected, &["other".into(), "chosen".into()]));
    }
    #[test]
    fn missing_stop_rows_never_claim_selected_devices_closed() {
        let selected = HashSet::from(["chosen".to_owned()]);
        let mut result = OperationStopResult {
            operation_id: "run".into(),
            state: "closed".into(),
            devices: vec![],
            stop_marker: None,
        };
        assert!(!selected_devices_closed(&result, &selected));
        result.devices.push(StopDeviceResult {
            udid: "other".into(),
            closed: true,
            message: String::new(),
        });
        assert!(!selected_devices_closed(&result, &selected));
        result.devices.push(StopDeviceResult {
            udid: "chosen".into(),
            closed: false,
            message: String::new(),
        });
        assert!(!selected_devices_closed(&result, &selected));
        result.devices[1].closed = true;
        assert!(selected_devices_closed(&result, &selected));
    }

    #[test]
    fn pending_operation_only_blocks_selected_devices_in_its_source_snapshot() {
        let selected = HashSet::from(["a".to_owned(), "b".to_owned(), "c".to_owned()]);
        let source = HashSet::from(["b".to_owned(), "other".to_owned()]);
        assert_eq!(selected_operation_scope(&selected, Some(&source)), ["b"]);
        assert!(selected_operation_scope(&selected, None).is_empty());
    }
}
