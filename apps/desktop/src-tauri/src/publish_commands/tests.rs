#[tokio::test]
async fn publish_scan_runs_off_the_async_worker_and_keeps_its_slot_after_caller_cancel() {
    let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let runtime_thread = std::thread::current().id();
    let task = tokio::spawn(super::bounded_publish_scan(slots.clone(), move || {
        started_tx.send(std::thread::current().id()).unwrap();
        release_rx.recv_timeout(std::time::Duration::from_secs(5))?;
        Ok(())
    }));
    let worker_thread = tokio::time::timeout(std::time::Duration::from_secs(2), started_rx)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(worker_thread, runtime_thread);
    assert_eq!(slots.available_permits(), 0);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(
        slots.available_permits(),
        0,
        "blocking work still owns its permit"
    );
    assert!(slots.try_acquire().is_err());
    release_tx.send(()).unwrap();
    let permit = tokio::time::timeout(std::time::Duration::from_secs(2), slots.acquire())
        .await
        .unwrap()
        .unwrap();
    drop(permit);
    assert_eq!(slots.available_permits(), 1);
}

#[tokio::test]
async fn publish_scan_propagates_errors_and_releases_capacity() {
    let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
    let error = super::bounded_publish_scan(slots.clone(), || -> anyhow::Result<()> {
        anyhow::bail!("fixture scan error")
    })
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "fixture scan error");
    assert_eq!(slots.available_permits(), 1);
    assert_eq!(
        super::bounded_publish_scan(slots, || Ok(42)).await.unwrap(),
        42
    );
}

#[test]
fn folder_scan_and_preflight_share_the_bounded_blocking_worker() {
    for signature in [
        "pub async fn publish_scan_folder(",
        "async fn build_publish_preflight(",
    ] {
        let body = code_of(signature).join("\n");
        assert!(
            body.contains("bounded_publish_scan(Arc::clone(&PUBLISH_SCAN_SLOTS), move || {"),
            "{signature} must scan inside the shared bounded worker"
        );
        assert_eq!(body.matches("scan_publish_folder(").count(), 1);
        assert!(body.contains(".await"));
    }
}

use super::account_status_text_is_locked;
use super::apply_caption_overrides;
use super::assignment_already_posted;
use super::assignment_may_hold_the_post;
use super::bundle_for_assignment;
use super::bundle_media_shape_is_ready;
use super::deliver_assignment_sheet_row;
use super::evidence_with_post_url;
use super::fold_cleanup_into;
use super::fresh_publish_preflight_issues;
use super::mark_publish_sheet_sent_and_reconcile;
use super::max_images_for;
use super::missing_link_locators;
use super::persist_publish_snapshot_then_announce;
use super::post_url_owed;
use super::poster_identity;
use super::publish_preflight_digest;
use super::readiness_of_build;
use super::record_transfer_write_ahead;
use super::refuse_assignments_whose_bundle_is_too_large;
use super::refuse_devices_whose_composer_is_not_measured;
use super::refuse_when_the_route_authorities_disagree;
use super::require_current_preflight_digest;
use super::resolve_preflight_target;
use super::settle_publish_sheet_delivery_and_announce;
use super::state_for_outcome;
use super::token_must_be_restated;
use super::video_plan_for_build;
use super::LockScreening;
use super::PostOutcome;
use super::IOS_PIXEL_GRID_MAX_IMAGES;
use super::PUBLISH_FAN_OUT_STAGGER;
use super::{PublishReadiness, PublishRoute};
use std::collections::HashMap;
use std::fs;
use std::time::Duration;
use uuid::Uuid;

/// **Two readings of the same phone, and the post waits until they agree.**
///
/// The campaign gate reads `reports_element_bounds` before any session exists; the
/// dispatch reads `supports_element_bounds` off the live session. The driver contract
/// permits those to differ, and nothing compared them: a `true` preflight with a `false`
/// session cleared the measured-label gate and then pressed iOS pixel coordinates.
#[test]
fn a_phone_that_answers_the_route_question_twice_over_does_not_post() {
    for (preflight, session) in [(true, false), (false, true)] {
        let refusal = refuse_when_the_route_authorities_disagree("SN-1", preflight, session)
            .expect("a disagreement is a refusal");
        let PostOutcome::NothingPublished(reason) = &refusal else {
            panic!("a disagreement reached no composer, so it published nothing");
        };
        assert!(
            reason.contains("SN-1"),
            "the operator has to know which phone: {reason}"
        );
        // Both readings, in the message. "They disagreed" without the two values sends
        // whoever reads it back to the phone to take the measurement again.
        assert!(
            reason.contains(&format!("{preflight}")) && reason.contains(&format!("{session}")),
            "both readings belong in the message: {reason}"
        );
    }
}

/// And a disagreement leaves the campaign runnable again.
///
/// `Unknown` is the permanently-unclaimable state, kept for a phone that may have posted.
/// Nothing has reached the composer here, so spending it on a disagreement would strand a
/// campaign that a second run might drive perfectly well.
#[test]
fn a_route_disagreement_is_retryable_not_uncertain() {
    let refusal = refuse_when_the_route_authorities_disagree("SN-1", true, false).expect("refusal");
    assert_eq!(
        state_for_outcome(&refusal).0,
        riviu_core::PublishCampaignState::FailedBeforeDispatch
    );
}

/// Agreement — either way — is not a refusal.
#[test]
fn two_authorities_that_agree_let_the_post_through() {
    for both in [true, false] {
        assert!(
            refuse_when_the_route_authorities_disagree("SN-1", both, both).is_none(),
            "agreeing on {both} is the ordinary case"
        );
    }
}

/// **"Nobody looked" is not "there was nothing to see".**
///
/// The screening used to be a `bool`, so a host with no OCR and a frame that OCR failed on
/// both came back `false` — the same value as a frame that was read and found clean. The
/// evidence then recorded `accountLockScreened: cfg!(target_os = "macos")`, which answers a
/// question about the build rather than about the run.
#[test]
fn an_unread_frame_is_not_a_frame_that_passed() {
    assert!(!LockScreening::Unavailable.is_locked());
    assert!(!LockScreening::NotLocked.is_locked());
    assert!(LockScreening::Locked.is_locked());
    // The three have to be distinguishable in the evidence, or the distinction only exists
    // in memory and the run cannot be judged afterwards.
    let written = [
        LockScreening::NotLocked.as_str(),
        LockScreening::Locked.as_str(),
        LockScreening::Unavailable.as_str(),
    ];
    assert_eq!(written, ["not_locked", "locked", "unavailable"]);
}

/// And the evidence reports the run's screening, not the build's capabilities.
#[test]
fn the_pixel_evidence_records_what_the_screening_produced() {
    let body = code_of("async fn post_through_the_pixel_grid(");
    assert!(
        body.iter()
            .any(|line| line.contains("\"accountLockScreened\": screening.as_str()")),
        "the evidence must carry this run's screening result"
    );
    // And `screening` has to be the half of the pair `wait_for_post_frame` returned —
    // the reading made on the very frame `frameSha256` hashes. A review found the
    // previous version satisfied by `let screening = LockScreening::NotLocked;`, and the
    // production code itself once recorded the `after_post_tap` reading here: right
    // token, wrong frame.
    assert!(
        body.iter()
            .any(|line| line.contains("let (posted, screening) = wait_for_post_frame(")),
        "the recorded screening must arrive with the accepted frame, not from an \
         earlier one or a local constant"
    );
    assert!(
        !body.iter().any(|line| line.contains("cfg!(target_os")),
        "a compile-time constant cannot say whether this frame was read"
    );
}

/// **The fork in the settle road: only a real link routes through the sheet-row write.**
///
/// `Some` means state and outbox row go in as one transaction; `None` means the plain
/// state write. The empty shapes matter because migration 18's CHECK refuses a blank
/// link — a `Some("")` here would turn a successful post into a failed recording.
///
/// **The evidence is folded through the real function, not hand-shaped.** The first
/// version of this test built its input by hand, at the top level, while every caller
/// passes what `fold_cleanup_into` produced — one layer down. So it passed on a
/// `post_url_owed` that could never find a link in production, and the `Some` arm was
/// dead. A fixture that models the caller's shape is the only kind that can catch that,
/// and folding through the real function is what keeps the two from drifting.
#[test]
fn only_a_real_link_owes_the_sheet_a_row() {
    let folded = |evidence: serde_json::Value| match fold_cleanup_into(
        PostOutcome::Posted(evidence),
        Ok(serde_json::json!({"state": "cleaned"})),
    ) {
        PostOutcome::Posted(value) => value,
        _ => panic!("folding a posted outcome must stay posted"),
    };

    let link = folded(serde_json::json!({
        "state": "posted",
        "postUrl": "https://www.tiktok.com/@a/photo/1"
    }));
    assert_eq!(
        post_url_owed(&link),
        Some("https://www.tiktok.com/@a/photo/1"),
        "the link the composer wrote is one layer down after the fold: {link}"
    );

    for evidence in [
        serde_json::json!({}),
        serde_json::json!({"postUrl": ""}),
        serde_json::json!({"postUrl": "   "}),
        serde_json::json!({"postUrl": 7}),
        serde_json::json!({"postUrl": "https://www.tiktok.com/@not-a-post"}),
        serde_json::json!({"postUrl": "https://example.com/@a/video/1"}),
        serde_json::json!({"linkCaptureReason": "chưa đo nút Chia sẻ trên bản build này"}),
    ] {
        let folded_evidence = folded(evidence.clone());
        assert_eq!(post_url_owed(&folded_evidence), None, "folded {evidence}");
        // And the unfolded level still reads, for any caller that has not folded yet.
        assert_eq!(post_url_owed(&evidence), None, "unfolded {evidence}");
    }
}

