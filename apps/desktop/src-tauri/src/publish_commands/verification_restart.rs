//! The submitted publication keeps its identity while TikTok restarts for Copy.
use riviu_core::{db::Database, DeviceControlPlane, PublishAssignmentRecord, UiSessionContext};

pub(super) fn requested(assignment: &PublishAssignmentRecord, package: &str) -> bool {
    if !matches!(
        package,
        "com.zhiliaoapp.musically" | "com.ss.android.ugc.trill"
    ) {
        return false;
    }
    let evidence: serde_json::Value = assignment
        .evidence_json
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default();
    let post = evidence.get("post").unwrap_or(&evidence);
    // Unknown Post outcomes and historical receipts stay observational. Only
    // an actual submitted/posted receipt can authorize interrupting this app.
    matches!(post["state"].as_str(), Some("submitted" | "posted"))
        && post["publicationVerified"] != true
        && post["postUrl"].as_str().is_none_or(str::is_empty)
}

pub(super) fn authorize(
    db: &Database,
    assignment: &PublishAssignmentRecord,
    observer: Option<&riviu_core::db::PendingPublishVerification>,
) -> anyhow::Result<()> {
    if let Some(observer) = observer {
        anyhow::ensure!(
            db.publish_verification_is_current(observer)?,
            "Lượt xác minh đã thay đổi hoặc đã dừng"
        );
    } else {
        let current = db
            .get_publish_assignment_detail(&assignment.campaign_id, &assignment.id)?
            .and_then(|detail| {
                detail
                    .assignments
                    .into_iter()
                    .find(|row| row.id == assignment.id)
            });
        anyhow::ensure!(
            !db.publish_operation_stopped(&assignment.campaign_id)?
                && current
                    .as_ref()
                    .is_some_and(|row| row.state == assignment.state
                        && row.effect_intent == assignment.effect_intent
                        && row.evidence_json == assignment.evidence_json),
            "Lượt xác minh đã thay đổi hoặc đã dừng"
        );
    }
    let guard = db.publish_device_guard(&assignment.udid)?;
    anyhow::ensure!(
        guard
            .blocking
            .iter()
            .all(|hold| hold.assignment_id == assignment.id),
        "Máy còn bài khác chưa xác minh; chưa khởi động lại TikTok"
    );
    Ok(())
}

pub(super) async fn foreground(
    control: &DeviceControlPlane,
    context: &UiSessionContext,
    package: &str,
    restart: bool,
    mut authorize: impl FnMut() -> anyhow::Result<()>,
) -> anyhow::Result<Option<serde_json::Value>> {
    authorize()?;
    if !restart {
        control.foreground_session_app(context, package).await?;
        return Ok(None);
    }
    let started_at = chrono::Utc::now().to_rfc3339();
    let stopped = control.terminate_session_app(context, package).await?;
    anyhow::ensure!(
        stopped.bundle_id == package,
        "Bằng chứng tắt app không khớp TikTok của lượt đã gửi"
    );
    authorize()?;
    control.foreground_session_app(context, package).await?;
    authorize()?;
    let running = control
        .inspect_session_app_process(context, package)
        .await?;
    anyhow::ensure!(
        running.bundle_id == package && running.running,
        "TikTok chưa chạy lại; giữ bài đã gửi để kiểm tra sau"
    );
    Ok(Some(
        serde_json::json!({"state":"restarted","package":package,
        "startedAt":started_at,"finishedAt":chrono::Utc::now().to_rfc3339(),
        "oldPid":stopped.old_pid,"newPid":running.pid}),
    ))
}
