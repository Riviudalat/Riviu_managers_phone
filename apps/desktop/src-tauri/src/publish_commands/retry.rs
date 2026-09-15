//! Explicit retry of one publication known not to have crossed the Post boundary.
use super::*;

#[tauri::command]
pub async fn publish_retry_assignment(
    state: State<'_, AppState>,
    assignment_id: String,
    confirmed: bool,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let result = async {
        anyhow::ensure!(confirmed, "Cần xác nhận thử lại đúng bài trên máy đã chọn");
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
        // Transfer and composer revalidate the frozen media, account and UI before Post.
        let run = state
            .db
            .claim_publish_assignment_retry(&assignment_id)?
            .context("Bài đã thay đổi hoặc đang có lượt chạy")?;
        execution::announce(&state.events, &state.db, &run.campaign_id);
        Ok::<_, anyhow::Error>(())
    }
    .await;
    result.map_err(preflight::err)
}
