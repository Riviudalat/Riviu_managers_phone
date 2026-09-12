use super::*;
use riviu_core::tiktok_share::{LinkCapture, OwnPostLink};

#[test]
fn metadata_retry_never_posts_an_untouched_sibling_of_a_submitted_post() {
    use riviu_core::{
        PublishAssignmentRecord, PublishCampaignState as State, PublishRetryScope as Scope,
    };
    let assignment = |id: &str, state| PublishAssignmentRecord {
        sheet_delivery: None,
        id: id.into(),
        campaign_id: "mixed-campaign".into(),
        bundle_id: id.into(),
        ordinal: 0,
        udid: id.into(),
        state,
        effect_intent: None,
        evidence_json: None,
        error_code: None,
    };
    for fresh in [
        State::Queued,
        State::Scheduled,
        State::Ready,
        State::Imported,
        State::FailedBeforeDispatch,
    ] {
        let mixed = [
            assignment("already-submitted", State::Verifying),
            assignment("fresh-sibling", fresh),
        ];
        for scope in [Scope::LinkAndSheet, Scope::SheetOnly] {
            assert!(
                !fresh_assignments_allowed_by_retry_scope(Some(scope), &mixed),
                "{scope:?} would send a new Post for {:?}",
                mixed[1].state
            );
        }
        assert!(fresh_assignments_allowed_by_retry_scope(
            Some(Scope::FullPipeline),
            &mixed
        ));
        assert!(fresh_assignments_allowed_by_retry_scope(None, &mixed));
    }
    let dispatched = [
        assignment("submitted", State::Verifying),
        assignment("published", State::Succeeded),
        assignment("unknown", State::Uncertain),
    ];
    assert!(!fresh_assignments_allowed_by_retry_scope(
        Some(Scope::FullPipeline),
        &dispatched
    ));
    let mut review = assignment("needs-review", State::Uncertain);
    review.error_code = Some("post_verification_needs_review".into());
    let mixed = [review, assignment("never-posted", State::Imported)];
    assert!(!fresh_assignments_allowed_by_retry_scope(None, &mixed));
    assert!(!fresh_assignments_allowed_by_retry_scope(
        Some(Scope::FullPipeline),
        &mixed
    ));
}

#[test]
fn review_retry_scope_only_permits_link_checks_and_keeps_unknown_posts_blocked() {
    let review = publish_issue("post_verification_needs_review", None, "needs review");
    assert_eq!(
        campaign_retry_scope(
            riviu_core::PublishExecutionStatus::Uncertain,
            &[],
            std::slice::from_ref(&review)
        ),
        riviu_core::PublishRetryScope::LinkAndSheet
    );
    let unknown = publish_issue("post_may_be_live", None, "ambiguous Post");
    assert_eq!(
        campaign_retry_scope(
            riviu_core::PublishExecutionStatus::Uncertain,
            &[],
            &[review, unknown]
        ),
        riviu_core::PublishRetryScope::None
    );
}

#[tokio::test]
async fn processing_post_retains_evidence_and_cannot_earn_success_or_cleanup() {
    let steps = std::sync::Mutex::new(Vec::new());
    let progress = |step| steps.lock().unwrap().push(step);
    for capture in [
        OwnPostLink::NoTiles,
        OwnPostLink::CaptionNotFound,
        OwnPostLink::DraftsPresent(1),
        OwnPostLink::Sheet(LinkCapture::CopyDidNotLand),
        OwnPostLink::ReadFailed("disconnected after Post".into()),
        OwnPostLink::Captured("https://example.com/@a/photo/123".into()),
    ] {
        let outcome = settle_submission_capture(
            serde_json::json!({
                "campaignId": "campaign", "bundleId": "bundle", "importId": "import",
                "captionSha256": "caption-hash", "state": "submitted",
                // Simulate a prior attempt carrying an unrelated/stale link.
                "postUrl": "https://www.tiktok.com/@stale/photo/1",
                "publicationVerified": true,
            }),
            capture,
            &progress,
        )
        .await;
        assert!(must_preserve_pending_upload(&outcome));
        let folded = fold_cleanup_into(outcome, Err(anyhow::anyhow!("stream close failed")));
        assert_eq!(
            state_for_outcome(&folded),
            (
                riviu_core::PublishCampaignState::Verifying,
                Some("post_verification_pending")
            )
        );
        let PostOutcome::Submitted(evidence) = folded else {
            panic!("missing proof must remain a submitted post")
        };
        assert_eq!(post_url_owed(&evidence), None);
        assert_eq!(evidence["post"]["importId"], "import");
        assert_eq!(evidence["post"]["publicationVerified"], false);
        assert_eq!(evidence["cleanup"]["state"], "not_cleaned");
        assert_eq!(evidence["post"]["captionSha256"], "caption-hash");
    }
    assert!(steps
        .lock()
        .unwrap()
        .iter()
        .all(|step| { matches!(step, PublishProgress::LinkPending { .. }) }));
}

