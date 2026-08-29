//! Publish campaigns and the analytics rollup over them.

use super::*;

impl Database {
    pub fn create_publish_campaign(
        &self,
        request: &crate::publish::PublishCampaignRequest,
        bundles: &[crate::publish::PublishBundle],
    ) -> anyhow::Result<crate::publish::PublishCampaignRecord> {
        let assignments =
            crate::publish::validate_publish_mapping(&request.bundle_ids, &request.udids)
                .map_err(|error| anyhow::anyhow!(error))?;
        if bundles.len() != request.bundle_ids.len()
            || bundles
                .iter()
                .zip(&request.bundle_ids)
                .any(|(bundle, id)| bundle.id != *id)
        {
            anyhow::bail!("selected bundle manifest does not match the campaign request");
        }

        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let campaign_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let state = if request.run_at.is_some() {
            crate::publish::PublishCampaignState::Scheduled
        } else {
            crate::publish::PublishCampaignState::Queued
        };
        let request_json = serde_json::to_string(request)?;
        transaction.execute(
            "INSERT INTO publish_campaigns
             (id, request_id, source_root, request_json, state, run_at, revision, error_code, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,0,NULL,?7,?7)",
            params![
                campaign_id,
                request.request_id,
                request.source_root,
                request_json,
                state.as_str(),
                request.run_at,
                now,
            ],
        )?;

        for (ordinal, bundle) in bundles.iter().enumerate() {
            transaction.execute(
                "INSERT INTO publish_bundles
                 (id,campaign_id,ordinal,name,source_path,caption,caption_sha256,manifest_json,created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    bundle.id,
                    campaign_id,
                    ordinal as i64,
                    bundle.name,
                    bundle.source_path,
                    bundle.caption,
                    bundle.caption_sha256,
                    serde_json::to_string(bundle)?,
                    now,
                ],
            )?;
        }

        for plan in &assignments {
            transaction.execute(
                "INSERT INTO publish_assignments
                 (id,campaign_id,bundle_id,ordinal,udid,state,effect_intent,evidence_json,error_code,revision,created_at,updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,NULL,NULL,NULL,0,?7,?7)",
                params![
                    Uuid::new_v4().to_string(),
                    campaign_id,
                    plan.bundle_id,
                    plan.ordinal as i64,
                    plan.udid,
                    state.as_str(),
                    now,
                ],
            )?;
        }
        transaction.execute(
            "INSERT INTO publish_events(campaign_id,revision,kind,payload_json,created_at) VALUES (?1,1,'created',?2,?3)",
            params![campaign_id, request_json, now],
        )?;
        transaction.execute(
            "UPDATE publish_campaigns SET revision=1 WHERE id=?1",
            params![campaign_id],
        )?;
        transaction.commit()?;

        Ok(crate::publish::PublishCampaignRecord {
            id: campaign_id,
            request_id: request.request_id.clone(),
            source_root: request.source_root.clone(),
            state,
            run_at: request.run_at.clone(),
            visibility: request.visibility.clone(),
            cleanup_policy: request.cleanup_policy.clone(),
            assignments,
            created_at: now.clone(),
            updated_at: now,
            error_code: None,
        })
    }
    pub fn list_publish_campaigns(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<crate::publish::PublishCampaignRecord>> {
        let conn = self.conn()?;
        let ids = {
            let mut stmt =
                conn.prepare("SELECT id FROM publish_campaigns ORDER BY created_at DESC LIMIT ?1")?;
            let rows = stmt.query_map(params![limit as i64], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        drop(conn);
        let mut campaigns = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(detail) = self.get_publish_campaign(&id)? {
                campaigns.push(detail.campaign);
            }
        }
        Ok(campaigns)
    }
    pub fn get_publish_campaign(
        &self,
        id: &str,
    ) -> anyhow::Result<Option<crate::publish::PublishCampaignDetail>> {
        let conn = self.conn()?;
        let Some((campaign, request)) = conn
            .query_row(
                "SELECT id,request_id,source_root,state,run_at,request_json,created_at,updated_at,error_code
                 FROM publish_campaigns WHERE id=?1",
                params![id],
                |row| {
                    let request_json: String = row.get(5)?;
                    let request: crate::publish::PublishCampaignRequest =
                        serde_json::from_str(&request_json).map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                5,
                                rusqlite::types::Type::Text,
                                Box::new(error),
                            )
                        })?;
                    Ok((
                        crate::publish::PublishCampaignRecord {
                            id: row.get(0)?,
                            request_id: row.get(1)?,
                            source_root: row.get(2)?,
                            state: publish_state_from_str(&row.get::<_, String>(3)?),
                            run_at: row.get(4)?,
                            visibility: request.visibility.clone(),
                            cleanup_policy: request.cleanup_policy.clone(),
                            assignments: Vec::new(),
                            created_at: row.get(6)?,
                            updated_at: row.get(7)?,
                            error_code: row.get(8)?,
                        },
                        request,
                    ))
                },
            )
            .optional()?
        else {
            return Ok(None);
        };

