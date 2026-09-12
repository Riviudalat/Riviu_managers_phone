use super::*;
use crate::publish_submission::{normalize_publish_account, PublishSubmissionProof};

impl Database {
    /// Reserve across every device and both TikTok packages. No clock expiry after dispatch.
    pub fn reserve_publish_account(&self, assignment: &str, account: &str) -> anyhow::Result<()> {
        let account = normalize_publish_account(account)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let eligible: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM publish_assignments a JOIN publish_campaigns c ON c.id=a.campaign_id
             WHERE a.id=?1 AND a.effect_intent IS NULL AND a.state IN ('imported','failed_before_dispatch')
             AND c.state='posting' AND json_extract(c.request_json,'$.verificationContractVersion')=1)",
            [assignment], |r| r.get(0))?;
        anyhow::ensure!(
            eligible,
            "Lượt đăng cần kiểm tra lại trước khi giữ tài khoản"
        );
        let owner: Option<String> = tx
            .query_row(
                "SELECT assignment_id FROM publish_account_reservations WHERE account=?1",
                [&account],
                |r| r.get(0),
            )
            .optional()?;
        anyhow::ensure!(
            owner.as_deref().is_none_or(|id| id == assignment),
            "Tài khoản @{account} đang chờ liên kết của lượt {}",
            owner.unwrap_or_default()
        );
        tx.execute("INSERT INTO publish_account_reservations(account,assignment_id,claimed_at) VALUES(?1,?2,?3)
            ON CONFLICT(account) DO NOTHING", params![account,assignment,Utc::now().to_rfc3339()])?;
        tx.commit()?;
        Ok(())
    }

    /// Used by the composer's drop guard, including cancellation before the Post callback.
    pub fn release_unsubmitted_publish_account(&self, assignment: &str) -> anyhow::Result<()> {
        self.conn()?.execute("DELETE FROM publish_account_reservations WHERE assignment_id=?1
            AND EXISTS(SELECT 1 FROM publish_assignments a WHERE a.id=?1 AND a.effect_intent IS NULL)", [assignment])?;
        Ok(())
    }
}

/// Run inside the same transaction as the assignment CAS. Historical DB APIs remain readable;
/// production dispatch requires the version marker before entering the composer.
pub(super) fn validate_submission_claim(
    conn: &Connection,
    assignment: &str,
    intent: &str,
) -> anyhow::Result<()> {
    let (request, bundle_id): (String,String) = conn.query_row(
        "SELECT c.request_json,a.bundle_id FROM publish_assignments a JOIN publish_campaigns c ON c.id=a.campaign_id WHERE a.id=?1",
        [assignment], |r| Ok((r.get(0)?,r.get(1)?)))?;
    let request: crate::PublishCampaignRequest = serde_json::from_str(&request)?;
    if request.verification_contract_version.is_none() {
        return Ok(());
    }
    anyhow::ensure!(
        request.verification_contract_version == Some(1),
        "Phiên bản xác minh bài đăng chưa được hỗ trợ"
    );
    let value: serde_json::Value = serde_json::from_str(intent)?;
    anyhow::ensure!(
        value["effectIntent"] == "post",
        "Thiếu ý định Đăng trong bằng chứng"
    );
    let proof: PublishSubmissionProof =
        serde_json::from_str(intent).context("Thiếu bằng chứng trước Đăng")?;
    proof.validate()?;
    let submitted = chrono::DateTime::parse_from_rfc3339(&proof.submitted_at)?.with_timezone(&Utc);
    let age = Utc::now().signed_duration_since(submitted).num_seconds();
    anyhow::ensure!(
        (0..=30).contains(&age),
        "Thời điểm gửi bài không thuộc lần Đăng hiện tại"
    );
    anyhow::ensure!(
        proof.bundle_id == bundle_id,
        "Bằng chứng Đăng thuộc nội dung khác"
    );
    let manifest: String = conn.query_row(
        "SELECT manifest_json FROM publish_bundles WHERE id=?1",
        [&bundle_id],
        |r| r.get(0),
    )?;
    let bundle: crate::PublishBundle = serde_json::from_str(&manifest)?;
    anyhow::ensure!(
        proof.caption_sha256 == bundle.caption_sha256 && proof.media_kind == bundle.media_kind,
        "Bằng chứng Đăng không khớp caption hoặc media đã duyệt"
    );
    let udid: String = conn.query_row(
        "SELECT udid FROM publish_assignments WHERE id=?1",
        [assignment],
        |r| r.get(0),
    )?;
    anyhow::ensure!(
        request
            .verification_builds
            .iter()
            .any(|build| build.udid == udid
                && build.package == proof.package
                && build.version == proof.version
                && build.locale == proof.locale),
        "TikTok đã thay đổi so với lần kiểm tra; kiểm tra lại trước Đăng"
    );
    let account = normalize_publish_account(&proof.expected_account)?;
    let owns: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM publish_account_reservations WHERE account=?1 AND assignment_id=?2)",
        params![account,assignment], |r| r.get(0))?;
    anyhow::ensure!(owns, "Lượt đăng chưa giữ tài khoản để xác minh liên kết");
    Ok(())
}