#[tokio::test]
async fn canonical_own_post_link_promotes_publication_and_sheet_obligation_together() {
    let steps = std::sync::Mutex::new(Vec::new());
    let progress = |step| steps.lock().unwrap().push(step);
    let outcome = settle_submission_capture(
        serde_json::json!({"bundleId":"bundle", "state":"submitted"}),
        OwnPostLink::Captured("https://www.tiktok.com/@owner/photo/123?tracking=removed".into()),
        &progress,
    )
    .await;
    assert!(!must_preserve_pending_upload(&outcome));
    let folded = fold_cleanup_into(outcome, Ok(serde_json::json!({"state":"kept"})));
    assert_eq!(
        state_for_outcome(&folded),
        (riviu_core::PublishCampaignState::Succeeded, None)
    );
    let PostOutcome::Posted(evidence) = folded else {
        panic!("a captured canonical own post must settle as published")
    };
    assert_eq!(
        post_url_owed(&evidence),
        Some("https://www.tiktok.com/@owner/photo/123")
    );
    assert_eq!(evidence["post"]["publicationVerified"], true);
    assert_eq!(
        *steps.lock().unwrap(),
        vec![
            PublishProgress::PostConfirmed,
            PublishProgress::LinkCaptured
        ]
    );
}

#[tokio::test]
async fn wrapped_pending_evidence_never_reuses_an_older_capture() {
    let outcome=settle_submission_capture(
        serde_json::json!({
            "postUrl":"https://www.tiktok.com/@stale/photo/1",
            "post":{"postUrl":"https://www.tiktok.com/@older/photo/2","publicationVerified":true,"importId":"import"},
            "cleanup":{"state":"kept"},
        }),OwnPostLink::CaptionNotFound,&|_|{}
    ).await;
    let PostOutcome::Submitted(evidence) = outcome else {
        panic!("stale nested URL must not confirm publication")
    };
    assert_eq!(post_url_owed(&evidence), None);
    assert_eq!(evidence["post"]["publicationVerified"], false);
    assert_eq!(evidence["post"]["importId"], "import");
}

#[test]
fn only_verified_or_never_submitted_posts_allow_app_shutdown() {
    assert!(must_preserve_pending_upload(&PostOutcome::Unknown(
        "tap transport lost".into()
    )));
    assert!(must_preserve_pending_upload(&PostOutcome::Submitted(
        serde_json::json!({})
    )));
    assert!(!must_preserve_pending_upload(
        &PostOutcome::NothingPublished("caption failed".into())
    ));
    assert!(!must_preserve_pending_upload(&PostOutcome::Posted(
        serde_json::json!({})
    )));

    let source = include_str!("execution.rs");
    let release = source
        .split("async fn release_pending_publish_context(")
        .nth(1)
        .unwrap()
        .split("/// Close TikTok")
        .next()
        .unwrap();
    assert!(release.contains("control.close_ui_context(context).await?"));
    assert!(!release.contains("finish_app_session"));
    assert!(!release.contains("terminate"));
    assert!(!release.contains("cleanup_publish_media"));
}

#[derive(Default)]
struct UploadDriver {
    terminations: std::sync::atomic::AtomicUsize,
    stream_stops: std::sync::atomic::AtomicUsize,
}

struct UploadSession;

#[async_trait::async_trait]
impl riviu_core::UiSession for UploadSession {
    async fn tap(&self, _: riviu_core::TapPoint) -> anyhow::Result<()> {
        anyhow::bail!("unexpected tap")
    }
    async fn swipe(&self, _: riviu_core::SwipeGesture) -> anyhow::Result<()> {
        anyhow::bail!("unexpected swipe")
    }
    async fn type_text(&self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("unexpected type")
    }
    async fn home(&self) -> anyhow::Result<()> {
        anyhow::bail!("unexpected Home")
    }
    async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("unexpected tap")
    }
    async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("unexpected read")
    }
    fn stream_url(&self) -> Option<String> {
        Some("http://fixture/stream".into())
    }
}

