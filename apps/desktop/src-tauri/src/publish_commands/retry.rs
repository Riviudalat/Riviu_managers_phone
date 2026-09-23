//! Explicit retry of one publication known not to have crossed the Post boundary.
use super::*;

#[tauri::command]
pub async fn publish_retry_sheet_assignment(
    state: State<'_, AppState>,
    assignment_id: String,
    expected_revision: i64,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let result = async {
        anyhow::ensure!(
            state.db.publish_assignment_revision(&assignment_id)? == expected_revision,
            "Bài đã thay đổi; tải lại tiến độ"
        );
        let campaign = state
            .db
            .publication_campaign_id(&assignment_id)?
            .context("Không tìm thấy bài")?;
        let detail = state
            .db
            .get_publish_assignment_detail(&campaign, &assignment_id)?
            .context("Không tìm thấy bài")?;
        let row = detail.assignments.first().context("Không tìm thấy bài")?;
        let e: serde_json::Value =
            serde_json::from_str(row.evidence_json.as_deref().unwrap_or("{}"))?;
        let post = e.get("post").unwrap_or(&e);
        anyhow::ensure!(
            post["publicationVerified"] == true
                && post["postUrl"]
                    .as_str()
                    .is_some_and(riviu_core::tiktok_share::looks_like_a_post_link),
            "Chưa có URL bài được xác minh; chưa ghi lại Sheet"
        );
        anyhow::ensure!(
            state.db.retry_bound_sheet_assignment(
                &assignment_id,
                chrono::Utc::now().timestamp_millis()
            )?,
            "Sheet đã ghi hoặc đích báo cáo không còn hiệu lực"
        );
        execution::announce(&state.events, &state.db, &campaign);
        Ok::<_, anyhow::Error>(())
    }
    .await;
    result.map_err(preflight::err)
}

#[tauri::command]
pub async fn publish_retry_assignment(
    state: State<'_, AppState>,
    assignment_id: String,
    confirmed: bool,
    expected_revision: i64,
    request_id: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let result = async {
        anyhow::ensure!(confirmed, "Cần xác nhận thử lại đúng bài trên máy đã chọn");
        if state
            .db
            .acknowledged_publish_retry(&assignment_id, expected_revision, &request_id)?
        {
            return Ok(());
        }
        let campaign = state
            .db
            .publication_campaign_id(&assignment_id)?
            .context("Không tìm thấy bài")?;
        let detail = state
            .db
            .get_publish_assignment_detail(&campaign, &assignment_id)?
            .context("Không tìm thấy bài")?;
        let a = detail.assignments.first().context("Không tìm thấy bài")?;
        anyhow::ensure!(
            a.state == riviu_core::PublishCampaignState::FailedBeforeDispatch
                && a.effect_intent.is_none(),
            "Chỉ thử lại bài chưa gửi; bài đã gửi cần xác minh liên kết"
        );
        let request = state
            .db
            .publish_campaign_request(&detail.campaign.id)?
            .context("Thiếu cấu hình bài")?;
        anyhow::ensure!(
            request.execution_confirmed && request.verification_contract_version == Some(1),
            "Bài thiếu xác nhận hoặc hợp đồng xác minh"
        );
        anyhow::ensure!(
            !matches!(
                detail.campaign.state,
                riviu_core::PublishCampaignState::Cancelled
                    | riviu_core::PublishCampaignState::Missed
            ),
            "Lượt đã hủy hoặc lỡ lịch"
        );
        if let Some(at) = detail.campaign.run_at.as_deref() {
            let at = NaiveDateTime::parse_from_str(at, "%Y-%m-%dT%H:%M:%S")
                .or_else(|_| NaiveDateTime::parse_from_str(at, "%Y-%m-%dT%H:%M"))?;
            anyhow::ensure!(at <= Local::now().naive_local(), "Lịch chưa tới giờ");
        }
        anyhow::ensure!(
            state.control.current_work_owner(&a.udid).is_none(),
            "Máy đang bận việc khác"
        );
        anyhow::ensure!(
            state
                .control
                .list_devices()
                .await?
                .iter()
                .any(|d| d.udid == a.udid
                    && matches!(
                        d.status,
                        riviu_core::DeviceStatus::Ready | riviu_core::DeviceStatus::Connected
                    )),
            "Máy chưa kết nối; kết nối đúng điện thoại rồi thử lại"
        );
        // Transfer and composer revalidate the frozen media, account and UI before Post.
        let run = state
            .db
            .claim_publish_assignment_retry_checked(&assignment_id, expected_revision, &request_id)?
            .context("Bài đã thay đổi hoặc đang có lượt chạy")?;
        execution::announce(&state.events, &state.db, &run.campaign_id);
        Ok::<_, anyhow::Error>(())
    }
    .await;
    result.map_err(preflight::err)
}
