//! Multiple daily slots reuse the durable single-campaign scheduler and its effect guards.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishScheduleSlot {
    pub bundle_id: String,
    pub udid: String,
    pub run_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishScheduleRequest {
    pub request_id: String,
    pub source_root: String,
    pub slots: Vec<PublishScheduleSlot>,
    pub caption_overrides: std::collections::BTreeMap<String, String>,
    pub sound_policy: riviu_core::PublishSoundPolicy,
    pub sheet_enabled: bool,
    pub delete_after_publish: bool,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishScheduleReport {
    pub input_digest: String,
    pub can_execute: bool,
    pub slots: Vec<riviu_core::PublishPreflightReport>,
}

fn validate_schedule(request: &PublishScheduleRequest) -> anyhow::Result<()> {
    Uuid::parse_str(&request.request_id).context("invalid schedule request ID")?;
    anyhow::ensure!(
        (1..=100).contains(&request.slots.len()),
        "chọn từ 1 đến 100 khung giờ"
    );
    let mut bundles = HashSet::new();
    let mut times = HashSet::new();
    for slot in &request.slots {
        let at = execution::parse_run_at(&slot.run_at).map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            !slot.udid.trim().is_empty() && !slot.bundle_id.trim().is_empty(),
            "mỗi khung giờ phải chọn bài và máy"
        );
        anyhow::ensure!(
            bundles.insert(&slot.bundle_id),
            "mỗi khung giờ phải dùng một bài khác nhau"
        );
        anyhow::ensure!(
            times.insert((&slot.udid, at)),
            "một máy không thể có hai bài cùng thời điểm"
        );
    }
    Ok(())
}
fn slot_id(request: &PublishScheduleRequest, index: usize) -> String {
    format!("schedule-{}-{index}", request.request_id)
}
fn request_digest(request: &PublishScheduleRequest) -> anyhow::Result<String> {
    Ok(execution::frame_sha256(&serde_json::to_vec(request)?))
}

async fn prepare_schedule(
    state: &AppState,
    request: &PublishScheduleRequest,
) -> anyhow::Result<(
    PublishScheduleReport,
    Vec<preflight::PreparedPublishPreflight>,
)> {
    validate_schedule(request)?;
    let manifest = preflight::scan_preflight_source(&request.source_root).await?;
    let sheet_choice =
        preflight::verify_sheet_delivery_choice(&state.db, request.sheet_enabled).await;
    let mut prepared = Vec::new();
    for (index, slot) in request.slots.iter().enumerate() {
        let input = riviu_core::PublishPreflightRequest {
            source_root: request.source_root.clone(),
            bundle_ids: vec![slot.bundle_id.clone()],
            udids: vec![slot.udid.clone()],
            target_ref: Some(riviu_core::TargetRef::Explicit {
                udids: vec![slot.udid.clone()],
            }),
            run_at: Some(slot.run_at.clone()),
            caption_overrides: request
                .caption_overrides
                .iter()
                .filter(|(id, _)| *id == &slot.bundle_id)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            sound_policy: request.sound_policy.clone(),
            sheet_enabled: request.sheet_enabled,
            delete_after_publish: request.delete_after_publish,
        };
        let mut checked = preflight::build_publish_preflight_from_manifest_with_sheet(
            &state.control,
            &state.registry,
            &state.db,
            input,
            &manifest,
            sheet_choice.clone(),
        )
        .await?;
        if state.db.publish_schedule_time_conflicts(
            &slot.udid,
            &slot.run_at,
            &slot_id(request, index),
        )? {
            let issue = preflight::preflight_issue(
                "schedule_time_conflict",
                &slot.udid,
                &slot.bundle_id,
                &format!(
                    "Máy đã có lịch lúc {}. Đổi giờ hoặc hủy lịch cũ.",
                    slot.run_at
                ),
            );
            checked.report.can_execute = false;
            checked.report.issues.push(issue.clone());
            if let Some(row) = checked.report.assignments.first_mut() {
                row.issues.push(issue);
            }
        }
        prepared.push(checked);
    }
    let slots = prepared
        .iter()
        .map(|p| p.report.clone())
        .collect::<Vec<_>>();
    let digest = execution::frame_sha256(&serde_json::to_vec(
        &serde_json::json!({"request":request,"digests":slots.iter().map(|r|&r.input_digest).collect::<Vec<_>>()}),
    )?);
    Ok((
        PublishScheduleReport {
            input_digest: digest,
            can_execute: slots.iter().all(|r| r.can_execute),
            slots,
        },
        prepared,
    ))
}

fn replay_schedule(
    db: &Database,
    request: &PublishScheduleRequest,
) -> anyhow::Result<Option<Vec<PublishCampaignRecord>>> {
    let mut records = Vec::new();
    let digest = request_digest(request)?;
    for i in 0..request.slots.len() {
        let id = slot_id(request, i);
        if let Some(detail) = db.get_publish_campaign(&id)? {
            let snapshot = db
                .get_publish_execution_snapshot(&id)?
                .context("schedule snapshot missing")?;
            // Initial snapshot can be replaced during execution; request ID remains immutable.
            let stored = db
                .publish_campaign_request(&id)?
                .context("schedule request missing")?;
            anyhow::ensure!(
                stored.request_id == format!("{}:{i}:{digest}", request.request_id),
                "schedule request changed; create a new schedule"
            );
            let _ = snapshot;
            records.push(detail.campaign);
        }
    }
    if records.is_empty() {
        return Ok(None);
    }
    anyhow::ensure!(
        records.len() == request.slots.len(),
        "incomplete schedule batch"
    );
    Ok(Some(records))
}

