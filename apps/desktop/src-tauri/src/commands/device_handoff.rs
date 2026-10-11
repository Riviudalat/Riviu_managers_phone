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

pub(crate) async fn wait_manual_handoff_until(
    deadline: tokio::time::Instant,
) -> Result<tokio::sync::MutexGuard<'static, ()>, CommandError> {
    tokio::time::timeout_at(deadline, HANDOFF.lock())
        .await
        .map_err(|_| CommandError::operation("Đã hết thời gian chờ lượt nhả thiết bị; chưa tạo chiến dịch"))
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
    /// Exact identity of what would be stopped (PLAN §3): the phone, the owner kind and the
    /// job (operation id, publish campaign/assignment, or lease generation for an owner
    /// without a listed job). Confirmation approves releasing these envelopes and nothing
    /// else; a different job on the same phone is a different envelope.
    pub envelope: String,
}

impl NeedsRelease {
    /// Direct control and the idle sweep hold a phone for moments and yield to a confirmed
    /// start; whether one happens to be there is not part of what the operator approved.
    fn transient(&self) -> bool {
        self.owner == "ManualControl" || self.owner == "IdleSweep"
    }
}

/// The first holder that the reviewed release envelope does not cover, if any. A holder
/// that went away needs no approval; a new or different one refuses the start before
/// anything is stopped, excluded or closed.
pub(crate) fn unapproved_release(needs: &[NeedsRelease], approved: &[String]) -> Option<String> {
    needs
        .iter()
        .filter(|row| !row.transient())
        .find(|row| !approved.contains(&row.envelope))
        .map(|row| {
            let job = if row.title.is_empty() { row.owner.clone() } else { format!("{} · {}", row.owner, row.title) };
            format!("{}: tác vụ đang giữ máy ({job}) khác với lúc xem lại; chưa dừng gì, kiểm tra lại rồi xác nhận", row.udid)
        })
}

/// Read-only: what a confirmed handoff would have to release for exactly `udids`.
/// Publish runs split per assignment and Nurture per device; any other run is
/// releasable only when all of its devices are inside the selection.
pub(crate) fn observe_release_needs(
    state: &AppState,
    udids: &[String],
) -> Result<Vec<NeedsRelease>, CommandError> {
    Ok(observe_release_plan(state, udids)?.needs)
}

/// One frozen observation of what a confirmed handoff may stop (PLAN §3, review R2-F1).
///
/// The rows the operator reviews and the exact items a handoff stops come from the same
/// enumeration, so each stop is backed by a reviewed envelope: an operation by id, a Nuôi
/// run by run id, a publish assignment by its revision. A handoff builds it once under the
/// start fence and applies exactly it; nothing found later is added.
pub(crate) struct ReleasePlan {
    pub(crate) needs: Vec<NeedsRelease>,
    /// Non-splittable operations wholly inside the selection, with their device scope.
    operations: Vec<(String, HashSet<String>)>,
    /// Publish assignments to exclude: (phone, assignment, revision observed and approved).
    assignments: Vec<(String, String, i64)>,
    /// Exact Nuôi runs to stop: (phone, run id).
    nurture: Vec<(String, uuid::Uuid)>,
}

