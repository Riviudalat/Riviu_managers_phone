use super::super::publish_pipeline::tests::fixture;
use super::*;

#[test]
fn transient_transfer_failure_is_requeued_without_touching_sibling_publications() {
    let (db, _, campaign, assignments) = fixture();
    db.claim_publish_pipeline(&campaign).unwrap().unwrap();
    let job = db.pending_publish_dispatch(10).unwrap().remove(0);
    assert!(db.claim_publish_dispatch(&job, 0).unwrap());
    assert!(db
        .finish_publish_dispatch(&job, Some("adb: connection reset"))
        .unwrap());
    let conn = db.conn().unwrap();
    let state: String = conn
        .query_row(
            "SELECT state FROM publish_dispatch_jobs WHERE assignment_id=?1",
            [&job.assignment_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        state, "queued",
        "transient pre-Post work must be retried by the durable dispatcher"
    );
    for a in assignments.iter().filter(|a| a.id != job.assignment_id) {
        let intent: Option<String> = conn
            .query_row(
                "SELECT effect_intent FROM publish_assignments WHERE id=?1",
                [&a.id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(intent.is_none());
    }
}

#[test]
fn explicit_failed_assignment_retry_preserves_identity_and_leaves_all_siblings_untouched() {
    let (db, _, campaign, assignments) = fixture();
    let prior_run = db.claim_publish_pipeline(&campaign).unwrap().unwrap();
    let prior = db
        .pending_publish_dispatch(10)
        .unwrap()
        .into_iter()
        .find(|job| job.assignment_id == assignments[0].id)
        .unwrap();
    assert!(db.claim_publish_dispatch(&prior, 0).unwrap());
    assert!(db
        .finish_publish_dispatch(&prior, Some("cold_launch_failed"))
        .unwrap());
    db.finish_publish_pipeline(&prior_run).unwrap();
    let conn = db.conn().unwrap();
    conn.execute("UPDATE publish_assignments SET state='verifying',effect_intent='{\"effectIntent\":\"post\"}',evidence_json='{\"awaiting\":true}' WHERE id=?1", [&assignments[1].id]).unwrap();
    conn.execute(
        "UPDATE publish_assignments SET state='failed_before_dispatch' WHERE id=?1",
        [&assignments[2].id],
    )
    .unwrap();
    conn.execute(
        "UPDATE publish_campaigns SET state='verifying' WHERE id=?1",
        [&campaign],
    )
    .unwrap();
    drop(conn);
    let before = db.get_publish_campaign(&campaign).unwrap().unwrap();
    let run = db
        .claim_publish_assignment_retry(&assignments[0].id)
        .unwrap()
        .unwrap();
    assert_eq!(run.campaign_id, campaign);
    assert_ne!(run.token, prior_run.token);
    assert!(db
        .claim_publish_assignment_retry(&assignments[0].id)
        .unwrap()
        .is_none());
    let jobs = db.pending_publish_dispatch(10).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].assignment_id, assignments[0].id);
    assert_ne!(jobs[0].attempt_id, prior.attempt_id);
    assert!(!db.finish_publish_dispatch(&prior, None).unwrap());
    let after = db.get_publish_campaign(&campaign).unwrap().unwrap();
    assert_eq!(
        after.assignments[0].publication_id,
        before.assignments[0].publication_id
    );
    for index in [1, 2] {
        assert_eq!(
            serde_json::to_value(&after.assignments[index]).unwrap(),
            serde_json::to_value(&before.assignments[index]).unwrap()
        );
    }
}

#[test]
fn explicit_retry_restores_three_sound_retries_without_reopening_post() {
    let (db, _, campaign, assignments) = fixture();
    let prior_run = db.claim_publish_pipeline(&campaign).unwrap().unwrap();
    let prior = db
        .pending_publish_dispatch(10)
        .unwrap()
        .into_iter()
        .find(|job| job.assignment_id == assignments[0].id)
        .unwrap();
    assert!(db.claim_publish_dispatch(&prior, 0).unwrap());
    db.init_publish_recovery(&prior.assignment_id, &prior_run.token)
        .unwrap();
    db.update_publish_recovery_step(&prior.assignment_id, &prior_run.token, "sound", None)
        .unwrap();
    for _ in 0..3 {
        assert!(db
            .reserve_publish_step_retry(&prior.assignment_id, &prior_run.token, "network")
            .unwrap()
            .is_some());
    }
    assert!(db
        .reserve_publish_step_retry(&prior.assignment_id, &prior_run.token, "network")
        .unwrap()
        .is_none());
    assert!(db
        .finish_publish_dispatch(&prior, Some("sound identity changed"))
        .unwrap());
    db.finish_publish_pipeline(&prior_run).unwrap();

    let revision = db
        .publish_assignment_revision(&prior.assignment_id)
        .unwrap();
    let run = db
        .claim_publish_assignment_retry_checked(
            &prior.assignment_id,
            revision,
            &Uuid::new_v4().to_string(),
        )
        .unwrap()
        .unwrap();
    let recovery = db
        .publish_recovery_state(&prior.assignment_id)
        .unwrap()
        .unwrap();
    assert!(recovery.manual);
    assert_eq!(recovery.max_retries, 3);
    assert_eq!(recovery.retries_used, 0);
    assert!(recovery.counts.is_empty());
    db.init_publish_recovery(&prior.assignment_id, &run.token)
        .unwrap();
    db.update_publish_recovery_step(&prior.assignment_id, &run.token, "sound", None)
        .unwrap();
    for _ in 0..3 {
        assert!(db
            .reserve_publish_step_retry(&prior.assignment_id, &run.token, "network")
            .unwrap()
            .is_some());
    }
    assert!(db
        .reserve_publish_step_retry(&prior.assignment_id, &run.token, "network")
        .unwrap()
        .is_none());
    db.conn()
        .unwrap()
        .execute(
            "UPDATE publish_assignments SET effect_intent='post' WHERE id=?1",
            [&prior.assignment_id],
        )
        .unwrap();
    assert!(db
        .reserve_publish_step_retry(&prior.assignment_id, &run.token, "network")
        .is_err());
}

#[test]
fn checked_pre_post_retry_rebinds_current_recommended_and_audits_prior_choice() {
    for (confirmed, step, checkpoint, pre_post_retries) in [
        (false, "sound", "mediaSelected", 0),
        (false, "prePost", "approved", 0),
        (true, "sound", "mediaSelected", 0),
        (false, "caption", "mediaSelected", 0),
        (false, "sound", "soundConfirmed", 0),
        (false, "prePost", "captionEntered", 0),
        (false, "prePost", "captionConfirmed", 0),
        (false, "sound", "mediaSelected", 1),
    ] {
        let (db, _, campaign, assignments) = fixture();
        let run = db.claim_publish_pipeline(&campaign).unwrap().unwrap();
        let job = db.pending_publish_dispatch(10).unwrap().remove(0);
        assert!(db.claim_publish_dispatch(&job, 0).unwrap());
        db.init_publish_recovery(&job.assignment_id, &run.token)
            .unwrap();
        let sound = crate::SoundSelectionEvidence {
            section: crate::publish::SoundSectionKind::Trending,
            title: "A stale Hot recommendation".into(),
            artist: "Artist".into(),
            index: 0,
            candidates_digest: "frozen-pool".into(),
            confirmed,
        };
        db.bind_publish_recovery_sound(&job.assignment_id, &run.token, Some(&sound))
            .unwrap();
        db.update_publish_recovery_step(&job.assignment_id, &run.token, step, Some(checkpoint))
            .unwrap();
        if pre_post_retries > 0 {
            db.conn().unwrap().execute(
                "UPDATE publish_recovery_state SET payload=json_set(payload,'$.counts.prePost',?2) WHERE assignment_id=?1",
                params![job.assignment_id, pre_post_retries],
            ).unwrap();
        }
        assert!(db
            .finish_publish_dispatch(&job, Some("sound identity changed"))
            .unwrap());
        db.finish_publish_pipeline(&run).unwrap();
        let revision = db.publish_assignment_revision(&job.assignment_id).unwrap();
        let request = Uuid::new_v4().to_string();
        db.claim_publish_assignment_retry_checked(&job.assignment_id, revision, &request)
            .unwrap()
            .unwrap();
        let recovery = db
            .publish_recovery_state(&job.assignment_id)
            .unwrap()
            .unwrap();
        assert!(recovery.sound.is_none());
        let current_sound = crate::SoundSelectionEvidence {
            title: "Current Recommended choice".into(),
            candidates_digest: "current-pool".into(),
            confirmed: false,
            ..sound.clone()
        };
        let retry_run = db.claim_publish_assignment_retry_checked(&job.assignment_id, revision, &request)
            .unwrap().unwrap();
        db.bind_publish_recovery_sound(&job.assignment_id, &retry_run.token, Some(&current_sound)).unwrap();
        db.claim_publish_assignment_retry_checked(&job.assignment_id, revision, &request).unwrap().unwrap();
        assert_eq!(db.publish_recovery_state(&job.assignment_id).unwrap().unwrap().sound, Some(current_sound));
        let conn = db.conn().unwrap();
        let archived: Option<String> = conn.query_row(
            "SELECT detail FROM operation_device_events WHERE source_kind='publish' AND source_id=?1 AND action='publishSoundRebind' ORDER BY rowid DESC LIMIT 1",
            [&campaign], |row| row.get(0),
        ).optional().unwrap();
        assert!(archived.is_some());
        if let Some(payload) = archived {
            let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
            assert_eq!(payload["assignmentId"], job.assignment_id);
            assert_eq!(payload["requestId"], request);
            assert_eq!(payload["priorSound"], serde_json::to_value(sound).unwrap());
        }
        assert!(assignments.iter().all(|assignment| {
            db.get_publish_assignment_detail(&campaign, &assignment.id)
                .unwrap()
                .unwrap()
                .assignments[0]
                .effect_intent
                .is_none()
        }));
    }
}

#[test]
fn explicit_failed_retry_rejects_effects_completed_assignments_and_terminal_parents() {
    for (state, intent, parent) in [
        (
            "failed_before_dispatch",
            Some("{\"effectIntent\":\"post\"}"),
            "verifying",
        ),
        ("succeeded", None, "verifying"),
        ("verifying", None, "verifying"),
        ("uncertain", None, "uncertain"),
        ("failed_before_dispatch", None, "cancelled"),
        ("failed_before_dispatch", None, "missed"),
        ("failed_before_dispatch", None, "scheduled"),
    ] {
        let (db, _, campaign, assignments) = fixture();
        let conn = db.conn().unwrap();
        conn.execute(
            "UPDATE publish_assignments SET state=?2,effect_intent=?3 WHERE id=?1",
            params![assignments[0].id, state, intent],
        )
        .unwrap();
        conn.execute(
            "UPDATE publish_campaigns SET state=?2 WHERE id=?1",
            params![campaign, parent],
        )
        .unwrap();
        drop(conn);
        assert!(
            db.claim_publish_assignment_retry(&assignments[0].id)
                .unwrap()
                .is_none(),
            "{state}/{parent}"
        );
        assert!(!db.has_active_publish_pipeline(&campaign).unwrap());
    }
}

#[test]
fn explicit_failed_retry_claim_is_atomic_across_connections() {
    let (db, path, campaign, assignments) = fixture();
    db.conn()
        .unwrap()
        .execute(
            "UPDATE publish_assignments SET state='failed_before_dispatch' WHERE campaign_id=?1",
            [&campaign],
        )
        .unwrap();
    db.conn()
        .unwrap()
        .execute(
            "UPDATE publish_campaigns SET state='failed_before_dispatch' WHERE id=?1",
            [&campaign],
        )
        .unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let mut callers = Vec::new();
    for _ in 0..2 {
        let db = Database::open(&path).unwrap();
        let barrier = barrier.clone();
        let assignment = assignments[0].id.clone();
        callers.push(std::thread::spawn(move || {
            barrier.wait();
            db.claim_publish_assignment_retry(&assignment).unwrap()
        }));
    }
    barrier.wait();
    assert_eq!(
        callers
            .into_iter()
            .filter_map(|caller| caller.join().unwrap())
            .count(),
        1
    );
    let jobs = db.pending_publish_dispatch(10).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].assignment_id, assignments[0].id);
}

