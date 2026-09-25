//! Manual replacement of old runs uses their existing Stop owners and receipts.
use super::*;
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
    if udids.is_empty() || udids.len() > 500 {
        return Err(CommandError::invalid_argument("Chọn từ 1 đến 500 thiết bị"));
    }
    let selected: HashSet<_> = udids.into_iter().collect();
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
    let mut ids: HashSet<String> = state
        .db
        .operation_source_ids(None, None)
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
    ids.extend(held.iter().cloned());
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
    operations.sort();
    // Request every relevant cancellation before waiting for any closer. Runs
    // with interdependent steps are cancelled as a unit by their original owner.
    let mut stopped = HandoffStops {
        operations,
        operation_devices,
        ..Default::default()
    };
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
    let mut close_results = std::collections::HashMap::new();
    for id in active.difference(&not_released) {
        // Multiple stopped publications can block each other's individual
        // closer. The explicit handoff owns all of these old stop generations.
        close_results.insert(id.clone(), close_handoff_device(state, id, &stopped).await);
    }
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

async fn close_handoff_device(
    state: &AppState,
    udid: &str,
    authorized: &HandoffStops,
) -> Result<(), CommandError> {
    let context = state
        .control
        .try_acquire_exclusive_keeping_stream(udid, DeviceWorkOwner::ManualControl)
        .await
        .map_err(CommandError::from)?;
    let result = async {
        for id in state
            .db
            .operation_source_ids(None, None)
            .map_err(CommandError::operation)?
        {
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
        let mut proofs = Vec::new();
        for hold in state
            .db
            .publish_device_guard(udid)
            .map_err(CommandError::operation)?
            .blocking
        {
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
        Ok(proofs)
    }
    .await;
    state
        .control
        .close_exclusive_context(context)
        .map_err(CommandError::from)?;
    for (campaign, marker) in result? {
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