pub(crate) fn observe_release_plan(
    state: &AppState,
    udids: &[String],
) -> Result<ReleasePlan, CommandError> {
    let selected: HashSet<_> = udids.iter().cloned().collect();
    let mut needs = Vec::new();
    let mut covered = HashSet::new();
    let mut operations = Vec::new();
    let mut nurture = Vec::new();
    // Publish rows get their envelope once the exact assignments are known: (row, phone, campaign).
    let mut publish_rows: Vec<(usize, String, String)> = Vec::new();
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
        let mut releasable = splittable || outside == 0;
        // A Nuôi run is stopped by its run id; one without an id cannot be stopped exactly.
        let nurture_run = (detail.summary.kind == riviu_core::OperationRunKind::Nurture)
            .then(|| uuid::Uuid::parse_str(&detail.summary.source_id).ok());
        if nurture_run == Some(None) {
            releasable = false;
        }
        let publish = detail.summary.kind == riviu_core::OperationRunKind::Publish;
        if releasable && !splittable {
            operations.push((id.clone(), devices.iter().cloned().collect::<HashSet<_>>()));
        }
        // Keyed per (device, owner): a publish run matches its own guard hold only.
        let key = if publish {
            format!("publish:{}", detail.summary.source_id)
        } else {
            detail.summary.kind.as_key().to_owned()
        };
        for udid in devices.iter().filter(|d| selected.contains(*d)) {
            covered.insert((udid.clone(), key.clone()));
            if let Some(Some(run)) = nurture_run {
                nurture.push((udid.clone(), run));
            }
            if publish {
                publish_rows.push((needs.len(), udid.clone(), detail.summary.source_id.clone()));
            }
            needs.push(NeedsRelease {
                envelope: format!("operation:{udid}:{}", detail.summary.id),
                udid: udid.clone(),
                owner: detail.summary.kind.as_key().into(),
                title: detail.summary.title.clone(),
                releasable,
                message: if releasable {
                    "Xác nhận sẽ chỉ dừng tác vụ cũ trên máy này".into()
                } else if nurture_run == Some(None) {
                    "Phiên Nuôi này không có mã lượt nên không dừng đúng phiên được; dừng nó ở Theo dõi trước".into()
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
            publish_rows.push((needs.len(), udid.clone(), hold.campaign_id.clone()));
            needs.push(NeedsRelease {
                envelope: String::new(),
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
            // No job is listed for this owner, so its lease generation is its identity.
            let generation = state
                .control
                .held_work(udid)
                .map_or_else(String::new, |held| held.token.to_string());
            needs.push(NeedsRelease {
                envelope: format!("lease:{udid}:{owner:?}:{generation}"),
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
    // The publish work a handoff excludes is exactly these assignments at these revisions; each
    // (phone, campaign) gets a reviewed row whose envelope names them, so a moved revision or a
    // new assignment is a different envelope.
    let mut assignments = Vec::new();
    let mut by_campaign: std::collections::BTreeMap<(String, String), Vec<String>> =
        Default::default();
    for udid in udids {
        for (assignment, revision) in state
            .db
            .publish_handoff_assignments(udid)
            .map_err(CommandError::operation)?
        {
            let campaign = state
                .db
                .publication_campaign_id(&assignment)
                .map_err(CommandError::operation)?
                .unwrap_or_default();
            by_campaign
                .entry((udid.clone(), campaign))
                .or_default()
                .push(format!("{assignment}@{revision}"));
            assignments.push((udid.clone(), assignment, revision));
        }
    }
    for (udid, campaign) in by_campaign.keys() {
        if covered.insert((udid.clone(), format!("publish:{campaign}"))) {
            publish_rows.push((needs.len(), udid.clone(), campaign.clone()));
            needs.push(NeedsRelease {
                envelope: String::new(),
                udid: udid.clone(),
                owner: "publish".into(),
                title: campaign.clone(),
                releasable: true,
                message: "Xác nhận sẽ loại bài cũ của lượt này khỏi máy và giữ việc kiểm tra link trong Theo dõi".into(),
            });
        }
    }
    for (row, udid, campaign) in publish_rows {
        let mut exact = by_campaign
            .get(&(udid.clone(), campaign.clone()))
            .cloned()
            .unwrap_or_default();
        exact.sort();
        needs[row].envelope = format!("publish:{udid}:{campaign}:{}", exact.join(","));
    }
    needs.sort_by(|a, b| a.udid.cmp(&b.udid));
    Ok(ReleasePlan {
        needs,
        operations,
        assignments,
        nurture,
    })
}

#[tauri::command]
pub async fn operation_prepare_devices(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    udids: Vec<String>,
    selected_only: Option<bool>,
) -> Result<OperationStopResult, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let handoff = lock_manual_handoff()?;
    if selected_only.unwrap_or(false) {
        // This command is itself the operator's explicit release of the selection, so it
        // approves exactly the holders it observes now (V6 replaces it with a reviewed list).
        let approved: Vec<String> = observe_release_needs(&state, &udids)?
            .into_iter()
            .map(|row| row.envelope)
            .collect();
        return prepare_publish_devices(
            &app,
            &state,
            udids,
            &approved,
            None,
            &handoff,
            tokio::time::Instant::now() + std::time::Duration::from_secs(125),
        )
        .await;
    }
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
    approved_release: &[String],
    account_lock: Option<&str>,
    _handoff: &tokio::sync::MutexGuard<'static, ()>,
    deadline: tokio::time::Instant,
) -> Result<OperationStopResult, CommandError> {
    let timing = StopTiming::new("publishHandoff");
    if tokio::time::Instant::now() >= deadline {
        return Err(CommandError::operation("Đã hết thời gian chờ lượt nhả thiết bị; chưa thay đổi tác vụ cũ"));
    }
    if udids.is_empty() || udids.len() > 500 {
        return Err(CommandError::invalid_argument("Chọn từ 1 đến 500 thiết bị"));
    }
    let selected: HashSet<_> = udids.into_iter().collect();
    let mut devices: Vec<_> = selected.iter().cloned().collect();
    devices.sort();
    // (1) Fence: from here until the phones are closed, no new work starts or re-acquires on
    // them through the ownership coordinator; only this handoff's own close may acquire.
    let _fence = state.control.fence_new_work(&devices);
    // (2) One observation under the fence is the plan, compared with what the operator
    // reviewed, and then applied exactly: no later enumeration adds a stop.
    let plan = observe_release_plan(state, &devices)?;
    if let Some(blocked) = plan.needs.iter().find(|row| !row.releasable) {
        return Err(CommandError::operation(format!("{}: {}", blocked.udid, blocked.message)));
    }
    if let Some(refusal) = unapproved_release(&plan.needs, approved_release) {
        return Err(CommandError::operation(refusal));
    }
    // Review R3-F1: a publish Start's phones have been account-locked since its acceptance.
    // This creation's own lock generation and the accounts it recorded are compared again
    // before the first exclusion or stop, so a stale approval refuses with no old owner changed.
    if let Some(generation) = account_lock {
        if let Some(refusal) = state
            .db
            .publish_account_lock_drift(generation, &devices)
            .map_err(CommandError::operation)?
        {
            return Err(CommandError::operation(refusal));
        }
    }
    let mut stopped = HandoffStops { scoped: true, ..Default::default() };
    for (id, scope) in &plan.operations {
        stopped.operation_devices.insert(id.clone(), scope.clone());
        stopped.operations.push(id.clone());
    }
    // (3) Exclusions at the approved revision (CAS): a row that moved since the review
    // refuses before any further stop. This fences the selected verifier as well as the
    // composer; no campaign-wide stop marker is written.
    for (udid, assignment, revision) in &plan.assignments {
        let request = uuid::Uuid::new_v4().to_string();
        state.db.request_publish_handoff_exclusion(assignment, *revision, &request)
            .map_err(|error| CommandError::operation(format!(
                "{udid}: bài đăng cũ đã đổi so với lúc xem lại ({error}); kiểm tra lại rồi xác nhận")))?;
        stopped
            .assignments
            .insert(assignment.clone(), (udid.clone(), request));
    }
    // (4) Exact Nuôi runs, compared with the reviewed run id under the runtime's lock. A
    // newer run on the same phone is never stopped; it refuses the handoff instead.
    for (udid, run) in &plan.nurture {
        if state.nurture.stop_run(udid, *run) == crate::nurture_commands::NurtureRunStop::Replaced {
            return Err(CommandError::operation(format!(
                "{udid}: phiên Nuôi khác với lúc xem lại đang giữ máy; chưa dừng phiên đó, kiểm tra lại rồi xác nhận")));
        }
    }
    // (5) Operations by id, only those in the plan.
    for id in &stopped.operations {
        super::operation_stop(app.clone(), app.state(), id.clone()).await?;
    }
    for udid in &devices {
        state.end_overlay_session(udid).await?;
    }
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

/// Helper qualification may acquire Repair after the handoff's first idle check.
/// Retry admission only; never revoke that owner or drop an in-flight acquisition.
async fn acquire_handoff_context(
    control: &DeviceControlPlane,
    udid: &str,
    deadline: tokio::time::Instant,
) -> Result<DeviceExclusiveContext, CommandError> {
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(CommandError::operation(
                "Đã hết thời gian chờ nhả quyền điều khiển; chưa đóng ứng dụng",
            ));
        }
        match control
            .try_acquire_exclusive_keeping_stream(udid, DeviceWorkOwner::ManualControl)
            .await
        {
            Ok(context) => return Ok(context),
            Err(riviu_core::DeviceControlError::Busy(_)) => {
                tokio::time::sleep_until(std::cmp::min(
                    deadline,
                    tokio::time::Instant::now() + std::time::Duration::from_millis(250),
                ))
                .await;
            }
            Err(error) => return Err(CommandError::from(error)),
        }
    }
}

async fn close_handoff_device(
    state: &AppState,
    udid: &str,
    authorized: &HandoffStops,
    deadline: tokio::time::Instant,
    timing: StopTiming,
) -> Result<(), CommandError> {
    let wait_timing = timing.phase("wait", None);
    let context = acquire_handoff_context(&state.control, udid, deadline).await?;
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
        // What a confirmed handoff may stop is the reviewed envelope, not whatever holds the
        // phone now (review F1): another job on the same phone refuses before any stop.
        let row = |owner: &str, envelope: &str| NeedsRelease {
            udid: "b".into(),
            owner: owner.into(),
            title: "Lượt cũ".into(),
            releasable: true,
            message: String::new(),
            envelope: envelope.into(),
        };
        let reviewed = vec!["operation:b:nurture:run-1".to_owned()];
        assert_eq!(
            unapproved_release(&[row("nurture", "operation:b:nurture:run-1")], &reviewed),
            None
        );
        let refusal = unapproved_release(
            &[row("interaction", "operation:b:interaction:run-2")],
            &reviewed,
        )
        .expect("a different job on a reviewed phone is not approved");
        assert!(
            refusal.starts_with("b: ") && refusal.contains("interaction"),
            "{refusal}"
        );
        assert_eq!(
            unapproved_release(&[], &reviewed),
            None,
            "a holder that left needs no approval"
        );
        assert_eq!(
            unapproved_release(&[row("IdleSweep", "lease:b:IdleSweep:token")], &[]),
            None,
            "momentary owners are not part of the approval"
        );
    }
    #[tokio::test(start_paused = true)]
    async fn handoff_waits_for_repair_without_stealing_or_outliving_its_deadline() {
        use riviu_core::{DeviceWorkCoordinator, StreamBudgetManager};
        use riviu_ios_driver::MockIosDriver;
        use std::task::Poll;
        use std::time::Duration;

        let driver = MockIosDriver::new();
        let control = DeviceControlPlane::new(
            Arc::new(driver.clone()),
            Arc::new(DeviceWorkCoordinator::new()),
            Arc::new(StreamBudgetManager::default()),
        );
        let udid = "handoff-repair";
        let repair = control
            .try_acquire_exclusive_keeping_stream(udid, DeviceWorkOwner::Repair)
            .await
            .expect("helper qualification owns the device");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let admission = acquire_handoff_context(&control, udid, deadline);
        tokio::pin!(admission);
        assert!(
            matches!(futures_util::poll!(&mut admission), Poll::Pending),
            "handoff must wait for transient Repair ownership instead of failing immediately"
        );
        assert_eq!(control.current_work_owner(udid), Some(DeviceWorkOwner::Repair));
        control.close_exclusive_context(repair).expect("Repair releases its own valid lease");
        tokio::time::advance(Duration::from_millis(250)).await;
        let manual = admission.await.expect("handoff acquires after Repair releases");
        assert_eq!(manual.owner(), DeviceWorkOwner::ManualControl);
        control.close_exclusive_context(manual).expect("handoff releases its own lease");

        let repair = control
            .try_acquire_exclusive_keeping_stream(udid, DeviceWorkOwner::Repair)
            .await
            .expect("a later Repair still owns the device");
        let deadline = tokio::time::Instant::now() + Duration::from_millis(100);
        let admission = acquire_handoff_context(&control, udid, deadline);
        tokio::pin!(admission);
        assert!(matches!(futures_util::poll!(&mut admission), Poll::Pending));
        tokio::time::advance(Duration::from_millis(100)).await;
        let error = admission.await.expect_err("existing handoff deadline bounds admission");
        assert_eq!(error.code, "OperationFailed");
        assert_eq!(tokio::time::Instant::now(), deadline);
        assert_eq!(control.current_work_owner(udid), Some(DeviceWorkOwner::Repair));
        control.close_exclusive_context(repair).expect("timeout must not revoke Repair");
        assert!(acquire_handoff_context(&control, udid, deadline).await.is_err());
        assert_eq!(control.current_work_owner(udid), None);
        assert_eq!(driver.ordinary_session_calls(), 0);
        assert_eq!(driver.stream_restart_calls(), 0);

        // Review R2-F1: while a handoff applies its plan, a run arriving after the envelope
        // comparison cannot take the phone (so it is never the thing stopped), and the
        // handoff's own close still can.
        let fence = control.fence_new_work(&[udid.to_owned()]);
        assert!(matches!(
            control
                .try_acquire_exclusive_keeping_stream(udid, DeviceWorkOwner::Interaction)
                .await,
            Err(riviu_core::DeviceControlError::Busy(_))
        ));
        let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
        let manual = acquire_handoff_context(&control, udid, deadline)
            .await
            .expect("the handoff's own close passes its fence");
        assert_eq!(manual.owner(), DeviceWorkOwner::ManualControl);
        control
            .close_exclusive_context(manual)
            .expect("handoff releases its own lease");
        drop(fence);
        let later = control
            .try_acquire_exclusive_keeping_stream(udid, DeviceWorkOwner::Interaction)
            .await
            .expect("new work starts once the handoff has finished");
        control.close_exclusive_context(later).expect("release");
    }

}
