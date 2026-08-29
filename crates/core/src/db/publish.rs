//! Publish campaigns and the analytics rollup over them.

use super::*;

/// How a posting run ended, as far as the **campaign** is concerned.
///
/// Three answers, not two. The middle one is the whole point: a run where every phone refused
/// before opening anything published nothing, and a campaign that says so stays claimable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishRunOutcome {
    /// Every assignment posted.
    AllPosted,
    /// Some assignments failed, and **none of them may have published**.
    NothingPublished,
    /// At least one assignment may be live and could not be confirmed.
    SomethingMayBeLive,
}

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
            // **The campaign follows its worst child, and `cancelled` is not one of the
            // answers.** It used to be, and it stranded exactly the work this function
            // labels retryable: an assignment moved to `failed_before_dispatch` sits under a
            // campaign, the run starts by claiming the campaign, and a cancelled campaign is
            // terminal. The child said "retry me" and the parent said "there is nothing to
            // retry".
            //
            // So: `uncertain` when some phone may already have published — which needs a
            // person, not a retry — and `failed_before_dispatch` when nothing did, which the
            // claim accepts.
            let anything_may_be_live: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM publish_assignments
                 WHERE campaign_id=?1 AND state='uncertain'",
                params![campaign_id],
                |row| row.get(0),
            )?;
            let (state, reason) = if anything_may_be_live > 0 {
                (
                    "uncertain",
                    "publish_worker_lost: app đóng khi đang đăng — có máy không xác nhận được",
                )
            } else {
                (
                    "failed_before_dispatch",
                    "publish_worker_lost: app đóng trước khi có gì rời máy tính — chạy lại được",
                )
            };
            transaction.execute(
                "UPDATE publish_campaigns
                 SET state=?3,revision=revision+1,updated_at=?2,
                     error_code=COALESCE(error_code,?4)
                 WHERE id=?1",
                params![campaign_id, now, state, reason],
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
        // The predicate is the claim: SQLite does this row update atomically inside an
        // IMMEDIATE transaction, so exactly one of two racing callers sees a non-zero count.
        //
        // **Two states, matching [`Self::claim_publish_assignment_for_posting`] exactly.**
        // `Imported` alone was a real trap: startup recovery settles an interrupted campaign
        // to `FailedBeforeDispatch` — the name is the guarantee, nothing reached a phone —
        // and marks its assignments claimable, and then this refused to start the run. The
        // work was labelled retryable and was not reachable, which is worse than either
        // answer on its own, because the operator sees rows saying "retry me" and a button
        // that will not.
        //
        // `Uncertain` stays excluded here for the same reason it is excluded one level down:
        // a campaign lands there only when some phone may already have published, and that
        // needs a person looking at the phone rather than a second dispatch.
        let claimed = transaction.execute(
            "UPDATE publish_campaigns SET state=?1,error_code=NULL,revision=?2,updated_at=?3 \
             WHERE id=?4 AND state IN (?5,?6)",
            params![
                posting.as_str(),
                revision,
                now,
                id,
                crate::publish::PublishCampaignState::Imported.as_str(),
                crate::publish::PublishCampaignState::FailedBeforeDispatch.as_str()
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
        // **And the parent has to still be posting.** The campaign state and this claim used
        // to be two statements with a gap between them, and the gap is where a cancel lands: a
        // task read `Posting`, the operator cancelled, and the task then claimed its row and
        // opened a phone that published after the run was stopped. One statement closes it —
        // SQLite evaluates the `EXISTS` inside the same update.
        let changed = conn.execute(
            "UPDATE publish_assignments SET state=?1,error_code=NULL,evidence_json=?2,\
             revision=revision+1,updated_at=?3 WHERE id=?4 AND state IN (?5,?6) \
             AND EXISTS (SELECT 1 FROM publish_campaigns c \
                         WHERE c.id = publish_assignments.campaign_id AND c.state = ?7)",
            params![
                crate::publish::PublishCampaignState::Posting.as_str(),
                intent_json,
                Utc::now().to_rfc3339(),
                assignment_id,
                crate::publish::PublishCampaignState::Imported.as_str(),
                crate::publish::PublishCampaignState::FailedBeforeDispatch.as_str(),
                crate::publish::PublishCampaignState::Posting.as_str(),
            ],
        )?;
        Ok(changed > 0)
    }

    /// Move one campaign into `Transferring`, and answer whether it was this caller's to move.
    ///
    /// **Transfer had no claim at all**, and that was the hole underneath every other guard.
    /// `transfer_publish_campaign_inner` wrote `Transferring` unconditionally and then walked
    /// its assignments to `Imported` — so running Transfer a second time on a campaign that
    /// had already **succeeded** put every row back into the one state the posting claim
    /// accepts, and the next Post published the same carousels again. Every compare-and-swap
    /// downstream is sound only while nothing can manufacture a claimable state; this could.
    ///
    /// The states it accepts are the ones where nothing has reached a phone:
    ///
    /// * `Queued` / `Scheduled` — never started.
    /// * `Ready` — prepared, not transferred.
    /// * `FailedBeforeDispatch` — the name is the guarantee.
    ///
    /// `Imported` is excluded on purpose too: the media is already on the phones and the next
    /// step is Post, not another transfer.
    pub fn claim_publish_campaign_for_transfer(&self, id: &str) -> anyhow::Result<bool> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE publish_campaigns SET state=?1,error_code=NULL,revision=revision+1,\
             updated_at=?2 WHERE id=?3 AND state IN (?4,?5,?6,?7)",
            params![
                crate::publish::PublishCampaignState::Transferring.as_str(),
                Utc::now().to_rfc3339(),
                id,
                crate::publish::PublishCampaignState::Queued.as_str(),
                crate::publish::PublishCampaignState::Scheduled.as_str(),
                crate::publish::PublishCampaignState::Ready.as_str(),
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

    /// Source bundle ids that some campaign has already **spoken for**.
    ///
    /// The input to [`crate::publish::auto_assign_bundles`], and the reason it needs no cursor.
    ///
    /// # Two mistakes this used to make, and the second one made it answer nothing
    ///
    /// **It compared two different id namespaces.** Campaign creation namespaces every staged
    /// bundle as `"{request_id}:{source_bundle_id}"`, so that is what the assignment rows hold
    /// — while the auto-deal rescans the folder and offers the scanner's own
    /// `source_bundle_id`. `req-1:bo1-abc` never equals `bo1-abc`, so the pool excluded
    /// *nothing* and every press of the deal button could re-publish posts that had already
    /// gone out. The prefix is stripped here, which is exact: a request id is a UUID and
    /// carries no colon.
    ///
    /// **And it only counted assignments that had reached a phone.** Everything from `queued`
    /// to `imported` is a campaign the operator has already committed this bundle to — an
    /// `imported` one has its images sitting in a phone's gallery — so leaving them in the pool
    /// let a second campaign be planned for the same carousel while the first was still on its
    /// way. Only the states that *released* the bundle come back:
    ///
    /// * `failed_before_dispatch` — the name is the guarantee, nothing reached a phone;
    /// * `cancelled` and `missed` — the operator stopped it, or its hour passed unrun.
    pub fn bundle_ids_already_dispatched(&self) -> anyhow::Result<Vec<String>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            // **The parent counts too.** Cancelling a campaign changes only the campaign
            // row, so its assignments keep whatever state they had — and reserving on the
            // assignment alone would hold those bundles out of the pool forever.
            "SELECT DISTINCT a.bundle_id FROM publish_assignments a
             JOIN publish_campaigns c ON c.id = a.campaign_id
             WHERE a.state NOT IN ('failed_before_dispatch','cancelled','missed')
               AND c.state NOT IN ('failed_before_dispatch','cancelled','missed')",
        )?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .map(|id| {
                id.map(|id| {
                    id.split_once(':')
                        .map(|(_request, source)| source.to_string())
                        .unwrap_or(id)
                })
            })
            .collect::<Result<Vec<String>, _>>()?;
        Ok(ids)
    }

    /// Every campaign still waiting for its scheduled time.
    ///
    /// **By state, not by taking the newest page and filtering it.** The scheduler asked for
    /// two hundred campaigns each tick and picked the `scheduled` ones out — so once two
    /// hundred newer rows existed, an older scheduled campaign fell off the end of every page
    /// and was never run and never marked missed. It simply sat there.
    pub fn scheduled_publish_campaigns(&self) -> anyhow::Result<Vec<(String, Option<String>)>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            "SELECT id,run_at FROM publish_campaigns WHERE state='scheduled' ORDER BY run_at ASC",
        )?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The campaign's revision, for an event payload.
    ///
    /// A separate read for the same reason [`Self::publish_campaign_state`] is one: the caller
    /// runs once per phone and `get_publish_campaign` pulls the whole manifest — every bundle,
    /// every image — to answer with one integer.
    ///
    /// `0` for a campaign that is gone, which is the right answer for a subscriber: it means
    /// "re-read", and the re-read will find nothing.
    pub fn publish_campaign_revision(&self, id: &str) -> anyhow::Result<u64> {
        let conn = self.conn()?;
        let revision: Option<i64> = conn
            .query_row(
                "SELECT revision FROM publish_campaigns WHERE id=?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(revision.unwrap_or_default().max(0) as u64)
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
        outcome: PublishRunOutcome,
    ) -> anyhow::Result<Option<crate::publish::PublishCampaignState>> {
        let wanted = match outcome {
            PublishRunOutcome::AllPosted => crate::publish::PublishCampaignState::Succeeded,
            // **Nothing may be live, so the campaign stays claimable.** It used to become
            // `uncertain` for *any* failure, which the claim refuses forever — so a run where
            // every phone refused before opening anything (an unmeasured build, an album that
            // was not there) sent an operator to look at phones where nothing had happened,
            // and could not be run again.
            PublishRunOutcome::NothingPublished => {
                crate::publish::PublishCampaignState::FailedBeforeDispatch
            }
            PublishRunOutcome::SomethingMayBeLive => {
                crate::publish::PublishCampaignState::Uncertain
            }
        };
        let conn = self.conn()?;
        conn.execute(
            "UPDATE publish_campaigns
             SET state=?1,error_code=?2,revision=revision+1,updated_at=?3
             WHERE id=?4 AND state=?5",
            params![
                wanted.as_str(),
                match outcome {
                    PublishRunOutcome::AllPosted => None,
                    PublishRunOutcome::NothingPublished => Some("post_refused_before_dispatch"),
                    PublishRunOutcome::SomethingMayBeLive => Some("post_or_cleanup_failed"),
                },
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
        // **Namespaced, the way `publish_create_campaign` stores them.** The fixture used a
        // bare `b-<udid>`, so nothing here could see that the auto-deal reads a different id
        // namespace than the one the database holds — the defect that made the pool exclude
        // nothing at all.
        let request_id = Uuid::new_v4().to_string();
        let bundle_ids: Vec<String> = udids
            .iter()
            .map(|udid| format!("{request_id}:b-{udid}"))
            .collect();
        let bundles: Vec<crate::publish::PublishBundle> =
            bundle_ids.iter().map(|id| bundle(id)).collect();
        let request = crate::publish::PublishCampaignRequest {
            request_id: request_id.clone(),
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

    use super::PublishRunOutcome;

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

    /// Drive every assignment of a campaign into a named state, then settle.
    fn seed_and_settle(
        db: &Database,
        record: &crate::publish::PublishCampaignRecord,
        states: &[(&str, crate::publish::PublishCampaignState)],
    ) {
        db.update_publish_campaign_state(
            &record.id,
            crate::publish::PublishCampaignState::Posting,
            None,
        )
        .expect("the campaign was mid-flight when the app died");
        let assignments = db
            .get_publish_campaign(&record.id)
            .expect("read back")
            .expect("campaign exists")
            .assignments;
        for assignment in &assignments {
            if let Some((_, state)) = states.iter().find(|(udid, _)| *udid == assignment.udid) {
                db.update_publish_assignment_state(&assignment.id, state.clone(), None, None)
                    .expect("seed the mid-flight state");
            }
        }
    }

    fn assignment_id(db: &Database, campaign_id: &str, udid: &str) -> String {
        db.get_publish_campaign(campaign_id)
            .expect("read back")
            .expect("campaign exists")
            .assignments
            .into_iter()
            .find(|assignment| assignment.udid == udid)
            .expect("assignment")
            .id
    }

    /// **A crash mid-post must strand nothing, and the halves must land differently.**
    ///
    /// The row that was `posting` — or `verifying`, which is the same risk one step later —
    /// may already be a carousel on a real account, so it becomes `uncertain`, which the claim
    /// refuses. The row that was `transferring` never reached TikTok, so it becomes
    /// `failed_before_dispatch`, which the claim accepts. Collapsing the two would either
    /// strand a retryable post or re-publish a live one, and there is no delete path on
    /// Android to undo the second.
    ///
    /// **All four states are seeded**, including `verifying`. The earlier version of this test
    /// seeded only `posting` and still passed with `verifying` deleted from both recovery
    /// predicates — so the state that means "the tap went out and we were checking" was
    /// covered by a name in a SQL string and nothing else.
    #[test]
    fn a_crash_mid_post_leaves_one_half_retryable_and_the_other_permanently_not() {
        let (db, path) = fixture();
        let record = campaign(
            &db,
            &[
                "phone-posting",
                "phone-verifying",
                "phone-transferring",
                "phone-imported",
            ],
        );
        seed_and_settle(
            &db,
            &record,
            &[
                (
                    "phone-posting",
                    crate::publish::PublishCampaignState::Posting,
                ),
                (
                    "phone-verifying",
                    crate::publish::PublishCampaignState::Verifying,
                ),
                (
                    "phone-transferring",
                    crate::publish::PublishCampaignState::Transferring,
                ),
                // The fourth stays `Imported`: the crash cost it nothing.
            ],
        );

        assert_eq!(
            db.interrupt_orphaned_publish_campaigns()
                .expect("settle the stranded campaign"),
            1
        );

        for udid in ["phone-posting", "phone-verifying"] {
            assert_eq!(
                state_of(&db, &record.id, udid),
                crate::publish::PublishCampaignState::Uncertain,
                "{udid}: a post that may have gone out must never be re-dispatched"
            );
            assert!(
                !db.claim_publish_assignment_for_posting(
                    &assignment_id(&db, &record.id, udid),
                    INTENT
                )
                .expect("claim query"),
                "{udid}: uncertain must be permanently unclaimable"
            );
        }
        assert_eq!(
            state_of(&db, &record.id, "phone-transferring"),
            crate::publish::PublishCampaignState::FailedBeforeDispatch,
            "media that never left the desktop is safe to send again"
        );
        assert_eq!(
            state_of(&db, &record.id, "phone-imported"),
            crate::publish::PublishCampaignState::Imported,
            "an assignment the crash did not touch is left where Post expects it"
        );

        // Something may be live, so the campaign says so and needs a person, not a retry.
        assert_eq!(
            db.get_publish_campaign(&record.id)
                .expect("read back")
                .expect("campaign exists")
                .campaign
                .state,
            crate::publish::PublishCampaignState::Uncertain
        );
        assert!(
            !db.claim_publish_campaign_for_posting(&record.id)
                .expect("claim query"),
            "a campaign that may hold a live post must not restart on its own"
        );
        let _ = std::fs::remove_file(path);
    }

    /// **A crash where nothing reached a phone is retryable end to end, not just on paper.**
    ///
    /// The failure this pins is the one that made the previous version dishonest: recovery
    /// marked the assignments `failed_before_dispatch` — claimable — and then cancelled the
    /// campaign, and a real run starts by claiming the *campaign*. So every row said "retry
    /// me" under a parent that could never be claimed again, and the earlier test never
    /// noticed because it only ever claimed the child directly.
    ///
    /// This one claims what the app claims, in the order the app claims it.
    #[test]
    fn a_crash_before_anything_left_the_desktop_can_actually_be_run_again() {
        let (db, path) = fixture();
        let record = campaign(&db, &["phone-a", "phone-b"]);
        seed_and_settle(
            &db,
            &record,
            &[
                (
                    "phone-a",
                    crate::publish::PublishCampaignState::Transferring,
                ),
                (
                    "phone-b",
                    crate::publish::PublishCampaignState::Transferring,
                ),
            ],
        );
        assert_eq!(
            db.interrupt_orphaned_publish_campaigns().expect("settle"),
            1
        );

        assert_eq!(
            db.get_publish_campaign(&record.id)
                .expect("read back")
                .expect("campaign exists")
                .campaign
                .state,
            crate::publish::PublishCampaignState::FailedBeforeDispatch,
            "nothing reached a phone, so the campaign must not be terminal"
        );
        assert!(
            db.claim_publish_campaign_for_posting(&record.id)
                .expect("claim query"),
            "the campaign is labelled retryable and the claim refuses it — work stranded"
        );
        for udid in ["phone-a", "phone-b"] {
            assert!(
                db.claim_publish_assignment_for_posting(
                    &assignment_id(&db, &record.id, udid),
                    INTENT
                )
                .expect("claim query"),
                "{udid} cannot be re-dispatched even though nothing left the desktop"
            );
        }
        let _ = std::fs::remove_file(path);
    }

    /// **What a bundle must be released by before it can be dealt again.**
    ///
    /// Two things this could not see before, and the second made the whole pool useless.
    ///
    /// It only excluded assignments that had reached a phone, so a bundle committed to a
    /// campaign that was merely `queued` or `imported` — images already in a phone's gallery —
    /// stayed on offer and could be planned into a second campaign.
    ///
    /// And it compared the wrong ids. Campaign creation namespaces bundles as
    /// `"{request_id}:{source}"`; the deal rescans the folder and offers `source`. The two sets
    /// never intersected, so **nothing was ever excluded** — the fixture hid it by reading its
    /// expected ids back out of the same rows.
    #[test]
    fn a_bundle_is_offered_again_only_after_a_campaign_lets_go_of_it() {
        let (db, path) = fixture();
        let record = campaign(
            &db,
            &[
                "p-succeeded",
                "p-posting",
                "p-uncertain",
                "p-failed",
                "p-imported",
                "p-cancelled",
            ],
        );
        let by_udid: std::collections::HashMap<String, (String, String)> = db
            .get_publish_campaign(&record.id)
            .expect("read back")
            .expect("campaign exists")
            .assignments
            .into_iter()
            .map(|assignment| {
                (
                    assignment.udid.clone(),
                    (assignment.id, assignment.bundle_id),
                )
            })
            .collect();
        for (udid, state) in [
            (
                "p-succeeded",
                crate::publish::PublishCampaignState::Succeeded,
            ),
            ("p-posting", crate::publish::PublishCampaignState::Posting),
            (
                "p-uncertain",
                crate::publish::PublishCampaignState::Uncertain,
            ),
            (
                "p-failed",
                crate::publish::PublishCampaignState::FailedBeforeDispatch,
            ),
            (
                "p-cancelled",
                crate::publish::PublishCampaignState::Cancelled,
            ),
            // `p-imported` keeps whatever `campaign()` left it at, which is `Imported`.
        ] {
            let (id, _) = &by_udid[udid];
            db.update_publish_assignment_state(id, state, None, None)
                .expect("seed");
        }

        let dispatched = db.bundle_ids_already_dispatched().expect("read");
        // **The stored id is namespaced and the pool speaks the scanner's language.** Asserted
        // on the raw row, so a test that read its expectation back out of the same rows cannot
        // pass by agreeing with itself.
        let stored = &by_udid["p-succeeded"].1;
        assert!(
            stored.contains(':'),
            "the fixture stopped namespacing bundle ids, so this test proves nothing: {stored}"
        );
        let source_of = |udid: &str| {
            by_udid[udid]
                .1
                .split_once(':')
                .map(|(_, source)| source.to_string())
                .expect("namespaced")
        };
        for udid in ["p-succeeded", "p-posting", "p-uncertain", "p-imported"] {
            assert!(
                dispatched.contains(&source_of(udid)),
                "{udid}: a campaign still holds this bundle and it was offered again"
            );
        }
        for udid in ["p-failed", "p-cancelled"] {
            assert!(
                !dispatched.contains(&source_of(udid)),
                "{udid}: this campaign let the bundle go and it stayed out of the pool"
            );
        }
        // And nothing namespaced leaks out, which is what made every comparison fail.
        assert!(
            dispatched.iter().all(|id| !id.contains(':')),
            "the pool still speaks the database's id namespace: {dispatched:?}"
        );

        // **Cancelling the campaign gives every one of its bundles back.** Cancel touches only
        // the campaign row, so reserving on the assignment alone would hold them out of the
        // pool for good — the operator would stop a run and never be able to deal those posts
        // again.
        db.cancel_publish_campaign(&record.id).expect("cancel");
        let after_cancel = db.bundle_ids_already_dispatched().expect("read");
        for udid in ["p-succeeded", "p-posting", "p-uncertain", "p-imported"] {
            assert!(
                !after_cancel.contains(&source_of(udid)),
                "{udid}: its campaign was cancelled and the bundle is still held"
            );
        }
        let _ = std::fs::remove_file(path);
    }

    /// The revision an event carries moves whenever the campaign does.
    #[test]
    fn the_event_revision_moves_with_every_state_write() {
        let (db, path) = fixture();
        let record = campaign(&db, &["phone-a"]);
        let first = db.publish_campaign_revision(&record.id).expect("read");
        db.update_publish_campaign_state(
            &record.id,
            crate::publish::PublishCampaignState::Transferring,
            None,
        )
        .expect("write");
        let second = db.publish_campaign_revision(&record.id).expect("read");
        assert!(
            second > first,
            "a subscriber cannot tell two events apart: {first} then {second}"
        );
        // A campaign that is gone reads 0, which tells a subscriber to re-read and find
        // nothing — rather than failing the emit.
        assert_eq!(
            db.publish_campaign_revision("no-such-campaign")
                .expect("read"),
            0
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

    /// **A finished campaign cannot be transferred again.**
    ///
    /// The hole underneath every other guard. Transfer wrote `Transferring` unconditionally and
    /// walked its assignments back to `Imported` — so a second Transfer on a campaign that had
    /// already succeeded rebuilt exactly the state the posting claim accepts, and the next Post
    /// published the same carousels a second time. Compare-and-swap protects nothing while
    /// another path can manufacture a claimable state.
    #[test]
    fn a_campaign_that_already_posted_cannot_be_transferred_again() {
        let (db, path) = fixture();
        let record = campaign(&db, &["phone-a"]);

        // The states a transfer may start from. (`campaign()` leaves the fixture `Imported`,
        // which is deliberately not one of them — the media is already on the phones.)
        db.update_publish_campaign_state(
            &record.id,
            crate::publish::PublishCampaignState::Queued,
            None,
        )
        .expect("seed");
        assert!(db
            .claim_publish_campaign_for_transfer(&record.id)
            .expect("queued is transferable"));
        for start in [
            crate::publish::PublishCampaignState::Ready,
            crate::publish::PublishCampaignState::Scheduled,
            crate::publish::PublishCampaignState::FailedBeforeDispatch,
        ] {
            db.update_publish_campaign_state(&record.id, start.clone(), None)
                .expect("seed");
            assert!(
                db.claim_publish_campaign_for_transfer(&record.id)
                    .expect("claim runs"),
                "{start:?} should be transferable"
            );
        }

        // And the ones it must not.
        for finished in [
            crate::publish::PublishCampaignState::Succeeded,
            crate::publish::PublishCampaignState::Posting,
            crate::publish::PublishCampaignState::Verifying,
            crate::publish::PublishCampaignState::Uncertain,
            crate::publish::PublishCampaignState::Cancelled,
            // Already on the phones; the next step is Post, not another transfer.
            crate::publish::PublishCampaignState::Imported,
        ] {
            db.update_publish_campaign_state(&record.id, finished.clone(), None)
                .expect("seed");
            assert!(
                !db.claim_publish_campaign_for_transfer(&record.id)
                    .expect("claim runs"),
                "{finished:?} was re-transferred, which rebuilds a claimable state"
            );
            assert_eq!(
                db.publish_campaign_state(&record.id).expect("read"),
                Some(finished),
                "and the refusal must not move the row"
            );
        }
        let _ = std::fs::remove_file(path);
    }

    /// **A run where nothing was published leaves a campaign that can run again.**
    ///
    /// The campaign used to take `uncertain` for *any* failure, which the claim refuses
    /// forever — while its children were correctly marked retryable underneath it. An operator
    /// was sent to look at phones where nothing had happened.
    #[test]
    fn a_campaign_ends_where_its_children_say_it_should() {
        let (db, path) = fixture();
        for (outcome, expected) in [
            (
                PublishRunOutcome::AllPosted,
                crate::publish::PublishCampaignState::Succeeded,
            ),
            (
                PublishRunOutcome::NothingPublished,
                crate::publish::PublishCampaignState::FailedBeforeDispatch,
            ),
            (
                PublishRunOutcome::SomethingMayBeLive,
                crate::publish::PublishCampaignState::Uncertain,
            ),
        ] {
            let record = campaign(&db, &[&format!("phone-{outcome:?}")]);
            assert!(db
                .claim_publish_campaign_for_posting(&record.id)
                .expect("claim"));
            assert_eq!(
                db.finish_publish_campaign(&record.id, outcome)
                    .expect("finish"),
                Some(expected.clone()),
                "{outcome:?} must end as {expected:?}"
            );
        }
        // And the retryable one really is claimable again, which is the whole point.
        let record = campaign(&db, &["phone-retry"]);
        assert!(db
            .claim_publish_campaign_for_posting(&record.id)
            .expect("claim"));
        db.finish_publish_campaign(&record.id, PublishRunOutcome::NothingPublished)
            .expect("finish");
        assert!(
            db.claim_publish_campaign_for_posting(&record.id)
                .expect("claim"),
            "a campaign that published nothing must be runnable again"
        );
        let _ = std::fs::remove_file(path);
    }

    /// **A cancel that lands between the read and the claim still stops the phone.**
    ///
    /// The task read the campaign state and claimed its row in two separate statements, and
    /// the gap between them is exactly where the operator's Cancel arrives: the task saw
    /// `Posting`, the campaign became `Cancelled`, and the task then claimed a row and opened a
    /// phone that published after the run was stopped. On Android that post cannot be taken
    /// down.
    ///
    /// The claim names the parent's state in its own `UPDATE`, so there is no gap left.
    #[test]
    fn a_cancel_that_lands_after_the_state_read_still_stops_the_claim() {
        let (db, path) = fixture();
        let record = campaign(&db, &["phone-a"]);
        let assignment = db
            .get_publish_campaign(&record.id)
            .expect("read back")
            .expect("campaign exists")
            .assignments
            .into_iter()
            .next()
            .expect("one assignment")
            .id;
        let intent = "{\"effectIntent\":\"post_carousel\"}";
        assert!(db
            .claim_publish_campaign_for_posting(&record.id)
            .expect("claim the campaign"));

        // What the task saw.
        assert_eq!(
            db.publish_campaign_state(&record.id).expect("read"),
            Some(crate::publish::PublishCampaignState::Posting)
        );
        // What the operator did next.
        db.cancel_publish_campaign(&record.id).expect("cancel");
        // What the task tried to do with what it saw.
        assert!(
            !db.claim_publish_assignment_for_posting(&assignment, intent)
                .expect("claim runs"),
            "a phone was claimed after the run was cancelled"
        );
        assert_eq!(
            state_of(&db, &record.id, "phone-a"),
            crate::publish::PublishCampaignState::Imported,
            "and the row is untouched, which is where a later run expects it"
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

        // **The parent has to be posting.** The claim now names its campaign's state in the
        // same statement, so a run that has not started — or has been cancelled — cannot have
        // its rows taken.
        assert!(
            !db.claim_publish_assignment_for_posting(&assignment, intent)
                .expect("claim runs"),
            "an assignment was claimed under a campaign that is not posting"
        );
        assert!(db
            .claim_publish_campaign_for_posting(&record.id)
            .expect("claim the campaign"));

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