#[async_trait::async_trait]
impl riviu_core::DeviceDriver for UploadDriver {
    async fn list_devices(&self) -> anyhow::Result<Vec<riviu_core::DeviceInfo>> {
        Ok(Vec::new())
    }
    async fn refresh_device(&self, _: &str) -> anyhow::Result<riviu_core::DeviceInfo> {
        anyhow::bail!("unexpected refresh")
    }
    async fn install_app(&self, _: &str, _: &std::path::Path) -> anyhow::Result<()> {
        anyhow::bail!("unexpected install")
    }
    async fn uninstall_app(&self, _: &str, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("unexpected uninstall")
    }
    async fn screenshot(&self, _: &str, _: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
        anyhow::bail!("unexpected screenshot")
    }
    async fn syslog_tail(&self, _: &str, _: usize) -> anyhow::Result<String> {
        anyhow::bail!("unexpected syslog")
    }
    async fn launch_app(&self, _: &str, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("unexpected launch")
    }
    async fn terminate_app(
        &self,
        _: &str,
        _: &str,
    ) -> anyhow::Result<riviu_core::ProcessAbsenceProof> {
        self.terminations
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        anyhow::bail!("pending upload was terminated")
    }
    async fn reboot(&self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("unexpected reboot")
    }
    async fn start_ui_session(&self, _: &str) -> anyhow::Result<Box<dyn riviu_core::UiSession>> {
        Ok(Box::new(UploadSession))
    }
    async fn start_interaction_session(
        &self,
        _: &str,
        _: &str,
        _: riviu_core::InteractionSessionKind,
    ) -> anyhow::Result<Box<dyn riviu_core::UiSession>> {
        Ok(Box::new(UploadSession))
    }
    async fn confirm_interaction_stream_stopped(
        &self,
        _: &str,
    ) -> anyhow::Result<riviu_core::StreamHandoffProof> {
        Ok(riviu_core::StreamHandoffProof { generation: 1 })
    }
    async fn start_stream_after_session(
        &self,
        _: &str,
    ) -> anyhow::Result<riviu_core::StreamStartProof> {
        Ok(riviu_core::StreamStartProof {
            generation: 1,
            first_frame_observed: true,
            stream_url: "http://fixture/stream".into(),
        })
    }
    async fn stop_owned_stream(
        &self,
        _: &str,
    ) -> anyhow::Result<riviu_core::stream_budget::StreamStopProof> {
        self.stream_stops
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(riviu_core::stream_budget::StreamStopProof {
            old_generation: 1,
            new_generation: 2,
            child_stopped: true,
        })
    }
    async fn ensure_stream(&self, _: &str) -> anyhow::Result<String> {
        anyhow::bail!("unexpected background stream")
    }
    async fn prepare_device(&self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("unexpected prepare")
    }
}

#[tokio::test]
async fn pending_upload_releases_real_control_plane_without_terminating_tiktok() {
    let driver = Arc::new(UploadDriver::default());
    let control = DeviceControlPlane::new(
        driver.clone(),
        Arc::new(riviu_core::DeviceWorkCoordinator::new()),
        Arc::new(riviu_core::StreamBudgetManager::new(1).unwrap()),
    );
    let lease = control
        .acquire_exclusive("phone", DeviceWorkOwner::Script)
        .await
        .unwrap();
    let (lease, capacity) = control.reserve_ui_capacity(lease).await.unwrap();
    let session = control
        .start_interaction_session(
            lease,
            "com.fixture.tiktok",
            riviu_core::InteractionSessionKind::Ordinary,
        )
        .await
        .unwrap();
    let context = control
        .start_reserved_stream(session, capacity)
        .await
        .unwrap();
    assert_eq!(
        control.current_work_owner("phone"),
        Some(DeviceWorkOwner::Script)
    );
    let evidence = release_pending_publish_context(&control, context, "import")
        .await
        .unwrap();
    assert_eq!(evidence["appCleanup"]["state"], "leftRunning");
    assert_eq!(control.current_work_owner("phone"), None);
    assert_eq!(control.reserved_stream_capacity(), 0);
    assert_eq!(
        driver
            .terminations
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(
        driver
            .stream_stops
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    control.shutdown_cleanup().await.unwrap();
}

#[tokio::test]
async fn scheduled_submission_returns_without_polling_capture_or_claiming_publication() {
    let steps = std::sync::Mutex::new(Vec::new());
    let progress = |step| steps.lock().unwrap().push(step);
    let outcome = capture_or_defer_submission(
        serde_json::json!({"state":"submitted","submittedAt":"2026-09-10T13:00:00Z","expectedAccount":"fixture","importId":"keep"}),
        true,
        async { panic!("scheduled Post must not enter profile/share capture") },
        &progress,
    ).await;
    assert!(must_preserve_pending_upload(&outcome));
    let PostOutcome::Submitted(evidence) = outcome else {
        panic!("must defer verification")
    };
    assert_eq!(evidence["importId"], "keep");
    assert_eq!(evidence["publicationVerified"], false);
    assert!(post_url_owed(&evidence).is_none());
    assert!(steps.lock().unwrap().is_empty());
    let manual = capture_or_defer_submission(
        serde_json::json!({"state":"submitted"}),
        false,
        async { OwnPostLink::Captured("https://www.tiktok.com/@fixture/photo/123".into()) },
        &progress,
    )
    .await;
    assert!(matches!(manual, PostOutcome::Posted(_)));
    assert_eq!(steps.lock().unwrap()[0], PublishProgress::CapturingLink);
}