/// **No link is read off the feed until the route to our own post is measured.**
///
/// The first wiring called `capture_post_link` straight on the `Posted` arm, believing
/// it would refuse until M7. It would not have: after Post the screen is the FEED,
/// Share there belongs to whoever's video is playing, that Share IS measured on the
/// fleet's build, and a stranger's post link passes `looks_like_a_post_link` because
/// it is one — a wrong link the outbox schema cannot tell from a right one. The
/// capture may only return to this function together with the M7-measured route that
/// first stands the phone on its own post; when that lands, this test is updated to
/// demand the route call BEFORE the capture instead of banning the capture outright.
///
/// **Scoped to the whole file, not to one function.** Scanning only
/// `post_through_the_composer` was bypassable three ways a review constructed: a helper
/// called from that arm, an aliased import (`… as grab`), or the capture moving to the
/// pixel route or `post_one_assignment`. The symbol is what matters, wherever it sits,
/// so the scan is the module minus its own test text — the same `#[cfg(test)]` cut the
/// fan-out gate uses, for the same reason: this assertion writes the needle out itself.
///
/// **Flipped 31/08/2026 (§9.136), and the shape of the flip is the point.** The route
/// exists now, so the rule is no longer "never capture" — it is "capture only through
/// the route". `capture_own_post_link` opens the share sheet only after a page has
/// rendered this run's caption; the bare `capture_post_link` trusts whatever is on
/// screen, and on this path what is on screen after Post is the feed.
#[test]
fn no_link_is_read_off_the_feed_until_the_route_is_measured() {
    let source = super::PRODUCTION_SOURCES;
    let module = source;
    // **Comments are not code, and this gate proved it the hard way on itself.** The
    // `Posted` arm's note has to name `capture_post_link` — the whole point of the note
    // is to say why that call is not there — and the first version of this scan read
    // its own explanation as the hazard. The mirror of the catalogued bypass where a
    // comment *satisfies* a gate: here it broke one. Strip the prose, scan the code.
    let code: String = module
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let module = code.as_str();
    // The bare capture, anywhere in the publish path. `capture_own_post_link` contains
    // the substring, so the check is on the call shape: `capture_post_link(` preceded
    // by nothing that makes it the routed one.
    assert!(
        !module.contains("::capture_post_link(") && !module.contains(" capture_post_link("),
        "the BARE capture_post_link is back in the publish path — after Post the screen \
         is the feed, and that reads a stranger's link and files it as ours. The routed \
         capture_own_post_link is the only one allowed here"
    );
    assert!(
        module.contains("capture_own_post_link("),
        "the Posted arm no longer captures at all — a published carousel owes the sheet \
         its link, and dropping the call loses it silently"
    );
    let body = code_of("async fn post_through_the_composer(");
    assert!(
        body.iter()
            .any(|line| line.contains("capture_own_post_link(")),
        "the routed capture left the Posted arm; the link is read there or nowhere"
    );
}

/// **Readiness answers about the build in front of it, not about the package.**
///
/// The old computation took the shortest gap across every catalogued set for the
/// package. As a refusal that was sound; as the page's positive claim it was a lie a
/// TikTok self-update could tell: `composer_caption` is keyed to `versionName`, so the
/// fleet's measured 38.3.2 set would have kept a phone on 46.x reading "ready" while
/// the composer refused it before the first tap. The version-blind lookup and the
/// version-keyed one differ on exactly that input, which is what this pins.
#[test]
fn readiness_asks_the_catalogue_about_this_phones_build() {
    // The one build measured end to end (§9.132).
    assert!(matches!(
        readiness_of_build("com.ss.android.ugc.trill", "en", "38.3.2"),
        PublishReadiness::HierarchyReady
    ));
    // Same phone, same language, after TikTok updated itself. The language set still
    // describes the strings — they are rendered text, not ids — so this does not become
    // "unknown build"; what drops out is the one control keyed to `versionName`. The
    // answer therefore NAMES `ComposerCaption`, which is both a refusal and the
    // instruction for closing it. (I expected `HierarchyUnknownBuild` here and the code
    // was more informative than the expectation; the assertion follows the code.)
    let updated = readiness_of_build("com.ss.android.ugc.trill", "en", "46.9.9");
    assert!(
        matches!(&updated, PublishReadiness::HierarchyMissing(missing)
            if missing.contains(&riviu_core::tiktok_labels::TikTokControl::ComposerCaption)),
        "an unmeasured version must lose its version-keyed control, not inherit another \
         version's verdict: {updated:?}"
    );
    // A version that was never read (the empty string a failed `dumpsys` leaves) is the
    // same answer for the same reason — it is not a licence to use another version's ids.
    assert!(matches!(
        readiness_of_build("com.ss.android.ugc.trill", "en", ""),
        PublishReadiness::HierarchyMissing(missing)
            if missing.contains(&riviu_core::tiktok_labels::TikTokControl::ComposerCaption)
    ));
    // A package nobody has catalogued at all is the one case that really is unknown.
    assert!(matches!(
        readiness_of_build("com.example.never-measured", "en", "1.0"),
        PublishReadiness::HierarchyUnknownBuild(_)
    ));
    // The second measured build answers ready — it graduated between this test being
    // written (morning of 31/08, tail unmeasured) and first run (evening, §9.135).
    assert!(matches!(
        readiness_of_build("com.zhiliaoapp.musically", "en", "46.2.1"),
        PublishReadiness::HierarchyReady
    ));
    // Its sibling version graduated the same evening (§9.135): the twentieth phone was
    // measured once its onboarding dialog cleared, and its ids turned out to have
    // MOVED — the shutter and both caption-screen ids differ from 46.2.1's — which is
    // the whole reason this lookup is keyed by version and not by package.
    assert!(matches!(
        readiness_of_build("com.zhiliaoapp.musically", "en", "46.2.42"),
        PublishReadiness::HierarchyReady
    ));
    // A version nobody has measured still names what it is missing rather than
    // borrowing a measured sibling's ids.
    assert!(matches!(
        readiness_of_build("com.zhiliaoapp.musically", "en", "47.0.0"),
        PublishReadiness::HierarchyMissing(missing) if !missing.is_empty()
    ));
}

/// **A token belongs to the endpoint it was issued for.**
///
/// `token: None` means "keep the stored one" — the convenience that lets an operator fix
/// a typo in the URL without re-pasting a credential. Applied to a *different* endpoint
/// it becomes: send webhook A's bearer token to webhook B, in the request body, where
/// whoever answers at B can then write into the operator's sheet. So the same-URL case
/// keeps the token and the changed-URL case demands it be restated. Trimming matters
/// because the field is typed by hand and a trailing space is not a new endpoint.
#[test]
fn changing_the_webhook_demands_the_token_for_that_webhook() {
    let a = "https://script.google.com/macros/s/AAA/exec";
    let b = "https://script.google.com/macros/s/BBB/exec";

    assert!(
        token_must_be_restated(a, b, None),
        "a new endpoint must not inherit the old endpoint's credential"
    );
    assert!(
        !token_must_be_restated(a, b, Some("fresh")),
        "restating the token is exactly what makes the change safe"
    );
    assert!(
        !token_must_be_restated(a, b, Some("")),
        "clearing it is also an answer: the new endpoint gets no credential"
    );
    assert!(
        !token_must_be_restated(a, a, None),
        "an unchanged URL keeps its token — the typo-fix path this exists for"
    );
    assert!(
        !token_must_be_restated(a, "  https://script.google.com/macros/s/AAA/exec  ", None),
        "whitespace around the same URL is not a new endpoint"
    );
    assert!(
        token_must_be_restated("", a, None),
        "configuring for the first time still has to say what the token is"
    );
}

/// **`bot`, because column B is a staff column — measured, not assumed.**
///
/// This test spent a day pinning the opposite: a device handle, falling back to `bot`.
/// The operator's real sheet settled it — column B is `Nhân Viên`, eleven people's names
/// over 1892 rows — so the app's rows say `bot` and a human can see at a glance which
/// rows a person posted. Whose account it was is still readable from the link itself.
///
/// Non-empty is the other half, and migration 18's CHECK is why: a blank poster is
/// refused by the database, so the one thing this must never become is a value that can
/// be empty.
#[test]
fn the_app_posts_as_bot_because_column_b_is_the_staff_column() {
    assert_eq!(poster_identity(), "bot");
    assert!(
        !poster_identity().trim().is_empty(),
        "migration 18's CHECK refuses a blank poster"
    );
}

