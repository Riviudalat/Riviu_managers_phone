//! Orphaned publish media on a phone: which Riviu-named directories still have a living
//! assignment, and the intent/receipt rows for deleting the ones that do not.
//!
//! The verified-cleanup worker only ever sees `succeeded` assignments. Everything that
//! stopped before Post (`failed_before_dispatch`), was cancelled or missed keeps its album
//! and its hidden staging scope on the phone, and so does any scope whose controller was
//! closed before the deferred cleanup got its turn. Measured 14/10/2026 on the 30-phone
//! fleet after the a62c6b72 wave: 12–30 leftover `riviu-*` / `.riviu-publish/*` entries
//! per phone with every campaign set to delete after verification, and TikTok's picker
//! timing out on an album list that long (`album_not_visible` on phone 37).
use super::*;

/// One deletion the sweep has committed to, recorded before the phone is touched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishMediaSweepClaim {
    pub id: String,
    pub udid: String,
    pub kind: PublishMediaSweepKind,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishMediaSweepKind {
    /// A visible `Pictures/riviu-<scope>-<hash>` album, deleted with its MediaStore rows.
    Album,
    /// A hidden `Pictures/.riviu-publish/<scope>` directory that never became an album.
    Stage,
}

impl PublishMediaSweepKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Album => "album",
            Self::Stage => "stage",
        }
    }
}

/// The staging scope a Riviu album name was derived from, or `None` for any other shape.
/// Mirrors the Android driver's `riviu-<scope>-<12 hex>` layout without depending on it.
pub fn publish_album_scope(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("riviu-")?;
    let (scope, hash) = rest.rsplit_once('-')?;
    (hash.len() == 12 && hash.bytes().all(|b| b.is_ascii_hexdigit()) && !scope.is_empty())
        .then_some(scope)
}

/// Where the publish path stages one assignment's media: `<campaign>-<ordinal>`. The
/// desktop composition root derives the same string before every transfer.
pub fn publish_device_scope(campaign_id: &str, ordinal: u32) -> String {
    format!("{campaign_id}-{ordinal}")
}

