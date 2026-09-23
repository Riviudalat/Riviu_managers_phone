use super::*;

#[test]
fn verified_stop_releases_account_reservation_without_reopening_old_post() {
    let (db, _, campaign, assignments) = legacy_pending();
    let a = &assignments[0];
    db.conn().unwrap().execute("INSERT INTO publish_account_reservations(account,assignment_id,claimed_at) VALUES('fixture',?1,'now')", [&a.id]).unwrap();
    db.begin_publish_operation_stop(&campaign).unwrap();
    let before = db
        .get_publish_campaign(&campaign)
        .unwrap()
        .unwrap()
        .assignments[0]
        .effect_intent
        .clone();
    let marker = db
        .get_setting(&format!("operation.stop.publish:{campaign}"))
        .unwrap()
        .unwrap();
    assert!(db
        .record_publish_stopped_device_release(&campaign, &a.udid, &marker)
        .unwrap());
    let remaining: i64 = db
        .conn()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM publish_account_reservations WHERE assignment_id=?1",
            [&a.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        remaining, 0,
        "a closed device must not retain the old account reservation"
    );
    let after = db.get_publish_campaign(&campaign).unwrap().unwrap();
    assert_eq!(after.assignments[0].effect_intent, before);
    assert_eq!(
        after.assignments[0].state,
        crate::PublishCampaignState::Uncertain
    );
    assert!(
        db.device_app_selection_block_reason(&a.udid)
            .unwrap()
            .is_none(),
        "the same verified release must unblock app selection as well as new Publish"
    );
    db.begin_publish_operation_stop(&campaign).unwrap();
    assert!(
        db.device_app_selection_block_reason(&a.udid)
            .unwrap()
            .is_some(),
        "a new Stop generation cannot reuse an earlier release proof"
    );
}

#[test]
fn stopped_device_release_removes_only_its_hold_and_survives_restart() {
    use sha2::Digest;
    let (db, path, campaign, assignments) = legacy_pending();
    db.begin_publish_operation_stop(&campaign).unwrap();
    let marker = db
        .get_setting(&format!("operation.stop.publish:{campaign}"))
        .unwrap()
        .unwrap();
    let a = &db
        .get_publish_campaign(&campaign)
        .unwrap()
        .unwrap()
        .assignments[0];
    db.set_setting(&format!("publish.stop-release.{}", a.id), &serde_json::json!({
        "assignmentId":a.id,"udid":a.udid,"stopMarker":marker,
        "intentSha256":a.effect_intent.as_ref().map(|s|format!("{:x}",sha2::Sha256::digest(s.as_bytes())))
    }).to_string()).unwrap();
    let guard = db.publish_device_guard(&a.udid).unwrap();
    assert!(
        guard.blocking.is_empty(),
        "Stopped and closed device must be available for new content"
    );
    assert_eq!(guard.link_review.len(), 1);
    assert!(db
        .has_pending_publish_for_device(&assignments[1].udid)
        .unwrap());
    let udid = a.udid.clone();
    drop(db);
    assert!(!Database::open(path)
        .unwrap()
        .has_pending_publish_for_device(&udid)
        .unwrap());
}

#[test]
fn stopped_device_is_released_while_sibling_pipeline_is_still_draining() {
    let (db, _, campaign, assignments) = legacy_pending();
    db.begin_publish_operation_stop(&campaign).unwrap();
    let marker = db
        .get_setting(&format!("operation.stop.publish:{campaign}"))
        .unwrap()
        .unwrap();
    assert!(db
        .record_publish_stopped_device_release(&campaign, &assignments[0].udid, &marker)
        .unwrap());
    db.conn().unwrap().execute("INSERT INTO publish_pipeline_runs(campaign_id,token,created_at) VALUES(?1,'sibling-closing','now')",[&campaign]).unwrap();
    assert!(!db
        .has_pending_publish_for_device(&assignments[0].udid)
        .unwrap());
    assert!(db
        .has_pending_publish_for_device(&assignments[1].udid)
        .unwrap());
}