/// The two participant filters, pinned variant by variant.
///
/// Mostly a typo pin, but the relationship at the end is the real contract: everything
/// the post loop steps over, the transfer loop steps over too — a state the post side
/// considers settled while the transfer side re-stages it would rebuild exactly the
/// claimable-state hole `claim_publish_campaign_for_transfer` exists to close.
#[test]
fn the_participant_filters_step_over_exactly_the_settled_states() {
    use riviu_core::PublishCampaignState as S;
    let all = [
        S::Queued,
        S::Scheduled,
        S::Preparing,
        S::Ready,
        S::Transferring,
        S::Imported,
        S::Posting,
        S::Verifying,
        S::Succeeded,
        S::FailedBeforeDispatch,
        S::Uncertain,
        S::Cancelled,
        S::Missed,
    ];
    for state in &all {
        assert_eq!(
            assignment_already_posted(state),
            matches!(state, S::Succeeded),
            "{state:?}"
        );
        assert_eq!(
            assignment_may_hold_the_post(state),
            matches!(
                state,
                S::Succeeded | S::Posting | S::Verifying | S::Uncertain
            ),
            "{state:?}"
        );
        assert!(
            !assignment_already_posted(state) || assignment_may_hold_the_post(state),
            "{state:?}: settled for posting must imply untouchable for transfer"
        );
    }
}

fn test_bundle(id: &str) -> riviu_core::PublishBundle {
    riviu_core::PublishBundle {
        id: id.into(),
        source_path: format!("/managed/req-7/{id}"),
        name: id.into(),
        media_kind: riviu_core::PublishMediaKind::Image,
        images: Vec::new(),
        video: None,
        caption_path: format!("/managed/req-7/{id}/caption.txt"),
        caption: String::new(),
        caption_sha256: String::new(),
        total_bytes: 0,
        partners: Vec::new(),
    }
}

fn test_video_bundle(id: &str) -> riviu_core::PublishBundle {
    let mut bundle = test_bundle(id);
    bundle.media_kind = riviu_core::PublishMediaKind::Video;
    bundle.video = Some(riviu_core::PublishVideo {
        path: format!("/managed/req-7/{id}/clip.mp4"),
        file_name: "clip.mp4".into(),
        sha256: "a".repeat(64),
        byte_len: 1_583_537,
        duration_ms: 8_000,
        video_codec: riviu_core::PublishVideoCodec::H264Avc,
        audio_codec: Some(riviu_core::PublishAudioCodec::Aac),
    });
    bundle.total_bytes = 1_583_537;
    bundle
}

fn test_assignment(id: &str, bundle_id: &str, udid: &str) -> riviu_core::PublishAssignmentRecord {
    riviu_core::PublishAssignmentRecord {
        id: id.into(),
        campaign_id: "campaign-1".into(),
        bundle_id: bundle_id.into(),
        ordinal: 0,
        udid: udid.into(),
        state: riviu_core::PublishCampaignState::Ready,
        effect_intent: None,
        evidence_json: None,
        error_code: None,
    }
}

#[test]
fn sheet_delivery_reconciles_the_operation_before_emitting_and_rejects_a_stale_revision() {
    let path = std::env::temp_dir().join(format!(
        "riviu-publish-sheet-convergence-{}.db",
        Uuid::new_v4()
    ));
    let db = super::Database::open(&path).expect("open fixture database");
    let mut bundle = test_bundle("bundle-sheet-convergence");
    bundle.caption = "caption".into();
    bundle.caption_sha256 = super::frame_sha256(bundle.caption.as_bytes());
    let request = riviu_core::PublishCampaignRequest {
        request_id: Uuid::new_v4().to_string(),
        source_root: "C:/fixture".into(),
        bundle_ids: vec![bundle.id.clone()],
        udids: vec!["phone-1".into()],
        run_at: None,
        visibility: riviu_core::PublishVisibility::Public,
        cleanup_policy: riviu_core::PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
        sound_policy: riviu_core::PublishSoundPolicy::Default,
        execution_confirmed: true,
        target_snapshot: None,
    };
    let initial = riviu_core::PublishExecutionSnapshotDraft {
        input_digest: "a".repeat(64),
        status: riviu_core::PublishExecutionStatus::Partial,
        retry_scope: riviu_core::PublishRetryScope::SheetOnly,
        report_json: serde_json::json!({"state": "sheet_pending"}),
    };
    let campaign = db
        .create_publish_campaign_with_snapshot(&request, &[bundle], &initial)
        .expect("create campaign and projection");
    let assignment = db
        .get_publish_campaign(&campaign.id)
        .expect("read campaign")
        .expect("campaign exists")
        .assignments
        .into_iter()
        .next()
        .expect("assignment exists");
    let link = "https://www.tiktok.com/@fixture/video/7400000000000000001";
    db.record_publish_success_with_sheet_row(
        &assignment.id,
        &serde_json::json!({"postUrl": link}).to_string(),
        &campaign.id,
        link,
        "bot",
        &[],
    )
    .expect("record post and outbox");
    db.update_publish_campaign_state(
        &campaign.id,
        riviu_core::PublishCampaignState::Succeeded,
        None,
    )
    .expect("settle campaign");

    let stale = db
        .pending_publish_sheet_row(&assignment.id)
        .expect("read pending row")
        .expect("pending row exists");
    db.queue_publish_sheet_row(&assignment.id, &campaign.id, link, "bot", &[])
        .expect("replace the in-flight row");
    let current = db
        .pending_publish_sheet_row(&assignment.id)
        .expect("read replacement row")
        .expect("replacement exists");
    assert!(current.revision > stale.revision);

    let events = riviu_core::events::EventBus::new(8);
    let mut receiver = events.subscribe();
    assert!(
        !mark_publish_sheet_sent_and_reconcile(&db, &events, &stale)
            .expect("stale CAS is an ordinary refusal"),
        "an old delivery must not settle newer Sheet content"
    );
    assert!(receiver.try_recv().is_err(), "a refused CAS emits nothing");
    assert_eq!(
        db.get_publish_execution_snapshot(&campaign.id)
            .expect("read projection")
            .expect("projection exists")
            .status,
        riviu_core::PublishExecutionStatus::Partial
    );

    let forced_failure = settle_publish_sheet_delivery_and_announce(
        &db,
        &events,
        &current,
        Some("not-a-valid-input-digest"),
        None,
    )
    .expect_err("snapshot failure must abort the whole Sheet settlement");
    assert!(
        forced_failure
            .to_string()
            .contains("64 lowercase hexadecimal"),
        "unexpected forced failure: {forced_failure:#}"
    );
    assert!(
        receiver.try_recv().is_err(),
        "a rolled-back settle emits nothing"
    );
    let retryable = db
        .pending_publish_sheet_row(&assignment.id)
        .expect("read row after snapshot failure")
        .expect("snapshot failure keeps the row retryable");
    assert_eq!(retryable.revision, current.revision);
    assert_eq!(retryable.attempts, current.attempts);
    assert_eq!(
        db.get_publish_execution_snapshot(&campaign.id)
            .expect("read projection after rollback")
            .expect("projection survives rollback")
            .status,
        riviu_core::PublishExecutionStatus::Partial,
        "the failed transaction must not publish half of its state"
    );

    assert!(
        mark_publish_sheet_sent_and_reconcile(&db, &events, &retryable)
            .expect("current delivery settles"),
        "the current revision must settle"
    );
    let event = receiver.try_recv().expect("durable settle emits an event");
    let riviu_core::events::AppEvent::PublishUpdated { campaign_id, .. } = event else {
        panic!("sheet settlement emitted the wrong event")
    };
    assert_eq!(campaign_id, campaign.id);

    // Read only after receiving the event. This is the frontend race: an event that can
    // overtake the snapshot makes the operations page repaint the old Partial forever.
    let snapshot = db
        .get_publish_execution_snapshot(&campaign.id)
        .expect("read projection after event")
        .expect("projection exists after event");
    assert_eq!(
        snapshot.status,
        riviu_core::PublishExecutionStatus::Complete
    );
    assert_eq!(snapshot.retry_scope, riviu_core::PublishRetryScope::None);
    let detail = db
        .get_publish_campaign(&campaign.id)
        .expect("read campaign after event")
        .expect("campaign exists after event");
    assert_eq!(
        riviu_core::project_publish_summary(&detail, Some(&snapshot)).state,
        riviu_core::OperationRunState::Succeeded,
        "the same event must expose a completed operation projection"
    );

    drop(db);
    std::fs::remove_file(path).expect("remove fixture database");
}

#[test]
fn a_failed_final_snapshot_write_emits_no_completion_event() {
    let path = std::env::temp_dir().join(format!(
        "riviu-publish-save-before-event-{}.db",
        Uuid::new_v4()
    ));
    let db = super::Database::open(&path).expect("open fixture database");
    let events = riviu_core::events::EventBus::new(4);
    let mut receiver = events.subscribe();
    let result: anyhow::Result<()> =
        persist_publish_snapshot_then_announce(&db, &events, "campaign-fixture", || {
            anyhow::bail!("fixture snapshot failure")
        });
    assert!(result.is_err());
    assert!(
        receiver.try_recv().is_err(),
        "an event must never announce a snapshot that did not commit"
    );
    drop(db);
    std::fs::remove_file(path).expect("remove fixture database");
}

