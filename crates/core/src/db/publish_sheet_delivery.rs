//! Persistent scheduling and one shared claim for each v2 Sheet assignment.
use super::*;
use crate::publish_sheet::{InternalReportMetadata, InternalSheetReportRow, SheetDeliveryTarget};

const CLAIM_MS: i64 = 120_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheetDeliveryKind {
    Canonical,
    Report,
}
impl SheetDeliveryKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Canonical => "canonical",
            Self::Report => "report",
        }
    }
}

#[derive(Debug, Clone)]
pub enum SheetDeliveryPayload {
    Canonical {
        row: SheetOutboxRow,
        metadata: Option<InternalReportMetadata>,
    },
    Report(InternalSheetReportRow),
}

#[derive(Debug, Clone)]
pub struct SheetDeliveryClaim {
    pub assignment_id: String,
    pub target: SheetDeliveryTarget,
    pub token: String,
    pub kind: SheetDeliveryKind,
    pub payload: SheetDeliveryPayload,
}

fn backoff_ms(attempts: i64) -> i64 {
    30_000i64
        .saturating_mul(1i64 << attempts.saturating_sub(1).clamp(0, 5))
        .min(900_000)
}

fn seed_states(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "INSERT OR IGNORE INTO publish_sheet_sync_state(assignment_id)
      SELECT a.id FROM publish_assignments a JOIN publish_campaigns c ON c.id=a.campaign_id
      WHERE json_valid(c.request_json) AND json_extract(c.request_json,'$.sheetDelivery.version')=2
        AND json_extract(c.request_json,'$.sheetEnabled')=1;
      INSERT OR IGNORE INTO publish_sheet_sync_state(assignment_id)
      SELECT assignment_id FROM publish_sheet_outbox WHERE delivery_target_json IS NOT NULL;",
    )?;
    Ok(())
}