#[test]
fn stopped_release_is_fenced_by_marker_intent_and_explicit_resume() {
    let (db, _, campaign, assignments) = legacy_pending();
    let a = &assignments[0];
    db.begin_publish_operation_stop(&campaign).unwrap();
    let marker = db
        .get_setting(&format!("operation.stop.publish:{campaign}"))
        .unwrap()
        .unwrap();
    assert!(db
        .record_publish_stopped_device_release(&campaign, &a.udid, &marker)
        .unwrap());
    assert!(!db.has_pending_publish_for_device(&a.udid).unwrap());
    db.set_setting(
        &format!("operation.stop.result:publish:{campaign}"),
        &serde_json::json!({"state":"closed","stopMarker":marker}).to_string(),
    )
    .unwrap();
    let revision = db.publish_assignment_revision(&a.id).unwrap();
    assert_eq!(
        db.resume_publish_verification(&a.id, true, revision)
            .unwrap()
            .state,
        PublishResumeVerificationState::Accepted
    );
    assert!(db.has_pending_publish_for_device(&a.udid).unwrap());
    assert!(!db
        .record_publish_stopped_device_release(&campaign, &a.udid, &marker)
        .unwrap());
    db.begin_publish_operation_stop(&campaign).unwrap();
    assert!(!db
        .record_publish_stopped_device_release(&campaign, &a.udid, &marker)
        .unwrap());
    let marker = db
        .get_setting(&format!("operation.stop.publish:{campaign}"))
        .unwrap()
        .unwrap();
    assert!(db
        .record_publish_stopped_device_release(&campaign, &a.udid, &marker)
        .unwrap());
    db.conn()
        .unwrap()
        .execute(
            "UPDATE publish_assignments SET effect_intent='changed' WHERE id=?1",
            [&a.id],
        )
        .unwrap();
    assert!(db.has_pending_publish_for_device(&a.udid).unwrap());
}

#[test]
fn stopped_release_does_not_cover_a_second_campaign_on_same_phone() {
    let (db, _, campaign, assignments) = legacy_pending();
    let a = &assignments[0];
    db.begin_publish_operation_stop(&campaign).unwrap();
    let marker = db
        .get_setting(&format!("operation.stop.publish:{campaign}"))
        .unwrap()
        .unwrap();
    assert!(db
        .record_publish_stopped_device_release(&campaign, &a.udid, &marker)
        .unwrap());
    db.conn().unwrap().execute("INSERT INTO publish_campaigns(id,request_id,source_root,request_json,state,created_at,updated_at) VALUES('other','other','fixture','{}','verifying','now','now')",[]).unwrap();
    db.conn().unwrap().execute("INSERT INTO publish_assignments(id,campaign_id,bundle_id,udid,ordinal,state,created_at,updated_at) VALUES('other','other',?2,?1,0,'verifying','now','now')",params![a.udid,a.bundle_id]).unwrap();
    let guard = db.publish_device_guard(&a.udid).unwrap();
    assert_eq!(guard.blocking.len(), 1);
    assert_eq!(guard.blocking[0].campaign_id, "other");
    assert_eq!(guard.link_review.len(), 1);
}

#[test]
fn old_closed_result_cannot_allow_resume_while_new_stop_is_closing() {
    let (db, _, campaign, assignments) = legacy_pending();
    db.begin_publish_operation_stop(&campaign).unwrap();
    let marker = db
        .get_setting(&format!("operation.stop.publish:{campaign}"))
        .unwrap()
        .unwrap();
    db.set_setting(
        &format!("operation.stop.result:publish:{campaign}"),
        &serde_json::json!({"state":"closed","stopMarker":marker}).to_string(),
    )
    .unwrap();
    db.begin_publish_operation_stop(&campaign).unwrap();
    let revision = db.publish_assignment_revision(&assignments[0].id).unwrap();
    let result = db
        .resume_publish_verification(&assignments[0].id, true, revision)
        .unwrap();
    assert_eq!(result.reason.as_deref(), Some("stopInProgress"));
}