#[test]
fn preflight_digest_binds_caption_target_and_observed_tiktok_build() {
    let request = riviu_core::PublishPreflightRequest {
        source_root: "C:/source".into(),
        bundle_ids: vec!["bundle-1".into()],
        udids: vec!["phone-1".into()],
        target_ref: Some(riviu_core::TargetRef::Explicit {
            udids: vec!["phone-1".into()],
        }),
        run_at: None,
        caption_overrides: Default::default(),
        sound_policy: riviu_core::PublishSoundPolicy::Default,
    };
    let mut bundle = test_bundle("bundle-1");
    bundle.caption = "caption-a".into();
    bundle.caption_sha256 = super::frame_sha256(bundle.caption.as_bytes());
    let observations = vec![serde_json::json!({
        "ordinal": 0,
        "udid": "phone-1",
        "number": 1,
        "alias": "Máy 1",
        "packageName": "com.ss.android.ugc.trill",
        "version": "38.3.2",
        "locale": "en",
        "requiredBytes": 1024,
        "storage": "pass",
        "availableBytes": 4096
    })];
    let target_snapshot = riviu_core::resolve_target(
        &riviu_core::TargetRef::Explicit {
            udids: request.udids.clone(),
        },
        &["phone-1".into()],
        &[],
        &[],
    )
    .expect("target snapshot");
    let approved = publish_preflight_digest(
        &request,
        std::slice::from_ref(&bundle),
        &target_snapshot,
        &observations,
    )
    .expect("digest");
    let report = riviu_core::PublishPreflightReport {
        input_digest: approved.clone(),
        target_snapshot: target_snapshot.clone(),
        can_execute: true,
        assignments: Vec::new(),
        issues: Vec::new(),
        sheet_configured: false,
    };
    require_current_preflight_digest(&report, &approved).expect("same snapshot is approved");

    let mut changed_free_space = observations.clone();
    changed_free_space[0]["availableBytes"] = serde_json::json!(8192);
    let changed_free_space_digest = publish_preflight_digest(
        &request,
        std::slice::from_ref(&bundle),
        &target_snapshot,
        &changed_free_space,
    )
    .expect("free-space digest");
    assert_eq!(
        approved, changed_free_space_digest,
        "exact free space may move while the stable threshold verdict stays approved"
    );

    let mut failed_storage = changed_free_space;
    failed_storage[0]["storage"] = serde_json::json!("fail");
    failed_storage[0]["availableBytes"] = serde_json::json!(512);
    let failed_storage_digest = publish_preflight_digest(
        &request,
        std::slice::from_ref(&bundle),
        &target_snapshot,
        &failed_storage,
    )
    .expect("storage verdict digest");
    assert_ne!(
        approved, failed_storage_digest,
        "crossing the required threshold must invalidate approval"
    );

    let mut changed_caption = bundle.clone();
    changed_caption.caption = "caption-b".into();
    changed_caption.caption_sha256 = super::frame_sha256(changed_caption.caption.as_bytes());
    let caption_digest = publish_preflight_digest(
        &request,
        &[changed_caption],
        &target_snapshot,
        &observations,
    )
    .expect("caption digest");
    assert_ne!(approved, caption_digest);

    let mut changed_target = request.clone();
    changed_target.udids[0] = "phone-2".into();
    let target_digest = publish_preflight_digest(
        &changed_target,
        &[bundle.clone()],
        &target_snapshot,
        &observations,
    )
    .expect("target digest");
    assert_ne!(approved, target_digest);

    let mut changed_roster = target_snapshot.clone();
    changed_roster.roster_sha256 = "f".repeat(64);
    let roster_digest =
        publish_preflight_digest(&request, &[bundle.clone()], &changed_roster, &observations)
            .expect("roster digest");
    assert_ne!(approved, roster_digest);

    let changed_build = vec![serde_json::json!({
        "ordinal": 0,
        "udid": "phone-1",
        "number": 1,
        "alias": "Máy 1",
        "packageName": "com.ss.android.ugc.trill",
        "version": "38.3.3",
        "locale": "en"
    })];
    let build_digest =
        publish_preflight_digest(&request, &[bundle], &target_snapshot, &changed_build)
            .expect("build digest");
    assert_ne!(approved, build_digest);

    let stale = riviu_core::PublishPreflightReport {
        input_digest: build_digest,
        ..report
    };
    let error = require_current_preflight_digest(&stale, &approved)
        .expect_err("changed observed build must invalidate approval");
    assert!(error.to_string().contains("preflight đã cũ"));
}

#[test]
fn semantic_publish_target_keeps_disconnected_group_members_in_the_snapshot() {
    let request = riviu_core::PublishPreflightRequest {
        source_root: "C:/source".into(),
        bundle_ids: vec!["bundle-1".into()],
        udids: vec!["phone-a".into()],
        target_ref: Some(riviu_core::TargetRef::Group {
            group_id: "morning".into(),
        }),
        run_at: None,
        caption_overrides: Default::default(),
        sound_policy: riviu_core::PublishSoundPolicy::Default,
    };
    let groups = vec![riviu_core::DeviceGroup {
        id: "morning".into(),
        name: "Ca sáng".into(),
        color: "#ff6a00".into(),
        udids: vec!["phone-a".into(), "phone-offline".into()],
        created_at: "2026-09-05T00:00:00Z".into(),
    }];
    let snapshot = resolve_preflight_target(&request, &["phone-a".into()], &[], &groups)
        .expect("resolve group");
    assert_eq!(snapshot.target_ref, request.target_ref.clone().unwrap());
    assert_eq!(snapshot.included[0].udid, "phone-a");
    assert_eq!(snapshot.excluded[0].device.udid, "phone-offline");
    assert_eq!(
        snapshot.excluded[0].reason,
        riviu_core::ExcludedDeviceReason::NotInRoster
    );

    let mut stale = request;
    stale.udids.clear();
    assert!(resolve_preflight_target(&stale, &["phone-a".into()], &[], &groups).is_err());
}

#[test]
fn caption_override_is_snapshotted_without_touching_the_source_file() {
    let temp = std::env::temp_dir().join(format!("riviu-caption-{}.txt", Uuid::new_v4()));
    fs::write(&temp, "caption from disk\n").expect("source caption");
    let mut bundles = vec![test_bundle("source-bundle")];
    bundles[0].caption_path = temp.display().to_string();
    bundles[0].caption = "caption from disk\n".into();
    bundles[0].caption_sha256 = super::frame_sha256(b"caption from disk\n");
    let overrides = HashMap::from([(
        "source-bundle".to_string(),
        "  caption edited in UI  \n".to_string(),
    )]);

    apply_caption_overrides(&mut bundles, Some(&overrides)).expect("valid override");

    assert_eq!(bundles[0].caption, "caption edited in UI");
    assert_eq!(
        bundles[0].caption_sha256,
        super::frame_sha256(b"caption edited in UI")
    );
    let managed_root =
        std::env::temp_dir().join(format!("riviu-caption-managed-{}", Uuid::new_v4()));
    let managed = riviu_core::copy_bundle_to_managed(&bundles[0], &managed_root)
        .expect("managed campaign snapshot");
    assert_eq!(managed.caption, "caption edited in UI");
    assert_eq!(
        fs::read_to_string(&managed.caption_path).expect("managed caption"),
        "caption edited in UI"
    );
    assert_eq!(
        fs::read_to_string(&temp).expect("source survives"),
        "caption from disk\n",
        "editing the campaign snapshot must not rewrite the user's source folder"
    );
    let _ = fs::remove_dir_all(managed_root);
    let _ = fs::remove_file(temp);
}

#[test]
fn caption_override_rejects_blank_or_unselected_bundle_keys() {
    let mut bundles = vec![test_bundle("selected")];
    let blank = HashMap::from([("selected".to_string(), " \r\n ".to_string())]);
    assert!(apply_caption_overrides(&mut bundles, Some(&blank))
        .expect_err("blank caption")
        .to_string()
        .contains("selected"));

    let unselected = HashMap::from([("not-selected".to_string(), "caption".to_string())]);
    assert!(apply_caption_overrides(&mut bundles, Some(&unselected))
        .expect_err("unselected bundle")
        .to_string()
        .contains("not-selected"));
}

