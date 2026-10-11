//! Threads shares the assignment ledger; reconciliation has no Post action.
use super::execution::{AssignmentPostAttempt, ConfirmedAssignmentLink, PostOutcome};
use super::*;
use riviu_core::publish_submission::PublishSubmissionProof;
use riviu_core::threads_publish::{self, mobile};

pub(super) async fn post(
    control: &DeviceControlPlane,
    db: &Database,
    assignment: &riviu_core::PublishAssignmentRecord,
    bundle: &riviu_core::PublishBundle,
    request: &PublishCampaignRequest,
    run: Option<&riviu_core::db::PublishPipelineRun>,
    revision: Option<i64>,
) -> AssignmentPostAttempt {
    let mut claimed = false;
    let mut refused = false;
    let mut selection = None;
    let progress =
        |step| super::progress::record_progress(db, &assignment.campaign_id, &assignment.id, step);
    let result = async {
        let (package, version, locale) = control.threads_build(&assignment.udid).await?;
        anyhow::ensure!(threads_publish::measured_mobile_build(&package, &version, &locale), "Threads build/locale unmeasured");
        let import = assignment.evidence_json.as_deref().and_then(execution::import_id_from_evidence).context("Threads native import proof missing")?;
        progress(super::progress::PublishProgress::WaitingControl);
        let exclusive = control.acquire_exclusive(&assignment.udid, DeviceWorkOwner::Script).await?;
        let (exclusive, capacity) = control.reserve_ui_capacity_until(exclusive, || {
            run.is_some_and(|r| !db.publish_pipeline_current(r).unwrap_or(false))
                || db.publish_campaign_state(&assignment.campaign_id).map(|s| s == Some(riviu_core::PublishCampaignState::Cancelled)).unwrap_or(true)
        }, || {}).await?;
        progress(super::progress::PublishProgress::OpeningApp);
        let context = control.start_clean_app_session(exclusive, capacity, &package, InteractionSessionKind::Ordinary).await?;
        let action: anyhow::Result<serde_json::Value> = async {
            let session = control.streaming_session(&context)?;
            mobile::ensure_foreground(session.as_ref()).await?;
            let account = mobile::observed_profile_account(session.as_ref()).await?;
            db.reserve_publish_account(&assignment.id, &account)?;
            progress(super::progress::PublishProgress::OpeningComposer);
            let prepared = mobile::prepare(session.as_ref(), bundle, &import, &account).await?;
            selection = Some(serde_json::to_value(&prepared)?);
            progress(super::progress::PublishProgress::MediaSelected { count: prepared.selected_count, video: false });
            progress(super::progress::PublishProgress::CaptionConfirmed);
            let proof = PublishSubmissionProof {
                verification_contract_version: 1, expected_account: account.clone(),
                submitted_at: chrono::Utc::now().to_rfc3339(), package, version, locale,
                caption_sha256: bundle.caption_sha256.clone(), bundle_id: bundle.id.clone(), media_kind: bundle.media_kind.clone(),
            };
            proof.validate()?;
            let post = mobile::validate_ready(&mobile::observe(session.as_ref()).await?, &bundle.caption, &account)?;
            if cfg!(debug_assertions) && std::env::var("RIVIU_PUBLISH_REHEARSAL").as_deref() == Ok("1")
                && db.get_setting("publish.rehearsal")?.as_deref() == Some("stopBeforePost") {
                anyhow::bail!("Threads rehearsal verified before public Post");
            }
            let mut intent = serde_json::to_value(&proof)?;
            intent["effectIntent"] = serde_json::json!("post");
            intent["mediaSelection"] = serde_json::to_value(&prepared)?;
            let accepted = match run {
                Some(run) => db.claim_pipeline_post(run, &assignment.id, revision.context("owned revision missing")?, &intent.to_string())?,
                None => db.claim_publish_assignment_for_posting(&assignment.id, &intent.to_string())?,
            };
            if !accepted { refused = true; anyhow::bail!("Threads Post claim refused; reconcile existing intent"); }
            claimed = true;
            progress(super::progress::PublishProgress::SubmittingPost);
            session.tap(post.centre()).await?;
            progress(super::progress::PublishProgress::PostSubmitted);
            tokio::time::sleep(Duration::from_secs(3)).await;
            let url = mobile::verify(session.as_ref(), &bundle.caption, &account, &proof.submitted_at).await?;
            progress(super::progress::PublishProgress::LinkCaptured);
            Ok(serde_json::json!({"state":"posted","verdict":"Posted","publicationVerified":true,
                "postUrl":url,"submittedAt":proof.submitted_at,"expectedAccount":account,"mediaSelection":prepared}))
        }.await;
        // Keep media and app alive after unverified dispatch.
        let cleanup = if action.is_ok() && request.cleanup_policy == PublishCleanupPolicy::DeleteImportedAssetsAfterVerified {
            control.cleanup_publish_media_with_ui(&context, &import).await.map_err(anyhow::Error::new)
        } else { Ok(serde_json::json!({"state":"retained","importId":import})) };
        let closed = control.close_ui_context(context).await;
        let outcome = match action {
            Ok(value) => PostOutcome::Posted(value),
            Err(error) if claimed => PostOutcome::Unknown(error.to_string()),
            Err(error) => PostOutcome::NothingPublished(error.to_string()),
        };
        let cleanup = match (cleanup, closed) {
            (Ok(value), Ok(_)) => Ok(value), (Err(error), _) => Err(error),
            (_, Err(error)) => Err(anyhow::Error::new(error)),
        };
        Ok::<_, anyhow::Error>(execution::fold_cleanup_into(outcome, cleanup))
    }.await;
    if !claimed {
        let _ = db.release_unsubmitted_publish_account(&assignment.id);
    }
    AssignmentPostAttempt {
        outcome: result.unwrap_or_else(|e| {
            if claimed {
                PostOutcome::Unknown(e.to_string())
            } else {
                PostOutcome::NothingPublished(e.to_string())
            }
        }),
        claim_refused: refused,
        final_revision: revision.map(|r| r + i64::from(claimed)),
        selection_diagnostic: selection,
    }
}