#[test]
fn operator_stop_preserves_post_intent_rejects_stale_observer_and_parks_checks_after_restart() {
    let (db, path, campaign, assignments) = legacy_pending();
    let before = db
        .publish_verifications_for_campaign(&campaign, 100)
        .unwrap();
    assert!(!before.is_empty());
    let intent = before[0].effect_intent.clone();
    db.conn()
        .unwrap()
        .execute(
            "UPDATE publish_assignments SET state='succeeded' WHERE id=?1",
            [&assignments[1].id],
        )
        .unwrap();
    let devices = db.stop_publish_operation(&campaign).unwrap();
    assert!(!devices.is_empty());
    assert!(!db
        .record_publish_verification_pending(&before[0], "late callback")
        .unwrap());
    let detail = db.get_publish_campaign(&campaign).unwrap().unwrap();
    assert_eq!(detail.assignments[0].effect_intent, intent);
    assert_eq!(
        detail.assignments[1].state,
        crate::PublishCampaignState::Succeeded
    );
    assert_eq!(
        detail.assignments[0].state,
        crate::PublishCampaignState::Uncertain
    );
    drop(db);
    let reopened = Database::open(path).unwrap();
    assert!(reopened.publish_operation_stopped(&campaign).unwrap());
    assert!(reopened
        .publish_verifications_for_campaign(&campaign, 100)
        .unwrap()
        .is_empty());
    assert!(reopened
        .pending_publish_verifications(100)
        .unwrap()
        .is_empty());
}

#[test]
fn stop_fences_late_success_and_diagnostic_for_legacy_succeeded_link_debt() {
    let (db, _, campaign, assignments) = legacy_pending();
    db.conn()
        .unwrap()
        .execute(
            "UPDATE publish_assignments SET state='succeeded' WHERE id=?1",
            [&assignments[0].id],
        )
        .unwrap();
    let before = db
        .publish_verifications_for_campaign(&campaign, 100)
        .unwrap()
        .remove(0);
    db.stop_publish_operation(&campaign).unwrap();
    assert!(
        !db.record_publish_verification_pending(&before, "late diagnostic")
            .unwrap(),
        "Stop must invalidate legacy success observers too"
    );
    let url = "https://www.tiktok.com/@fixture.account/photo/7400000000000000001";
    assert!(!db
        .record_verified_publish_with_sheet_row(
            &before,
            &serde_json::json!({"publicationVerified":true,"postUrl":url}).to_string(),
            url,
            "bot",
            &[]
        )
        .unwrap());
}

#[test]
fn stop_rejects_unbound_idle_observation() {
    let (db, _, campaign, assignments) = legacy_pending();
    db.stop_publish_operation(&campaign).unwrap();
    assert!(!db
        .observe_stale_publish_idle(
            &assignments[0].id,
            "fixture.account",
            "com.ss.android.ugc.trill",
            &"a".repeat(64)
        )
        .unwrap());
    assert!(db
        .has_pending_publish_for_device(&assignments[0].udid)
        .unwrap());
}

#[test]
fn confirmed_resume_only_enrolls_selected_stopped_assignment_and_survives_restart() {
    let (db, path, campaign, assignments) = legacy_pending();
    db.stop_publish_operation(&campaign).unwrap();
    let before = db.get_publish_campaign(&campaign).unwrap().unwrap();
    let caps = db.publish_recovery_capabilities(&campaign).unwrap();
    assert_eq!(caps.len(), 3);
    let cap = caps
        .iter()
        .find(|c| c.assignment_id == assignments[0].id)
        .unwrap();
    assert!(cap.resume_verification.allowed);
    assert!(!cap.check_link.allowed);
    assert!(!cap.retry_before_post.allowed);
    let result = db
        .resume_publish_verification(&cap.assignment_id, true, cap.revision)
        .unwrap();
    assert_eq!(result.state, PublishResumeVerificationState::Accepted);
    let row = db
        .pending_current_publish_verifications(100)
        .unwrap()
        .remove(0);
    assert_eq!(row.assignment_id, assignments[0].id);
    assert_eq!(row.effect_intent, before.assignments[0].effect_intent);
    let evidence: serde_json::Value =
        serde_json::from_str(row.evidence_json.as_deref().unwrap()).unwrap();
    assert_eq!(evidence["verificationResume"]["version"], 2);
    assert_eq!(evidence["verificationResume"]["campaignId"], campaign);
    assert!(
        db.has_pending_publish_for_device(&row.udid).unwrap(),
        "resume never releases upload guard"
    );
    let again = db
        .resume_publish_verification(&cap.assignment_id, true, cap.revision)
        .unwrap();
    assert_eq!(again.state, PublishResumeVerificationState::AlreadyPending);
    assert_eq!(
        db.pending_current_publish_verifications(100).unwrap()[0].evidence_json,
        row.evidence_json
    );
    assert_eq!(
        db.get_publish_campaign(&campaign)
            .unwrap()
            .unwrap()
            .campaign
            .state,
        crate::PublishCampaignState::Cancelled
    );
    assert_eq!(
        db.get_publish_campaign(&campaign)
            .unwrap()
            .unwrap()
            .assignments[1]
            .evidence_json,
        before.assignments[1].evidence_json
    );
    assert!(db.claim_publish_pipeline(&campaign).unwrap().is_none());
    let conn = db.conn().unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM publish_dispatch_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(conn);
    drop(db);
    let db = Database::open(path).unwrap();
    assert_eq!(
        db.pending_current_publish_verifications(100).unwrap().len(),
        1
    );
    db.stop_publish_operation(&campaign).unwrap();
    assert!(db
        .pending_current_publish_verifications(100)
        .unwrap()
        .is_empty());
    assert!(!db
        .record_publish_verification_pending(&row, "late after second stop")
        .unwrap());
    let revision = db.publish_assignment_revision(&row.assignment_id).unwrap();
    assert_eq!(
        db.resume_publish_verification(&row.assignment_id, true, cap.revision)
            .unwrap()
            .state,
        PublishResumeVerificationState::Stale
    );
    assert_eq!(
        db.resume_publish_verification(&row.assignment_id, true, revision)
            .unwrap()
            .state,
        PublishResumeVerificationState::Accepted
    );
    assert!(!db
        .record_publish_verification_pending(&row, "late after second resume")
        .unwrap());
}

