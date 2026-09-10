use super::*;
pub(super) use riviu_core::tiktok_composer::PublishProgress;

/// Telemetry failures must not turn a confirmed Post into a retryable failure.
pub(super) fn record_progress(
    db: &Database,
    campaign_id: &str,
    assignment_id: &str,
    step: PublishProgress,
) {
    if let Err(error) = db.append_publish_progress(campaign_id, assignment_id, &step) {
        log::warn!(
            "Không lưu được bước đăng bài {} cho {campaign_id}/{assignment_id}: {error}",
            step.state()
        );
    }
}