#[test]
fn video_snapshot_is_validated_and_picker_readiness_is_tuple_scoped() {
    let mut image = test_bundle("image");
    image.images.push(riviu_core::PublishImage {
        path: "/managed/image/01.jpg".into(),
        file_name: "01.jpg".into(),
        order: 0,
        sha256: "b".repeat(64),
        byte_len: 100,
        width: 1080,
        height: 1920,
    });
    image.caption = "caption".into();
    image.caption_sha256 = super::frame_sha256(image.caption.as_bytes());
    let mut video = test_video_bundle("video");
    video.caption = "caption".into();
    video.caption_sha256 = super::frame_sha256(video.caption.as_bytes());
    assert!(bundle_media_shape_is_ready(&image, PublishRoute::Hierarchy));
    assert!(bundle_media_shape_is_ready(&video, PublishRoute::Hierarchy));
    assert!(video_plan_for_build("com.ss.android.ugc.trill", "en", "38.3.2").is_ok());
    assert!(video_plan_for_build("com.zhiliaoapp.musically", "en", "46.2.1").is_err());

    video.video = None;
    assert!(!bundle_media_shape_is_ready(
        &video,
        PublishRoute::Hierarchy
    ));
    video.video = test_video_bundle("video").video;
    video.images = image.images;
    assert!(
        !bundle_media_shape_is_ready(&video, PublishRoute::Hierarchy),
        "mixed media must fail before transfer"
    );
}

#[test]
fn hierarchy_video_uses_the_typed_sound_and_one_shot_post_state_machine() {
    let body = code_of("async fn post_through_the_composer(");
    let joined = body.join("\n");
    assert!(joined.contains("publish_video_with_sound_effect_intent("));
    assert!(joined.contains("video_plan_for_build(&package, &language, &version)"));
    assert!(joined.contains("&mut record_effect_intent"));
    assert!(joined.contains("crossed_effect_boundary = true"));
    assert!(joined.contains("videoPickerProvenance"));
    assert!(
        !joined.contains("reach_video_edit_step("),
        "production must continue through sound/readback/effect intent, not stop at the scout boundary"
    );
}

#[test]
fn scheduled_campaigns_use_the_same_typed_runtime_as_the_manual_command() {
    let source = include_str!("../state.rs");
    let start = source
        .find("for (campaign_id, raw) in scheduled")
        .expect("scheduled publish loop");
    let end = source[start..]
        .find("// Flow orphan sweep")
        .map(|offset| start + offset)
        .expect("end of scheduled publish loop");
    let body = &source[start..end];
    assert!(
        body.contains("execute_scheduled_publish_campaign_inner"),
        "the scheduler must enter the typed one-confirm runtime"
    );
    assert!(
        !body.contains("park_legacy_scheduled_publish")
            && !body.contains("transfer_publish_campaign_inner")
            && !body.contains("post_publish_campaign_inner"),
        "the scheduler still has a route around the typed runtime"
    );
}

#[test]
fn the_production_executor_calls_the_core_publish_runtime() {
    let source = super::PRODUCTION_SOURCES;
    let production = source;
    assert!(
        production.contains("riviu_core::run_publish_pipeline("),
        "publish_execute still leaves the tested core pipeline dead"
    );
}

#[test]
fn fresh_executor_accepts_structurally_valid_video_and_defers_to_the_live_tuple_gate() {
    let mut image = test_bundle("image");
    image.caption = "caption".into();
    image.caption_sha256 = super::frame_sha256(image.caption.as_bytes());
    let mut video = test_video_bundle("video");
    video.caption = "caption".into();
    video.caption_sha256 = super::frame_sha256(video.caption.as_bytes());
    let detail = |bundle: riviu_core::PublishBundle| {
        let assignment = test_assignment("assignment-1", &bundle.id, "phone-1");
        riviu_core::PublishCampaignDetail {
            campaign: riviu_core::PublishCampaignRecord {
                id: "campaign-1".into(),
                request_id: "request-1".into(),
                source_root: "C:/fixture".into(),
                state: riviu_core::PublishCampaignState::Ready,
                run_at: None,
                visibility: riviu_core::PublishVisibility::Public,
                cleanup_policy: riviu_core::PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
                assignments: Vec::new(),
                created_at: "2026-09-03T00:00:00Z".into(),
                updated_at: "2026-09-03T00:00:00Z".into(),
                error_code: None,
            },
            bundles: vec![bundle],
            assignments: vec![assignment],
            events: Vec::new(),
        }
    };
    let image_issues = fresh_publish_preflight_issues(
        &detail(image),
        &riviu_core::PublishSoundPolicy::TrendingAny {
            pool_size: 5,
            seed: 17,
        },
        true,
    );
    assert!(image_issues.is_empty());

    let mut sheet_pending = test_bundle("sheet-pending");
    sheet_pending.caption = "caption".into();
    sheet_pending.caption_sha256 = super::frame_sha256(sheet_pending.caption.as_bytes());
    let missing_sheet_issues = fresh_publish_preflight_issues(
        &detail(sheet_pending),
        &riviu_core::PublishSoundPolicy::Default,
        true,
    );
    assert!(
        missing_sheet_issues.is_empty(),
        "missing Sheet config is a downstream Partial, not a reason to block Post"
    );

    let video_issues = fresh_publish_preflight_issues(
        &detail(video),
        &riviu_core::PublishSoundPolicy::Default,
        true,
    );
    assert!(
        video_issues.is_empty(),
        "the live preflight and deep composer gate own tuple readiness: {video_issues:?}"
    );

    let mut cancelled = detail(test_bundle("cancelled"));
    cancelled.campaign.state = riviu_core::PublishCampaignState::Cancelled;
    cancelled.assignments[0].state = riviu_core::PublishCampaignState::Scheduled;
    let cancelled_issues =
        fresh_publish_preflight_issues(&cancelled, &riviu_core::PublishSoundPolicy::Default, true);
    assert!(cancelled_issues
        .iter()
        .any(|issue| issue.code == "campaign_terminal"));
}

#[tokio::test]
async fn missing_sheet_config_keeps_the_confirmed_post_in_a_pending_outbox() {
    let path = std::env::temp_dir().join(format!("riviu-publish-pending-{}.db", Uuid::new_v4()));
    let db = super::Database::open(&path).expect("open fixture database");
    let mut bundle = test_bundle("bundle-pending");
    bundle.caption = "caption".into();
    bundle.caption_sha256 = super::frame_sha256(bundle.caption.as_bytes());
    let request = riviu_core::PublishCampaignRequest {
        request_id: Uuid::new_v4().to_string(),
        source_root: "C:/fixture".into(),
        bundle_ids: vec![bundle.id.clone()],
        udids: vec!["phone-1".into()],
        run_at: None,
        visibility: riviu_core::PublishVisibility::Public,
        cleanup_policy: riviu_core::PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
        sound_policy: riviu_core::PublishSoundPolicy::Default,
        execution_confirmed: true,
        target_snapshot: None,
    };
    let campaign = db
        .create_publish_campaign(&request, &[bundle])
        .expect("create campaign");
    let assignment = db
        .get_publish_campaign(&campaign.id)
        .expect("read campaign")
        .expect("campaign exists")
        .assignments
        .into_iter()
        .next()
        .expect("assignment exists");
    let link = "https://www.tiktok.com/@fixture/video/7400000000000000001";
    db.record_publish_success_with_sheet_row(
        &assignment.id,
        &serde_json::json!({"postUrl": link}).to_string(),
        &campaign.id,
        link,
        "bot",
        &[],
    )
    .expect("record post and outbox atomically");

    let events = riviu_core::events::EventBus::new(8);
    let error = deliver_assignment_sheet_row(&db, &events, &assignment.id)
        .await
        .expect_err("unconfigured Sheet remains pending");
    assert!(error.contains("sheet_not_ready"), "{error}");
    let pending = db
        .pending_publish_sheet_row(&assignment.id)
        .expect("read outbox")
        .expect("row remains pending");
    assert_eq!(pending.attempts, 0, "no HTTP attempt was made");
    assert_eq!(pending.last_error, None);

    drop(db);
    std::fs::remove_file(path).expect("remove fixture database");
}

#[test]
fn transfer_write_ahead_failure_stops_before_any_device_call() {
    let path =
        std::env::temp_dir().join(format!("riviu-transfer-write-ahead-{}.db", Uuid::new_v4()));
    let backup = path.with_extension("db.fixture-backup");
    let db = super::Database::open(&path).expect("open fixture database");
    let mut bundle = test_bundle("bundle-write-ahead");
    bundle.caption = "caption".into();
    bundle.caption_sha256 = super::frame_sha256(bundle.caption.as_bytes());
    let request = riviu_core::PublishCampaignRequest {
        request_id: Uuid::new_v4().to_string(),
        source_root: "C:/fixture".into(),
        bundle_ids: vec![bundle.id.clone()],
        udids: vec!["phone-1".into()],
        run_at: None,
        visibility: riviu_core::PublishVisibility::Public,
        cleanup_policy: riviu_core::PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
        sound_policy: riviu_core::PublishSoundPolicy::Default,
        execution_confirmed: true,
        target_snapshot: None,
    };
    let campaign = db
        .create_publish_campaign(&request, &[bundle])
        .expect("create campaign");
    let assignment = db
        .get_publish_campaign(&campaign.id)
        .expect("read campaign")
        .expect("campaign exists")
        .assignments
        .into_iter()
        .next()
        .expect("assignment exists");

    // Database opens are per operation. Replacing the file with a directory injects a
    // deterministic connection failure without exposing a production SQL backdoor.
    std::fs::rename(&path, &backup).expect("move fixture database");
    std::fs::create_dir(&path).expect("install write failpoint");
    let device_calls = std::sync::atomic::AtomicUsize::new(0);
    let result = (|| -> anyhow::Result<()> {
        record_transfer_write_ahead(&db, &assignment.id)?;
        device_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    })();
    assert!(result.is_err(), "the injected write failure must propagate");
    assert_eq!(
        device_calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "no device operation may run without durable transfer ownership"
    );

    std::fs::remove_dir(&path).expect("remove failpoint");
    std::fs::rename(&backup, &path).expect("restore fixture database");
    drop(db);
    std::fs::remove_file(path).expect("remove fixture database");
}

