//! On-demand observation; does not settle or retry historical action journals.
use super::*;
use anyhow::Context;
use riviu_core::{
    tiktok_labels::{TikTokControl, TikTokControls},
    SaveAdapter,
};
use sha2::{Digest, Sha256};

pub(super) async fn comment_link(
    control: &DeviceControlPlane,
    db: &riviu_core::db::Database,
    campaign: &str,
    assignment: &str,
    receiver: Option<&str>,
    capture_device: Option<&str>,
) -> anyhow::Result<riviu_core::tiktok_comment_link::SharedCommentLink> {
    let detail = db
        .get_interaction_campaign(campaign)?
        .context("Campaign missing")?;
    anyhow::ensure!(
        detail.summary.state != ThreadCampaignState::Running,
        "Chờ tác vụ đang chạy kết thúc"
    );
    let row = detail
        .assignments
        .iter()
        .find(|a| a.id == assignment)
        .context("Comment missing")?;
    anyhow::ensure!(
        row.comment_verification.as_ref().is_some_and(
            |v| v.state == riviu_core::comment_verification::VerificationState::Verified
        ),
        "Chỉ lấy ID của bình luận đã xác minh"
    );
    let identity = row.posted_identity().context("Comment evidence missing")?;
    if receiver.is_none() && capture_device.is_none() {
        if let Some(link) = &identity.comment_link {
            return Ok(link.clone());
        }
    }
    let (request, _) = db
        .get_interaction_campaign_request(campaign)?
        .context("Request missing")?;
    let target = request
        .targets
        .iter()
        .find(|t| t.target_key == row.target_key)
        .context("Post missing")?;
    anyhow::ensure!(
        receiver.is_none() || capture_device.is_none(),
        "Chọn lấy ID hoặc mở ID, không dùng cả hai"
    );
    let actor = receiver.or(capture_device).unwrap_or(&row.actor_udid);
    anyhow::ensure!(
        request.actor_udids.iter().any(|s| s == actor),
        "Máy nằm ngoài phạm vi chiến dịch"
    );
    let device = open_interaction_context(control, actor).await?;
    let result = async {
        let labels = labels(control, actor).await?;
        anyhow::ensure!(
            riviu_core::tiktok_comment_link::supported(labels),
            "Chưa đo lấy ID bình luận trên bản TikTok này"
        );
        let session = control.streaming_session(&device.context)?;
        let stop = std::sync::atomic::AtomicBool::new(false);
        session.set_gui_scope(riviu_core::ui_automation::GuiScope {
            run_id: campaign.into(),
            assignment_id: Some(assignment.into()),
            device_id: actor.into(),
            deadline_ms: Some(chrono::Utc::now().timestamp_millis() + 150000),
        });
        if receiver.is_some() {
            let found = riviu_core::tiktok_comment_link::open_and_verify(
                session.as_ref(),
                labels,
                &identity,
                &stop,
            )
            .await?;
            return found
                .identity
                .comment_link
                .context("Missing copied comment ID");
        }
        riviu_core::interaction_hierarchy::open_pinned_target_by_hierarchy(
            session.as_ref(),
            labels,
            &device.target_package,
            target,
            &stop,
        )
        .await?;
        riviu_core::comment_verification::search::open_drawer(session.as_ref(), labels).await?;
        let root = row
            .parent_assignment_id
            .as_ref()
            .and_then(|id| detail.assignments.iter().find(|a| &a.id == id))
            .and_then(|a| a.posted_identity());
        let found = riviu_core::comment_verification::search::find(
            session.as_ref(),
            labels,
            &identity.text,
            Some(&identity.author_label),
            root.as_ref(),
            &stop,
        )
        .await?;
        let account = request
            .seeding
            .as_ref()
            .and_then(|s| s.expected_accounts.get(&row.actor_udid))
            .cloned()
            .unwrap_or(db.get_device_meta(&row.actor_udid)?.handle);
        anyhow::ensure!(!account.is_empty(), "Comment author account missing");
        riviu_core::comment_verification::search::verify_author(
            session.as_ref(),
            labels,
            &found,
            &account,
        )
        .await?;
        let link = riviu_core::tiktok_comment_link::capture(
            session.as_ref(),
            labels,
            &identity,
            &target.content_id,
            &stop,
        )
        .await?;
        db.store_comment_link(campaign, assignment, &identity, &link)?;
        Ok(link)
    }
    .await;
    let closed = control.close_ui_context(device.context).await;
    let link = result?;
    closed?;
    Ok(link)
}

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
    pub follow: Option<&'static str>,
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
    let device = open_interaction_context(control, udid).await?;
    let result = async {
        let session = control.streaming_session(&device.context)?;
        read_account_from_session(control, db, udid, session.as_ref()).await
    }
    .await;
    let closed = control.close_ui_context(device.context).await;
    let reading = result?;
    closed?;
    Ok(reading)
}

pub(super) async fn read_account_from_session(
    control: &DeviceControlPlane,
    db: &riviu_core::db::Database,
    udid: &str,
    session: &dyn riviu_core::driver::UiSession,
) -> anyhow::Result<AccountReading> {
    let expected = db.get_device_meta(udid)?.handle;
    let labels = labels(control, udid).await?;
    anyhow::ensure!(
        riviu_core::tiktok_account::account_read_supported(labels),
        "Chưa hỗ trợ đọc tài khoản trên bản TikTok/ngôn ngữ này"
    );
    riviu_core::tiktok_share::navigate_own_profile(session, &labels).await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    let observed = loop {
        let observed = riviu_core::tiktok_account::observe_own_account(session, labels).await?;
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
        Some(handle) if handle.eq_ignore_ascii_case(expected.trim().trim_start_matches('@')) => {
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
    let device = if request.actions.follow {
        // TikTok can retain a profile destination from an earlier feed card.
        // Match the production runner's clean app session before account/link
        // proof; this remains navigation only and cannot re-arm an action.
        riviu_core::interaction_campaign::open_clean_interaction_context(control, &row.actor_udid)
            .await?
    } else {
        open_interaction_context(control, &row.actor_udid).await?
    };
    let result = async {
        let labels = labels(control, &row.actor_udid).await?;
        let session = control.streaming_session(&device.context)?;
        session.set_gui_scope(riviu_core::ui_automation::GuiScope {
            run_id: campaign.into(),
            assignment_id: Some(assignment.into()),
            device_id: row.actor_udid.clone(),
            deadline_ms: None,
        });
        let actor = if request.actions.follow {
            let expected = db.get_device_meta(&row.actor_udid)?.handle;
            let observed =
                riviu_core::tiktok_share::observe_publish_account(session.as_ref(), &labels)
                    .await?;
            anyhow::ensure!(
                !expected.is_empty()
                    && observed.eq_ignore_ascii_case(expected.trim_start_matches('@')),
                "Tài khoản đã đổi; chưa thể đối chiếu Follow"
            );
            Some(observed)
        } else {
            None
        };
        // Readback uses the same exact URL resolver as comment verification;
        // it needs no initial feed card and never re-arms a public action.
        riviu_core::interaction_hierarchy::open_pinned_target_by_hierarchy(
            session.as_ref(),
            labels,
            &device.target_package,
            target,
            &std::sync::atomic::AtomicBool::new(false),
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
        let follow = match actor {
            Some(account) => Some(
                riviu_core::tiktok_follow_target::observe_follow_profile(
                    session.as_ref(),
                    labels,
                    target,
                    &account,
                )
                .await?,
            ),
            None => None,
        };
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
            follow,
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
