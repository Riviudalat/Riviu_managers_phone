//! On-demand observation; does not settle or retry historical action journals.
use super::*;
use riviu_core::{
    tiktok_labels::{TikTokControl, TikTokControls},
    SaveAdapter,
};
use sha2::{Digest, Sha256};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountReading {
    pub udid: String,
    pub expected_handle: String,
    pub observed_handle: Option<String>,
    pub status: &'static str,
    pub checked_at: String,
    pub snapshot_sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionReadback {
    pub assignment_id: String,
    pub target_url: String,
    pub checked_at: String,
    pub like: &'static str,
    pub save: riviu_core::BookmarkState,
    pub snapshot_sha256: String,
}

async fn labels(control: &DeviceControlPlane, udid: &str) -> anyhow::Result<TikTokControls> {
    let (package, version, language) = control.tiktok_build(udid).await?;
    riviu_core::tiktok_labels::controls_for(&package, &language, &version)
        .ok_or_else(|| anyhow::anyhow!("Chưa đo bản TikTok trên máy này"))
}

pub(super) async fn read_account(
    control: &DeviceControlPlane,
    db: &riviu_core::db::Database,
    udid: &str,
) -> anyhow::Result<AccountReading> {
    let expected = db.get_device_meta(udid)?.handle;
    let device = open_interaction_context(control, udid).await?;
    let result = async {
        let labels = labels(control, udid).await?;
        anyhow::ensure!(
            riviu_core::tiktok_account::account_read_supported(labels),
            "Chưa hỗ trợ đọc tài khoản trên bản TikTok/ngôn ngữ này"
        );
        let session = control.streaming_session(&device.context)?;
        let profile = labels
            .label(TikTokControl::ProfileTab)
            .ok_or_else(|| anyhow::anyhow!("Chưa đo tab Hồ sơ"))?;
        let button = session
            .locate(profile.to_query())
            .await?
            .ok_or_else(|| anyhow::anyhow!("Không thấy tab Hồ sơ"))?;
        session.tap(button.centre()).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
        let observed = loop {
            let observed =
                riviu_core::tiktok_account::observe_own_account(session.as_ref(), labels).await?;
            if observed.is_some() || tokio::time::Instant::now() >= deadline {
                break observed;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        };
        let current = db.get_device_meta(udid)?.handle;
        anyhow::ensure!(
            current == expected,
            "Nick đã gán vừa thay đổi; kiểm tra lại"
        );
        let status = match &observed {
            None => "unknown",
            Some(_) if expected.trim().is_empty() => "unassigned",
            Some(handle)
                if handle.eq_ignore_ascii_case(expected.trim().trim_start_matches('@')) =>
            {
                "matched"
            }
            Some(_) => "mismatch",
        };
        Ok(AccountReading {
            udid: udid.into(),
            expected_handle: expected,
            observed_handle: observed,
            status,
            checked_at: chrono::Utc::now().to_rfc3339(),
            snapshot_sha256: format!(
                "{:x}",
                Sha256::digest(session.hierarchy_source_snapshot().await?.xml.as_bytes())
            ),
        })
    }
    .await;
    let closed = control.close_ui_context(device.context).await;
    let reading = result?;
    closed?;
    Ok(reading)
}

pub(super) async fn readback(
    control: &DeviceControlPlane,
    db: &riviu_core::db::Database,
    campaign: &str,
    assignment: &str,
) -> anyhow::Result<ActionReadback> {
    let detail = db
        .get_interaction_campaign(campaign)?
        .ok_or_else(|| anyhow::anyhow!("Chiến dịch không còn tồn tại"))?;
    anyhow::ensure!(
        detail.summary.state != ThreadCampaignState::Running,
        "Chờ chiến dịch dừng trước khi đọc lại"
    );
    let row = detail
        .assignments
        .iter()
        .find(|row| row.id == assignment)
        .ok_or_else(|| anyhow::anyhow!("Không tìm thấy lượt của máy"))?;
    let (request, _) = db
        .get_interaction_campaign_request(campaign)?
        .ok_or_else(|| anyhow::anyhow!("Thiếu snapshot chiến dịch"))?;
    let target = request
        .targets
        .iter()
        .find(|t| t.target_key == row.target_key)
        .ok_or_else(|| anyhow::anyhow!("Thiếu bài gốc"))?;
    let device = open_interaction_context(control, &row.actor_udid).await?;
    let result = async {
        let labels = labels(control, &row.actor_udid).await?;
        let session = control.streaming_session(&device.context)?;
        let arrival = riviu_core::interaction_hierarchy::open_target_by_hierarchy(
            session.as_ref(),
            labels,
            &device.target_package,
            &target.normalized_url,
            &target.author,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .await
        .map_err(|e| anyhow::anyhow!(e.code()))?;
        // Even a matching author can own several posts: this readback requires the exact URL.
        let _ = arrival;
        riviu_core::interaction_hierarchy::confirm_target_from_share_link(
            session.as_ref(),
            labels,
            target,
        )
        .await?;
        let mut adapter = riviu_core::HierarchySaveAdapter::new(session.as_ref(), labels);
        let before = adapter.observe().await?;
        let liked = match labels.label(TikTokControl::Liked) {
            Some(label) => session.locate(label.to_query()).await?.is_some(),
            None => false,
        };
        let like = match labels.label(TikTokControl::Like) {
            Some(label) => session.locate(label.to_query()).await?.is_some(),
            None => false,
        };
        let after = adapter.observe().await?;
        anyhow::ensure!(
            before.identity.is_some() && before.identity == after.identity,
            "Bài đã đổi trong lúc kiểm tra"
        );
        Ok(ActionReadback {
            assignment_id: assignment.into(),
            target_url: target.normalized_url.clone(),
            checked_at: chrono::Utc::now().to_rfc3339(),
            like: if liked {
                "present"
            } else if like {
                "absent"
            } else {
                "unknown"
            },
            save: after.state,
            snapshot_sha256: format!(
                "{:x}",
                Sha256::digest(session.hierarchy_source_snapshot().await?.xml.as_bytes())
            ),
        })
    }
    .await;
    let closed = control.close_ui_context(device.context).await;
    let reading = result?;
    closed?;
    Ok(reading)
}
