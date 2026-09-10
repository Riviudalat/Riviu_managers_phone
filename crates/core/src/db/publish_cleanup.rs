//! Durable media cleanup after a delayed, independently verified publication.
use super::*;

#[derive(Debug, Clone)]
pub struct PendingPublishCleanup {
    pub assignment_id: String,
    pub campaign_id: String,
    pub udid: String,
    pub revision: i64,
    pub import_id: String,
    pub evidence_json: String,
}

fn prior_import(evidence: &serde_json::Value, remaining: usize) -> Option<String> {
    if let Some(native) = evidence.get("nativeImport") {
        if let Some(id) = native
            .get("value")
            .unwrap_or(native)
            .get("importId")
            .and_then(|value| value.as_str())
        {
            return Some(id.to_owned());
        }
    }
    if remaining == 0 {
        return None;
    }
    let prior = evidence.get("priorEvidenceJson")?;
    let parsed = if let Some(raw) = prior.as_str() {
        serde_json::from_str(raw).ok()?
    } else {
        prior.clone()
    };
    prior_import(&parsed, remaining - 1)
}

fn cleanup_import(evidence: &serde_json::Value) -> Option<String> {
    let post = evidence.get("post").unwrap_or(evidence);
    if post.get("publicationVerified")?.as_bool() != Some(true) {
        return None;
    }
    let link = post.get("postUrl")?.as_str()?;
    let url = url::Url::parse(link).ok()?;
    if url.scheme() != "https"
        || !matches!(
            url.host_str(),
            Some("www.tiktok.com" | "tiktok.com" | "m.tiktok.com")
        )
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
    {
        return None;
    }
    let targets = crate::interaction::parse_tiktok_links(link);
    if targets.len() != 1 || targets[0].target.as_ref()?.normalized_url != link {
        return None;
    }
    let import = if let Some(cleanup) = evidence.get("cleanup").filter(|value| value.is_object()) {
        if cleanup.get("state").and_then(|value| value.as_str()) == Some("cleaned") {
            return None;
        }
        if cleanup.get("reason").and_then(|value| value.as_str())
            != Some("post_verification_pending")
            && cleanup.get("source").and_then(|value| value.as_str()) != Some("verified_deferred")
        {
            return None;
        }
        cleanup
            .get("importId")
            .and_then(|value| value.as_str())
            .or_else(|| post.get("importId").and_then(|value| value.as_str()))?
            .to_owned()
    } else {
        // A lost Post response retained the successful native import in prior evidence.
        // Recovery adds publication proof later; never infer an import from a directory.
        prior_import(evidence, 3)?
    };
    if import.is_empty() || import.len() > 512 || import.chars().any(char::is_control) {
        return None;
    }
    if post
        .get("importId")
        .and_then(|value| value.as_str())
        .is_some_and(|other| other != import)
    {
        return None;
    }
    Some(import)
}

fn candidate_from_connection(
    conn: &Connection,
    id: &str,
) -> anyhow::Result<Option<PendingPublishCleanup>> {
    let row: Option<(String, String, i64, String, String, String)> = conn.query_row(
        "SELECT a.campaign_id,a.udid,a.revision,a.evidence_json,a.state,c.request_json
         FROM publish_assignments a JOIN publish_campaigns c ON c.id=a.campaign_id WHERE a.id=?1 AND a.evidence_json IS NOT NULL",
        [id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))
    ).optional()?;
    let Some((campaign_id, udid, revision, evidence_json, state, request_json)) = row else {
        return Ok(None);
    };
    if state != "succeeded" {
        return Ok(None);
    }
    let request: crate::PublishCampaignRequest = serde_json::from_str(&request_json)?;
    if request.cleanup_policy != crate::PublishCleanupPolicy::DeleteImportedAssetsAfterVerified {
        return Ok(None);
    }
    let evidence = serde_json::from_str(&evidence_json)?;
    Ok(
        cleanup_import(&evidence).map(|import_id| PendingPublishCleanup {
            assignment_id: id.into(),
            campaign_id,
            udid,
            revision,
            import_id,
            evidence_json,
        }),
    )
}