impl Database {
    /// Staging scopes on this phone that still belong to a living assignment.
    ///
    /// Living means: not yet terminal (anything before Post, posting, verifying,
    /// uncertain), or `succeeded` without a native cleanup proof. The second clause keeps
    /// the media of a campaign that chose `keepImportedAssets` and of a verified post whose
    /// deferred cleanup has not run yet; both belong to the verified-cleanup worker, not to
    /// the sweep. Terminal failures, cancellations and misses are not living: their media
    /// was never posted and nothing will ever reuse that scope.
    pub fn live_publish_media_scopes(
        &self,
        udid: &str,
    ) -> anyhow::Result<std::collections::BTreeSet<String>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            "SELECT a.campaign_id, a.ordinal FROM publish_assignments a
             WHERE a.udid=?1 AND (
               a.state NOT IN ('succeeded','failed_before_dispatch','cancelled','missed')
               OR (a.state='succeeded'
                   AND COALESCE(json_extract(a.evidence_json,'$.cleanup.state'),'')<>'cleaned'
                   AND COALESCE(json_extract(a.evidence_json,'$.cleanup.value.state'),'')<>'cleaned')
             )",
        )?;
        let rows = statement.query_map([udid], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        let mut scopes = std::collections::BTreeSet::new();
        for row in rows {
            let (campaign_id, ordinal) = row?;
            scopes.insert(publish_device_scope(
                &campaign_id,
                u32::try_from(ordinal).unwrap_or(u32::MAX),
            ));
        }
        Ok(scopes)
    }

    /// Record the intent to delete one directory. The row exists before the phone is
    /// touched, so a crash between the two leaves a `claimed` row rather than silence.
    pub fn claim_publish_media_sweep(
        &self,
        udid: &str,
        kind: PublishMediaSweepKind,
        name: &str,
    ) -> anyhow::Result<PublishMediaSweepClaim> {
        let id = Uuid::new_v4().to_string();
        self.conn()?.execute(
            "INSERT INTO publish_media_sweeps(id,udid,kind,name,state,claimed_at_ms)
             VALUES(?1,?2,?3,?4,'claimed',?5)",
            params![id, udid, kind.as_str(), name, Utc::now().timestamp_millis()],
        )?;
        Ok(PublishMediaSweepClaim {
            id,
            udid: udid.into(),
            kind,
            name: name.into(),
        })
    }

    /// Record what the phone said back. `cleaned` only when the backend read back that the
    /// directory is gone; everything else is `failed` and backs the name off for a day.
    pub fn finish_publish_media_sweep(
        &self,
        claim: &PublishMediaSweepClaim,
        cleaned: bool,
        proof: &serde_json::Value,
    ) -> anyhow::Result<bool> {
        let changed = self.conn()?.execute(
            "UPDATE publish_media_sweeps SET state=?1,finished_at_ms=?2,proof_json=?3
             WHERE id=?4 AND state='claimed'",
            params![
                if cleaned { "cleaned" } else { "failed" },
                Utc::now().timestamp_millis(),
                proof.to_string(),
                claim.id
            ],
        )?;
        Ok(changed == 1)
    }

    /// Names this phone's sweep failed on (or crashed inside) since `since_ms`. The sweep
    /// skips them so a directory the phone refuses to delete is retried once a day, not
    /// every visit.
    pub fn publish_media_sweep_backoff(
        &self,
        udid: &str,
        since_ms: i64,
    ) -> anyhow::Result<std::collections::BTreeSet<String>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            "SELECT DISTINCT name FROM publish_media_sweeps
             WHERE udid=?1 AND state IN ('claimed','failed') AND claimed_at_ms>=?2",
        )?;
        let rows = statement.query_map(params![udid, since_ms], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<_, _>>().map_err(Into::into)
    }

    /// Campaigns whose trace folders must survive retention: an assignment is still
    /// posting, verifying or uncertain, or its verification was handed to review.
    pub fn publish_campaigns_with_open_outcomes(&self) -> anyhow::Result<Vec<String>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            "SELECT DISTINCT campaign_id FROM publish_assignments
             WHERE state IN ('posting','verifying','uncertain')
                OR json_extract(evidence_json,'$.verificationStatus.state')='needsReview'",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<_, _>>().map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PublishCampaignState as State, PublishCleanupPolicy as Policy};

    fn seed(
        db: &Database,
        policy: Policy,
        state: State,
        evidence: Option<&serde_json::Value>,
    ) -> (String, u32) {
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
            sheet_delivery: None,
            verification_contract_version: Some(1),
            verification_builds: vec![],
            sheet_enabled: false,
            request_id: id.clone(),
            source_root: "C:/fixture".into(),
            bundle_ids: vec![id],
            udids: vec!["phone".into()],
            run_at: None,
            visibility: crate::PublishVisibility::Public,
            cleanup_policy: policy,
            network: crate::SocialNetwork::TikTok,
            sound_policy: crate::PublishSoundPolicy::Default,
            execution_confirmed: true,
            target_snapshot: None,
            approval: None,
        };
        let campaign = db.create_publish_campaign(&request, &[bundle]).unwrap();
        let assignment = db
            .get_publish_campaign(&campaign.id)
            .unwrap()
            .unwrap()
            .assignments[0]
            .clone();
        let evidence = evidence.map(|value| value.to_string());
        db.update_publish_assignment_state(&assignment.id, state, None, evidence.as_deref())
            .unwrap();
        (campaign.id, assignment.ordinal)
    }

    fn delete() -> Policy {
        Policy::DeleteImportedAssetsAfterVerified
    }

    fn open() -> (Database, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("media-sweep-{}.db", Uuid::new_v4()));
        (Database::open(&path).unwrap(), path)
    }

    #[test]
    fn album_scope_follows_the_driver_layout_and_refuses_other_shapes() {
        assert_eq!(
            publish_album_scope("riviu-a62c6b72-de17-41c2-885f-4c586f835bed-28-915e4170286a"),
            Some("a62c6b72-de17-41c2-885f-4c586f835bed-28")
        );
        for other in [
            "riviufarm-shot",
            "riviu-aaaaaaaaaaaa",
            "riviu-x-nothex",
            "Camera",
        ] {
            assert_eq!(publish_album_scope(other), None, "{other}");
        }
    }

    #[test]
    fn terminal_failures_free_their_scope_and_everything_else_keeps_it() {
        let (db, path) = open();
        let cleaned = serde_json::json!({"post":{"importId":"riviu-x-aaaaaaaaaaaa"},"cleanup":{"state":"cleaned","importId":"riviu-x-aaaaaaaaaaaa"}});
        let nested = serde_json::json!({"cleanup":{"state":"kept","value":{"state":"cleaned"}}});
        let kept =
            serde_json::json!({"cleanup":{"state":"kept","reason":"post_verification_pending"}});
        let cases = [
            (delete(), State::FailedBeforeDispatch, None, false),
            (delete(), State::Cancelled, None, false),
            (delete(), State::Missed, None, false),
            (delete(), State::Succeeded, Some(&cleaned), false),
            (delete(), State::Succeeded, Some(&nested), false),
            (delete(), State::Succeeded, Some(&kept), true),
            (Policy::KeepImportedAssets, State::Succeeded, None, true),
            (delete(), State::Verifying, Some(&kept), true),
            (delete(), State::Uncertain, Some(&kept), true),
            (delete(), State::Posting, None, true),
            (delete(), State::Queued, None, true),
            (delete(), State::Scheduled, None, true),
            (delete(), State::Imported, None, true),
        ];
        let mut expected = std::collections::BTreeSet::new();
        for (policy, state, evidence, live) in cases {
            let (campaign, ordinal) = seed(&db, policy, state, evidence);
            let scope = publish_device_scope(&campaign, ordinal);
            if live {
                expected.insert(scope);
            }
        }
        assert_eq!(db.live_publish_media_scopes("phone").unwrap(), expected);
        assert!(db
            .live_publish_media_scopes("other-phone")
            .unwrap()
            .is_empty());
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sweep_rows_record_intent_then_receipt_and_back_off_failures() {
        let (db, path) = open();
        let claim = db
            .claim_publish_media_sweep(
                "phone",
                PublishMediaSweepKind::Album,
                "riviu-x-aaaaaaaaaaaa",
            )
            .unwrap();
        let since = Utc::now().timestamp_millis() - 1_000;
        // A claimed row with no receipt is a crash in flight: backed off like a failure.
        assert!(db
            .publish_media_sweep_backoff("phone", since)
            .unwrap()
            .contains("riviu-x-aaaaaaaaaaaa"));
        assert!(db
            .finish_publish_media_sweep(&claim, true, &serde_json::json!({"state":"cleaned"}))
            .unwrap());
        assert!(!db
            .finish_publish_media_sweep(&claim, true, &serde_json::json!({}))
            .unwrap());
        assert!(db
            .publish_media_sweep_backoff("phone", since)
            .unwrap()
            .is_empty());
        let failed = db
            .claim_publish_media_sweep("phone", PublishMediaSweepKind::Stage, "scope-1")
            .unwrap();
        assert!(db
            .finish_publish_media_sweep(&failed, false, &serde_json::json!({"error":"x"}))
            .unwrap());
        assert_eq!(
            db.publish_media_sweep_backoff("phone", since)
                .unwrap()
                .into_iter()
                .collect::<Vec<_>>(),
            vec!["scope-1".to_string()]
        );
        assert!(db
            .publish_media_sweep_backoff("other", since)
            .unwrap()
            .is_empty());
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn open_outcomes_protect_their_campaign_traces() {
        let (db, path) = open();
        let review = serde_json::json!({"verificationStatus":{"state":"needsReview"}});
        let (verifying, _) = seed(&db, delete(), State::Verifying, None);
        let (uncertain, _) = seed(&db, delete(), State::Uncertain, None);
        let (reviewed, _) = seed(&db, delete(), State::Succeeded, Some(&review));
        let (_done, _) = seed(&db, delete(), State::Succeeded, None);
        let (_failed, _) = seed(&db, delete(), State::FailedBeforeDispatch, None);
        let mut open = db.publish_campaigns_with_open_outcomes().unwrap();
        open.sort();
        let mut expected = vec![verifying, uncertain, reviewed];
        expected.sort();
        assert_eq!(open, expected);
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}