#[test]
fn confirmed_completion_preserves_folded_evidence_and_requires_own_post_locators() {
    let link = "https://www.tiktok.com/@fixture/video/7400000000000000001";
    let evidence = evidence_with_post_url(
        Some(serde_json::json!({
            "post": {"state": "posted", "soundSelection": {"index": 2}},
            "cleanup": {"state": "cleaned"}
        })),
        link,
    );
    assert_eq!(evidence["post"]["postUrl"], link);
    assert_eq!(evidence["post"]["soundSelection"]["index"], 2);
    assert_eq!(evidence["cleanup"]["state"], "cleaned");

    assert!(missing_link_locators("com.ss.android.ugc.trill", "en", "38.3.2").is_empty());
    assert_eq!(
        missing_link_locators("com.example.unknown", "en", "1.0"),
        ["build_label_set"]
    );
}

#[test]
fn every_phone_is_given_its_own_bundle_and_not_the_campaign_root() {
    // The defect this pins: the transfer took ONE source root for the whole campaign --
    // `bundles[0].source_path.parent()` -- and staged it to every phone, so the mapping
    // that pairs N folders with N phones decided nothing and phones published each
    // other's pictures under each other's captions, to live accounts.
    let bundles = vec![test_bundle("req-7:bundle-a"), test_bundle("req-7:bundle-b")];
    let first = test_assignment("assign-1", "req-7:bundle-a", "phone-1");
    let second = test_assignment("assign-2", "req-7:bundle-b", "phone-2");

    let for_first = bundle_for_assignment(&bundles, &first).expect("bundle a");
    let for_second = bundle_for_assignment(&bundles, &second).expect("bundle b");

    assert_eq!(for_first.id, "req-7:bundle-a");
    assert_eq!(for_second.id, "req-7:bundle-b");
    assert_ne!(for_first.source_path, for_second.source_path);
}

#[test]
fn a_staged_root_holds_exactly_one_bundle_and_is_removed_afterwards() {
    // The shape is the whole point and it is easy to get wrong -- I got it wrong once
    // already. The iOS sidecar's manifest walker iterates the root's *subdirectories*
    // and only then reads files, so handing it the bundle directory itself yields an
    // empty manifest and stages nothing. The root must contain one bundle DIRECTORY.
    let temp = std::env::temp_dir().join(format!("riviu-stage-{}", Uuid::new_v4()));
    let bundle_dir = temp.join("bundle-a");
    fs::create_dir_all(&bundle_dir).expect("create the source bundle");
    fs::write(bundle_dir.join("01.png"), b"png").expect("write an image");

    let mut bundle = test_bundle("bundle-a");
    bundle.source_path = bundle_dir.display().to_string();
    bundle.caption = "xin chào".into();
    bundle.caption_sha256 = riviu_core::frame_sha256("xin chào".as_bytes());
    bundle.images = vec![riviu_core::PublishImage {
        path: bundle_dir.join("01.png").display().to_string(),
        file_name: "01.png".into(),
        order: 1,
        sha256: riviu_core::frame_sha256(b"png"),
        byte_len: 3,
        width: 1,
        height: 1,
    }];

    let root_path;
    {
        let staged = super::stage_one_bundle(&bundle, 0).expect("stage one bundle");
        root_path = staged.path().to_path_buf();
        let children: Vec<_> = fs::read_dir(staged.path())
            .expect("read the staged root")
            .map(|entry| entry.expect("entry"))
            .collect();
        assert_eq!(children.len(), 1, "exactly one bundle directory");
        assert!(children[0].file_type().expect("file type").is_dir());
        assert!(children[0].path().join("01.png").is_file());
        assert!(children[0].path().join("caption.txt").is_file());
    }
    // The guard drops with the scope, including on the error paths that bail out of the
    // transfer loop.
    assert!(!root_path.exists(), "the scratch root is removed");
    let _ = fs::remove_dir_all(&temp);
}

#[test]
fn a_device_scope_is_a_component_every_backend_accepts() {
    // Two phones, two scopes -- which is what gives them two staging directories, two
    // manifest hashes and two albums. And the string has to survive validators written
    // in three languages: `[A-Za-z0-9._-]`, 1..=128.
    let first = super::device_campaign_id("0f8f0e1e-1c4a-4b6f-9a2e-7c5d3b9a1f22", 0);
    let second = super::device_campaign_id("0f8f0e1e-1c4a-4b6f-9a2e-7c5d3b9a1f22", 1);
    assert_ne!(first, second);
    for scope in [&first, &second] {
        assert!(!scope.is_empty() && scope.len() <= 128, "{scope}");
        assert!(
            scope
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')),
            "{scope}"
        );
        assert!(scope != "." && scope != "..", "{scope}");
    }
}

#[test]
fn an_assignment_naming_a_bundle_the_campaign_lost_is_refused_not_guessed() {
    // Falling back to "the first bundle" is exactly how this broke. A campaign whose
    // rows disagree must stop, not publish something plausible.
    let bundles = vec![test_bundle("req-7:bundle-a")];
    let orphan = test_assignment("assign-9", "req-7:bundle-missing", "phone-9");
    let error = bundle_for_assignment(&bundles, &orphan).expect_err("must refuse");
    assert!(
        error.to_string().contains("req-7:bundle-missing"),
        "{error}"
    );
}

/// **A phone whose build is unmeasured is refused; Android as such is not.**
///
/// This gate used to refuse every device that reported element bounds, because there was
/// no composer for them. There is one now, so the question narrowed: not "is this
/// Android" but "has this phone's TikTok had the controls read off it". A mixed fleet
/// must run the phones that are measured and refuse only the ones that are not.
/// **A cleanup failure never turns a published post into a failed one.**
///
/// `Unknown` is permanently unclaimable, so downgrading a good post to it means a person
/// has to go and look at a phone whose only problem is some files left in a folder. The
/// reversal that found this gap made exactly that change and every test stayed green.
#[test]
fn files_left_on_the_phone_do_not_unpublish_a_carousel() {
    let posted = PostOutcome::Posted(serde_json::json!({"state": "posted"}));
    let folded = fold_cleanup_into(posted, Err(anyhow::anyhow!("adb went away")));
    assert!(
        matches!(folded, PostOutcome::Posted(_)),
        "a cleanup failure downgraded a published post"
    );
    let (state, code) = state_for_outcome(&folded);
    assert_eq!(state, riviu_core::PublishCampaignState::Succeeded);
    assert_eq!(code, None);
    // And the problem is still recorded, rather than swallowed.
    if let PostOutcome::Posted(evidence) = folded {
        assert_eq!(evidence["cleanup"]["state"], "not_cleaned");
        assert!(evidence["cleanup"]["message"]
            .as_str()
            .is_some_and(|text| text.contains("adb went away")));
    }

    // **And the production path routes through it**, which testing the helper alone
    // cannot show: `let _ = cleanup; Ok(outcome)` at the call site left this green.
    let body = code_of("async fn post_one_assignment(");
    assert!(
        body.iter()
            .any(|line| line.contains("fold_cleanup_into(action_result, cleanup)")),
        "the posting path stopped folding the cleanup result into its outcome"
    );
    assert!(
        body.iter()
            .any(|line| line.contains("tidy_up_the_imported_media")),
        "the imported media is no longer cleaned up at all"
    );
}

/// **Three outcomes, three states — and the retryable one must not be stranded.**
///
/// Every failure used to become `uncertain`, which the claim refuses forever. Most of what
/// that stranded had refused before opening anything: an unmeasured build, a picker that
/// would not arm, an album that was not there. Those need another run, not a person.
#[test]
fn only_an_outcome_that_may_have_published_is_made_unclaimable() {
    assert_eq!(
        state_for_outcome(&PostOutcome::NothingPublished("album not found".into())),
        (
            riviu_core::PublishCampaignState::FailedBeforeDispatch,
            Some("post_refused_before_dispatch")
        ),
        "a run that published nothing must stay claimable"
    );
    assert_eq!(
        state_for_outcome(&PostOutcome::Unknown("tapped Post, lost the answer".into())),
        (
            riviu_core::PublishCampaignState::Uncertain,
            Some("post_or_cleanup_failed")
        )
    );
    // The two must not be the same state, which is the whole content of this test.
    assert_ne!(
        state_for_outcome(&PostOutcome::NothingPublished(String::new())).0,
        state_for_outcome(&PostOutcome::Unknown(String::new())).0
    );
}

