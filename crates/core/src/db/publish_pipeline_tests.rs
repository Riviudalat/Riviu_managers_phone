use super::*;
use crate::{
    PublishBundle, PublishCampaignRequest, PublishCampaignState as S, PublishCleanupPolicy,
    PublishSoundPolicy, PublishVisibility,
};
fn fixture() -> (
    Database,
    PathBuf,
    String,
    Vec<crate::PublishAssignmentRecord>,
) {
    let path = std::env::temp_dir().join(format!("pipeline-{}.db", Uuid::new_v4()));
    let db = Database::open(&path).unwrap();
    let bundles: Vec<_> = (0..3)
        .map(|i| PublishBundle {
            id: format!("b{i}"),
            source_path: "C:/fixture".into(),
            name: format!("b{i}"),
            media_kind: crate::PublishMediaKind::Image,
            images: vec![],
            video: None,
            caption_path: "C:/fixture/caption.txt".into(),
            caption: "caption".into(),
            caption_sha256: "0".repeat(64),
            total_bytes: 0,
            partners: vec![format!("partner{i}")],
        })
        .collect();
    let request = PublishCampaignRequest {
        sheet_delivery: None,
        verification_contract_version: None,
        verification_builds: vec![],
        request_id: Uuid::new_v4().to_string(),
        source_root: "C:/fixture".into(),
        bundle_ids: bundles.iter().map(|b| b.id.clone()).collect(),
        udids: vec!["a".into(), "b".into(), "c".into()],
        run_at: None,
        visibility: PublishVisibility::Public,
        cleanup_policy: PublishCleanupPolicy::KeepImportedAssets,
        network: crate::SocialNetwork::TikTok,
        sound_policy: PublishSoundPolicy::Default,
        sheet_enabled: true,
        execution_confirmed: true,
        target_snapshot: None,
    };
    let c = db.create_publish_campaign(&request, &bundles).unwrap();
    let a = db.get_publish_campaign(&c.id).unwrap().unwrap().assignments;
    (db, path, c.id, a)
}
fn imported(db: &Database, run: &PublishPipelineRun, a: &crate::PublishAssignmentRecord) -> i64 {
    let rev = db.publish_assignment_revision(&a.id).unwrap();
    assert!(db
        .advance_publish_pipeline_assignment(
            run,
            &a.id,
            rev,
            a.state.clone(),
            S::Transferring,
            None,
            None
        )
        .unwrap());
    assert!(db
        .advance_publish_pipeline_assignment(
            run,
            &a.id,
            rev + 1,
            S::Transferring,
            S::Imported,
            None,
            Some("{}")
        )
        .unwrap());
    rev + 2
}
#[test]
fn one_ready_device_posts_while_sibling_is_transferring_and_stale_writes_lose() {
    let (db, path, id, a) = fixture();
    let run = db.claim_publish_pipeline(&id).unwrap().unwrap();
    assert!(db.has_active_publish_pipeline(&id).unwrap());
    assert!(db.claim_publish_pipeline(&id).unwrap().is_none());
    assert!(!db.claim_publish_campaign_for_posting(&id).unwrap());
    let a_rev = imported(&db, &run, &a[0]);
    let b_rev = db.publish_assignment_revision(&a[1].id).unwrap();
    assert!(db
        .advance_publish_pipeline_assignment(
            &run,
            &a[1].id,
            b_rev,
            S::Queued,
            S::Transferring,
            None,
            None
        )
        .unwrap());
    assert!(db
        .claim_pipeline_post(&run, &a[0].id, a_rev, "{\"effectIntent\":\"post\"}")
        .unwrap());
    assert!(!db.claim_pipeline_post(&run, &a[0].id, a_rev, "{}").unwrap());
    let wrong = PublishPipelineRun {
        campaign_id: id.clone(),
        token: "stale".into(),
    };
    assert!(!db
        .advance_publish_pipeline_assignment(
            &wrong,
            &a[1].id,
            b_rev + 1,
            S::Transferring,
            S::Imported,
            None,
            None
        )
        .unwrap());
    assert!(!db
        .settle_pipeline_post(
            &run,
            &a[0].id,
            a_rev,
            S::Succeeded,
            None,
            "{}",
            None,
            "bot",
            &[]
        )
        .unwrap());
    let url = "https://www.tiktok.com/@a/photo/1234567890123456789";
    let e = serde_json::json!({"postUrl":url,"publicationVerified":true}).to_string();
    assert!(db
        .settle_pipeline_post(
            &run,
            &a[0].id,
            a_rev + 1,
            S::Succeeded,
            None,
            &e,
            Some(url),
            "bot",
            &["partner0".into()]
        )
        .unwrap());
    assert_eq!(db.publish_campaign_state(&id).unwrap(), Some(S::Posting));
    assert!(db.pending_publish_verifications(10).unwrap().is_empty());
    assert!(db.finish_publish_pipeline(&run).unwrap());
    assert!(!db.finish_publish_pipeline(&run).unwrap());
    let d = db.get_publish_campaign(&id).unwrap().unwrap();
    assert_eq!(d.assignments[0].state, S::Succeeded);
    assert_eq!(d.assignments[1].state, S::FailedBeforeDispatch);
    assert_eq!(
        db.pending_publish_sheet_rows(10).unwrap()[0].partners,
        vec!["partner0"]
    );
    assert!(!db
        .advance_publish_pipeline_assignment(
            &run,
            &a[1].id,
            b_rev + 2,
            S::FailedBeforeDispatch,
            S::Transferring,
            None,
            None
        )
        .unwrap());
    drop(db);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn cancel_blocks_new_post_but_preserves_an_already_dispatched_result() {
    let (db, path, id, a) = fixture();
    let run = db.claim_publish_pipeline(&id).unwrap().unwrap();
    let rev = imported(&db, &run, &a[0]);
    let rev_b = imported(&db, &run, &a[1]);
    assert!(db
        .claim_pipeline_post(&run, &a[0].id, rev, "{\"effectIntent\":\"post\"}")
        .unwrap());
    db.cancel_publish_campaign(&id).unwrap();
    assert!(!db.publish_pipeline_current(&run).unwrap());
    assert!(!db.claim_pipeline_post(&run, &a[1].id, rev_b, "{}").unwrap());
    assert!(db
        .settle_pipeline_post(
            &run,
            &a[0].id,
            rev + 1,
            S::Verifying,
            None,
            "{}",
            None,
            "bot",
            &[]
        )
        .unwrap());
    db.finish_publish_pipeline(&run).unwrap();
    assert_eq!(db.publish_campaign_state(&id).unwrap(), Some(S::Cancelled));
    assert_eq!(
        db.get_publish_campaign(&id).unwrap().unwrap().assignments[0].state,
        S::Verifying
    );
    drop(db);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn restart_invalidates_old_token_without_reopening_a_dispatched_post() {
    let (db, path, id, a) = fixture();
    let run = db.claim_publish_pipeline(&id).unwrap().unwrap();
    let rev = imported(&db, &run, &a[0]);
    let rev_b = db.publish_assignment_revision(&a[1].id).unwrap();
    assert!(db
        .advance_publish_pipeline_assignment(
            &run,
            &a[1].id,
            rev_b,
            S::Queued,
            S::Transferring,
            None,
            None
        )
        .unwrap());
    assert!(db
        .claim_pipeline_post(&run, &a[0].id, rev, "{\"effectIntent\":\"post\"}")
        .unwrap());
    drop(db);
    let db = Database::open(&path).unwrap();
    db.interrupt_orphaned_publish_campaigns().unwrap();
    assert!(!db.publish_pipeline_current(&run).unwrap());
    assert!(!db
        .settle_pipeline_post(
            &run,
            &a[0].id,
            rev + 1,
            S::Succeeded,
            None,
            "{}",
            None,
            "bot",
            &[]
        )
        .unwrap());
    let d = db.get_publish_campaign(&id).unwrap().unwrap();
    assert_eq!(d.assignments[0].state, S::Uncertain);
    assert_eq!(d.assignments[1].state, S::FailedBeforeDispatch);
    assert!(db.claim_publish_pipeline(&id).unwrap().is_none());
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn future_schedule_and_cancelled_parent_never_claim_a_pipeline() {
    let (db, path, id, _) = fixture();
    db.conn().unwrap().execute("UPDATE publish_campaigns SET state='scheduled',run_at='2099-01-01T00:00:00' WHERE id=?1",[&id]).unwrap();
    assert!(db.claim_publish_pipeline(&id).unwrap().is_none());
    db.cancel_publish_campaign(&id).unwrap();
    assert!(db.claim_publish_pipeline(&id).unwrap().is_none());
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn verified_first_device_does_not_finish_campaign_or_lose_its_sheet_under_slow_sibling() {
    let (db, path, id, a) = fixture();
    let run = db.claim_publish_pipeline(&id).unwrap().unwrap();
    let rev = imported(&db, &run, &a[0]);
    let intent=serde_json::json!({"effectIntent":"post","expectedAccount":"fixture","submittedAt":chrono::Utc::now().to_rfc3339()}).to_string();
    assert!(db
        .claim_pipeline_post(&run, &a[0].id, rev, &intent)
        .unwrap());
    assert!(db
        .settle_pipeline_post(
            &run,
            &a[0].id,
            rev + 1,
            S::Verifying,
            None,
            "{}",
            None,
            "bot",
            &[]
        )
        .unwrap());
    let candidate = db.pending_publish_verifications(10).unwrap().remove(0);
    let url = "https://www.tiktok.com/@fixture/photo/1234567890123456789";
    let proof = serde_json::json!({"postUrl":url,"publicationVerified":true}).to_string();
    assert!(db
        .record_verified_publish_with_sheet_row(
            &candidate,
            &proof,
            url,
            "bot",
            &["partner0".into()]
        )
        .unwrap());
    assert_eq!(db.publish_campaign_state(&id).unwrap(), Some(S::Posting));
    assert_eq!(
        db.reconciled_publish_execution_status(&id).unwrap(),
        (
            crate::PublishExecutionStatus::Partial,
            crate::PublishRetryScope::None
        )
    );
    assert!(db.publish_pipeline_current(&run).unwrap());
    assert_eq!(db.pending_publish_sheet_rows(10).unwrap().len(), 1);
    db.finish_publish_pipeline(&run).unwrap();
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn verifying_sibling_does_not_block_retry_of_failed_machines() {
    let (db, path, id, a) = fixture();
    let run = db.claim_publish_pipeline(&id).unwrap().unwrap();
    let rev = imported(&db, &run, &a[0]);
    assert!(db
        .claim_pipeline_post(&run, &a[0].id, rev, "{\"effectIntent\":\"post\"}")
        .unwrap());
    assert!(db
        .settle_pipeline_post(
            &run,
            &a[0].id,
            rev + 1,
            S::Verifying,
            None,
            "{}",
            None,
            "bot",
            &[]
        )
        .unwrap());
    assert!(db.finish_publish_pipeline(&run).unwrap());
    assert_eq!(db.publish_campaign_state(&id).unwrap(), Some(S::Verifying));
    assert_eq!(
        db.get_publish_campaign(&id).unwrap().unwrap().assignments[1].state,
        S::Queued
    );
    let retry = db
        .claim_publish_pipeline(&id)
        .unwrap()
        .expect("retry while sibling verifies");
    let rev_b = db.publish_assignment_revision(&a[1].id).unwrap();
    assert!(db
        .advance_publish_pipeline_assignment(
            &retry,
            &a[1].id,
            rev_b,
            S::Queued,
            S::Transferring,
            None,
            None
        )
        .unwrap());
    assert!(db.finish_publish_pipeline(&retry).unwrap());
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn simultaneous_callers_only_one_claims_a_campaign() {
    let (db, path, id, _) = fixture();
    let db = std::sync::Arc::new(db);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let mut jobs = Vec::new();
    for _ in 0..2 {
        let (db, id, barrier) = (db.clone(), id.clone(), barrier.clone());
        jobs.push(std::thread::spawn(move || {
            barrier.wait();
            db.claim_publish_pipeline(&id).unwrap()
        }));
    }
    barrier.wait();
    let winners: Vec<_> = jobs.into_iter().filter_map(|j| j.join().unwrap()).collect();
    assert_eq!(winners.len(), 1);
    db.finish_publish_pipeline(&winners[0]).unwrap();
    drop(db);
    std::fs::remove_file(path).unwrap();
}