        let mut bundle_stmt = conn.prepare(
            "SELECT manifest_json FROM publish_bundles WHERE campaign_id=?1 ORDER BY ordinal",
        )?;
        let bundles = bundle_stmt
            .query_map(params![id], |row| {
                let json: String = row.get(0)?;
                serde_json::from_str(&json).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })
            })?
            .collect::<Result<Vec<crate::publish::PublishBundle>, _>>()?;

        let mut assignment_stmt = conn.prepare(
            "SELECT id,bundle_id,ordinal,udid,state,effect_intent,evidence_json,error_code
             FROM publish_assignments WHERE campaign_id=?1 ORDER BY ordinal",
        )?;
        let assignments = assignment_stmt
            .query_map(params![id], |row| {
                Ok(crate::publish::PublishAssignmentRecord {
                    id: row.get(0)?,
                    campaign_id: id.to_string(),
                    bundle_id: row.get(1)?,
                    ordinal: narrow(row.get::<_, i64>(2)?, "ordinal")?,
                    udid: row.get(3)?,
                    state: publish_state_from_str(&row.get::<_, String>(4)?),
                    effect_intent: row.get(5)?,
                    evidence_json: row.get(6)?,
                    error_code: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut event_stmt = conn.prepare(
            "SELECT revision,kind,payload_json,created_at FROM publish_events WHERE campaign_id=?1 ORDER BY revision",
        )?;
        let events = event_stmt
            .query_map(params![id], |row| {
                Ok(crate::publish::PublishEventRecord {
                    revision: row.get::<_, i64>(0)? as u64,
                    kind: row.get(1)?,
                    payload_json: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut campaign = campaign;
        campaign.assignments = request
            .bundle_ids
            .iter()
            .zip(&request.udids)
            .enumerate()
            .map(
                |(ordinal, (bundle_id, udid))| crate::publish::PublishAssignmentPlan {
                    bundle_id: bundle_id.clone(),
                    udid: udid.clone(),
                    // Not a `narrow`: this is `enumerate()` over a list the caller just
                    // built, not a value read back out of a column. There is no stored
                    // number here that could disagree with the type.
                    ordinal: ordinal as u32,
                },
            )
            .collect();
        Ok(Some(crate::publish::PublishCampaignDetail {
            campaign,
            bundles,
            assignments,
            events,
        }))
    }
    pub fn update_publish_campaign_state(
        &self,
        id: &str,
        state: crate::publish::PublishCampaignState,
        error_code: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current_revision: i64 = transaction.query_row(
            "SELECT revision FROM publish_campaigns WHERE id=?1",
            params![id],
            |row| row.get(0),
        )?;
        let revision = current_revision + 1;
        let now = Utc::now().to_rfc3339();
        let payload = serde_json::json!({"state": state.as_str(), "errorCode": error_code});
        transaction.execute(
            "UPDATE publish_campaigns SET state=?1,error_code=?2,revision=?3,updated_at=?4 WHERE id=?5",
            params![state.as_str(), error_code, revision, now, id],
        )?;
        transaction.execute(
            "INSERT INTO publish_events(campaign_id,revision,kind,payload_json,created_at) VALUES (?1,?2,'state',?3,?4)",
            params![id, revision, payload.to_string(), now],
        )?;
        transaction.commit()?;
        Ok(())
    }
    /// Move a campaign into `Posting`, and answer whether **this** caller is the one that
    /// moved it.
    ///
    /// **A publish run needs an owner, and it had none.** `post_publish_campaign_inner` read
    /// the campaign, accepted `Imported`/`Posting`/`Verifying`, then called
    /// `update_publish_campaign_state(.., Posting, ..)` unconditionally. Two commands could
    /// both read `Imported`, both write `Posting`, and both walk the same stale assignment
    /// list. The exclusive device context serialises *access to a phone*, so the second one
    /// simply waited and then posted the same bundle to the same account again -- and the row
    /// records only the last success, so nothing afterwards shows that it happened twice.
    ///
    /// A carousel post is not undoable, so this is the shape that matters more than any
    /// throughput concern: exactly one caller may claim a campaign.
    ///
    /// `Posting` is **not** an accepted starting state here. It used to be, to let a caller
    /// resume an interrupted run, but "resume" and "a second caller arriving" are
    /// indistinguishable from inside this function. A stuck campaign is recovered by an
    /// explicit operator action, not by a second post racing the first.
    ///
    /// Migration 16 dropped `publish_dispatch`, which was shaped like a claim table nothing
    /// ever claimed. This is the claim, done with a conditional `UPDATE` instead of a table.
    ///
    /// Found by an independent review on 27/08/2026.
    /// Settle every campaign the process died in the middle of.
    ///
    /// Called once at startup, **before** commands are accepted, exactly like
    /// `interrupt_orphaned_interaction_campaigns`. Without it a crash during a post leaves rows
    /// reading `posting` forever: nothing re-enters them, nothing cleans up the media already
    /// on the phone, and the operator's only signal is a campaign that never finishes.
    ///
    /// # The states this touches, and the ones it must not
    ///
    /// Only `preparing`, `transferring`, `posting`, `verifying` are mid-flight. `queued`,
    /// `scheduled`, `ready` and `imported` are **at rest** and waiting for someone: the
    /// operator's Prepare, the `run_at` the scheduler will pick up next launch, Transfer, Post.
    /// Interaction's `queued` means "a worker is about to take this", which is a different
    /// thing — copying its state list here would cancel every campaign the operator had lined
    /// up.
    ///
    /// # Why the two assignment updates differ, and why that asymmetry is the safety
    ///
    /// An assignment that was `posting` or `verifying` may have reached TikTok. Nobody can tell
    /// from here, so it becomes **`uncertain`** — which
    /// [`Self::claim_publish_assignment_for_posting`] deliberately refuses to claim, making the
    /// row permanently unclaimable. That is correct: re-posting would publish a second carousel
    /// to a real account, and there is no delete path on Android to undo it.
    ///
    /// An assignment that was `transferring` never reached TikTok — the media had not finished
    /// leaving the desktop — so it becomes **`failed_before_dispatch`**, which *is* claimable.
    /// The name is the guarantee.
    ///
    /// `imported` assignments are untouched: the crash cost them nothing and they are still
    /// exactly where Post expects to find them.
    pub fn interrupt_orphaned_publish_campaigns(&self) -> anyhow::Result<usize> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = Utc::now().to_rfc3339();
        let stranded: Vec<String> = transaction
            .prepare(
                "SELECT id FROM publish_campaigns
                 WHERE state IN ('preparing','transferring','posting','verifying')",
            )?
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        for campaign_id in &stranded {
            transaction.execute(
                "UPDATE publish_assignments
                 SET state='uncertain',revision=revision+1,updated_at=?2,
                     error_code=COALESCE(error_code,'publish_worker_lost: app đóng khi bài này đang đăng — không xác nhận được là đã lên hay chưa, nên không đăng lại')
                 WHERE campaign_id=?1 AND state IN ('posting','verifying')",
                params![campaign_id, now],
            )?;
            transaction.execute(
                "UPDATE publish_assignments
                 SET state='failed_before_dispatch',revision=revision+1,updated_at=?2,
                     error_code=COALESCE(error_code,'publish_worker_lost: app đóng trước khi ảnh rời máy tính — vẫn đăng lại được')
                 WHERE campaign_id=?1 AND state='transferring'",
                params![campaign_id, now],
            )?;
            transaction.execute(
                "UPDATE publish_campaigns
                 SET state='cancelled',revision=revision+1,updated_at=?2,
                     error_code=COALESCE(error_code,'publish_worker_lost: app đóng khi chiến dịch đang chạy')
                 WHERE id=?1",
                params![campaign_id, now],
            )?;
        }
        transaction.commit()?;
        Ok(stranded.len())
    }

    pub fn claim_publish_campaign_for_posting(&self, id: &str) -> anyhow::Result<bool> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current_revision: i64 = transaction.query_row(
            "SELECT revision FROM publish_campaigns WHERE id=?1",
            params![id],
            |row| row.get(0),
        )?;
        let revision = current_revision + 1;
        let now = Utc::now().to_rfc3339();
        let posting = crate::publish::PublishCampaignState::Posting;
        // The predicate is the claim: only a campaign still sitting at `Imported` moves, and
        // SQLite does this row update atomically inside an IMMEDIATE transaction, so exactly
        // one of two racing callers sees a non-zero count.
        let claimed = transaction.execute(
            "UPDATE publish_campaigns SET state=?1,error_code=NULL,revision=?2,updated_at=?3 \
             WHERE id=?4 AND state=?5",
            params![
                posting.as_str(),
                revision,
                now,
                id,
                crate::publish::PublishCampaignState::Imported.as_str()
            ],
        )?;
        if claimed == 0 {
            transaction.rollback()?;
            return Ok(false);
        }
        let payload = serde_json::json!({"state": posting.as_str(), "errorCode": null});
        transaction.execute(
            "INSERT INTO publish_events(campaign_id,revision,kind,payload_json,created_at) VALUES (?1,?2,'state',?3,?4)",
            params![id, revision, payload.to_string(), now],
        )?;
        transaction.commit()?;
        Ok(true)
    }
    /// Move one assignment into `Posting`, and answer whether it was this caller's to move.
    ///
    /// The same rule one level down, and it is the level that stops the worst case: an
    /// assignment already `Succeeded` must never be walked back to `Posting` and posted a
    /// second time.
    ///
    /// Exactly two states are claimable, and the exclusions matter more than the inclusions:
    ///
    /// * `Imported` -- never attempted.
    /// * `FailedBeforeDispatch` -- the name is the guarantee: it failed *before* anything
    ///   reached the phone, so retrying cannot duplicate anything.
    ///
    /// **`Uncertain` is deliberately not claimable.** It means the post may have landed and
    /// the evidence did not come back -- which is precisely the case where re-posting would
    /// publish a second carousel to a real account. An uncertain assignment needs a human to
    /// look at the phone, not an automatic retry. `Posting` is excluded because it is in
    /// flight, and `Succeeded`/`Cancelled` because they are terminal.
    pub fn claim_publish_assignment_for_posting(
        &self,
        assignment_id: &str,
        intent_json: &str,
    ) -> anyhow::Result<bool> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE publish_assignments SET state=?1,error_code=NULL,evidence_json=?2,\
             revision=revision+1,updated_at=?3 WHERE id=?4 AND state IN (?5,?6)",
            params![
                crate::publish::PublishCampaignState::Posting.as_str(),
                intent_json,
                Utc::now().to_rfc3339(),
                assignment_id,
                crate::publish::PublishCampaignState::Imported.as_str(),
                crate::publish::PublishCampaignState::FailedBeforeDispatch.as_str(),
            ],
        )?;
        Ok(changed > 0)
    }
    pub fn update_publish_assignment_state(
        &self,
        assignment_id: &str,
        state: crate::publish::PublishCampaignState,
        error_code: Option<&str>,
        evidence_json: Option<&str>,
    ) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE publish_assignments SET state=?1,error_code=?2,evidence_json=?3,revision=revision+1,updated_at=?4 WHERE id=?5",
            params![state.as_str(), error_code, evidence_json, Utc::now().to_rfc3339(), assignment_id],
        )?;
        Ok(())
    }
    /// The campaign's own state, without loading its bundles and assignments.
    ///
    /// A light read because the one caller is inside the post loop, checking between phones
    /// whether the operator has cancelled. `get_publish_campaign` pulls the whole manifest —
    /// every bundle, every image, every assignment — which is a lot of work to ask for once
    /// per phone to read one column.
    pub fn publish_campaign_state(
        &self,
        id: &str,
    ) -> anyhow::Result<Option<crate::publish::PublishCampaignState>> {
        let conn = self.conn()?;
        let state: Option<String> = conn
            .query_row(
                "SELECT state FROM publish_campaigns WHERE id=?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(state.as_deref().map(publish_state_from_str))
    }

    /// Settle a finished run — **without overwriting a cancel that landed while it ran.**
    ///
    /// The bug this closes: the post loop ended with an unconditional write of `Succeeded`
    /// or `Uncertain`, so an operator who pressed Cancel mid-run watched the campaign come
    /// back reading `succeeded`. Their cancel was recorded, honoured for the phones that had
    /// not started, and then erased by the last statement of the run.
    ///
    /// So the write is conditional **in SQL**, on the campaign still being `Posting` — the
    /// state the claim put it in. Guarding in Rust instead would mean re-reading and then
    /// writing, with the cancel free to land in between.
    ///
    /// Returns the state the campaign actually ended in, which is not always the one asked
    /// for; the caller reports that rather than assuming.
    pub fn finish_publish_campaign(
        &self,
        id: &str,
        had_failures: bool,
    ) -> anyhow::Result<Option<crate::publish::PublishCampaignState>> {
        let wanted = if had_failures {
            crate::publish::PublishCampaignState::Uncertain
        } else {
            crate::publish::PublishCampaignState::Succeeded
        };
        let conn = self.conn()?;
        conn.execute(
            "UPDATE publish_campaigns
             SET state=?1,error_code=?2,revision=revision+1,updated_at=?3
             WHERE id=?4 AND state=?5",
            params![
                wanted.as_str(),
                had_failures.then_some("post_or_cleanup_failed"),
                Utc::now().to_rfc3339(),
                id,
                crate::publish::PublishCampaignState::Posting.as_str(),
            ],
        )?;
        self.publish_campaign_state(id)
    }

    pub fn cancel_publish_campaign(&self, id: &str) -> anyhow::Result<()> {
        self.update_publish_campaign_state(
            id,
            crate::publish::PublishCampaignState::Cancelled,
            None,
        )
    }
    pub fn analytics_summary(
        &self,
        device_total: usize,
        device_ready: usize,
    ) -> anyhow::Result<crate::types::AnalyticsSummary> {
        let jobs = self.list_jobs(500)?;
        let scripts = self.list_scripts()?;
        let materials = self.list_materials()?;
        let apps = self.list_apps_library()?;
        let schedules = self.list_schedules()?;
        Ok(crate::types::AnalyticsSummary {
            device_total,
            device_ready,
            jobs_total: jobs.len(),
            jobs_succeeded: jobs
                .iter()
                .filter(|j| matches!(j.status, JobStatus::Succeeded))
                .count(),
            jobs_failed: jobs
                .iter()
                .filter(|j| matches!(j.status, JobStatus::Failed))
                .count(),
            jobs_running: jobs
                .iter()
                .filter(|j| matches!(j.status, JobStatus::Running | JobStatus::Queued))
                .count(),
            scripts_total: scripts.len(),
            materials_total: materials.len(),
            apps_total: apps.len(),
            schedules_enabled: schedules.iter().filter(|s| s.enabled).count(),
            recent_logs: self.list_op_logs(20)?,
        })
    }
}
#[cfg(test)]
mod claim_tests {
    use super::*;

    fn fixture() -> (Database, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("riviu-publish-claim-{}.db", Uuid::new_v4()));
        (Database::open(&path).expect("open fixture database"), path)
    }

    fn bundle(id: &str) -> crate::publish::PublishBundle {
        crate::publish::PublishBundle {
            id: id.to_string(),
            source_path: format!("C:/fixture/{id}"),
            name: id.to_string(),
            media_kind: crate::publish::PublishMediaKind::Image,
            images: Vec::new(),
            caption_path: format!("C:/fixture/{id}/caption.txt"),
            caption: "caption".into(),
            caption_sha256: "0".repeat(64),
            total_bytes: 1,
        }
    }

    fn campaign(db: &Database, udids: &[&str]) -> crate::publish::PublishCampaignRecord {
        let bundle_ids: Vec<String> = udids.iter().map(|udid| format!("b-{udid}")).collect();
        let bundles: Vec<crate::publish::PublishBundle> =
            bundle_ids.iter().map(|id| bundle(id)).collect();
        let request = crate::publish::PublishCampaignRequest {
            request_id: Uuid::new_v4().to_string(),
            source_root: "C:/fixture".into(),
            bundle_ids,
            udids: udids.iter().map(|udid| udid.to_string()).collect(),
            run_at: None,
            visibility: crate::publish::PublishVisibility::Public,
            cleanup_policy: crate::publish::PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
        };
        let record = db
            .create_publish_campaign(&request, &bundles)
            .expect("create the campaign");
        // Creation lands at `Queued`; the transfer command is what moves a campaign to
        // `Imported`, and posting is only reachable from there. Doing it here rather than
        // claiming from `Queued` keeps the fixture on the real precondition -- the first
        // draft of this test skipped the step and failed, which is the test being wrong
        // about the state machine rather than the claim being too strict.
        db.update_publish_campaign_state(
            &record.id,
            crate::publish::PublishCampaignState::Imported,
            None,
        )
        .expect("the transfer step marks the campaign imported");
        for assignment in db
            .get_publish_campaign(&record.id)
            .expect("read back")
            .expect("campaign exists")
            .assignments
        {
            db.update_publish_assignment_state(
                &assignment.id,
                crate::publish::PublishCampaignState::Imported,
                None,
                None,
            )
            .expect("assignments are imported with their campaign");
        }
        record
    }

    const INTENT: &str = r#"{"effectIntent":"post_carousel"}"#;

    /// Read one assignment's state back, by udid.
    fn state_of(
        db: &Database,
        campaign_id: &str,
        udid: &str,
    ) -> crate::publish::PublishCampaignState {
        db.get_publish_campaign(campaign_id)
            .expect("read back")
            .expect("campaign exists")
            .assignments
            .into_iter()
            .find(|assignment| assignment.udid == udid)
            .expect("assignment for udid")
            .state
    }

    /// **A crash mid-post must strand nothing, and the two halves must land differently.**
    ///
    /// The row that was `posting` may already be a carousel on a real account — nobody can
    /// tell from here — so it becomes `uncertain`, which the claim refuses. The row that was
    /// `transferring` never reached TikTok, so it becomes `failed_before_dispatch`, which the
    /// claim accepts. Collapsing the two into one state would either strand a retryable post
    /// or re-publish a live one, and there is no delete path on Android to undo the second.
    #[test]
    fn a_crash_mid_post_leaves_one_half_retryable_and_the_other_permanently_not() {
        let (db, path) = fixture();
        let record = campaign(
            &db,
            &["phone-posting", "phone-transferring", "phone-imported"],
        );

        db.update_publish_campaign_state(
            &record.id,
            crate::publish::PublishCampaignState::Posting,
            None,
        )
        .expect("the campaign was mid-post when the app died");
        let assignments = db
            .get_publish_campaign(&record.id)
            .expect("read back")
            .expect("campaign exists")
            .assignments;
        for assignment in &assignments {
            let state = match assignment.udid.as_str() {
                "phone-posting" => crate::publish::PublishCampaignState::Posting,
                "phone-transferring" => crate::publish::PublishCampaignState::Transferring,
                // The third stays `Imported`: the crash cost it nothing.
                _ => continue,
            };
            db.update_publish_assignment_state(&assignment.id, state, None, None)
                .expect("seed the mid-flight state");
        }

        let settled = db
            .interrupt_orphaned_publish_campaigns()
            .expect("settle the stranded campaign");
        assert_eq!(settled, 1);

        assert_eq!(
            state_of(&db, &record.id, "phone-posting"),
            crate::publish::PublishCampaignState::Uncertain,
            "a post that may have gone out must never be re-dispatched"
        );
        assert_eq!(
            state_of(&db, &record.id, "phone-transferring"),
            crate::publish::PublishCampaignState::FailedBeforeDispatch,
            "media that never left the desktop is safe to send again"
        );
        assert_eq!(
            state_of(&db, &record.id, "phone-imported"),
            crate::publish::PublishCampaignState::Imported,
            "an assignment the crash did not touch is left exactly where Post expects it"
        );

        // And the claim agrees with the states, which is the point of choosing them.
        let by_udid = |udid: &str| {
            db.get_publish_campaign(&record.id)
                .expect("read back")
                .expect("campaign exists")
                .assignments
                .into_iter()
                .find(|assignment| assignment.udid == udid)
                .expect("assignment")
                .id
        };
        assert!(
            !db.claim_publish_assignment_for_posting(&by_udid("phone-posting"), INTENT)
                .expect("claim query"),
            "uncertain is permanently unclaimable"
        );
        assert!(
            db.claim_publish_assignment_for_posting(&by_udid("phone-transferring"), INTENT)
                .expect("claim query"),
            "failed_before_dispatch is what makes a retry possible"
        );
        let _ = std::fs::remove_file(path);
    }

    /// A campaign at rest is not stranded, and settling it would cancel the operator's plan.
    #[test]
    fn a_campaign_waiting_for_the_operator_is_left_alone() {
        let (db, path) = fixture();
        // `campaign()` leaves it `Imported`, i.e. waiting for Post.
        let waiting = campaign(&db, &["phone-a"]);
        assert_eq!(
            db.interrupt_orphaned_publish_campaigns()
                .expect("nothing to settle"),
            0
        );
        assert_eq!(
            db.get_publish_campaign(&waiting.id)
                .expect("read back")
                .expect("campaign exists")
                .campaign
                .state,
            crate::publish::PublishCampaignState::Imported
        );
        let _ = std::fs::remove_file(path);
    }

    /// **Only one caller may start posting a campaign.**
    ///
    /// The bug this pins: `post_publish_campaign_inner` read the campaign, accepted
    /// `Imported`, and wrote `Posting` unconditionally. Two commands could both read
    /// `Imported`, both write, and both walk the same assignment list — serialised on the
    /// device lease, so the second simply waited its turn and posted the same carousel to the
    /// same account again. A post cannot be taken back, and the row keeps only the last
    /// success, so nothing afterwards showed it had happened twice.
    #[test]
    fn only_the_first_caller_claims_a_campaign_for_posting() {
        let (db, path) = fixture();
        let record = campaign(&db, &["phone-a", "phone-b"]);

        assert!(
            db.claim_publish_campaign_for_posting(&record.id)
                .expect("first claim runs"),
            "a campaign sitting at Imported must be claimable"
        );
        assert!(
            !db.claim_publish_campaign_for_posting(&record.id)
                .expect("second claim runs"),
            "the second caller must be refused -- this is the duplicate post"
        );

        let detail = db
            .get_publish_campaign(&record.id)
            .expect("read back")
            .expect("campaign exists");
        assert_eq!(
            detail.campaign.state,
            crate::publish::PublishCampaignState::Posting
        );
        let _ = std::fs::remove_file(path);
    }

    /// An assignment that already succeeded is never posted again.
    ///
    /// And neither is an `Uncertain` one, which is the sharper half: uncertain means the post
    /// *may* have landed with the evidence lost. Retrying that is how one account gets two
    /// carousels. Only `Imported` and `FailedBeforeDispatch` — a name that promises nothing
    /// reached the phone — are claimable.
    #[test]
    fn an_assignment_is_claimable_only_when_nothing_reached_the_phone() {
        let (db, path) = fixture();
        let record = campaign(&db, &["phone-a"]);
        let detail = db
            .get_publish_campaign(&record.id)
            .expect("read back")
            .expect("campaign exists");
        let assignment = detail
            .assignments
            .first()
            .expect("one assignment")
            .id
            .clone();
        let intent = "{\"effectIntent\":\"post_carousel\"}";

        assert!(
            db.claim_publish_assignment_for_posting(&assignment, intent)
                .expect("claim runs"),
            "Imported is claimable"
        );
        assert!(
            !db.claim_publish_assignment_for_posting(&assignment, intent)
                .expect("claim runs"),
            "Posting is in flight and must not be re-claimed"
        );

        for (state, claimable) in [
            (crate::publish::PublishCampaignState::Succeeded, false),
            (crate::publish::PublishCampaignState::Uncertain, false),
            (crate::publish::PublishCampaignState::Cancelled, false),
            (
                crate::publish::PublishCampaignState::FailedBeforeDispatch,
                true,
            ),
        ] {
            db.update_publish_assignment_state(&assignment, state.clone(), None, None)
                .expect("set the state under test");
            assert_eq!(
                db.claim_publish_assignment_for_posting(&assignment, intent)
                    .expect("claim runs"),
                claimable,
                "{state:?} should {} be claimable",
                if claimable { "" } else { "not" }
            );
        }
        let _ = std::fs::remove_file(path);
    }
}
