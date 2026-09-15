use super::*;

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
