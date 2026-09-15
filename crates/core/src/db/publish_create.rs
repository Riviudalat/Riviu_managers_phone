//! Stable UI request identity survives a lost create ACK independently of execution revisions.
use super::*;

fn replay_on(
    conn: &Connection,
    request_id: &str,
    fingerprint: &str,
) -> anyhow::Result<Option<crate::PublishCampaignRecord>> {
    let found:Option<(String,String)>=conn.query_row("SELECT campaign_id,request_fingerprint FROM publish_create_requests WHERE request_id=?1",
        [request_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let Some((id, prior)) = found else {
        return Ok(None);
    };
    anyhow::ensure!(
        prior == fingerprint,
        "requestId đã dùng cho nội dung khác; tạo lượt mới để thay đổi nội dung"
    );
    Ok(Some(
        Database::get_publish_campaign_from_connection(conn, &id)?
            .context("publication create receipt lost campaign")?
            .campaign,
    ))
}

impl Database {
    pub fn publication_campaign_id(&self, id: &str) -> anyhow::Result<Option<String>> {
        self.conn()?
            .query_row(
                "SELECT campaign_id FROM publish_assignments WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()
            .map_err(Into::into)
    }
    pub fn replay_publish_create(
        &self,
        request_id: &str,
        fingerprint: &str,
    ) -> anyhow::Result<Option<crate::PublishCampaignRecord>> {
        Uuid::parse_str(request_id)?;
        replay_on(&self.conn()?, request_id, fingerprint)
    }

    /// The request receipt, campaign and first projection commit together. Losing
    /// the reply or racing another caller cannot create a second publication.
    pub fn create_publish_with_receipt(
        &self,
        request: &crate::PublishCampaignRequest,
        bundles: &[crate::PublishBundle],
        snapshot: &crate::PublishExecutionSnapshotDraft,
        fingerprint: &str,
    ) -> anyhow::Result<(crate::PublishCampaignRecord, bool)> {
        Uuid::parse_str(&request.request_id)?;
        anyhow::ensure!(
            fingerprint.len() == 64 && fingerprint.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid create fingerprint"
        );
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(prior) = replay_on(&tx, &request.request_id, fingerprint)? {
            return Ok((prior, false));
        }
        let id = Uuid::new_v4().to_string();
        Self::insert_publish_campaign(&tx, &id, request, bundles, Some(snapshot))?;
        tx.execute(
            "INSERT INTO publish_create_requests VALUES(?1,?2,?3,?4)",
            params![request.request_id, id, fingerprint, Utc::now().to_rfc3339()],
        )?;
        let record = Self::get_publish_campaign_from_connection(&tx, &id)?
            .context("created campaign missing")?
            .campaign;
        tx.commit()?;
        Ok((record, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt_input(
        db: &Database,
        seed_campaign: &str,
    ) -> (
        crate::PublishCampaignRequest,
        crate::PublishBundle,
        crate::PublishExecutionSnapshotDraft,
    ) {
        let mut request = db.publish_campaign_request(seed_campaign).unwrap().unwrap();
        request.request_id = Uuid::new_v4().to_string();
        let mut bundle = db
            .get_publish_campaign(seed_campaign)
            .unwrap()
            .unwrap()
            .bundles
            .remove(0);
        bundle.id = format!("{}:{}", request.request_id, bundle.id);
        request.bundle_ids = vec![bundle.id.clone()];
        request.udids.truncate(1);
        let snapshot = crate::PublishExecutionSnapshotDraft {
            input_digest: "1".repeat(64),
            status: crate::PublishExecutionStatus::Partial,
            retry_scope: crate::PublishRetryScope::FullPipeline,
            report_json: serde_json::json!({}),
        };
        (request, bundle, snapshot)
    }

    #[test]
    fn concurrent_create_receipts_keep_only_the_winning_managed_media_and_projection() {
        let (db, path, seed, _) = super::super::publish_pipeline::tests::fixture();
        let (request, bundle, snapshot) = receipt_input(&db, &seed);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let mut workers = Vec::new();
        for attempt in ["prepare-a", "prepare-b"] {
            let connection = Database::open(&path).unwrap();
            let (request, mut bundle, mut snapshot, barrier) = (
                request.clone(),
                bundle.clone(),
                snapshot.clone(),
                barrier.clone(),
            );
            let managed = format!("C:/isolated-receipt-fixture/{attempt}");
            bundle.source_path = managed.clone();
            bundle.caption_path = format!("{managed}/caption.txt");
            snapshot.report_json = serde_json::json!({"managedSource":managed});
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                let (campaign, created) = connection
                    .create_publish_with_receipt(&request, &[bundle], &snapshot, &"2".repeat(64))
                    .unwrap();
                (managed, campaign.id, created)
            }));
        }
        barrier.wait();
        let outcomes: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(
            outcomes.iter().filter(|(_, _, created)| *created).count(),
            1
        );
        assert_eq!(outcomes[0].1, outcomes[1].1);
        let winner = outcomes.iter().find(|(_, _, created)| *created).unwrap();
        let loser = outcomes.iter().find(|(_, _, created)| !*created).unwrap();
        let detail = db.get_publish_campaign(&winner.1).unwrap().unwrap();
        assert_eq!(detail.assignments.len(), 1);
        assert_eq!(detail.bundles.len(), 1);
        assert_eq!(detail.bundles[0].source_path, winner.0);
        let conn = db.conn().unwrap();
        let (campaigns, publications, receipts): (i64, i64, i64) = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM publish_campaigns WHERE request_id=?1),
             (SELECT COUNT(*) FROM publish_assignments WHERE campaign_id=?2),
             (SELECT COUNT(*) FROM publish_create_requests WHERE request_id=?1)",
                params![request.request_id, winner.1],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!((campaigns, publications, receipts), (1, 1, 1));
        let (manifest, projection): (String, String) = conn.query_row(
            "SELECT b.manifest_json,s.report_json FROM publish_bundles b
             JOIN publish_execution_snapshots s ON s.campaign_id=b.campaign_id WHERE b.campaign_id=?1",
            [&winner.1], |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert!(!manifest.contains(&loser.0));
        assert!(!projection.contains(&loser.0));
        let projection: serde_json::Value = serde_json::from_str(&projection).unwrap();
        assert_eq!(projection["managedSource"], winner.0);
    }

    #[test]
    fn changed_request_after_restart_is_rejected_without_replacing_media_or_publication() {
        let (db, path, seed, _) = super::super::publish_pipeline::tests::fixture();
        let (mut request, mut bundle, snapshot) = receipt_input(&db, &seed);
        let (first, _) = db
            .create_publish_with_receipt(
                &request,
                std::slice::from_ref(&bundle),
                &snapshot,
                &"2".repeat(64),
            )
            .unwrap();
        let original = db.get_publish_campaign(&first.id).unwrap().unwrap();
        drop(db);
        let db = Database::open(path).unwrap();
        request.udids = vec!["changed-account-device".into()];
        bundle.caption = "Changed caption after a lost create response".into();
        bundle.source_path = "C:/new-uncommitted-preparation".into();
        assert!(db
            .replay_publish_create(&request.request_id, &"3".repeat(64))
            .is_err());
        assert!(db
            .create_publish_with_receipt(&request, &[bundle], &snapshot, &"3".repeat(64))
            .is_err());
        let replay = db
            .replay_publish_create(&request.request_id, &"2".repeat(64))
            .unwrap()
            .unwrap();
        assert_eq!(replay.id, first.id);
        let current = db.get_publish_campaign(&first.id).unwrap().unwrap();
        assert_eq!(
            current.assignments[0].publication_id,
            original.assignments[0].publication_id
        );
        assert_eq!(current.assignments[0].udid, original.assignments[0].udid);
        assert_eq!(current.bundles[0].caption, original.bundles[0].caption);
        assert_eq!(
            current.bundles[0].source_path,
            original.bundles[0].source_path
        );
        let count: i64 = db
            .conn()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM publish_campaigns WHERE request_id=?1",
                [&request.request_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn receipt_write_failure_rolls_back_campaign_publication_and_first_projection() {
        let (db, _, seed, _) = super::super::publish_pipeline::tests::fixture();
        let (request, bundle, snapshot) = receipt_input(&db, &seed);
        db.conn()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER reject_receipt BEFORE INSERT ON publish_create_requests
             BEGIN SELECT RAISE(ABORT,'fixture receipt storage failure'); END;",
            )
            .unwrap();
        assert!(db
            .create_publish_with_receipt(
                &request,
                std::slice::from_ref(&bundle),
                &snapshot,
                &"2".repeat(64)
            )
            .is_err());
        let conn = db.conn().unwrap();
        let (campaigns, media, publications, projections): (i64, i64, i64, i64) = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM publish_campaigns WHERE request_id=?1),
             (SELECT COUNT(*) FROM publish_bundles WHERE id=?2),
             (SELECT COUNT(*) FROM publish_assignments WHERE bundle_id=?2),
             (SELECT COUNT(*) FROM publish_execution_snapshots)",
                params![request.request_id, bundle.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!((campaigns, media, publications, projections), (0, 0, 0, 0));
        drop(conn);
        assert!(db
            .replay_publish_create(&request.request_id, &"2".repeat(64))
            .unwrap()
            .is_none());
    }
    #[test]
    fn lost_ack_replays_the_same_publications_after_restart_and_execution_changes() {
        let (db, path, id, _) = super::super::publish_pipeline::tests::fixture();
        let mut request = db.publish_campaign_request(&id).unwrap().unwrap();
        request.request_id = Uuid::new_v4().to_string();
        let mut bundles = db.get_publish_campaign(&id).unwrap().unwrap().bundles;
        for b in &mut bundles {
            b.id = format!("{}:{}", request.request_id, b.id);
        }
        request.bundle_ids = bundles.iter().map(|b| b.id.clone()).collect();
        let snapshot = crate::PublishExecutionSnapshotDraft {
            input_digest: "1".repeat(64),
            status: crate::PublishExecutionStatus::Partial,
            retry_scope: crate::PublishRetryScope::FullPipeline,
            report_json: serde_json::json!({}),
        };
        let (first, created) = db
            .create_publish_with_receipt(&request, &bundles, &snapshot, &"2".repeat(64))
            .unwrap();
        assert!(created);
        let assignments = db
            .get_publish_campaign(&first.id)
            .unwrap()
            .unwrap()
            .assignments;
        db.claim_publish_pipeline(&first.id).unwrap();
        let reopened = Database::open(path).unwrap();
        let (again, created) = reopened
            .create_publish_with_receipt(&request, &bundles, &snapshot, &"2".repeat(64))
            .unwrap();
        assert!(!created);
        assert_eq!(first.id, again.id);
        assert_eq!(
            reopened
                .get_publish_campaign(&again.id)
                .unwrap()
                .unwrap()
                .assignments
                .iter()
                .map(|a| &a.id)
                .collect::<Vec<_>>(),
            assignments.iter().map(|a| &a.id).collect::<Vec<_>>()
        );
        assert!(reopened
            .replay_publish_create(&request.request_id, &"3".repeat(64))
            .is_err());
    }
}