#[test]
fn resume_rejects_unconfirmed_stale_incomplete_identity_active_pipeline_and_explicit_review() {
    let (db, _, campaign, assignments) = legacy_pending();
    db.stop_publish_operation(&campaign).unwrap();
    let id = &assignments[0].id;
    let revision = db.publish_assignment_revision(id).unwrap();
    assert_eq!(
        db.resume_publish_verification(id, false, revision)
            .unwrap()
            .state,
        PublishResumeVerificationState::Ineligible
    );
    assert_eq!(
        db.resume_publish_verification(id, true, revision - 1)
            .unwrap()
            .state,
        PublishResumeVerificationState::Stale
    );
    db.conn()
        .unwrap()
        .execute(
            "INSERT INTO publish_pipeline_runs VALUES(?1,'active',?2)",
            params![campaign, Utc::now().to_rfc3339()],
        )
        .unwrap();
    assert_eq!(
        db.resume_publish_verification(id, true, revision)
            .unwrap()
            .state,
        PublishResumeVerificationState::Ineligible
    );
    db.conn()
        .unwrap()
        .execute("DELETE FROM publish_pipeline_runs", [])
        .unwrap();
    db.conn()
        .unwrap()
        .execute(
            "UPDATE publish_assignments SET effect_intent='{}' WHERE id=?1",
            [id],
        )
        .unwrap();
    assert_eq!(
        db.resume_publish_verification(id, true, revision)
            .unwrap()
            .state,
        PublishResumeVerificationState::Ineligible
    );
    let (db, _, campaign, assignments) = legacy_pending();
    db.conn().unwrap().execute("UPDATE publish_assignments SET evidence_json='{\"verificationStatus\":{\"state\":\"needsReview\",\"cause\":\"accountMismatch\"}}' WHERE id=?1",[&assignments[0].id]).unwrap();
    db.stop_publish_operation(&campaign).unwrap();
    let revision = db.publish_assignment_revision(&assignments[0].id).unwrap();
    assert_eq!(
        db.resume_publish_verification(&assignments[0].id, true, revision)
            .unwrap()
            .state,
        PublishResumeVerificationState::Ineligible
    );
}

#[test]
fn retry_capability_matches_checked_claim_even_with_an_unrelated_live_sibling() {
    for active in [false, true] {
        let (db, _, campaign, assignments) = legacy_pending();
        db.conn().unwrap().execute("UPDATE publish_assignments SET state='failed_before_dispatch',effect_intent=NULL WHERE id=?1",[&assignments[0].id]).unwrap();
        if active {
            db.conn()
                .unwrap()
                .execute(
                    "INSERT INTO publish_pipeline_runs VALUES(?1,'active',?2)",
                    params![campaign, Utc::now().to_rfc3339()],
                )
                .unwrap();
        }
        let caps = db.publish_recovery_capabilities(&campaign).unwrap();
        let cap = caps
            .iter()
            .find(|c| c.assignment_id == assignments[0].id)
            .unwrap();
        assert!(cap.retry_before_post.allowed);
        assert!(db
            .claim_publish_assignment_retry_checked(
                &assignments[0].id,
                cap.revision,
                &Uuid::new_v4().to_string()
            )
            .unwrap()
            .is_some());
    }
}