#[tauri::command]
pub async fn publish_schedule_preflight(
    state: State<'_, AppState>,
    request: PublishScheduleRequest,
) -> Result<PublishScheduleReport, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    prepare_schedule(&state, &request)
        .await
        .map(|(r, _)| r)
        .map_err(preflight::err)
}

#[tauri::command]
pub async fn publish_schedule_create(
    state: State<'_, AppState>,
    request: PublishScheduleRequest,
    approved_input_digest: String,
    confirmed: bool,
) -> Result<Vec<PublishCampaignRecord>, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    if !confirmed {
        return Err(preflight::err("cần xác nhận lịch đăng công khai"));
    }
    if let Some(records) = replay_schedule(&state.db, &request).map_err(preflight::err)? {
        return Ok(records);
    }
    let (report, prepared) = prepare_schedule(&state, &request)
        .await
        .map_err(preflight::err)?;
    if !report.can_execute || report.input_digest != approved_input_digest {
        return Err(preflight::err(
            "lịch hoặc đầu vào đã thay đổi; kiểm tra lại",
        ));
    }
    let staging = state
        .artifacts_dir
        .join("publish-schedules")
        .join(Uuid::new_v4().to_string());
    let result: anyhow::Result<Vec<PublishCampaignRecord>> = (|| {
        let digest = request_digest(&request)?;
        let mut records = Vec::new();
        for (index, (slot, prepared)) in request.slots.iter().zip(prepared).enumerate() {
            let id = slot_id(&request, index);
            let selected = prepared
                .bundles
                .first()
                .context("schedule bundle missing")?;
            let mut bundle = copy_bundle_to_managed(selected, &staging.join(index.to_string()))?;
            bundle.id = format!("{id}:{}", selected.id);
            let run_at = execution::parse_run_at(&slot.run_at)
                .map_err(anyhow::Error::msg)?
                .format("%Y-%m-%dT%H:%M:%S")
                .to_string();
            let campaign = PublishCampaignRequest {
                sheet_delivery: prepared.report.sheet_delivery.clone(),
                verification_contract_version: Some(1),
                verification_builds: riviu_core::publish_submission::builds_from_preflight(
                    &prepared.report,
                ),
                request_id: format!("{}:{index}:{digest}", request.request_id),
                source_root: request.source_root.clone(),
                bundle_ids: vec![bundle.id.clone()],
                udids: vec![slot.udid.clone()],
                run_at: Some(run_at),
                visibility: PublishVisibility::Public,
                cleanup_policy: if request.delete_after_publish {
                    PublishCleanupPolicy::DeleteImportedAssetsAfterVerified
                } else {
                    PublishCleanupPolicy::KeepImportedAssets
                },
                network: riviu_core::SocialNetwork::TikTok,
                sound_policy: request.sound_policy.clone(),
                sheet_enabled: request.sheet_enabled,
                execution_confirmed: true,
                target_snapshot: Some(prepared.report.target_snapshot.clone()),
            };
            let snapshot = riviu_core::PublishExecutionSnapshotDraft {
                input_digest: prepared.report.input_digest.clone(),
                status: riviu_core::PublishExecutionStatus::Partial,
                retry_scope: riviu_core::PublishRetryScope::FullPipeline,
                report_json: serde_json::to_value(&prepared.report)?,
            };
            records.push((id, campaign, vec![bundle], snapshot));
        }
        state.db.create_publish_schedule_batch(&records)
    })();
    match result {
        Ok(records) => {
            for record in &records {
                execution::announce(&state.events, &state.db, &record.id);
            }
            Ok(records)
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            if let Some(records) = replay_schedule(&state.db, &request).map_err(preflight::err)? {
                return Ok(records);
            }
            Err(preflight::err(error))
        }
    }
}

#[tauri::command]
pub fn publish_schedule_reschedule(
    state: State<'_, AppState>,
    campaign_id: String,
    expected_updated_at: String,
    run_at: String,
) -> Result<PublishCampaignRecord, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let at = execution::parse_run_at(&run_at).map_err(preflight::err)?;
    let record = state
        .db
        .reschedule_publish_campaign(
            &campaign_id,
            &expected_updated_at,
            &at.format("%Y-%m-%dT%H:%M:%S").to_string(),
        )
        .map_err(preflight::err)?;
    execution::announce(&state.events, &state.db, &campaign_id);
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schedule_requires_distinct_posts_and_distinct_times_per_phone() {
        let at = Local::now().naive_local() + chrono::Duration::days(1);
        let mut r = PublishScheduleRequest {
            request_id: Uuid::new_v4().to_string(),
            source_root: "fixture".into(),
            slots: vec![
                PublishScheduleSlot {
                    bundle_id: "a".into(),
                    udid: "one".into(),
                    run_at: at.format("%Y-%m-%dT%H:%M").to_string(),
                },
                PublishScheduleSlot {
                    bundle_id: "b".into(),
                    udid: "one".into(),
                    run_at: (at + chrono::Duration::minutes(5))
                        .format("%Y-%m-%dT%H:%M")
                        .to_string(),
                },
            ],
            caption_overrides: Default::default(),
            sound_policy: Default::default(),
            sheet_enabled: false,
            delete_after_publish: false,
        };
        assert!(validate_schedule(&r).is_ok());
        r.slots[1].run_at = r.slots[0].run_at.clone();
        assert!(validate_schedule(&r).is_err());
        r.slots[1].udid = "two".into();
        assert!(validate_schedule(&r).is_ok());
        r.slots[1].bundle_id = "a".into();
        assert!(validate_schedule(&r).is_err());
    }
}