impl Database {
    /// Changing the credential connection reopens paused v2 delivery only. No history is enrolled.
    pub fn configure_bound_sheet_delivery(
        &self,
        fingerprint: &str,
        now_ms: i64,
    ) -> anyhow::Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        seed_states(&tx)?;
        tx.execute("UPDATE publish_sheet_outbox SET next_attempt_at_ms=?2 WHERE state<>'sent'
          AND delivery_target_json IS NOT NULL AND assignment_id IN (
            SELECT assignment_id FROM publish_sheet_sync_state WHERE connection_fingerprint IS NOT ?1)",params![fingerprint,now_ms])?;
        tx.execute("UPDATE publish_sheet_sync_state SET connection_fingerprint=?1,report_next_attempt_at_ms=?2,
          report_attempts=0,report_last_error=NULL WHERE connection_fingerprint IS NOT ?1",params![fingerprint,now_ms])?;
        tx.commit()?;
        Ok(())
    }

    /// Both manual delivery and the sweeper acquire here. At most two HTTP requests,
    /// including at most one report, can hold a live claim across the database.
    pub fn claim_bound_sheet_delivery(
        &self,
        kind: SheetDeliveryKind,
        assignment_id: Option<&str>,
        now_ms: i64,
    ) -> anyhow::Result<Option<SheetDeliveryClaim>> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        seed_states(&tx)?;
        let (active, reports): (i64, i64) = tx.query_row(
            "SELECT COUNT(*),COALESCE(SUM(claim_kind='report'),0)
          FROM publish_sheet_sync_state WHERE claim_token IS NOT NULL AND claim_until_ms>?1",
            [now_ms],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if active >= 2 || (kind == SheetDeliveryKind::Report && reports >= 1) {
            return Ok(None);
        }
        let candidate:Option<(String,String)> = match kind {
            SheetDeliveryKind::Canonical => tx.query_row(
                "SELECT o.assignment_id,o.delivery_target_json FROM publish_sheet_outbox o
                 JOIN publish_sheet_sync_state s ON s.assignment_id=o.assignment_id
                 WHERE o.delivery_target_json IS NOT NULL AND o.state<>'sent' AND o.next_attempt_at_ms<=?1
                   AND (s.claim_token IS NULL OR s.claim_until_ms<=?1) AND (?2 IS NULL OR o.assignment_id=?2)
                 ORDER BY o.next_attempt_at_ms,o.created_at,o.assignment_id LIMIT 1",
                params![now_ms,assignment_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?,
            SheetDeliveryKind::Report => tx.query_row(
                "SELECT a.id,json_extract(c.request_json,'$.sheetDelivery')
                 FROM publish_assignments a JOIN publish_campaigns c ON c.id=a.campaign_id
                 JOIN publish_sheet_sync_state s ON s.assignment_id=a.id
                 WHERE json_valid(c.request_json) AND json_extract(c.request_json,'$.sheetDelivery.version')=2
                   AND json_extract(c.request_json,'$.sheetDelivery.internalReporting')=1
                   AND json_extract(c.request_json,'$.sheetEnabled')=1
                   AND a.revision+c.revision>s.report_acked_revision
                   AND (s.report_attempt_revision<>a.revision+c.revision OR s.report_next_attempt_at_ms<=?1)
                   AND (s.claim_token IS NULL OR s.claim_until_ms<=?1) AND (?2 IS NULL OR a.id=?2)
                   AND NOT EXISTS(SELECT 1 FROM publish_sheet_outbox o WHERE o.assignment_id=a.id
                     AND o.delivery_target_json IS NOT NULL AND o.state<>'sent')
                 ORDER BY CASE WHEN s.report_attempt_revision<>a.revision+c.revision THEN 0 ELSE s.report_next_attempt_at_ms END,a.created_at,a.id LIMIT 1",
                params![now_ms,assignment_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?,
        };
        let Some((id, raw_target)) = candidate else {
            return Ok(None);
        };
        let prepared = (|| {
            let target: SheetDeliveryTarget = serde_json::from_str(&raw_target)?;
            target.validate()?;
            let payload = match kind {
                SheetDeliveryKind::Canonical => {
                    let row=tx.query_row("SELECT assignment_id,campaign_id,post_url,poster,partners_json,attempts,revision,last_error,posted_at
                  FROM publish_sheet_outbox WHERE assignment_id=?1",[&id],|r| {
                    let partners:String=r.get(4)?;
                    let partners=serde_json::from_str(&partners).map_err(|e| rusqlite::Error::FromSqlConversionFailure(4,rusqlite::types::Type::Text,Box::new(e)))?;
                    Ok(SheetOutboxRow{assignment_id:r.get(0)?,campaign_id:r.get(1)?,post_url:r.get(2)?,poster:r.get(3)?,partners,
                        attempts:r.get::<_,i64>(5)?.clamp(0,u32::MAX as i64) as u32,revision:r.get(6)?,last_error:r.get(7)?,posted_at:r.get(8)?})
                })?;
                    let metadata = if target.internal_reporting {
                        let current = super::publish_report::internal_report_on(&tx, &id)?
                            .filter(|r| r.metadata.status == "Đã xác minh")
                            .map(|r| r.metadata);
                        let stored:Option<String>=tx.query_row("SELECT report_metadata_json FROM publish_sheet_outbox WHERE assignment_id=?1",[&id],|r|r.get(0))?;
                        Some(
                            current
                                .or(stored.map(|raw| serde_json::from_str(&raw)).transpose()?)
                                .context("verified internal Sheet metadata unavailable")?,
                        )
                    } else {
                        None
                    };
                    SheetDeliveryPayload::Canonical { row, metadata }
                }
                SheetDeliveryKind::Report => {
                    let report = super::publish_report::internal_report_on(&tx, &id)?
                        .context("bound report disappeared")?;
                    tx.execute("UPDATE publish_sheet_sync_state SET report_attempts=CASE WHEN report_attempt_revision=?2 THEN report_attempts ELSE 0 END,
                  report_attempt_revision=?2 WHERE assignment_id=?1",params![id,report.metadata.row_revision])?;
                    SheetDeliveryPayload::Report(report)
                }
            };
            Ok::<_, anyhow::Error>((target, payload))
        })();
        let (target, payload) = match prepared {
            Ok(value) => value,
            Err(error) => {
                pause_invalid_row(&tx, &id, kind, &error.to_string())?;
                tx.commit()?;
                return Err(error);
            }
        };
        let token = Uuid::new_v4().to_string();
        tx.execute("UPDATE publish_sheet_sync_state SET claim_token=?2,claim_until_ms=?3,claim_kind=?4 WHERE assignment_id=?1",
            params![id,token,now_ms.saturating_add(CLAIM_MS),kind.as_str()])?;
        tx.commit()?;
        Ok(Some(SheetDeliveryClaim {
            assignment_id: id,
            target,
            token,
            kind,
            payload,
        }))
    }

    /// Only explicit retry opens paused rows; it never revives a historical obligation.
    pub fn retry_bound_sheet_assignment(&self, id: &str, now_ms: i64) -> anyhow::Result<bool> {
        let conn = self.conn()?;
        Ok(conn.execute(
            "UPDATE publish_sheet_outbox SET next_attempt_at_ms=?2 WHERE assignment_id=?1
          AND delivery_target_json IS NOT NULL AND state<>'sent'",
            params![id, now_ms],
        )? > 0)
    }

    pub fn fail_bound_sheet_delivery(
        &self,
        claim: &SheetDeliveryClaim,
        reason: &str,
        retryable: bool,
        now_ms: i64,
    ) -> anyhow::Result<bool> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !claim_current(&tx, claim, now_ms)? {
            return Ok(false);
        }
        let changed = match &claim.payload {
            SheetDeliveryPayload::Canonical { row, .. } => {
                let next = retryable
                    .then(|| now_ms.saturating_add(backoff_ms(i64::from(row.attempts) + 1)));
                tx.execute("UPDATE publish_sheet_outbox SET state='failed',attempts=attempts+1,last_error=?3,
                  next_attempt_at_ms=?4,updated_at=?5 WHERE assignment_id=?1 AND revision=?2 AND state<>'sent'",
                  params![row.assignment_id,row.revision,reason,next,Utc::now().to_rfc3339()])?
            }
            SheetDeliveryPayload::Report(row) => {
                let attempts: i64 = tx.query_row(
                    "SELECT report_attempts FROM publish_sheet_sync_state WHERE assignment_id=?1",
                    [&claim.assignment_id],
                    |r| r.get(0),
                )?;
                let next = retryable.then(|| now_ms.saturating_add(backoff_ms(attempts + 1)));
                tx.execute("UPDATE publish_sheet_sync_state SET report_attempts=report_attempts+1,report_next_attempt_at_ms=?2,report_last_error=?3
                  WHERE assignment_id=?1 AND report_attempt_revision=?4",params![claim.assignment_id,next,reason,row.metadata.row_revision])?
            }
        };
        release_claim(&tx, claim)?;
        tx.commit()?;
        Ok(changed > 0)
    }

    pub fn settle_bound_sheet_delivery(
        &self,
        claim: &SheetDeliveryClaim,
        input_digest: Option<&str>,
        target_snapshot: Option<&crate::ResolvedTargetSnapshot>,
        now_ms: i64,
    ) -> anyhow::Result<SheetOutboxSettlement> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !claim_current(&tx, claim, now_ms)? {
            return Ok(SheetOutboxSettlement::StaleRevision);
        }
        let (result, report_revision) = match &claim.payload {
            SheetDeliveryPayload::Canonical { row, metadata } => (
                super::publish_sheet::settle_delivery_on(
                    &tx,
                    &row.assignment_id,
                    &row.campaign_id,
                    row.revision,
                    input_digest,
                    target_snapshot,
                )?,
                metadata.as_ref().map(|m| m.row_revision),
            ),
            SheetDeliveryPayload::Report(row) => (
                SheetOutboxSettlement::DeliveredWithoutCampaign,
                Some(row.metadata.row_revision),
            ),
        };
        if matches!(result, SheetOutboxSettlement::StaleRevision) {
            release_claim(&tx, claim)?;
            tx.commit()?;
            return Ok(result);
        }
        if let Some(revision) = report_revision {
            tx.execute("UPDATE publish_sheet_sync_state SET report_acked_revision=MAX(report_acked_revision,?2),report_attempts=0,
              report_last_error=NULL,report_next_attempt_at_ms=0 WHERE assignment_id=?1",params![claim.assignment_id,revision])?;
        }
        release_claim(&tx, claim)?;
        tx.commit()?;
        Ok(result)
    }
}

fn claim_current(
    conn: &Connection,
    claim: &SheetDeliveryClaim,
    now_ms: i64,
) -> anyhow::Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM publish_sheet_sync_state WHERE assignment_id=?1
      AND claim_token=?2 AND claim_kind=?3 AND claim_until_ms>?4)",
        params![
            claim.assignment_id,
            claim.token,
            claim.kind.as_str(),
            now_ms
        ],
        |r| r.get(0),
    )?)
}
fn release_claim(conn: &Connection, claim: &SheetDeliveryClaim) -> anyhow::Result<()> {
    conn.execute("UPDATE publish_sheet_sync_state SET claim_token=NULL,claim_until_ms=NULL,claim_kind=NULL WHERE assignment_id=?1 AND claim_token=?2",
      params![claim.assignment_id,claim.token])?;
    Ok(())
}

fn pause_invalid_row(
    conn: &Connection,
    id: &str,
    kind: SheetDeliveryKind,
    reason: &str,
) -> anyhow::Result<()> {
    match kind {
        SheetDeliveryKind::Canonical => {
            conn.execute("UPDATE publish_sheet_outbox SET state='failed',last_error=?2,next_attempt_at_ms=NULL WHERE assignment_id=?1",params![id,reason])?;
        }
        SheetDeliveryKind::Report => {
            conn.execute("UPDATE publish_sheet_sync_state SET report_next_attempt_at_ms=NULL,report_last_error=?2,
          report_attempt_revision=(SELECT a.revision+c.revision FROM publish_assignments a JOIN publish_campaigns c ON c.id=a.campaign_id WHERE a.id=?1) WHERE assignment_id=?1",params![id,reason])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        db: Database,
        path: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("sheet-delivery-{}.db", Uuid::new_v4()));
            Self {
                db: Database::open(&path).unwrap(),
                path,
            }
        }
        fn canonical(&self, id: &str, versioned: bool) {
            let target = versioned.then(|| serde_json::to_string(&target(false)).unwrap());
            self.db.conn().unwrap().execute("INSERT INTO publish_sheet_outbox(assignment_id,campaign_id,post_url,poster,partners_json,state,created_at,updated_at,delivery_target_json,posted_at)
              VALUES(?1,'detached',?2,'bot','[]','pending',?1,?1,?3,'2026-09-12T00:00:00Z')",params![id,format!("https://www.tiktok.com/@fixture/photo/{id}"),target]).unwrap();
        }
        fn report(&self) -> String {
            let id = Uuid::new_v4().to_string();
            let bundle = crate::PublishBundle {
                id: id.clone(),
                source_path: "/fixture".into(),
                name: "fixture".into(),
                media_kind: crate::PublishMediaKind::Image,
                images: vec![],
                video: None,
                caption_path: "/fixture/caption.txt".into(),
                caption: "fixture".into(),
                caption_sha256: "a".repeat(64),
                total_bytes: 0,
                partners: vec![],
            };
            let request = crate::PublishCampaignRequest {
                sheet_delivery: Some(target(true)),
                verification_contract_version: Some(1),
                verification_builds: vec![crate::publish_submission::PublishVerificationBuild {
                    udid: "fixture-phone".into(),
                    package: "com.zhiliaoapp.musically".into(),
                    version: "45.7.3".into(),
                    locale: "en".into(),
                }],
                sheet_enabled: true,
                request_id: Uuid::new_v4().to_string(),
                source_root: "/fixture".into(),
                bundle_ids: vec![id],
                udids: vec!["fixture-phone".into()],
                run_at: None,
                visibility: crate::PublishVisibility::Public,
                cleanup_policy: crate::PublishCleanupPolicy::KeepImportedAssets,
                network: crate::SocialNetwork::TikTok,
                sound_policy: crate::PublishSoundPolicy::Default,
                execution_confirmed: false,
                target_snapshot: None,
            };
            let campaign = self
                .db
                .create_publish_campaign(&request, &[bundle])
                .unwrap();
            self.db
                .get_publish_campaign(&campaign.id)
                .unwrap()
                .unwrap()
                .assignments[0]
                .id
                .clone()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
    fn target(internal_reporting: bool) -> SheetDeliveryTarget {
        SheetDeliveryTarget {
            version: 2,
            spreadsheet_id: "fixture-book".into(),
            sheet_gid: 0,
            internal_reporting,
        }
    }
    fn claim(
        db: &Database,
        kind: SheetDeliveryKind,
        id: Option<&str>,
        now: i64,
    ) -> Option<SheetDeliveryClaim> {
        db.claim_bound_sheet_delivery(kind, id, now).unwrap()
    }

    #[test]
    fn more_than_one_hundred_poisoned_rows_do_not_hold_a_healthy_row() {
        let f = Fixture::new();
        for n in 0..=100 {
            f.canonical(&format!("{n:04}"), true);
        }
        f.canonical("history", false);
        for n in 0..100 {
            let c = claim(&f.db, SheetDeliveryKind::Canonical, None, 1000).unwrap();
            assert_eq!(c.assignment_id, format!("{n:04}"));
            f.db.fail_bound_sheet_delivery(&c, "wrong header", false, 1001)
                .unwrap();
        }
        let healthy = claim(&f.db, SheetDeliveryKind::Canonical, None, 1002).unwrap();
        assert_eq!(healthy.assignment_id, "0100");
        assert!(matches!(
            f.db.settle_bound_sheet_delivery(&healthy, None, None, 1003)
                .unwrap(),
            SheetOutboxSettlement::DeliveredWithoutCampaign
        ));
        assert!(claim(&f.db, SheetDeliveryKind::Canonical, None, 1004).is_none());
        assert!(!f.db.retry_bound_sheet_assignment("history", 1004).unwrap());
    }

    #[test]
    fn claims_limit_parallel_writers_and_expired_claims_cannot_settle() {
        let f = Fixture::new();
        for id in ["1", "2", "3"] {
            f.canonical(id, true);
        }
        let first = claim(&f.db, SheetDeliveryKind::Canonical, Some("1"), 0).unwrap();
        assert!(claim(&f.db, SheetDeliveryKind::Canonical, Some("1"), 0).is_none());
        let second = claim(&f.db, SheetDeliveryKind::Canonical, None, 0).unwrap();
        assert!(claim(&f.db, SheetDeliveryKind::Canonical, None, 0).is_none());
        let restarted = Database::open(&f.path).unwrap();
        let replacement = claim(
            &restarted,
            SheetDeliveryKind::Canonical,
            Some("1"),
            CLAIM_MS + 1,
        )
        .unwrap();
        assert_ne!(first.token, replacement.token);
        assert!(matches!(
            f.db.settle_bound_sheet_delivery(&first, None, None, CLAIM_MS + 2)
                .unwrap(),
            SheetOutboxSettlement::StaleRevision
        ));
        assert!(!f
            .db
            .fail_bound_sheet_delivery(&second, "late", true, CLAIM_MS + 2)
            .unwrap());
        assert!(matches!(
            f.db.settle_bound_sheet_delivery(&replacement, None, None, CLAIM_MS + 2)
                .unwrap(),
            SheetOutboxSettlement::DeliveredWithoutCampaign
        ));
    }

    #[test]
    fn transient_backoff_and_permanent_pause_survive_restart() {
        let f = Fixture::new();
        f.canonical("1", true);
        let mut now = 0;
        for delay in [30_000, 60_000, 120_000, 240_000, 480_000, 900_000, 900_000] {
            let c = claim(&f.db, SheetDeliveryKind::Canonical, None, now).unwrap();
            f.db.fail_bound_sheet_delivery(&c, "network", true, now)
                .unwrap();
            let reopened = Database::open(&f.path).unwrap();
            assert!(claim(
                &reopened,
                SheetDeliveryKind::Canonical,
                None,
                now + delay - 1
            )
            .is_none());
            now += delay;
        }
        let c = claim(&f.db, SheetDeliveryKind::Canonical, None, now).unwrap();
        f.db.fail_bound_sheet_delivery(&c, "target conflict", false, now)
            .unwrap();
        assert!(claim(&f.db, SheetDeliveryKind::Canonical, None, now + 9_000_000).is_none());
        f.db.retry_bound_sheet_assignment("1", now + 1).unwrap();
        assert!(claim(&f.db, SheetDeliveryKind::Canonical, None, now + 1).is_some());
    }

    #[test]
    fn malformed_row_is_paused_without_discarding_partner_names_or_blocking_next() {
        let f = Fixture::new();
        f.canonical("1", true);
        f.canonical("2", true);
        f.db.conn()
            .unwrap()
            .execute(
                "UPDATE publish_sheet_outbox SET partners_json='broken' WHERE assignment_id='1'",
                [],
            )
            .unwrap();
        assert!(f
            .db
            .claim_bound_sheet_delivery(SheetDeliveryKind::Canonical, None, 0)
            .is_err());
        assert_eq!(
            claim(&f.db, SheetDeliveryKind::Canonical, None, 0)
                .unwrap()
                .assignment_id,
            "2"
        );
        assert_eq!(
            f.db.conn()
                .unwrap()
                .query_row(
                    "SELECT partners_json FROM publish_sheet_outbox WHERE assignment_id='1'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "broken"
        );
    }

    #[test]
    fn one_report_slot_leaves_canonical_capacity_and_ack_survives_restart() {
        let f = Fixture::new();
        let id = f.report();
        let other = f.report();
        f.canonical("1", true);
        let report = claim(&f.db, SheetDeliveryKind::Report, Some(&id), 0).unwrap();
        assert!(claim(&f.db, SheetDeliveryKind::Report, Some(&other), 0).is_none());
        let canonical = claim(&f.db, SheetDeliveryKind::Canonical, None, 0).unwrap();
        f.db.settle_bound_sheet_delivery(&report, None, None, 1)
            .unwrap();
        f.db.settle_bound_sheet_delivery(&canonical, None, None, 1)
            .unwrap();
        let restarted = Database::open(&f.path).unwrap();
        assert!(claim(&restarted, SheetDeliveryKind::Report, Some(&id), 2).is_none());
        f.db.update_publish_assignment_state(
            &id,
            crate::PublishCampaignState::Preparing,
            None,
            None,
        )
        .unwrap();
        assert!(claim(&restarted, SheetDeliveryKind::Report, Some(&id), 3).is_some());
    }

    #[test]
    fn paused_report_revision_advances_and_claim_is_shared_with_canonical() {
        let f = Fixture::new();
        let id = f.report();
        let report = claim(&f.db, SheetDeliveryKind::Report, Some(&id), 0).unwrap();
        f.db.fail_bound_sheet_delivery(&report, "conflict", false, 1)
            .unwrap();
        assert!(claim(&f.db, SheetDeliveryKind::Report, Some(&id), 2).is_none());
        f.db.update_publish_assignment_state(
            &id,
            crate::PublishCampaignState::Preparing,
            None,
            None,
        )
        .unwrap();
        let report = claim(&f.db, SheetDeliveryKind::Report, Some(&id), 3).unwrap();
        f.canonical(&id, true);
        assert!(claim(&f.db, SheetDeliveryKind::Canonical, Some(&id), 3).is_none());
        f.db.settle_bound_sheet_delivery(&report, None, None, 4)
            .unwrap();
        assert!(claim(&f.db, SheetDeliveryKind::Report, Some(&id), 5).is_none());
        assert!(claim(&f.db, SheetDeliveryKind::Canonical, Some(&id), 5).is_some());
    }

    #[test]
    fn unchanged_connection_does_not_reset_pauses_and_new_connection_reopens_v2_only() {
        let f = Fixture::new();
        f.canonical("1", true);
        f.canonical("history", false);
        f.db.configure_bound_sheet_delivery("connection1", 0)
            .unwrap();
        let c = claim(&f.db, SheetDeliveryKind::Canonical, None, 0).unwrap();
        f.db.fail_bound_sheet_delivery(&c, "credentials", false, 1)
            .unwrap();
        f.db.configure_bound_sheet_delivery("connection1", 2)
            .unwrap();
        assert!(claim(&f.db, SheetDeliveryKind::Canonical, None, 2).is_none());
        f.db.configure_bound_sheet_delivery("connection2", 3)
            .unwrap();
        assert_eq!(
            claim(&f.db, SheetDeliveryKind::Canonical, None, 3)
                .unwrap()
                .assignment_id,
            "1"
        );
        assert!(claim(&f.db, SheetDeliveryKind::Canonical, Some("history"), 3).is_none());
    }

    #[test]
    fn bound_outbox_pins_target_and_submission_time_and_survives_campaign_deletion() {
        let f = Fixture::new();
        let id = f.report();
        let conn = f.db.conn().unwrap();
        let campaign: String = conn
            .query_row(
                "SELECT campaign_id FROM publish_assignments WHERE id=?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        let url = "https://www.tiktok.com/@fixture/photo/123456";
        let bundle_id: String = conn
            .query_row(
                "SELECT bundle_id FROM publish_assignments WHERE id=?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        let proof = crate::publish_submission::PublishSubmissionProof {
            verification_contract_version: 1,
            expected_account: "fixture".into(),
            submitted_at: "2026-09-12T00:00:00Z".into(),
            package: "com.zhiliaoapp.musically".into(),
            version: "45.7.3".into(),
            locale: "en".into(),
            caption_sha256: "a".repeat(64),
            bundle_id,
            media_kind: crate::PublishMediaKind::Image,
        };
        let mut intent = serde_json::to_value(proof).unwrap();
        intent["effectIntent"] = serde_json::json!("post");
        conn.execute(
            "UPDATE publish_assignments SET effect_intent=?2 WHERE id=?1",
            params![id, intent.to_string()],
        )
        .unwrap();
        f.db.record_publish_success_with_sheet_row(
            &id,
            &serde_json::json!({"postUrl":url,"publicationVerified":true}).to_string(),
            &campaign,
            url,
            "bot",
            &[],
        )
        .unwrap();
        let before =
            f.db.pending_publish_sheet_row(&id)
                .unwrap()
                .unwrap()
                .revision;
        f.db.queue_publish_sheet_row(&id, &campaign, url, "bot", &[])
            .unwrap();
        assert_eq!(
            f.db.pending_publish_sheet_row(&id)
                .unwrap()
                .unwrap()
                .revision,
            before
        );
        assert!(f
            .db
            .queue_publish_sheet_row(
                &id,
                &campaign,
                "https://www.tiktok.com/@other/photo/123456",
                "bot",
                &[]
            )
            .is_err());
        conn.execute("DELETE FROM publish_campaigns WHERE id=?1", [&campaign])
            .unwrap();
        let delivery = claim(&f.db, SheetDeliveryKind::Canonical, Some(&id), 0).unwrap();
        assert_eq!(delivery.target, target(true));
        let SheetDeliveryPayload::Canonical { row, metadata } = &delivery.payload else {
            panic!("canonical")
        };
        assert_eq!(row.posted_at.as_deref(), Some("2026-09-12T00:00:00Z"));
        assert_eq!(metadata.as_ref().unwrap().status, "Đã xác minh");
        assert!(matches!(
            f.db.settle_bound_sheet_delivery(&delivery, None, None, 1)
                .unwrap(),
            SheetOutboxSettlement::DeliveredWithoutCampaign
        ));
    }

    #[test]
    fn settlement_failure_rolls_back_outbox_and_keeps_claim_for_retry() {
        let f = Fixture::new();
        let id = f.report();
        f.canonical(&id, true);
        let campaign: String =
            f.db.conn()
                .unwrap()
                .query_row(
                    "SELECT campaign_id FROM publish_assignments WHERE id=?1",
                    [&id],
                    |r| r.get(0),
                )
                .unwrap();
        f.db.conn()
            .unwrap()
            .execute(
                "UPDATE publish_sheet_outbox SET campaign_id=?2 WHERE assignment_id=?1",
                params![id, campaign],
            )
            .unwrap();
        let delivery = claim(&f.db, SheetDeliveryKind::Canonical, Some(&id), 0).unwrap();
        assert!(f
            .db
            .settle_bound_sheet_delivery(&delivery, Some("bad-digest"), None, 1)
            .is_err());
        assert_eq!(
            f.db.publish_sheet_outbox_state(&id).unwrap(),
            Some(SheetOutboxState::Pending)
        );
        assert!(claim(&f.db, SheetDeliveryKind::Canonical, Some(&id), 1).is_none());
        f.db.fail_bound_sheet_delivery(&delivery, "database failure after remote commit", true, 2)
            .unwrap();
        let retry = claim(&f.db, SheetDeliveryKind::Canonical, Some(&id), 30_002).unwrap();
        assert!(matches!(
            f.db.settle_bound_sheet_delivery(&retry, Some(&"a".repeat(64)), None, 30_003)
                .unwrap(),
            SheetOutboxSettlement::Delivered(_)
        ));
    }
}
