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
    // Samsung Android 9 / Global 46.2.42: a Share recipient picker can remain on
    // top after TikTok is stopped. Launch receipts say OK but TikTok stays behind it.
    // Back only from the measured empty recipient picker; never select a recipient
    // or interact with a message composer or another application.
    let session = control.session(context)?;
    for _ in 0..2 {
        if session.active_app_bundle().await.ok().as_deref()
            != Some("com.samsung.android.messaging")
        {
            break;
        }
        let rows = session
            .locate_all(riviu_core::ElementQuery::Text {
                value: "Select recipients",
                exact: true,
            })
            .await?;
        if rows.len() != 1 {
            break;
        }
        let empty = session
            .locate_all(riviu_core::ElementQuery::Text {
                value: "No contacts",
                exact: true,
            })
            .await?;
        if empty.len() != 1 {
            break;
        }
        authorize()?;
        session.back().await?;
        tokio::time::sleep(std::time::Duration::from_millis(350)).await;
    }
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
