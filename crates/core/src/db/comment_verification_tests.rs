use super::*;

#[test]
fn observed_account_preserves_operator_metadata_and_rejects_invalid_handles() {
    let (db, _, _, _) = fixture();
    let mut meta = db.get_device_meta("phone").unwrap();
    meta.alias = "Desk phone".into();
    meta.number = Some(16);
    meta.notes = "keep".into();
    db.upsert_device_meta(&meta).unwrap();
    db.record_observed_interaction_account("phone", "@actual.user")
        .unwrap();
    let read = db.get_device_meta("phone").unwrap();
    assert_eq!(read.handle, "actual.user");
    assert_eq!(read.alias, meta.alias);
    assert_eq!(read.number, meta.number);
    assert_eq!(read.notes, meta.notes);
    assert!(db
        .record_observed_interaction_account("phone", "another.account")
        .is_err());
    assert_eq!(db.get_device_meta("phone").unwrap().handle, "actual.user");
    for invalid in ["", " ", "display name", "ends."] {
        assert!(db
            .record_observed_interaction_account("phone", invalid)
            .is_err());
    }
    assert_eq!(db.get_device_meta("phone").unwrap().handle, "actual.user");
}

#[test]
fn explicit_recheck_repairs_only_a_blank_account_and_does_not_reopen_send() {
    for initial in ["", "original.user"] {
        let (db, campaign, id, mut context) = fixture();
        context.account = initial.into();
        let now = armed(&db, &id, &context);
        db.conn().unwrap().execute("UPDATE interaction_comment_verification SET state='needsReview' WHERE assignment_id=?1",[&id]).unwrap();
        db.record_observed_interaction_account("phone", "current.user")
            .unwrap();
        db.request_comment_verification(&campaign, &id).unwrap();
        let json: String = db
            .conn()
            .unwrap()
            .query_row(
                "SELECT context_json FROM interaction_comment_verification WHERE assignment_id=?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        let stored: VerificationContext = serde_json::from_str(&json).unwrap();
        assert_eq!(
            stored.account,
            if initial.is_empty() {
                "current.user"
            } else {
                initial
            }
        );
        assert_eq!(stored.text, context.text);
        assert!(db
            .claim_interaction_assignment_for_send(&id)
            .unwrap()
            .is_none());
        assert!(now > 0);
    }
}
fn fixture() -> (Database, String, String, VerificationContext) {
    let path = std::env::temp_dir().join(format!("comment-verification-{}.db", Uuid::new_v4()));
    let db = Database::open(path).unwrap();
    let request:crate::ThreadCampaignRequest=serde_json::from_value(serde_json::json!({"requestId":"verify-fixture","targets":[{"originalUrl":"https://www.tiktok.com/@creator/photo/123","normalizedUrl":"https://www.tiktok.com/@creator/photo/123","targetKey":"content:123","contentId":"123","author":"creator","kind":"photo"}],"actorUdids":["phone"],"messageCount":1,"instruction":"","maxWords":12,"mode":"standalone","shape":"chain","manualComments":["hello"],"actions":{"like":false,"save":false,"comment":true},"mentions":[],"mentionParent":false})).unwrap();
    let plan = crate::plan_threads(&request).unwrap();
    let campaign = db.create_interaction_campaign(&request, &plan).unwrap();
    db.update_interaction_campaign_state(&campaign, crate::ThreadCampaignState::Running, None)
        .unwrap();
    let id = db
        .get_interaction_campaign(&campaign)
        .unwrap()
        .unwrap()
        .assignments[0]
        .id
        .clone();
    let context = VerificationContext {
        target: request.targets[0].clone(),
        device_id: "phone".into(),
        account: "actor".into(),
        text: "hello".into(),
        mentions: vec![],
        parent: None,
        root: None,
    };
    (db, campaign, id, context)
}
fn armed(db: &Database, id: &str, context: &VerificationContext) -> i64 {
    let revision = db
        .claim_interaction_assignment_for_send(id)
        .unwrap()
        .unwrap();
    db.prepare_comment_verification(id, context).unwrap();
    let action = db
        .claim_interaction_action(id, crate::InteractionActionKind::Comment)
        .unwrap()
        .unwrap();
    let armed = db
        .begin_interaction_comment_action_effect(id, revision, action, "post_comment")
        .unwrap()
        .unwrap();
    db.settle_interaction_action(
        id,
        crate::InteractionActionKind::Comment,
        armed,
        crate::InteractionActionState::Uncertain,
        Some("{\"postedIdentity\":null}"),
        Some("verification pending"),
    )
    .unwrap();
    db.conn()
        .unwrap()
        .execute(
            "UPDATE interaction_assignments SET state='uncertain',revision=revision+1 WHERE id=?1",
            [id],
        )
        .unwrap();
    db.conn()
        .unwrap()
        .query_row(
            "SELECT sent_at_ms FROM interaction_comment_verification WHERE assignment_id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
}
#[test]
fn send_boundary_durably_creates_readback_and_restart_never_reopens_send() {
    let (db, _, id, context) = fixture();
    let now = armed(&db, &id, &context);
    assert!(db
        .claim_interaction_assignment_for_send(&id)
        .unwrap()
        .is_none());
    let reopened = Database::open(&db.path).unwrap();
    let a = reopened
        .claim_comment_verification(&id, now + 5000)
        .unwrap()
        .unwrap();
    assert_eq!(a.attempts, 1);
    assert_eq!(a.context.text, "hello");
    assert!(db
        .claim_comment_verification(&id, now + 5000)
        .unwrap()
        .is_none());
    assert_eq!(
        db.comment_verification(&id).unwrap().unwrap().deadline_ms,
        Some(now + 120000)
    );
}
#[test]
fn verification_atomically_settles_identity_and_never_double_counts() {
    let (db, campaign, id, context) = fixture();
    let now = armed(&db, &id, &context);
    db.update_interaction_campaign_state(&campaign, crate::ThreadCampaignState::Failed, None)
        .unwrap();
    let job = db
        .claim_comment_verification(&id, now + 5000)
        .unwrap()
        .unwrap();
    let identity = crate::CommentLocatorIdentity {
        comment_link: None,
        author_label: "Actor".into(),
        text: "hello".into(),
        locator_version: "snapshot".into(),
        frame_sha256: "hash".into(),
    };
    assert!(db
        .settle_comment_verification(&job, Some(&identity), "{}", None, now + 6000)
        .unwrap());
    assert!(!db
        .settle_comment_verification(&job, Some(&identity), "{}", None, now + 6001)
        .unwrap());
    let detail = db.get_interaction_campaign(&campaign).unwrap().unwrap();
    assert_eq!(detail.summary.action_counters.confirmed, 1);
    assert_eq!(detail.assignments[0].posted_identity(), Some(identity));
    assert_eq!(detail.summary.state, crate::ThreadCampaignState::Succeeded);
}

#[test]
fn comment_link_enrichment_preserves_verified_effect_and_rejects_wrong_post_or_stale_identity() {
    let (db, campaign, id, context) = fixture();
    let now = armed(&db, &id, &context);
    let job = db
        .claim_comment_verification(&id, now + 5000)
        .unwrap()
        .unwrap();
    let identity = crate::CommentLocatorIdentity {
        author_label: "Actor".into(),
        text: "hello".into(),
        locator_version: "snapshot".into(),
        frame_sha256: "hash".into(),
        comment_link: None,
    };
    let link = crate::tiktok_comment_link::parse(
        "https://www.tiktok.com/@creator/photo/123?share_item_id=123&share_comment_id=456",
    )
    .unwrap();
    assert!(db
        .store_comment_link(&campaign, &id, &identity, &link)
        .is_err());
    db.settle_comment_verification(&job, Some(&identity), "{}", None, now + 6000)
        .unwrap();
    let wrong = crate::tiktok_comment_link::parse("aweme://aweme/detail?id=999&cid=456").unwrap();
    assert!(db
        .store_comment_link(&campaign, &id, &identity, &wrong)
        .is_err());
    db.store_comment_link(&campaign, &id, &identity, &link)
        .unwrap();
    assert!(
        db.store_comment_link(&campaign, &id, &identity, &link)
            .is_err(),
        "stale identity cannot overwrite enrichment"
    );
    let detail = db.get_interaction_campaign(&campaign).unwrap().unwrap();
    assert_eq!(
        detail.assignments[0]
            .posted_identity()
            .unwrap()
            .comment_link,
        Some(link)
    );
    assert_eq!(
        detail.assignments[0].state,
        crate::ThreadMessageState::Succeeded
    );
    assert_eq!(
        detail.assignments[0]
            .comment_verification
            .as_ref()
            .unwrap()
            .state,
        VerificationState::Verified
    );
    assert_eq!(detail.summary.action_counters.confirmed, 1);
    assert!(db
        .claim_interaction_assignment_for_send(&id)
        .unwrap()
        .is_none());
}
#[test]
fn exhausted_readback_keeps_uncertain_and_does_not_send_again() {
    let (db, _, id, context) = fixture();
    let now = armed(&db, &id, &context);
    for offset in [5000, 20000, 60000] {
        let job = db
            .claim_comment_verification(&id, now + offset)
            .unwrap()
            .unwrap();
        assert!(db
            .settle_comment_verification(
                &job,
                None,
                "{}",
                Some("comment_not_visible"),
                now + offset + 10
            )
            .unwrap());
    }
    assert_eq!(
        db.comment_verification(&id).unwrap().unwrap().state,
        VerificationState::NeedsReview
    );
    assert!(db
        .claim_interaction_assignment_for_send(&id)
        .unwrap()
        .is_none());
    assert!(db
        .claim_interaction_action(&id, crate::InteractionActionKind::Comment)
        .unwrap()
        .is_none());
}
#[test]
fn stale_observer_transaction_does_not_partially_confirm() {
    let (db, _, id, context) = fixture();
    let now = armed(&db, &id, &context);
    let job = db
        .claim_comment_verification(&id, now + 5000)
        .unwrap()
        .unwrap();
    db.conn()
        .unwrap()
        .execute(
            "UPDATE interaction_assignments SET revision=revision+1 WHERE id=?1",
            [&id],
        )
        .unwrap();
    let identity = crate::CommentLocatorIdentity {
        comment_link: None,
        author_label: "Actor".into(),
        text: "hello".into(),
        locator_version: "snapshot".into(),
        frame_sha256: "hash".into(),
    };
    assert!(db
        .settle_comment_verification(&job, Some(&identity), "{}", None, now + 6000)
        .is_err());
    assert_eq!(
        db.list_interaction_action_runs(&id).unwrap()[0].state,
        crate::InteractionActionState::Uncertain
    );
}
#[test]
fn repeated_manual_request_returns_same_pending_budget() {
    let (db, campaign, id, context) = fixture();
    let now = armed(&db, &id, &context);
    let a = db.request_comment_verification(&campaign, &id).unwrap();
    let b = db.request_comment_verification(&campaign, &id).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.deadline_ms, Some(now + 120000));
}

#[test]
fn manual_reply_recheck_restores_root_from_durable_parent_without_changing_text() {
    let (db, campaign, id, mut context) = fixture();
    let parent_id = Uuid::new_v4().to_string();
    let root = crate::CommentLocatorIdentity {
        comment_link: None,
        author_label: "Root author".into(),
        text: "Root text".into(),
        locator_version: "android-snapshot-v2".into(),
        frame_sha256: "a".repeat(64),
    };
    let conn = db.conn().unwrap();
    conn.execute("INSERT INTO interaction_assignments(id,campaign_id,target_id,message_ordinal,actor_udid,parent_assignment_id,state,prepared_json,effect_intent,evidence_json,error_code,revision,created_at,updated_at) SELECT ?2,campaign_id,target_id,10,'root-phone',NULL,'succeeded',NULL,'post_comment',?3,NULL,1,created_at,updated_at FROM interaction_assignments WHERE id=?1",params![id,parent_id,serde_json::json!({"postedIdentity":root}).to_string()]).unwrap();
    conn.execute(
        "UPDATE interaction_assignments SET parent_assignment_id=?2 WHERE id=?1",
        params![id, parent_id],
    )
    .unwrap();
    context.parent = Some(root.clone());
    context.root = None;
    armed(&db, &id, &context);
    conn.execute(
        "UPDATE interaction_comment_verification SET state='needsReview' WHERE assignment_id=?1",
        [&id],
    )
    .unwrap();
    db.request_comment_verification(&campaign, &id).unwrap();
    let raw: String = conn
        .query_row(
            "SELECT context_json FROM interaction_comment_verification WHERE assignment_id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    let stored: VerificationContext = serde_json::from_str(&raw).unwrap();
    assert_eq!(stored.root, Some(root));
    assert_eq!(stored.text, context.text);
    assert_eq!(stored.account, context.account);
    assert!(db
        .claim_interaction_assignment_for_send(&id)
        .unwrap()
        .is_none());
}

#[test]
fn expired_and_cancelled_jobs_do_not_reset_budget_or_release_send_lock() {
    let (db, campaign, id, context) = fixture();
    let now = armed(&db, &id, &context);
    db.cancel_interaction_campaign(&campaign).unwrap();
    let reopened = Database::open(&db.path).unwrap();
    assert!(reopened
        .claim_comment_verification(&id, now + 120001)
        .unwrap()
        .is_none());
    assert!(reopened
        .claim_interaction_assignment_for_send(&id)
        .unwrap()
        .is_none());
    assert_eq!(
        reopened
            .comment_verification(&id)
            .unwrap()
            .unwrap()
            .deadline_ms,
        Some(now + 120000)
    );
}

#[test]
fn parent_recheck_budget_is_persistent_and_bounded() {
    let (db, _, id, _) = fixture();
    let now = Utc::now().timestamp_millis();
    assert!(db.defer_parent_recheck(&id, now).unwrap());
    assert!(!db.parent_recheck_due(&id, now + 14999).unwrap());
    let reopened = Database::open(&db.path).unwrap();
    assert!(reopened.parent_recheck_due(&id, now + 15000).unwrap());
    assert!(reopened.defer_parent_recheck(&id, now + 15000).unwrap());
    assert!(!db.parent_recheck_due(&id, now + 44999).unwrap());
    assert!(!db.defer_parent_recheck(&id, now + 45000).unwrap());
}

#[test]
fn lease_expiry_preserves_attempt_count_and_deadline() {
    let (db, _, id, context) = fixture();
    let now = armed(&db, &id, &context);
    let first = db
        .claim_comment_verification(&id, now + 5000)
        .unwrap()
        .unwrap();
    db.conn()
        .unwrap()
        .execute(
            "UPDATE interaction_comment_verification SET lease_until_ms=0 WHERE assignment_id=?1",
            [&id],
        )
        .unwrap();
    db.recover_comment_verifications().unwrap();
    let second = db
        .claim_comment_verification(&id, now + 6000)
        .unwrap()
        .unwrap();
    assert_eq!(second.attempts, 2);
    assert_eq!(first.deadline_ms, second.deadline_ms);
    assert!(!db
        .settle_comment_verification(&first, None, "{}", Some("stale"), now + 6001)
        .unwrap());
}