#[test]
fn resumed_idle_proof_is_generation_fenced_and_requires_exact_package() {
    let (db, _, campaign, assignments) = legacy_pending();
    db.conn().unwrap().execute("UPDATE publish_assignments SET effect_intent=json_set(effect_intent,'$.package','com.ss.android.ugc.trill') WHERE id=?1",[&assignments[0].id]).unwrap();
    db.stop_publish_operation(&campaign).unwrap();
    let rev = db.publish_assignment_revision(&assignments[0].id).unwrap();
    db.resume_publish_verification(&assignments[0].id, true, rev)
        .unwrap();
    let row = db
        .pending_current_publish_verifications(100)
        .unwrap()
        .remove(0);
    assert!(!db
        .observe_stale_publish_idle_for_candidate(
            &row,
            "fixture.account",
            "wrong.package",
            &"a".repeat(64)
        )
        .unwrap());
    assert!(db
        .observe_stale_publish_idle_for_candidate(
            &row,
            "fixture.account",
            "com.ss.android.ugc.trill",
            &"a".repeat(64)
        )
        .unwrap());
    assert!(!db.has_pending_publish_for_device(&row.udid).unwrap());
    db.stop_publish_operation(&campaign).unwrap();
    assert!(
        db.has_pending_publish_for_device(&row.udid).unwrap(),
        "Stop revokes prior idle proof generation"
    );
    assert!(!db
        .observe_stale_publish_idle_for_candidate(
            &row,
            "fixture.account",
            "com.ss.android.ugc.trill",
            &"b".repeat(64)
        )
        .unwrap());
    let rev = db.publish_assignment_revision(&row.assignment_id).unwrap();
    db.resume_publish_verification(&row.assignment_id, true, rev)
        .unwrap();
    assert!(!db
        .observe_stale_publish_idle_for_candidate(
            &row,
            "fixture.account",
            "com.ss.android.ugc.trill",
            &"b".repeat(64)
        )
        .unwrap());
    let fresh = db
        .pending_current_publish_verifications(100)
        .unwrap()
        .remove(0);
    let url = "https://www.tiktok.com/@fixture.account/photo/7400000000000000001";
    let evidence = serde_json::json!({"publicationVerified":true,"postUrl":url}).to_string();
    assert!(!db
        .record_verified_publish_with_sheet_row(&row, &evidence, url, "bot", &[])
        .unwrap());
    assert!(db
        .record_verified_publish_with_sheet_row(&fresh, &evidence, url, "bot", &[])
        .unwrap());
    assert!(!db
        .record_verified_publish_with_sheet_row(&fresh, &evidence, url, "bot", &[])
        .unwrap());
}

#[test]
fn changed_stop_marker_fences_diagnostic_and_success_even_without_assignment_revision_change() {
    let (db, _, campaign, _) = legacy_pending();
    let candidate = db
        .publish_verifications_for_campaign(&campaign, 100)
        .unwrap()
        .remove(0);
    db.set_setting(
        &format!("operation.stop.publish:{campaign}"),
        "{\"requestedAt\":\"legacy\"}",
    )
    .unwrap();
    assert!(!db
        .record_publish_verification_pending(&candidate, "late")
        .unwrap());
    let url = "https://www.tiktok.com/@fixture.account/photo/7400000000000000001";
    assert!(!db
        .record_verified_publish_with_sheet_row(
            &candidate,
            &serde_json::json!({"publicationVerified":true,"postUrl":url}).to_string(),
            url,
            "bot",
            &[]
        )
        .unwrap());
}