/// Permanent link ownership applies even when writing Sheet is disabled.
pub(super) fn record_canonical_identity(
    conn: &Connection,
    assignment: &str,
    post_url: &str,
) -> anyhow::Result<()> {
    let row: Option<(String, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT c.request_json,a.effect_intent,a.evidence_json FROM publish_assignments a
         JOIN publish_campaigns c ON c.id=a.campaign_id WHERE a.id=?1 AND a.state='succeeded'",
            [assignment],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((request, intent, evidence)) = row else {
        return Ok(());
    };
    let request: crate::PublishCampaignRequest = serde_json::from_str(&request)?;
    if request.verification_contract_version != Some(1) {
        return Ok(());
    }
    let proof: PublishSubmissionProof =
        serde_json::from_str(intent.as_deref().context("Missing submission proof")?)?;
    let evidence: serde_json::Value =
        serde_json::from_str(evidence.as_deref().context("Missing publication proof")?)?;
    let observed = evidence.get("post").unwrap_or(&evidence);
    anyhow::ensure!(
        observed["publicationVerified"] == true && observed["postUrl"].as_str() == Some(post_url),
        "Liên kết chưa có bằng chứng xác minh bài đăng"
    );
    let targets = crate::interaction::parse_tiktok_links(post_url);
    anyhow::ensure!(
        targets.len() == 1
            && targets[0]
                .target
                .as_ref()
                .is_some_and(|target| target.normalized_url == post_url
                    && target
                        .author
                        .eq_ignore_ascii_case(proof.expected_account.trim_start_matches('@'))),
        "Liên kết không khớp tài khoản trước Đăng"
    );
    let owner: Option<String> = conn
        .query_row(
            "SELECT assignment_id FROM publish_post_identities WHERE post_url=?1",
            [post_url],
            |r| r.get(0),
        )
        .optional()?;
    anyhow::ensure!(
        owner.as_deref().is_none_or(|id| id == assignment),
        "Liên kết đã thuộc lượt đăng khác"
    );
    conn.execute(
        "INSERT INTO publish_post_identities(post_url,assignment_id) VALUES(?1,?2)
        ON CONFLICT(post_url) DO NOTHING",
        params![post_url, assignment],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PublishBundle, PublishCampaignState as S};

    fn intent(proof: &PublishSubmissionProof) -> String {
        let mut value = serde_json::to_value(proof).unwrap();
        value["effectIntent"] = serde_json::json!("post");
        value.to_string()
    }

    fn setup() -> (
        Database,
        PathBuf,
        String,
        Vec<crate::PublishAssignmentRecord>,
        PublishSubmissionProof,
    ) {
        let path = std::env::temp_dir().join(format!("submission-proof-{}.db", Uuid::new_v4()));
        let db = Database::open(&path).unwrap();
        let bundles: Vec<_> = (0..2)
            .map(|index| PublishBundle {
                id: format!("proof-bundle-{index}"),
                source_path: "fixture".into(),
                name: "fixture".into(),
                media_kind: crate::PublishMediaKind::Image,
                images: vec![],
                video: None,
                caption_path: "caption.txt".into(),
                caption: "fixture caption".into(),
                caption_sha256: "a".repeat(64),
                total_bytes: 0,
                partners: vec![],
            })
            .collect();
        let request = crate::PublishCampaignRequest {
            sheet_delivery: None,
            verification_contract_version: Some(1),
            verification_builds: ["device-a", "device-b"]
                .iter()
                .map(|udid| crate::publish_submission::PublishVerificationBuild {
                    udid: (*udid).into(),
                    package: "com.zhiliaoapp.musically".into(),
                    version: "45.7.3".into(),
                    locale: "en".into(),
                })
                .collect(),
            sheet_enabled: false,
            request_id: Uuid::new_v4().to_string(),
            source_root: "fixture".into(),
            bundle_ids: bundles.iter().map(|b| b.id.clone()).collect(),
            udids: vec!["device-a".into(), "device-b".into()],
            run_at: None,
            visibility: crate::PublishVisibility::Public,
            cleanup_policy: crate::PublishCleanupPolicy::KeepImportedAssets,
            network: crate::SocialNetwork::TikTok,
            sound_policy: crate::PublishSoundPolicy::Default,
            execution_confirmed: true,
            target_snapshot: None,
        };
        let campaign = db.create_publish_campaign(&request, &bundles).unwrap();
        db.update_publish_campaign_state(&campaign.id, S::Posting, None)
            .unwrap();
        let assignments = db
            .get_publish_campaign(&campaign.id)
            .unwrap()
            .unwrap()
            .assignments;
        for assignment in &assignments {
            db.update_publish_assignment_state(&assignment.id, S::Imported, None, None)
                .unwrap();
        }
        let proof = PublishSubmissionProof {
            verification_contract_version: 1,
            expected_account: "Fixture.Account".into(),
            submitted_at: Utc::now().to_rfc3339(),
            package: "com.zhiliaoapp.musically".into(),
            version: "45.7.3".into(),
            locale: "en".into(),
            caption_sha256: bundles[0].caption_sha256.clone(),
            bundle_id: bundles[0].id.clone(),
            media_kind: crate::PublishMediaKind::Image,
        };
        (db, path, campaign.id, assignments, proof)
    }

    #[test]
    fn missing_proof_fields_roll_back_the_post_claim() {
        let (db, _path, campaign, assignments, proof) = setup();
        db.reserve_publish_account(&assignments[0].id, &proof.expected_account)
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&intent(&proof)).unwrap();
        for field in [
            "verificationContractVersion",
            "expectedAccount",
            "submittedAt",
            "package",
            "version",
            "locale",
            "captionSha256",
            "bundleId",
            "mediaKind",
        ] {
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(
                db.claim_publish_assignment_for_posting(&assignments[0].id, &missing.to_string())
                    .is_err(),
                "{field}"
            );
            let detail = db.get_publish_campaign(&campaign).unwrap().unwrap();
            assert_eq!(detail.assignments[0].state, S::Imported);
            assert!(detail.assignments[0].effect_intent.is_none());
        }
        let mut mismatched = proof.clone();
        mismatched.caption_sha256 = "b".repeat(64);
        assert!(db
            .claim_publish_assignment_for_posting(&assignments[0].id, &intent(&mismatched))
            .is_err());
        assert!(db
            .claim_publish_assignment_for_posting(&assignments[0].id, &intent(&proof))
            .unwrap());
        assert!(!db
            .claim_publish_assignment_for_posting(&assignments[0].id, &intent(&proof))
            .unwrap());
    }

    #[test]
    fn two_devices_share_one_durable_normalized_account_reservation() {
        let (db, path, _campaign, assignments, proof) = setup();
        db.reserve_publish_account(&assignments[0].id, "@Fixture.Account")
            .unwrap();
        assert!(db
            .reserve_publish_account(&assignments[1].id, "fixture.account")
            .is_err());
        assert!(db
            .claim_publish_assignment_for_posting(&assignments[0].id, &intent(&proof))
            .unwrap());
        db.release_unsubmitted_publish_account(&assignments[0].id)
            .unwrap();
        drop(db);
        let reopened = Database::open(&path).unwrap();
        reopened.interrupt_orphaned_publish_campaigns().unwrap();
        assert!(reopened
            .reserve_publish_account(&assignments[1].id, "fixture.account")
            .is_err());
        let owner: String = reopened
            .conn()
            .unwrap()
            .query_row(
                "SELECT assignment_id FROM publish_account_reservations",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(owner, assignments[0].id);
    }

    #[test]
    fn pre_dispatch_failure_releases_account_and_wrong_owner_cannot_post() {
        let (db, _path, _campaign, assignments, proof) = setup();
        assert!(db
            .claim_publish_assignment_for_posting(&assignments[0].id, &intent(&proof))
            .is_err());
        db.reserve_publish_account(&assignments[0].id, &proof.expected_account)
            .unwrap();
        db.release_unsubmitted_publish_account(&assignments[0].id)
            .unwrap();
        db.reserve_publish_account(&assignments[1].id, &proof.expected_account)
            .unwrap();
        assert!(db
            .claim_publish_assignment_for_posting(&assignments[0].id, &intent(&proof))
            .is_err());
    }

    #[test]
    fn verified_link_releases_account_even_when_sheet_is_disabled() {
        let (db, _path, campaign, assignments, proof) = setup();
        db.reserve_publish_account(&assignments[0].id, &proof.expected_account)
            .unwrap();
        db.claim_publish_assignment_for_posting(&assignments[0].id, &intent(&proof))
            .unwrap();
        db.update_publish_assignment_state(&assignments[0].id, S::Verifying, None, Some("{}"))
            .unwrap();
        let candidate = db
            .publish_verifications_for_campaign(&campaign, 10)
            .unwrap()
            .into_iter()
            .find(|c| c.assignment_id == assignments[0].id)
            .unwrap();
        let link = "https://www.tiktok.com/@fixture.account/photo/1234567890123";
        let evidence = serde_json::json!({"postUrl":link,"publicationVerified":true});
        assert!(db
            .record_verified_publish_with_sheet_row(
                &candidate,
                &evidence.to_string(),
                link,
                "bot",
                &[]
            )
            .unwrap());
        db.update_publish_campaign_state(&campaign, S::Posting, None)
            .unwrap();
        db.reserve_publish_account(&assignments[1].id, &proof.expected_account)
            .unwrap();
        assert!(db.pending_publish_sheet_rows(10).unwrap().is_empty());
        let mut second = proof.clone();
        second.bundle_id = assignments[1].bundle_id.clone();
        second.submitted_at = Utc::now().to_rfc3339();
        assert!(db
            .claim_publish_assignment_for_posting(&assignments[1].id, &intent(&second))
            .unwrap());
        db.update_publish_assignment_state(&assignments[1].id, S::Verifying, None, Some("{}"))
            .unwrap();
        let candidate = db
            .publish_verifications_for_campaign(&campaign, 10)
            .unwrap()
            .into_iter()
            .find(|row| row.assignment_id == assignments[1].id)
            .unwrap();
        assert!(db
            .record_verified_publish_with_sheet_row(
                &candidate,
                &evidence.to_string(),
                link,
                "bot",
                &[]
            )
            .is_err());
        assert_eq!(
            db.get_publish_campaign(&campaign)
                .unwrap()
                .unwrap()
                .assignments[1]
                .state,
            S::Verifying
        );
    }

    #[test]
    fn current_worker_does_not_enroll_legacy_history() {
        let (db, _path, campaign, assignments, proof) = setup();
        db.reserve_publish_account(&assignments[0].id, &proof.expected_account)
            .unwrap();
        db.claim_publish_assignment_for_posting(&assignments[0].id, &intent(&proof))
            .unwrap();
        db.update_publish_assignment_state(&assignments[0].id, S::Verifying, None, Some("{}"))
            .unwrap();
        assert_eq!(
            db.pending_current_publish_verifications(10).unwrap().len(),
            1
        );
        db.conn().unwrap().execute("UPDATE publish_campaigns SET request_json=json_remove(request_json,'$.verificationContractVersion') WHERE id=?1",[campaign]).unwrap();
        assert!(db
            .pending_current_publish_verifications(10)
            .unwrap()
            .is_empty());
        assert_eq!(db.pending_publish_verifications(10).unwrap().len(), 1);
        assert!(db
            .expire_current_publish_verifications()
            .unwrap()
            .is_empty());
    }
}