impl Database {
    /// Read media debt independently of link debt; a known link does not erase cleanup.
    pub fn pending_publish_cleanups(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<PendingPublishCleanup>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let conn = self.conn()?;
        let mut statement = conn.prepare("SELECT id FROM publish_assignments WHERE state='succeeded' AND evidence_json IS NOT NULL ORDER BY updated_at,id")?;
        let ids = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut result = Vec::new();
        for id in ids {
            if let Some(candidate) = candidate_from_connection(&conn, &id?)? {
                result.push(candidate);
                if result.len() >= limit.min(1000) {
                    break;
                }
            }
        }
        Ok(result)
    }

    pub fn pending_publish_cleanup(
        &self,
        assignment_id: &str,
    ) -> anyhow::Result<Option<PendingPublishCleanup>> {
        candidate_from_connection(&self.conn()?, assignment_id)
    }

    /// Called only under the exclusive device lease. Running debt is recoverable after
    /// restart because the driver removes only the same immutable import, idempotently.
    pub fn claim_publish_cleanup(
        &self,
        candidate: &PendingPublishCleanup,
    ) -> anyhow::Result<Option<PendingPublishCleanup>> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(mut current) = candidate_from_connection(&transaction, &candidate.assignment_id)?
        else {
            return Ok(None);
        };
        if current.revision != candidate.revision
            || current.campaign_id != candidate.campaign_id
            || current.udid != candidate.udid
            || current.import_id != candidate.import_id
        {
            return Ok(None);
        }
        let mut evidence: serde_json::Value = serde_json::from_str(&current.evidence_json)?;
        if !evidence
            .get("cleanup")
            .is_some_and(serde_json::Value::is_object)
        {
            evidence["cleanup"] = serde_json::json!({"importId":current.import_id,"appCleanup":{"state":"leftRunning"}});
        }
        let cleanup = &mut evidence["cleanup"];
        cleanup["source"] = serde_json::json!("verified_deferred");
        cleanup["state"] = serde_json::json!("not_cleaned");
        cleanup["deferredStatus"] = serde_json::json!("running");
        cleanup["claimedAt"] = serde_json::json!(Utc::now().to_rfc3339());
        let updated = evidence.to_string();
        let changed = transaction.execute(
            "UPDATE publish_assignments SET evidence_json=?1,revision=revision+1,updated_at=?2 WHERE id=?3 AND revision=?4 AND state='succeeded'",
            params![updated,Utc::now().to_rfc3339(),current.assignment_id,current.revision])?;
        if changed != 1 {
            return Ok(None);
        }
        transaction.commit()?;
        current.revision += 1;
        current.evidence_json = updated;
        Ok(Some(current))
    }

    /// Merge only media cleanup proof; never change publication, its canonical link or Sheet.
    pub fn finish_publish_cleanup(
        &self,
        candidate: &PendingPublishCleanup,
        cleanup: &serde_json::Value,
    ) -> anyhow::Result<bool> {
        anyhow::ensure!(cleanup.is_object(), "cleanup outcome must be an object");
        anyhow::ensure!(
            cleanup.get("importId").and_then(|value| value.as_str())
                == Some(candidate.import_id.as_str()),
            "cleanup proof names another import"
        );
        anyhow::ensure!(
            matches!(
                cleanup.get("state").and_then(|value| value.as_str()),
                Some("cleaned" | "not_cleaned")
            ),
            "cleanup outcome missing readback state"
        );
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(current) = candidate_from_connection(&transaction, &candidate.assignment_id)?
        else {
            return Ok(false);
        };
        if current.revision != candidate.revision
            || current.import_id != candidate.import_id
            || current.campaign_id != candidate.campaign_id
            || current.udid != candidate.udid
        {
            return Ok(false);
        }
        let mut evidence: serde_json::Value = serde_json::from_str(&current.evidence_json)?;
        let mut cleanup = cleanup.clone();
        cleanup["source"] = serde_json::json!("verified_deferred");
        cleanup["checkedAt"] = serde_json::json!(Utc::now().to_rfc3339());
        evidence["cleanup"] = cleanup;
        let changed = transaction.execute(
            "UPDATE publish_assignments SET evidence_json=?1,revision=revision+1,updated_at=?2 WHERE id=?3 AND revision=?4 AND state='succeeded'",
            params![evidence.to_string(),Utc::now().to_rfc3339(),current.assignment_id,current.revision])?;
        transaction.commit()?;
        Ok(changed == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(
        db: &Database,
        policy: crate::PublishCleanupPolicy,
        state: crate::PublishCampaignState,
        evidence: &serde_json::Value,
    ) -> String {
        let id = Uuid::new_v4().to_string();
        let bundle = crate::PublishBundle {
            id: id.clone(),
            source_path: "C:/fixture".into(),
            name: id.clone(),
            media_kind: crate::PublishMediaKind::Image,
            images: Vec::new(),
            video: None,
            caption_path: "C:/fixture/caption.txt".into(),
            caption: "fixture".into(),
            caption_sha256: "a".repeat(64),
            total_bytes: 1,
            partners: Vec::new(),
        };
        let request = crate::PublishCampaignRequest {
            sheet_enabled: false,
            request_id: id.clone(),
            source_root: "C:/fixture".into(),
            bundle_ids: vec![id],
            udids: vec!["phone".into()],
            run_at: None,
            visibility: crate::PublishVisibility::Public,
            cleanup_policy: policy,
            sound_policy: crate::PublishSoundPolicy::Default,
            execution_confirmed: true,
            target_snapshot: None,
        };
        let campaign = db.create_publish_campaign(&request, &[bundle]).unwrap();
        let assignment = db
            .get_publish_campaign(&campaign.id)
            .unwrap()
            .unwrap()
            .assignments[0]
            .id
            .clone();
        db.update_publish_assignment_state(&assignment, state, None, Some(&evidence.to_string()))
            .unwrap();
        assignment
    }

    fn proof() -> serde_json::Value {
        serde_json::json!({"post":{"state":"posted","publicationVerified":true,"postUrl":"https://www.tiktok.com/@fixture/photo/123","importId":"riviu-fixture-aa"},
            "cleanup":{"state":"kept","reason":"post_verification_pending","importId":"riviu-fixture-aa","appCleanup":{"state":"leftRunning"}}})
    }

    #[test]
    fn cleanup_debt_requires_verified_canonical_link_and_explicit_delete_policy() {
        use crate::{PublishCampaignState as State, PublishCleanupPolicy as Policy};
        let path = std::env::temp_dir().join(format!("cleanup-proof-{}.db", Uuid::new_v4()));
        let db = Database::open(&path).unwrap();
        for state in [
            State::Verifying,
            State::Uncertain,
            State::Posting,
            State::FailedBeforeDispatch,
        ] {
            let id = seed(
                &db,
                Policy::DeleteImportedAssetsAfterVerified,
                state,
                &proof(),
            );
            assert!(db.pending_publish_cleanup(&id).unwrap().is_none());
        }
        let id = seed(&db, Policy::KeepImportedAssets, State::Succeeded, &proof());
        assert!(db.pending_publish_cleanup(&id).unwrap().is_none());
        for link in [
            "https://vt.tiktok.com/abc/",
            "https://example.com/@fixture/photo/123",
            "https://www.tiktok.com/@fixture/photo/123?tracking=1",
            "https://www.tiktok.com/@fixture",
        ] {
            let mut invalid = proof();
            invalid["post"]["postUrl"] = serde_json::json!(link);
            let id = seed(
                &db,
                Policy::DeleteImportedAssetsAfterVerified,
                State::Succeeded,
                &invalid,
            );
            assert!(db.pending_publish_cleanup(&id).unwrap().is_none(), "{link}");
        }
        let mut invalid = proof();
        invalid["post"]["publicationVerified"] = serde_json::json!(false);
        let id = seed(
            &db,
            Policy::DeleteImportedAssetsAfterVerified,
            State::Succeeded,
            &invalid,
        );
        assert!(db.pending_publish_cleanup(&id).unwrap().is_none());
        invalid = proof();
        invalid["cleanup"]["importId"] = serde_json::json!("other-import");
        let id = seed(
            &db,
            Policy::DeleteImportedAssetsAfterVerified,
            State::Succeeded,
            &invalid,
        );
        assert!(db.pending_publish_cleanup(&id).unwrap().is_none());
        assert!(db.pending_publish_cleanups(100).unwrap().is_empty());
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cleanup_claim_survives_restart_and_stale_settlement_cannot_replace_new_proof() {
        let path = std::env::temp_dir().join(format!("cleanup-restart-{}.db", Uuid::new_v4()));
        let db = Database::open(&path).unwrap();
        let id = seed(
            &db,
            crate::PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
            crate::PublishCampaignState::Succeeded,
            &proof(),
        );
        let initial = db.pending_publish_cleanup(&id).unwrap().unwrap();
        let claimed = db.claim_publish_cleanup(&initial).unwrap().unwrap();
        assert!(db.claim_publish_cleanup(&initial).unwrap().is_none());
        let failed = serde_json::json!({"state":"not_cleaned","importId":claimed.import_id,"message":"disconnected"});
        assert!(!db.finish_publish_cleanup(&initial, &failed).unwrap());
        assert!(db.finish_publish_cleanup(&claimed, &failed).unwrap());
        drop(db);
        let db = Database::open(&path).unwrap();
        let retry = db.pending_publish_cleanups(1).unwrap().remove(0);
        assert_eq!(retry.import_id, initial.import_id);
        let running = db.claim_publish_cleanup(&retry).unwrap().unwrap();
        drop(db);
        let db = Database::open(&path).unwrap();
        let resumed = db.pending_publish_cleanup(&id).unwrap().unwrap();
        assert_eq!(resumed.revision, running.revision);
        let resumed = db.claim_publish_cleanup(&resumed).unwrap().unwrap();
        let cleaned = serde_json::json!({"state":"cleaned","importId":resumed.import_id,"appCleanup":{"state":"leftRunning"}});
        assert!(!db.finish_publish_cleanup(&running, &cleaned).unwrap());
        assert!(db.finish_publish_cleanup(&resumed, &cleaned).unwrap());
        assert!(db.pending_publish_cleanups(1).unwrap().is_empty());
        let detail = db
            .get_publish_campaign(&resumed.campaign_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            detail.assignments[0].state,
            crate::PublishCampaignState::Succeeded
        );
        let stored: serde_json::Value =
            serde_json::from_str(detail.assignments[0].evidence_json.as_deref().unwrap()).unwrap();
        assert_eq!(stored["post"], proof()["post"]);
        assert_eq!(stored["cleanup"]["state"], "cleaned");
        assert!(db.pending_publish_sheet_row(&id).unwrap().is_none());
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn verified_recovery_uses_original_native_import_when_post_response_was_lost() {
        let path = std::env::temp_dir().join(format!("cleanup-prior-{}.db", Uuid::new_v4()));
        let db = Database::open(&path).unwrap();
        let original=serde_json::json!({"nativeImport":{"value":{"state":"imported","importId":"riviu-lost-aa"}}}).to_string();
        let unknown=serde_json::json!({"message":"Post response lost","effectIntent":"post_carousel","priorEvidenceJson":original}).to_string();
        let evidence = serde_json::json!({"state":"posted","publicationVerified":true,"postUrl":"https://www.tiktok.com/@fixture/photo/123","priorEvidenceJson":unknown});
        let id = seed(
            &db,
            crate::PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
            crate::PublishCampaignState::Succeeded,
            &evidence,
        );
        let candidate = db.pending_publish_cleanup(&id).unwrap().unwrap();
        assert_eq!(candidate.import_id, "riviu-lost-aa");
        let claimed = db.claim_publish_cleanup(&candidate).unwrap().unwrap();
        assert!(db
            .finish_publish_cleanup(
                &claimed,
                &serde_json::json!({"state":"cleaned","importId":"riviu-lost-aa"})
            )
            .unwrap());
        assert!(db.pending_publish_cleanup(&id).unwrap().is_none());
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}