#[test]
fn automatic_pending_resume_keeps_existing_cadence_without_enrollment() {
    let (db, _, campaign, assignments) = legacy_pending();
    db.conn().unwrap().execute("UPDATE publish_campaigns SET request_json=json_set(request_json,'$.verificationContractVersion',1) WHERE id=?1",[&campaign]).unwrap();
    let row = db
        .pending_current_publish_verifications(100)
        .unwrap()
        .remove(0);
    db.record_publish_verification_pending(&row, "pending")
        .unwrap();
    let row = db
        .pending_current_publish_verifications(100)
        .unwrap()
        .into_iter()
        .find(|r| r.assignment_id == assignments[0].id)
        .unwrap();
    assert_eq!(
        db.resume_publish_verification(&row.assignment_id, true, row.revision)
            .unwrap()
            .state,
        PublishResumeVerificationState::AlreadyPending
    );
    let after = db
        .pending_current_publish_verifications(100)
        .unwrap()
        .into_iter()
        .find(|r| r.assignment_id == row.assignment_id)
        .unwrap();
    assert_eq!(after.evidence_json, row.evidence_json);
    assert_eq!(after.revision, row.revision);
}

#[test]
fn existing_submitted_sibling_verifies_while_campaign_pipeline_runs() {
    let (db, _, campaign, _) = legacy_pending();
    db.conn()
        .unwrap()
        .execute(
            "INSERT INTO publish_pipeline_runs VALUES(?1,'active',?2)",
            params![campaign, Utc::now().to_rfc3339()],
        )
        .unwrap();
    let rows = db
        .publish_verifications_for_campaign(&campaign, 100)
        .unwrap();
    assert_eq!(rows.len(), 3);
    assert!(db.publish_verification_is_current(&rows[0]).unwrap());
    assert!(
        !db.publish_recovery_capabilities(&campaign).unwrap()[0]
            .resume_verification
            .allowed
    );
}

#[test]
fn legacy_v1_cannot_cross_stop_and_v2_cannot_move_campaign_or_intent() {
    let (db, _, campaign, assignments) = legacy_pending();
    let old = db
        .publish_verifications_for_campaign(&campaign, 100)
        .unwrap()
        .remove(0);
    db.record_manual_publish_verification_diagnostic(&old, "pending", "clipboardUnchanged", None)
        .unwrap();
    db.set_setting(
        &format!("operation.stop.publish:{campaign}"),
        "{\"requestedAt\":\"old\"}",
    )
    .unwrap();
    assert!(db
        .pending_current_publish_verifications(100)
        .unwrap()
        .is_empty());
    let revision = db.publish_assignment_revision(&assignments[0].id).unwrap();
    assert_eq!(
        db.resume_publish_verification(&assignments[0].id, true, revision)
            .unwrap()
            .state,
        PublishResumeVerificationState::Accepted
    );
    let row = db
        .pending_current_publish_verifications(100)
        .unwrap()
        .remove(0);
    let mut copied = row.clone();
    copied.campaign_id = "another-campaign".into();
    assert!(!explicitly_resumed_verification(&copied));
    db.conn().unwrap().execute("UPDATE publish_assignments SET effect_intent=json_set(effect_intent,'$.expectedAccount','another.actor') WHERE id=?1",[&row.assignment_id]).unwrap();
    assert!(!db
        .record_publish_verification_pending(&row, "stale intent")
        .unwrap());
    let url = "https://www.tiktok.com/@fixture.account/photo/7400000000000000001";
    assert!(!db
        .record_verified_publish_with_sheet_row(
            &row,
            &serde_json::json!({"publicationVerified":true,"postUrl":url}).to_string(),
            url,
            "bot",
            &[]
        )
        .unwrap());
    assert!(db
        .pending_current_publish_verifications(100)
        .unwrap()
        .is_empty());
}