/// **A phone already inside the composer is never abandoned there.**
///
/// The cancel is read once, before opening the phone, and nowhere else. The assignment
/// claim itself belongs to the one-shot callback immediately before Post: claiming it here
/// made a crash anywhere in the whole composer walk look as if Post may have gone out.
///
/// A source gate because the ordering it pins lives in device code: the function acquires
/// a stream permit, reads the cancel, and delegates to the effect-aware assignment driver.
#[test]
fn a_cancel_is_read_before_the_phone_and_the_claim_lives_at_the_post_boundary() {
    // **Comments are stripped before anything is searched.** The first version counted
    // token occurrences over the raw lines, so writing
    // `// PublishCampaignState::Cancelled` above the claim satisfied it while the real
    // check was deleted — the gate measured the file's prose, not its behaviour.
    let body = code_of("async fn post_one_phone(");
    let at = |needle: &str| body.iter().position(|line| line.contains(needle));

    let cancel = at("PublishCampaignState::Cancelled")
        .expect("the cancel is no longer read; the button writes a flag nobody honours");
    let post = at("post_one_assignment(").expect("this is what touches the phone");
    assert!(cancel < post, "the cancel is read after the phone starts");
    assert!(
        at("claim_publish_assignment_for_posting").is_none(),
        "claiming in post_one_phone recreates the crash-before-Post uncertainty window"
    );
    // And exactly once: a second check further down is the one that would abandon a phone
    // inside the composer, in the `uncertain` state that can never be retried.
    assert_eq!(
        body.iter()
            .filter(|line| line.contains("PublishCampaignState::Cancelled"))
            .count(),
        1,
        "the cancel is read more than once; the later read stops a phone mid-post"
    );

    // The two helpers this function must actually route through, rather than merely
    // mention. Hard-coding either one's answer inline left the pure tests green.
    assert!(
        at("state_for_outcome(&outcome)").is_some(),
        "the assignment state is decided somewhere other than `state_for_outcome`"
    );
    assert!(
        at("gate.acquire()").is_some(),
        "the fan-out permit is never acquired, so the semaphore bounds nothing"
    );
    assert!(
        at("tokio::time::sleep(stagger)").is_some(),
        "the stagger argument is not what delays the task"
    );

    let assignment = code_of("async fn post_one_assignment(");
    let claim = assignment
        .iter()
        .position(|line| line.contains("claim_publish_assignment_for_posting"))
        .expect("the Post callback no longer owns the assignment CAS");
    let hierarchy = assignment
        .iter()
        .position(|line| line.contains("post_through_the_composer("))
        .expect("hierarchy route missing");
    let pixel = assignment
        .iter()
        .position(|line| line.contains("post_through_the_pixel_grid("))
        .expect("pixel route missing");
    assert!(claim < hierarchy && claim < pixel);
    assert_eq!(
        assignment
            .iter()
            .filter(|line| line.contains("&mut before_post"))
            .count(),
        1,
        "the hierarchy route must receive the one-shot claim callback directly"
    );
    assert!(
        assignment
            .iter()
            .any(|line| line.contains("before_pixel_post = || before_post(None)"))
            && assignment
                .iter()
                .any(|line| line.contains("&mut before_pixel_post")),
        "the pixel adapter must erase only sound evidence while preserving the same claim"
    );
}

/// The lines of one top-level function, **with comments and blank lines removed**.
///
/// Every source gate in this module goes through here. A gate that reads raw lines is
/// satisfied by a comment saying the right words, which is the opposite of what it is for.
fn code_of(signature: &str) -> Vec<&'static str> {
    let lines: Vec<&str> = super::PRODUCTION_SOURCES.lines().collect();
    let start = lines
        .iter()
        .position(|line| {
            line.strip_prefix("pub(super) ")
                .unwrap_or(line)
                .starts_with(signature)
        })
        .unwrap_or_else(|| panic!("{signature} is no longer in this file"));
    let length = lines[start..]
        .iter()
        .position(|line| *line == "}")
        .expect("the function terminates at column zero");
    lines[start..start + length]
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .collect()
}

/// **And `post_one_assignment` actually asks, before it picks a composer.**
///
/// The three tests above prove what the refusal says; none of them proves it is ever
/// reached. A pure decision nothing calls is the exact shape of the bug this replaced —
/// two readings existed, and no line compared them. So this reads the function's own body:
/// the check has to appear, and it has to appear **before** the branch on
/// `supports_element_bounds`, because after that branch the route is already chosen.
#[test]
fn the_post_path_reconciles_the_two_route_authorities_before_it_branches() {
    let body = code_of("async fn post_one_assignment(");
    let asks = body
        .iter()
        .position(|line| line.contains("refuse_when_the_route_authorities_disagree"))
        .expect("post_one_assignment must reconcile the two readings");
    let branches = body
        .iter()
        .position(|line| line.contains("if session.supports_element_bounds()"))
        .expect("post_one_assignment must still branch on the session's answer");
    assert!(
        asks < branches,
        "the check is at line {asks} of the body and the branch at {branches}; \
         a check after the branch has nothing left to refuse"
    );
    // **And the answer has to leave the function.**
    //
    // Ordering alone was too weak, and the review found the stub that passes it:
    //
    // ```rust
    // let _ignored = refuse_when_the_route_authorities_disagree(...);
    // let _cleanup = tidy_up_the_imported_media(...).await;
    // let action_result = if session.supports_element_bounds() { /* still posts */ };
    // ```
    //
    // Both tokens, in the right order, and the phone posts anyway. And the next review
    // found the second stub: keep a `return` in the window — inside some unrelated error
    // branch — while the question's answer still goes to `_ignored`. So the assertion now
    // follows the *answer*: the ask must bind through `if let Some(refusal)`, and a
    // return in the window must carry that binding out. Asking and discarding the answer
    // is not a check, and returning something else is not a refusal.
    assert!(
        body[asks..branches]
            .iter()
            .any(|line| line.contains("if let Some(refusal) =")),
        "the question's answer must be bound, not discarded"
    );
    assert!(
        body[asks..branches]
            .iter()
            .any(|line| line.contains("return finish(fold_cleanup_into(refusal")),
        "the refusal has to return — and it has to be the refusal that returns"
    );
    // And the media is taken back off the phone on that path — it was imported before any
    // of this ran, and a refusal that leaves the campaign's images in the gallery is a
    // refusal the operator has to clean up by hand on twenty phones.
    assert!(
        body[asks..branches]
            .iter()
            .any(|line| line.contains("tidy_up_the_imported_media")),
        "a route refusal still has to clear the imported media"
    );
    // The session has to exist before the question is asked: `supports_element_bounds` is
    // the session's own answer, and there is nothing to compare the preflight against until
    // `streaming_session` has handed one over.
    let opens = body
        .iter()
        .position(|line| line.contains("control.streaming_session("))
        .expect("post_one_assignment must still open a session");
    assert!(
        opens < asks,
        "the session opens at line {opens} and the question is asked at {asks}; \
         the question needs both answers to exist"
    );
}

/// **The fan-out is bounded by the stream budget and staggered.**
///
/// Both measured facts about this fleet rather than preferences. Each post holds a
/// UI-with-stream context, and running past `stream_capacity` does not queue — it fails, on
/// a phone whose gallery already holds the campaign's images. The stagger is the same two
/// seconds the interaction path measured: twenty cold starts at once share one USB bus, and
/// the tail runs past the 40-second foreground window.
#[test]
fn the_publish_fan_out_is_bounded_and_staggered() {
    // **Scoped to the module, not the file.** Two reversals proved why: this searched the
    // whole source, and the strings it looks for are written out again in its own
    // assertions — so removing them from the code left the test green on the strength of
    // its own text. The same shape once let `locate` stop reading an attribute.
    let source = super::PRODUCTION_SOURCES;
    let module = source;
    assert!(
        // Two facts, matched separately, because `cargo fmt` decides where the line
        // breaks go and a gate that pins the whole expression breaks on reformatting
        // rather than on a real change. This one already did once.
        module.contains("Semaphore::new(") && module.contains("stream_capacity().max(1)"),
        "the fan-out no longer bounds itself by the stream budget"
    );
    assert!(
        module.contains("PUBLISH_FAN_OUT_STAGGER * index"),
        "the fan-out starts every phone at once again"
    );
    assert!(
        PUBLISH_FAN_OUT_STAGGER >= Duration::from_secs(1),
        "a stagger this short does not separate twenty cold starts"
    );
}

#[test]
fn the_transfer_path_claims_the_campaign_instead_of_writing_it() {
    let body = code_of("pub(crate) async fn transfer_publish_campaign_inner(");
    assert!(
        body.iter()
            .any(|line| line.contains("claim_publish_campaign_for_transfer")),
        "transfer writes `Transferring` unconditionally again — which on a campaign that              already succeeded rebuilds exactly the state the posting claim accepts"
    );
    // And it must not go back to the unconditional write.
    assert!(
        !body
            .iter()
            .any(|line| line.contains("PublishCampaignState::Transferring")
                && line.contains("update_publish_campaign_state")),
        "the unconditional write is back"
    );
    // A finished assignment is skipped, so the two guards do not depend on each other.
    // The skip goes through the named predicate — whose variant set is pinned by
    // `the_participant_filters_step_over_exactly_the_settled_states` — so this line and
    // that test together are the chain: loop → predicate → the four states.
    assert!(
        body.iter()
            .any(|line| line.contains("assignment_may_hold_the_post(&assignment.state)")),
        "the loop no longer skips assignments that already reached a phone"
    );
}