pub(super) async fn capture(
    db: &Database,
    control: &DeviceControlPlane,
    assignment: &riviu_core::PublishAssignmentRecord,
    bundle: &riviu_core::PublishBundle,
    observer: Option<&riviu_core::db::PendingPublishVerification>,
) -> anyhow::Result<ConfirmedAssignmentLink> {
    let proof: PublishSubmissionProof = serde_json::from_str(
        assignment
            .effect_intent
            .as_deref()
            .context("Threads submission identity missing")?,
    )?;
    proof.validate()?;
    anyhow::ensure!(
        proof.package == threads_publish::ANDROID_PACKAGE
            && proof.bundle_id == bundle.id
            && proof.caption_sha256 == bundle.caption_sha256,
        "Threads submission identity differs from bundle"
    );
    let current = control.threads_build(&assignment.udid).await?;
    anyhow::ensure!(
        current
            == (
                proof.package.clone(),
                proof.version.clone(),
                proof.locale.clone()
            ),
        "Threads build changed since submission"
    );
    let context = control
        .open_manual_session(&assignment.udid, DeviceWorkOwner::Script)
        .await?;
    let result = async {
        super::verification_restart::authorize(db, assignment, observer)?;
        control.foreground_session_app(&context, &proof.package).await?;
        let session = control.session(&context)?;
        let authorize = || super::verification_restart::authorize(db, assignment, observer);
        let guarded = super::verification_session::VerificationSession::new(session.as_ref(), &authorize);
        guarded.check()?;
        let url = mobile::verify_after_refresh(&guarded, &bundle.caption, &proof.expected_account, &proof.submitted_at).await?;
        guarded.check()?;
        Ok(ConfirmedAssignmentLink { url, diagnostic: serde_json::json!({"stage":"threadsOwnProfileCaptionAuthorTimeAndCanonicalLink","package":proof.package}) })
    }.await;
    let closed = control.close_manual_session(context);
    match (result, closed) {
        (Ok(link), Ok(_)) => Ok(link),
        (Err(error), _) => Err(error),
        (_, Err(error)) => Err(anyhow::Error::new(error)),
    }
}