#[test]
fn resumed_verified_post_does_not_resurrect_superseded_sheet_or_cancelled_campaign() {
    let (db, _, campaign, assignments) = legacy_pending();
    db.stop_publish_operation(&campaign).unwrap();
    let id = &assignments[0].id;
    db.conn().unwrap().execute("INSERT INTO publish_sheet_sync_state(assignment_id,superseded_epoch) VALUES(?1,'closed-epoch')",[id]).unwrap();
    let request = db.publish_campaign_request(&campaign).unwrap();
    db.resume_publish_verification(id, true, db.publish_assignment_revision(id).unwrap())
        .unwrap();
    let row = db
        .pending_current_publish_verifications(100)
        .unwrap()
        .remove(0);
    db.record_publish_verification_pending(&row, "pending link")
        .unwrap();
    let row = db
        .pending_current_publish_verifications(100)
        .unwrap()
        .remove(0);
    let evidence: serde_json::Value =
        serde_json::from_str(row.evidence_json.as_deref().unwrap()).unwrap();
    let checked = DateTime::parse_from_rfc3339(
        evidence["verificationStatus"]["checkedAt"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let due = DateTime::parse_from_rfc3339(
        evidence["verificationStatus"]["nextCheckAt"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!((due - checked).num_seconds(), 300);
    let url = "https://www.tiktok.com/@fixture.account/photo/7400000000000000001";
    assert!(db
        .record_verified_publish_with_sheet_row(
            &row,
            &serde_json::json!({"publicationVerified":true,"postUrl":url}).to_string(),
            url,
            "bot",
            &[]
        )
        .unwrap());
    assert!(db.pending_publish_sheet_row(id).unwrap().is_none());
    assert_eq!(
        serde_json::to_value(db.publish_campaign_request(&campaign).unwrap()).unwrap(),
        serde_json::to_value(request).unwrap()
    );
    assert_eq!(
        db.get_publish_campaign(&campaign)
            .unwrap()
            .unwrap()
            .campaign
            .state,
        crate::PublishCampaignState::Cancelled
    );
    let caps = db.publish_recovery_capabilities(&campaign).unwrap();
    let cap = caps.iter().find(|c| c.assignment_id == *id).unwrap();
    assert!(!cap.verification_resumed);
    assert!(!cap.resume_verification.allowed);
    assert_eq!(
        db.resume_publish_verification(id, true, row.revision)
            .unwrap()
            .state,
        PublishResumeVerificationState::AlreadyVerified
    );
}

fn legacy_pending() -> (
    Database,
    PathBuf,
    String,
    Vec<crate::PublishAssignmentRecord>,
) {
    let (db, path, campaign, assignments) = super::super::publish_pipeline::tests::fixture();
    let intent=serde_json::json!({"effectIntent":"post","expectedAccount":"fixture.account","submittedAt":"2026-09-09T11:49:12Z"}).to_string();
    db.conn().unwrap().execute("UPDATE publish_assignments SET state='verifying',effect_intent=?2,evidence_json='{}' WHERE campaign_id=?1",params![campaign,intent]).unwrap();
    db.conn()
        .unwrap()
        .execute(
            "UPDATE publish_campaigns SET state='verifying' WHERE id=?1",
            [&campaign],
        )
        .unwrap();
    (db, path, campaign, assignments)
}

#[test]
fn explicit_legacy_check_resumes_only_its_publication_after_restart_without_post_or_epoch_changes()
{
    let (db, path, campaign, assignments) = legacy_pending();
    assert!(db
        .pending_current_publish_verifications(100)
        .unwrap()
        .is_empty());
    let candidate = db
        .publish_verifications_for_campaign(&campaign, 100)
        .unwrap()
        .into_iter()
        .find(|row| row.assignment_id == assignments[0].id)
        .unwrap();
    db.conn().unwrap().execute("INSERT INTO publish_sheet_sync_state(assignment_id,superseded_epoch) VALUES(?1,'closed-old-epoch')",[&candidate.assignment_id]).unwrap();
    assert!(db
        .record_manual_publish_verification_diagnostic(
            &candidate,
            "checked exact publication; link pending",
            "searchBudgetExhausted",
            Some(&serde_json::json!({"contractVersion":1}))
        )
        .unwrap());
    assert!(!db
        .record_manual_publish_verification_diagnostic(&candidate, "stale", "readFailed", None)
        .unwrap());
    drop(db);
    let db = Database::open(path).unwrap();
    let rows = db.pending_current_publish_verifications(100).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].assignment_id, candidate.assignment_id);
    assert_eq!(rows[0].effect_intent, candidate.effect_intent);
    let evidence: serde_json::Value =
        serde_json::from_str(rows[0].evidence_json.as_deref().unwrap()).unwrap();
    let checked = DateTime::parse_from_rfc3339(
        evidence["verificationStatus"]["checkedAt"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let due = DateTime::parse_from_rfc3339(
        evidence["verificationStatus"]["nextCheckAt"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!((due - checked).num_seconds(), 300);
    assert!(!rows[0].is_due(checked.with_timezone(&Utc)));
    assert!(rows[0].is_due(due.with_timezone(&Utc)));
    assert!(db.claim_publish_pipeline(&campaign).unwrap().is_none());
    let conn = db.conn().unwrap();
    let (epoch,outbox,jobs):(String,i64,i64)=conn.query_row("SELECT (SELECT superseded_epoch FROM publish_sheet_sync_state WHERE assignment_id=?1),(SELECT COUNT(*) FROM publish_sheet_outbox),(SELECT COUNT(*) FROM publish_dispatch_jobs)",[&candidate.assignment_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(epoch, "closed-old-epoch");
    assert_eq!((outbox, jobs), (0, 0));
    drop(conn);
    let url = "https://www.tiktok.com/@fixture.account/photo/7400000000000000001";
    assert!(db
        .record_verified_publish_with_sheet_row(
            &rows[0],
            &serde_json::json!({"publicationVerified":true,"postUrl":url}).to_string(),
            url,
            "bot",
            &[]
        )
        .unwrap());
    assert!(db
        .pending_publish_sheet_row(&candidate.assignment_id)
        .unwrap()
        .is_none());
    assert_eq!(
        db.get_publish_assignment_detail(&campaign, &candidate.assignment_id)
            .unwrap()
            .unwrap()
            .assignments[0]
            .state,
        crate::PublishCampaignState::Succeeded
    );
}

#[test]
fn legacy_automatic_diagnostic_does_not_enroll_unrelated_history() {
    let (db, _, campaign, _) = legacy_pending();
    let row = db
        .publish_verifications_for_campaign(&campaign, 100)
        .unwrap()
        .remove(0);
    db.record_publish_verification_diagnostic(
        &row,
        "old observation",
        "searchBudgetExhausted",
        Some(&serde_json::json!({"contractVersion":1})),
    )
    .unwrap();
    assert!(db
        .pending_current_publish_verifications(100)
        .unwrap()
        .is_empty());
}

#[test]
fn legacy_manual_missing_identity_drafts_and_composer_remain_review_without_next_check() {
    for code in [
        "submissionIdentityMissing",
        "draftsObserved",
        "composerOrUpload",
    ] {
        let (db, _, campaign, assignments) = legacy_pending();
        if code == "submissionIdentityMissing" {
            db.conn().unwrap().execute("UPDATE publish_assignments SET effect_intent='{\"effectIntent\":\"post\"}' WHERE id=?1",[&assignments[0].id]).unwrap();
        }
        let row = db
            .publish_verifications_for_campaign(&campaign, 100)
            .unwrap()
            .into_iter()
            .find(|r| r.assignment_id == assignments[0].id)
            .unwrap();
        db.record_manual_publish_verification_diagnostic(&row, "needs review", code, None)
            .unwrap();
        assert!(db
            .pending_current_publish_verifications(100)
            .unwrap()
            .is_empty());
        let detail = db
            .get_publish_assignment_detail(&campaign, &row.assignment_id)
            .unwrap()
            .unwrap();
        let evidence: serde_json::Value =
            serde_json::from_str(detail.assignments[0].evidence_json.as_deref().unwrap()).unwrap();
        assert_eq!(evidence["verificationStatus"]["state"], "needsReview");
        assert!(evidence["verificationStatus"]["nextCheckAt"].is_null());
        assert!(evidence.get("verificationResume").is_none());
    }
}

#[test]
fn explicit_resume_marker_cannot_move_to_another_assignment_or_changed_post_intent() {
    let (db, _, campaign, _) = legacy_pending();
    let row = db
        .publish_verifications_for_campaign(&campaign, 100)
        .unwrap()
        .remove(0);
    db.record_manual_publish_verification_diagnostic(&row, "pending", "clipboardUnchanged", None)
        .unwrap();
    let original = db
        .pending_current_publish_verifications(100)
        .unwrap()
        .remove(0);
    assert!(explicitly_resumed_verification(&original));
    let mut altered = original.clone();
    altered.assignment_id = "another-publication".into();
    assert!(!explicitly_resumed_verification(&altered));
    let mut altered = original.clone();
    altered.effect_intent=Some("{\"effectIntent\":\"post\",\"expectedAccount\":\"changed.account\",\"submittedAt\":\"2026-09-09T11:49:12Z\"}".into());
    assert!(!explicitly_resumed_verification(&altered));
}