#[test]
fn publication_survives_retries_and_stale_claims_cannot_finish_new_attempt() {
    let (db, _path, campaign, assignments) = fixture();
    let run = db.claim_publish_pipeline(&campaign).unwrap().unwrap();
    assert!(db.claim_publish_pipeline(&campaign).unwrap().is_none());
    let first = db.pending_publish_dispatch(10).unwrap().remove(0);
    assert!(db.claim_publish_dispatch(&first, 0).unwrap());
    assert!(!db.claim_publish_dispatch(&first, 0).unwrap());
    assert!(db
        .finish_publish_dispatch(&first, Some("before_send"))
        .unwrap());
    db.finish_publish_pipeline(&run).unwrap();
    let next = db.claim_publish_pipeline(&campaign).unwrap().unwrap();
    let second = db
        .pending_publish_dispatch(10)
        .unwrap()
        .into_iter()
        .find(|j| j.assignment_id == first.assignment_id)
        .unwrap();
    assert_ne!(first.attempt_id, second.attempt_id);
    assert!(!db.finish_publish_dispatch(&first, None).unwrap());
    let detail = db.get_publish_campaign(&campaign).unwrap().unwrap();
    for row in detail.assignments {
        assert_eq!(row.publication_id, row.id);
    }
    assert_eq!(assignments.len(), 3);
    db.finish_publish_pipeline(&next).unwrap();
}