/// **The fan-out runs the unposted participants, and judges only them.**
///
/// The chain this pins: the participant set is built through
/// `assignment_already_posted` (whose variant set has its own test), the spawn loop and
/// the counting loop walk that same set, and a campaign with nothing left to run settles
/// as `AllPosted`. Before this, a retry of a partially posted campaign spawned a task
/// for every `succeeded` row, counted its claim refusal as a failure, and finished the
/// campaign `failed_before_dispatch` with every carousel live — the state whose parent
/// the pool used to read as releasing those bundles.
#[test]
fn the_post_fan_out_runs_only_the_unposted_participants() {
    let body = code_of("pub(crate) async fn post_publish_campaign_inner(");
    assert!(
        body.iter().any(|line| line
            .contains(".filter(|assignment| !assignment_already_posted(&assignment.state))")),
        "the participant set is no longer filtered by what already posted"
    );
    assert!(
        body.iter()
            .any(|line| line.contains("for (index, assignment) in participants.iter()")),
        "the fan-out spawns from the unfiltered assignment list again"
    );
    assert!(
        body.iter()
            .any(|line| line.contains("participants.iter().zip(running)")),
        "the counting walks a different set than the one that spawned"
    );
    let empty = body
        .iter()
        .position(|line| line.contains("if participants.is_empty()"))
        .expect("a campaign with nothing left to run must be settled, not judged");
    assert!(
        body[empty..(empty + 6).min(body.len())]
            .iter()
            .any(|line| line.contains("PublishRunOutcome::AllPosted")),
        "an all-posted campaign must settle as what it is"
    );
}

#[test]
fn the_publish_session_targets_the_device_own_tiktok_build() {
    let body = code_of("async fn open_publish_context(");
    assert!(
        body.iter()
            .any(|line| line.contains("resolve_tiktok_package")),
        "the publish context stopped asking the device which TikTok it runs"
    );
    assert!(
        !body.iter().any(|line| line.contains("IOS_TIKTOK_BUNDLE")),
        "the publish context is back to assuming the iOS bundle on every backend"
    );
    // **And the answer has to reach both calls.** Resolving the package and then passing a
    // literal to `terminate_app` satisfies the two checks above while doing exactly what
    // they exist to prevent.
    for call in [
        "terminate_app(&exclusive, &target_package)",
        "start_interaction_session(exclusive, &target_package",
    ] {
        assert!(
            body.iter().any(|line| line.contains(call)),
            "the resolved package does not reach `{call}`"
        );
    }
}

#[test]
fn a_phone_whose_build_is_unmeasured_is_refused_and_its_neighbours_are_not() {
    let error = refuse_devices_whose_composer_is_not_measured([
        ("00008030-iphone", PublishReadiness::PixelGrid),
        ("ce0617164585646f0d7e", PublishReadiness::HierarchyReady),
        (
            "ce9917160000000000",
            PublishReadiness::HierarchyUnknownBuild("bản TikTok lạ".into()),
        ),
    ])
    .expect_err("the unmeasured phone must be refused");
    let message = format!("{error:#}");
    // Names the offending device: a fleet is mixed, and "some device" sends the
    // operator hunting through sixteen phones.
    assert!(message.contains("ce9917160000000000"), "{message}");
    assert!(!message.contains("00008030-iphone"), "{message}");
    assert!(!message.contains("ce0617164585646f0d7e"), "{message}");
    // And it says how to close the gap, because the reader is the person who would.
    assert!(message.contains("composer_scout"), "{message}");
}

/// A build missing labels is refused **by name**, so the measuring run knows what to get.
#[test]
fn a_build_missing_labels_is_refused_and_the_missing_ones_are_listed() {
    let error = refuse_devices_whose_composer_is_not_measured([(
        "ce0617164585646f0d7e",
        PublishReadiness::HierarchyMissing(vec![
            riviu_core::tiktok_labels::TikTokControl::PostButton,
        ]),
    )])
    .expect_err("a build without a Post button cannot publish");
    assert!(format!("{error:#}").contains("PostButton"), "{error:#}");
}

/// **Both routes pass when they are ready**, which is the case that must not regress.
#[test]
fn a_mixed_fleet_that_is_fully_measured_runs() {
    refuse_devices_whose_composer_is_not_measured([
        ("a-iphone", PublishReadiness::PixelGrid),
        ("an-android", PublishReadiness::HierarchyReady),
    ])
    .expect("both routes are measured");
}

/// **The composer's grid, refused before the media leaves the desktop.**
///
/// `post_one_assignment` already refuses an over-sized bundle, but it refuses after
/// `stage`/`prepare`/`import` have put the images into a real phone's gallery and made
/// them visible to TikTok — where they stay, with no cleanup owner, because the campaign
/// never reached a state that owns cleanup.
#[test]
fn a_bundle_too_wide_for_the_tap_grid_is_refused_before_transfer() {
    let fits = bundle_of("set1 13 spotlight", 11);
    let too_wide = bundle_of("set1 19 spotlightv3", 13);
    let error = refuse_assignments_whose_bundle_is_too_large([
        ("an-iphone", &fits, IOS_PIXEL_GRID_MAX_IMAGES),
        ("an-iphone-2", &too_wide, IOS_PIXEL_GRID_MAX_IMAGES),
    ])
    .expect_err("thirteen images cannot be reached by a twelve-cell grid");
    let message = format!("{error:#}");
    // Names the offending bundle, its count and the phone: an operator with twenty-one
    // folders and twenty phones needs all three.
    assert!(message.contains("set1 19 spotlightv3"), "{message}");
    assert!(message.contains("13"), "{message}");
    assert!(message.contains("an-iphone-2"), "{message}");
    // And does not accuse the one that fits.
    assert!(!message.contains("set1 13 spotlight"), "{message}");
}

/// **The ceiling is the device's, not the campaign's.**
///
/// One number for the whole run refused Android bundles that its own composer selects
/// fine — it locates each cell rather than tapping twelve coordinates somebody wrote
/// down, so its grid is wider. The same twelve-image bundle passes on one route and
/// refuses on the other, and that is the point.
#[test]
fn each_device_is_measured_against_its_own_composer() {
    let twelve = bundle_of("twelve", 12);
    assert!(refuse_assignments_whose_bundle_is_too_large([(
        "an-iphone",
        &twelve,
        max_images_for(PublishRoute::PixelGrid)
    )])
    .is_err());
    refuse_assignments_whose_bundle_is_too_large([(
        "an-android",
        &twelve,
        max_images_for(PublishRoute::Hierarchy),
    )])
    .expect("the hierarchy composer reaches twelve cells");
    assert!(
        max_images_for(PublishRoute::Hierarchy) > max_images_for(PublishRoute::PixelGrid),
        "if these are equal the split above proves nothing"
    );
}

#[test]
fn a_bundle_that_fits_the_grid_passes() {
    // Exactly at the limit is inside it: eleven images is what the guard has always
    // allowed, and moving the constant must not move the boundary.
    let eleven = bundle_of("eleven", 11);
    refuse_assignments_whose_bundle_is_too_large([(
        "an-iphone",
        &eleven,
        IOS_PIXEL_GRID_MAX_IMAGES,
    )])
    .expect("eleven is the limit, not one past it");
}

/// A bundle with `count` images and nothing else that matters here.
fn bundle_of(name: &str, count: usize) -> riviu_core::PublishBundle {
    riviu_core::PublishBundle {
        id: format!("{name}-id"),
        source_path: String::new(),
        name: name.to_string(),
        media_kind: riviu_core::PublishMediaKind::Image,
        images: (1..=count)
            .map(|order| riviu_core::PublishImage {
                path: format!("{order:02}-slide.png"),
                file_name: format!("{order:02}-slide.png"),
                order: order as u32,
                sha256: "11".repeat(32),
                byte_len: 1,
                width: 995,
                height: 1405,
            })
            .collect(),
        video: None,
        caption_path: String::new(),
        caption: String::new(),
        caption_sha256: "00".repeat(32),
        total_bytes: count as u64,
        partners: Vec::new(),
    }
}

#[test]
fn an_empty_assignment_list_is_not_the_refusal_this_gate_is_for() {
    // Emptiness is checked by its own error with its own message; this gate must not
    // steal that case and report a platform problem instead.
    refuse_devices_whose_composer_is_not_measured(std::iter::empty()).expect("nothing to refuse");
}

#[test]
fn account_lock_alert_is_rejected_in_vietnamese() {
    assert!(account_status_text_is_locked(
        "trạng thái tài khoản tài khoản của bạn đã bị khóa"
    ));
}

#[test]
fn account_lock_alert_is_rejected_in_english() {
    assert!(account_status_text_is_locked(
        "account status account locked"
    ));
}

#[test]
fn ordinary_post_confirmation_is_not_account_lock() {
    assert!(!account_status_text_is_locked("đăng công khai xác nhận"));
}