#[test]
fn permits_are_global_across_connections_and_release_by_exact_device() {
    let (db, path, campaign, assignments) = fixture();
    // This case owns configured admission, independent of the default fleet cap.
    db.set_setting("publish.dispatch.limits", r#"{"transfer":4,"compose":4,"verify":4,"deviceTotal":8}"#).unwrap();
    let other = Database::open(path).unwrap();
    let mut permits = Vec::new();
    for n in 0..4 {
        permits.push(
            db.try_publish_work(&format!("t{n}"), "transfer", "owner")
                .unwrap()
                .unwrap(),
        );
    }
    assert!(other
        .try_publish_work("extra", "transfer", "owner")
        .unwrap()
        .is_none());
    assert!(other
        .try_publish_work("t0", "verify", "observer")
        .unwrap()
        .is_none());
    for n in 0..65 {
        permits.push(
            other
                .try_publish_work(&format!("v{n}"), "verify", "observer")
                .unwrap()
                .unwrap(),
        );
    }
    assert!(db
        .try_publish_work("post", "compose", "poster")
        .unwrap()
        .is_some());
    permits.remove(0);
    assert!(db
        .try_publish_work("post", "compose", "poster")
        .unwrap()
        .is_some());
    // A durable candidate can read metadata while its physical phone is composing.
    // Both connections use production admission, not a mocked grant.
    let assignment = &assignments[0];
    let intent = serde_json::json!({
        "effectIntent":"post", "verificationContractVersion":1, "expectedAccount":"fixture",
        "submittedAt":"2026-10-06T01:00:00Z", "package":"com.zhiliaoapp.musically",
        "version":"45.7.3", "locale":"en", "captionSha256":"a".repeat(64),
        "bundleId":assignment.bundle_id, "mediaKind":"image"
    }).to_string();
    let evidence = serde_json::json!({"verificationDiagnostic":{"pendingMetadataCandidate":{
        "schemaVersion":1,"campaignId":campaign,"assignmentId":assignment.id,
        "bundleId":assignment.bundle_id,"intentSha256":crate::frame_sha256(intent.as_bytes()),
        "captionSha256":"a".repeat(64),"captured":{
            "canonicalUrl":"https://www.tiktok.com/@fixture/photo/7550000000000000000",
            "postId":"7550000000000000000","expectedAccount":"fixture",
            "normalizedCaptionSha256":"a".repeat(64),"preparedAt":null,
            "submittedAt":"2026-10-06T01:00:00Z","capturedAt":"2026-10-06T01:01:00Z",
            "provenance":"measuredViewerClipboard","metadata":{"state":"unavailable","attempts":1,
                "checkedAt":"2026-10-06T01:01:00Z","stage":"httpStatus","httpStatus":400}
        }
    }}}).to_string();
    db.conn().unwrap().execute("UPDATE publish_assignments SET state='verifying',effect_intent=?2,evidence_json=?3 WHERE id=?1",
        params![assignment.id,intent,evidence]).unwrap();
    let candidate = PendingPublishVerification {
        assignment_id:assignment.id.clone(),campaign_id:campaign,bundle_id:assignment.bundle_id.clone(),
        udid:assignment.udid.clone(),scheduled:false,revision:db.publish_assignment_revision(&assignment.id).unwrap(),
        effect_intent:Some(intent),evidence_json:Some(evidence),stop_marker:None,
    };
    let phone = db.try_publish_work(&candidate.udid, "compose", "independent-post").unwrap().unwrap();
    assert!(other.try_publish_work(&candidate.udid, "verify", &candidate.assignment_id).unwrap().is_none());
    let metadata = db.try_publish_metadata_work(&candidate, false).unwrap().expect("busy phone cannot block network metadata");
    assert!(other.try_publish_metadata_work(&candidate, false).unwrap().is_none(), "no duplicate observer across connections");
    drop(phone);
    let phone = other.try_publish_work(&candidate.udid, "compose", "independent-post").unwrap()
        .expect("metadata cannot hold a phone work permit");
    drop(phone);
    assert!(other.try_publish_work(&candidate.udid, "verify", &candidate.assignment_id).unwrap().is_none(),
        "phone verification cannot race the same assignment metadata observation");
    drop(metadata);
    let verify = db.try_publish_work(&candidate.udid, "verify", &candidate.assignment_id).unwrap().unwrap();
    assert!(other.try_publish_metadata_work(&candidate, false).unwrap().is_none());
    drop(verify);
    let mut stale = candidate.clone();
    stale.revision += 1;
    assert!(db.try_publish_metadata_work(&stale, false).unwrap().is_none());
    let mut deferred = candidate.clone();
    let mut deferred_evidence: serde_json::Value = serde_json::from_str(deferred.evidence_json.as_deref().unwrap()).unwrap();
    deferred_evidence["verificationStatus"] = serde_json::json!({"nextCheckAt":(Utc::now()+chrono::Duration::hours(1)).to_rfc3339()});
    deferred.evidence_json = Some(deferred_evidence.to_string());
    assert!(db.try_publish_metadata_work(&deferred, false).unwrap().is_none());
    let mut rejected = candidate.clone();
    rejected.evidence_json = Some(rejected.evidence_json.unwrap().replace("unavailable", "rejected"));
    for row in [&deferred, &rejected] {
        db.conn().unwrap().execute("UPDATE publish_assignments SET evidence_json=?2 WHERE id=?1",
            params![row.assignment_id,row.evidence_json]).unwrap();
        let automatic = db.try_publish_metadata_work(row, false).unwrap();
        assert_eq!(automatic.is_some(), row.is_due(Utc::now()));
        drop(automatic);
        assert!(db.try_publish_metadata_work(row, true).unwrap().is_some(), "future-due manual and Rejected one-shot are eligible");
    }
    for mut row in [deferred, rejected] {
        // Persist the exact candidate so this exercises policy, not stale refusal.
        let mut evidence: serde_json::Value = serde_json::from_str(row.evidence_json.as_deref().unwrap()).unwrap();
        evidence["verificationBudget"] = serde_json::json!({"metadataObservations":12,"noProgressObservations":3});
        evidence["verificationStatus"]["state"] = serde_json::json!("needsReview");
        row.evidence_json = Some(evidence.to_string());
        db.conn().unwrap().execute("UPDATE publish_assignments SET evidence_json=?2 WHERE id=?1",
            params![row.assignment_id,row.evidence_json]).unwrap();
        row.revision = db.publish_assignment_revision(&row.assignment_id).unwrap();
        assert!(db.try_publish_metadata_work(&row, false).unwrap().is_none());
        let manual = db.try_publish_metadata_work(&row, true).unwrap().expect("manual one-shot bypasses automatic budget and due only");
        assert!(other.try_publish_metadata_work(&row, true).unwrap().is_none());
        drop(manual);
        let saved: String = db.conn().unwrap().query_row("SELECT evidence_json FROM publish_assignments WHERE id=?1",
            [&row.assignment_id], |r|r.get(0)).unwrap();
        assert_eq!(Some(saved), row.evidence_json, "admission cannot reset budget or resume");
        assert!(db.record_manual_publish_verification_diagnostic(&row, "still unavailable", "metadataUnavailable", None).unwrap());
        let saved: String = db.conn().unwrap().query_row("SELECT evidence_json FROM publish_assignments WHERE id=?1",
            [&row.assignment_id], |r|r.get(0)).unwrap();
        let saved: serde_json::Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(saved["verificationBudget"], evidence["verificationBudget"]);
        assert_eq!(saved["verificationStatus"]["state"], "needsReview");
        assert!(saved["verificationStatus"]["nextCheckAt"].is_null());
        assert!(saved.get("verificationResume").is_none());
    }
    db.conn().unwrap().execute("UPDATE publish_assignments SET evidence_json=?2,revision=?3 WHERE id=?1",
        params![candidate.assignment_id,candidate.evidence_json,candidate.revision]).unwrap();
    assert!(other.try_publish_metadata_work(&candidate, false).unwrap().is_some(), "exact-token release permits another observation");
    drop(permits);
    assert_eq!(
        db.conn()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM publish_work_claims", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn schedule_expires_only_before_first_admission_and_never_replays_effect() {
    let (db, _, campaign, _) = fixture();
    db.claim_publish_pipeline(&campaign).unwrap().unwrap();
    db.conn()
        .unwrap()
        .execute("UPDATE publish_dispatch_jobs SET deadline_ms=30000", [])
        .unwrap();
    let jobs = db.pending_publish_dispatch(10).unwrap();
    assert!(db.claim_publish_dispatch(&jobs[0], 30_000).unwrap());
    assert!(!db.claim_publish_dispatch(&jobs[1], 30_001).unwrap());
    db.expire_publish_dispatch(30_001).unwrap();
    assert!(db.finish_publish_dispatch(&jobs[0], None).unwrap());
    let compose = db.pending_publish_dispatch(10).unwrap().remove(0);
    assert_eq!(compose.phase, "compose");
    assert!(db.claim_publish_dispatch(&compose, 600_000).unwrap());
    assert_eq!(
        db.conn()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM publish_assignments WHERE state='missed'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
}

#[test]
fn acceptance_deadline_settlement_does_not_touch_out_of_scope_schedules() {
    let (db, _, campaign, _) = fixture();
    db.claim_publish_pipeline(&campaign).unwrap().unwrap();
    db.conn()
        .unwrap()
        .execute("UPDATE publish_dispatch_jobs SET deadline_ms=30000", [])
        .unwrap();
    db.expire_publish_dispatch_for_campaign(30001, "different-campaign")
        .unwrap();
    assert_eq!(db.pending_publish_dispatch(10).unwrap().len(), 3);
    db.expire_publish_dispatch_for_campaign(30001, &campaign)
        .unwrap();
    assert!(db.pending_publish_dispatch(10).unwrap().is_empty());
    let detail = db.get_publish_campaign(&campaign).unwrap().unwrap();
    assert!(detail
        .assignments
        .iter()
        .all(|a| a.state == crate::PublishCampaignState::Missed && a.effect_intent.is_none()));
}

#[test]
fn restart_retains_queued_work_and_clears_only_dead_process_permits() {
    let (db, path, campaign, _) = fixture();
    db.claim_publish_pipeline(&campaign).unwrap();
    let queued = db.pending_publish_dispatch(10).unwrap();
    let permit = db
        .try_publish_work("dead", "compose", "previous-process")
        .unwrap()
        .unwrap();
    let reopened = Database::open(path).unwrap();
    reopened.interrupt_orphaned_publish_campaigns().unwrap();
    let restored = reopened.pending_publish_dispatch(10).unwrap();
    assert_eq!(restored.len(), 3);
    assert_eq!(restored[0].attempt_id, queued[0].attempt_id);
    let replacement = reopened
        .try_publish_work("dead", "compose", "new-process")
        .unwrap()
        .unwrap();
    drop(permit);
    assert!(reopened
        .try_publish_work("dead", "verify", "third")
        .unwrap()
        .is_none());
    drop(replacement);
}

#[test]
fn busy_devices_rotate_without_starving_the_tail() {
    let (db, _, campaign, _) = fixture();
    db.claim_publish_pipeline(&campaign).unwrap();
    let first = db.pending_publish_dispatch(1).unwrap().remove(0);
    db.defer_publish_dispatch(&first, "device_busy").unwrap();
    let second = db.pending_publish_dispatch(1).unwrap().remove(0);
    assert_ne!(first.udid, second.udid);
    db.defer_publish_dispatch(&second, "device_busy").unwrap();
    let third = db.pending_publish_dispatch(1).unwrap().remove(0);
    assert_ne!(first.udid, third.udid);
    assert_ne!(second.udid, third.udid);
}

#[test]
fn cached_candidates_cannot_start_two_publications_on_the_same_device() {
    let (db, _, campaign, _) = fixture();
    seed_load(&db, &campaign, 1, 2);
    let pending = db.pending_publish_dispatch(10).unwrap();
    assert_eq!(pending.len(), 2);
    let first = &pending[0];
    let next = &pending[1];
    assert_eq!(first.udid, next.udid);
    let permit = db
        .try_publish_work(&first.udid, &first.phase, &first.attempt_id)
        .unwrap()
        .unwrap();
    assert!(db.claim_publish_dispatch(first, 0).unwrap());
    // The future has returned; JoinSet has not settled its durable job yet.
    drop(permit);
    assert!(!db.claim_publish_dispatch(next, 0).unwrap());
    assert!(db.finish_publish_dispatch(first, None).unwrap());
    // Media for the next publication must not replace the imported first one.
    assert!(!db.claim_publish_dispatch(next, 0).unwrap());
    let compose = db.pending_publish_dispatch(10).unwrap().remove(0);
    assert_eq!(compose.assignment_id, first.assignment_id);
    assert_eq!(compose.phase, "compose");
    assert!(db.claim_publish_dispatch(&compose, 0).unwrap());
    assert!(db.finish_publish_dispatch(&compose, None).unwrap());
    assert!(db.claim_publish_dispatch(next, 0).unwrap());
}

#[test]
fn held_phone_does_not_block_more_than_old_cap_of_compose_dispatches() {
    let (db, _, campaign, _) = fixture();
    seed_load(&db, &campaign, 66, 66);
    db.conn()
        .unwrap()
        .execute("UPDATE publish_dispatch_jobs SET phase='compose'", [])
        .unwrap();
    let pending = db.pending_publish_dispatch(128).unwrap();
    assert_eq!(pending.len(), 66);
    let held = &pending[0];
    let held_permit = db
        .try_publish_work(&held.udid, &held.phase, &held.attempt_id)
        .unwrap()
        .unwrap();
    assert!(db.claim_publish_dispatch(held, 0).unwrap());
    let mut siblings = Vec::new();
    for job in db.pending_publish_dispatch(128).unwrap() {
        let permit = db
            .try_publish_work(&job.udid, &job.phase, &job.attempt_id)
            .unwrap()
            .unwrap();
        assert!(db.claim_publish_dispatch(&job, 0).unwrap());
        siblings.push((job, permit));
    }
    assert_eq!(siblings.len(), 65);
    for (job, permit) in siblings {
        assert!(db.finish_publish_dispatch(&job, None).unwrap());
        drop(permit);
    }
    assert!(db
        .try_publish_work(&held.udid, "verify", "sibling")
        .unwrap()
        .is_none());
    assert!(db.finish_publish_dispatch(held, None).unwrap());
    drop(held_permit);
}

#[test]
fn restart_settles_all_unstarted_scheduled_jobs_as_missed_at_campaign_level() {
    let (db, path, campaign, _) = fixture();
    db.claim_publish_pipeline(&campaign).unwrap();
    db.conn()
        .unwrap()
        .execute("UPDATE publish_dispatch_jobs SET deadline_ms=30000", [])
        .unwrap();
    let reopened = Database::open(path).unwrap();
    reopened.interrupt_orphaned_publish_campaigns().unwrap();
    assert!(reopened.pending_publish_dispatch(10).unwrap().is_empty());
    let detail = reopened.get_publish_campaign(&campaign).unwrap().unwrap();
    assert!(detail
        .assignments
        .iter()
        .all(|a| a.state == crate::PublishCampaignState::Missed));
    assert_eq!(detail.campaign.state, crate::PublishCampaignState::Missed);
    assert_eq!(
        detail.campaign.error_code.as_deref(),
        Some("app_opened_after_deadline")
    );
}

#[test]
fn shutdown_paused_jobs_are_not_finished_runs() {
    let (db, _, campaign, _) = fixture();
    db.claim_publish_pipeline(&campaign).unwrap();
    db.pause_publish_dispatch().unwrap();
    assert!(db.finished_publish_dispatch_runs().unwrap().is_empty());
}

#[test]
fn lowering_transfer_capacity_does_not_block_other_device_stages() {
    let (db, _, _, _) = fixture();
    let first = db.try_publish_work("a", "transfer", "a").unwrap().unwrap();
    let second = db.try_publish_work("b", "transfer", "b").unwrap().unwrap();
    db.set_setting(
        "publish.dispatch.limits",
        r#"{"transfer":1,"compose":1,"verify":1,"deviceTotal":1}"#,
    )
    .unwrap();
    let compose = db.try_publish_work("c", "compose", "c").unwrap().unwrap();
    assert!(db.try_publish_work("c", "verify", "c").unwrap().is_none());
    assert!(db.try_publish_work("d", "transfer", "d").unwrap().is_none());
    drop(first);
    assert!(db.try_publish_work("d", "transfer", "d").unwrap().is_none());
    drop(second);
    assert!(db.try_publish_work("d", "transfer", "d").unwrap().is_some());
    drop(compose);
}

#[test]
fn admitted_stage_hydrates_only_its_own_publication_and_no_history() {
    let (db, _, campaign, assignments) = fixture();
    db.claim_publish_pipeline(&campaign).unwrap();
    db.conn()
        .unwrap()
        .execute(
            "UPDATE publish_bundles SET manifest_json='corrupt unrelated media' WHERE id=?1",
            [&assignments[1].bundle_id],
        )
        .unwrap();
    assert!(db.get_publish_campaign(&campaign).is_err());
    let detail = db
        .get_publish_assignment_detail(&campaign, &assignments[0].id)
        .unwrap()
        .unwrap();
    assert_eq!(detail.bundles.len(), 1);
    assert_eq!(detail.bundles[0].id, assignments[0].bundle_id);
    assert_eq!(detail.assignments.len(), 1);
    assert_eq!(detail.assignments[0].id, assignments[0].id);
    assert_eq!(detail.campaign.assignments.len(), 1);
    assert!(detail.events.is_empty());
}

#[test]
fn schedule_compare_and_swap_enforces_the_thirty_second_boundary() {
    for (delay, expected) in [(29, true), (30, true), (31, false)] {
        let (db, _, campaign, _) = fixture();
        db.conn().unwrap().execute("UPDATE publish_campaigns SET state='scheduled',run_at='2099-01-01T12:00:00' WHERE id=?1",[&campaign]).unwrap();
        assert_eq!(
            db.claim_due_publish_schedule(
                &campaign,
                "2099-01-01T12:00:00",
                &format!("2099-01-01T12:00:{delay}")
            )
            .unwrap(),
            expected
        );
    }
}

/// Seed a large queue in one transaction; stage admission and settlement below
/// call the production implementation. Device operations are simulated.
fn seed_load(db: &Database, campaign: &str, devices: usize, total: usize) {
    let mut conn = db.conn().unwrap();
    let tx = conn.transaction().unwrap();
    tx.execute("DELETE FROM publish_dispatch_jobs", []).unwrap();
    tx.execute("DELETE FROM publish_attempts", []).unwrap();
    let mut campaigns = Vec::new();
    for n in 0..total {
        let group = format!("load-campaign-{}", n / devices);
        if n % devices == 0 {
            tx.execute("INSERT INTO publish_campaigns(id,request_id,source_root,request_json,state,revision,created_at,updated_at)
                SELECT ?1,?1,source_root,request_json,'queued',0,created_at,updated_at FROM publish_campaigns WHERE id=?2",params![group,campaign]).unwrap();
            campaigns.push(group.clone());
        }
        let id = format!("load-{n:05}");
        tx.execute("INSERT INTO publish_bundles(id,campaign_id,ordinal,name,source_path,caption,caption_sha256,manifest_json,created_at)
            SELECT ?1,?2,?3,name,source_path,caption,caption_sha256,manifest_json,created_at FROM publish_bundles WHERE id='b0'",params![id,group,i64::try_from(n%devices).unwrap()]).unwrap();
        tx.execute("INSERT INTO publish_assignments(id,campaign_id,bundle_id,ordinal,udid,state,revision,created_at,updated_at)
            VALUES(?1,?2,?1,?3,?4,'queued',0,'2026-09-14T00:00:00Z','2026-09-14T00:00:00Z')",
            params![id,group,i64::try_from(n%devices).unwrap(),format!("phone-{:03}",n%devices)]).unwrap();
    }
    tx.commit().unwrap();
    for id in campaigns {
        db.claim_publish_pipeline(&id).unwrap().unwrap();
    }
}

#[test]
#[ignore = "60 minute mixed load with real SQLite admission; no physical phones"]
fn publish_dispatch_sixty_minute_soak() {
    let seconds = std::env::var("RIVIU_SOAK_SECONDS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(3600);
    let started = std::time::Instant::now();
    for devices in [100, 500] {
        let (db, _, campaign, _) = fixture();
        seed_load(&db, &campaign, devices, 10_000);
        let mut completions = 0usize;
        let mut rounds = 0usize;
        let mut passes = 0usize;
        let mut max_claims = 0i64;
        let mut visited = std::collections::HashSet::new();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(seconds / 2);
        while std::time::Instant::now() < until || completions < 20_000 {
            let pending = db.pending_publish_dispatch(128).unwrap();
            if pending.is_empty() {
                assert_eq!(completions, 20_000);
                rounds += 1;
                println!(
                    "SOAK progress devices={devices} rounds={rounds} elapsed_seconds={}",
                    started.elapsed().as_secs()
                );
                if std::time::Instant::now() >= until {
                    break;
                }
                // New execution attempts of the same publications reuse the queue rows.
                let runs = db.finished_publish_dispatch_runs().unwrap();
                for run in runs {
                    db.finish_publish_pipeline(&run).unwrap();
                    db.claim_publish_pipeline(&run.campaign_id)
                        .unwrap()
                        .unwrap();
                }
                completions = 0;
                continue;
            }
            let verify = db
                .try_publish_work("reconnect", "verify", "mixed-load")
                .unwrap();
            let cleanup = db
                .try_publish_work("cleanup", "cleanup", "mixed-load")
                .unwrap();
            let mut active = Vec::new();
            for job in pending {
                if passes.is_multiple_of(17) {
                    db.defer_publish_dispatch(&job, "simulated_disconnect")
                        .unwrap();
                    passes += 1;
                    continue;
                }
                if let Some(permit) = db
                    .try_publish_work(&job.udid, &job.phase, &job.attempt_id)
                    .unwrap()
                {
                    assert!(db.claim_publish_dispatch(&job, 0).unwrap());
                    visited.insert(job.udid.clone());
                    active.push((job, permit));
                }
                passes += 1;
                if active.len() >= 6 {
                    break;
                }
            }
            let count = db
                .conn()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM publish_work_claims", [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap();
            max_claims = max_claims.max(count);
            assert!(count <= 8);
            assert!(
                active.iter().filter(|(j, _)| j.phase == "transfer").count()
                    <= db.publish_limits().unwrap().transfer
            );
            assert_eq!(
                active
                    .iter()
                    .map(|(job, _)| &job.udid)
                    .collect::<std::collections::HashSet<_>>()
                    .len(),
                active.len()
            );
            drop(verify);
            drop(cleanup);
            for (job, permit) in active.into_iter().rev() {
                assert!(db.finish_publish_dispatch(&job, None).unwrap());
                assert!(!db.finish_publish_dispatch(&job, None).unwrap());
                completions += 1;
                drop(permit);
            }
        }
        assert_eq!(visited.len(), devices);
        assert_eq!(completions, 20_000);
        let conn = db.conn().unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM publish_dispatch_jobs", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            10_000
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM publish_work_claims", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        println!("SOAK devices={devices} jobs=10000 completed_stages={completions} max_claims={max_claims} visited={} elapsed_seconds={}",visited.len(),started.elapsed().as_secs());
    }
}
