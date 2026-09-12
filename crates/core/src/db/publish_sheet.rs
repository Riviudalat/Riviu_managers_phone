//! The outbox between a published carousel and the operator's partner sheet.
//!
//! # Two deliveries, and only one of them is the post
//!
//! A campaign with Sheet reporting enabled owes a row: its link in column D, `bot` as
//! the poster, and the partner names out of the campaign's workbook from column K onward.
//! The obvious shape is an HTTP call at the end of the post step, and it is wrong in a way
//! that costs real money: a network error would then make a **published** post read as a
//! failed one — and a failed post is exactly what an operator retries, which on Android
//! publishes a second carousel that nothing here can take down.
//!
//! So the row lands in the database first and travels afterwards. The two failures stay
//! separate all the way through: [`crate::publish::PublishCampaignState`] says whether the
//! carousel is on the account, and `state` here says whether the sheet knows about it.
//! Retrying the second never re-runs the first.

use super::*;

/// One row owed to the sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetOutboxRow {
    pub assignment_id: String,
    pub campaign_id: String,
    pub post_url: String,
    pub poster: String,
    /// Partner names in workbook order — the order they are written across columns K+.
    pub partners: Vec<String>,
    pub attempts: u32,
    /// The version of this row's **content**, bumped whenever it changes.
    ///
    /// Carried so a completion can name what it actually delivered. Marking by id alone was a
    /// race with a real consequence: a sweep reads the row, a re-queue replaces its URL, the
    /// sweep finishes and marks the row sent — and the **newer** URL, the one nobody has
    /// pushed, is now marked delivered and never travels.
    pub revision: i64,
    pub last_error: Option<String>,
    pub posted_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheetOutboxState {
    Pending,
    Failed,
    Sent,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SheetOutboxSettlement {
    StaleRevision,
    DeliveredWithoutCampaign,
    Delivered(crate::publish_runtime::PublishExecutionSnapshot),
}

pub(super) fn evidence_has_post_link(raw: Option<&str>) -> bool {
    raw.and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .as_ref()
        .and_then(|evidence| {
            evidence
                .get("post")
                .and_then(|post| post.get("postUrl"))
                .or_else(|| evidence.get("postUrl"))
        })
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .is_some_and(crate::tiktok_share::looks_like_a_post_link)
}

pub(super) fn reconciled_sheet_delivery_status(
    connection: &Connection,
    campaign_id: &str,
) -> anyhow::Result<(
    crate::publish_runtime::PublishExecutionStatus,
    crate::publish_runtime::PublishRetryScope,
)> {
    use crate::publish::PublishCampaignState as CampaignState;
    use crate::publish_runtime::{PublishExecutionStatus as Status, PublishRetryScope as Scope};

    let campaign_state: String = connection.query_row(
        "SELECT state FROM publish_campaigns WHERE id=?1",
        [campaign_id],
        |row| row.get(0),
    )?;
    let campaign_state = publish_state_from_str(&campaign_state);
    if matches!(
        campaign_state,
        CampaignState::Cancelled | CampaignState::Missed
    ) {
        return Ok((Status::Partial, Scope::None));
    }

    if connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM publish_pipeline_runs WHERE campaign_id=?1)",
        [campaign_id],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok((Status::Partial, Scope::None));
    }
    let mut statement = connection.prepare(
        "SELECT a.state,a.evidence_json,o.state
         FROM publish_assignments a
         LEFT JOIN publish_sheet_outbox o ON o.assignment_id=a.id
         WHERE a.campaign_id=?1
         ORDER BY a.ordinal",
    )?;
    let rows = statement
        .query_map([campaign_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);

    if rows.iter().any(|(state, _, _)| {
        matches!(
            publish_state_from_str(state),
            CampaignState::Posting | CampaignState::Uncertain
        )
    }) {
        let manual_link_only = rows.iter().any(|(state, evidence, _)| {
            publish_state_from_str(state) == CampaignState::Uncertain
                && super::publish_verification::needs_review(evidence.as_deref())
        }) && rows.iter().all(|(state, evidence, _)| {
            match publish_state_from_str(state) {
                CampaignState::Posting => false,
                CampaignState::Uncertain => {
                    super::publish_verification::needs_review(evidence.as_deref())
                }
                _ => true,
            }
        });
        return Ok((
            Status::Uncertain,
            if manual_link_only {
                Scope::LinkAndSheet
            } else {
                Scope::None
            },
        ));
    }
    if rows
        .iter()
        .any(|(state, _, _)| publish_state_from_str(state) == CampaignState::Verifying)
    {
        return Ok((Status::Partial, Scope::LinkAndSheet));
    }
    let all_succeeded = !rows.is_empty()
        && rows
            .iter()
            .all(|(state, _, _)| publish_state_from_str(state) == CampaignState::Succeeded);
    if !all_succeeded {
        return Ok((Status::Partial, Scope::FullPipeline));
    }

    let all_links_known = rows
        .iter()
        .all(|(_, evidence, _)| evidence_has_post_link(evidence.as_deref()));
    let sheet_owed = rows
        .iter()
        .any(|(_, _, state)| matches!(state.as_deref(), Some("pending" | "failed")));
    let every_sheet_row_sent = rows
        .iter()
        .all(|(_, _, state)| state.as_deref() == Some("sent"));
    if all_links_known
        && (!campaign_sheet_enabled(connection, campaign_id)? || every_sheet_row_sent)
    {
        return Ok((Status::Complete, Scope::None));
    }
    Ok((
        Status::Partial,
        if sheet_owed && all_links_known {
            Scope::SheetOnly
        } else {
            Scope::LinkAndSheet
        },
    ))
}

fn campaign_sheet_enabled(connection: &Connection, campaign_id: &str) -> anyhow::Result<bool> {
    let raw: Option<String> = connection
        .query_row(
            "SELECT request_json FROM publish_campaigns WHERE id=?1",
            [campaign_id],
            |row| row.get(0),
        )
        .optional()?;
    // Detached legacy outbox fixtures predate campaign persistence.
    raw.map(|raw| {
        serde_json::from_str::<crate::PublishCampaignRequest>(&raw)
            .map(|request| request.sheet_enabled)
            .map_err(Into::into)
    })
    .unwrap_or(Ok(true))
}

impl Database {
    /// Derive the restart-safe Publish outcome from the same facts the atomic Sheet settle uses.
    pub fn reconciled_publish_execution_status(
        &self,
        campaign_id: &str,
    ) -> anyhow::Result<(
        crate::publish_runtime::PublishExecutionStatus,
        crate::publish_runtime::PublishRetryScope,
    )> {
        reconciled_sheet_delivery_status(&self.conn()?, campaign_id)
    }

    /// Read the durable delivery state without conflating a sent row with a missing row.
    pub fn publish_sheet_outbox_state(
        &self,
        assignment_id: &str,
    ) -> anyhow::Result<Option<SheetOutboxState>> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT state FROM publish_sheet_outbox WHERE assignment_id=?1",
            [assignment_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|state| match state.as_str() {
            "pending" => Ok(SheetOutboxState::Pending),
            "failed" => Ok(SheetOutboxState::Failed),
            "sent" => Ok(SheetOutboxState::Sent),
            other => anyhow::bail!("invalid publish Sheet outbox state: {other}"),
        })
        .transpose()
    }

    /// Record that a published post owes the sheet a row.
    ///
    /// Called in the same breath as the post's own success, and **never** in a way that can
    /// fail the post: the caller treats an error here as a logged problem, not as a failed
    /// carousel.
    ///
    /// Re-queuing the same assignment replaces the pending row rather than adding one.
    /// Store the webhook URL and its token as **one** value.
    ///
    /// They were two `set_setting` calls, and the sweeper reads the pair on every tick — so
    /// a crash between the writes, or a tick landing in the gap, could pair a new endpoint
    /// with the previous endpoint's bearer token and send it there. The credential belongs
    /// to the URL it was issued for, so the two move together or not at all.
    ///
    /// `token: None` leaves the stored token untouched (the caller decides whether that is
    /// allowed for this URL change); `Some("")` clears it.
    pub fn set_publish_sheet_config(
        &self,
        webhook_url: &str,
        token: Option<&str>,
    ) -> anyhow::Result<()> {
        self.set_publish_sheet_config_with_reporting(webhook_url, token, None)
    }

    pub fn set_publish_sheet_config_with_reporting(
        &self,
        webhook_url: &str,
        token: Option<&str>,
        internal_reporting: Option<bool>,
    ) -> anyhow::Result<()> {
        if let Some(secrets) = &self.secrets {
            let previous = self.publish_sheet_delivery_settings()?;
            let value = serde_json::json!({"webhookUrl":webhook_url,"token":token.unwrap_or(&previous.token),"internalReporting":internal_reporting.unwrap_or(previous.internal_reporting)});
            secrets.set_secret("publish-sheet-connection", &value.to_string())?;
            let mut conn = self.conn()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![crate::publish_sheet::WEBHOOK_URL_SETTING,webhook_url])?;
            tx.execute(
                "DELETE FROM settings WHERE key=?1",
                params![crate::publish_sheet::WEBHOOK_TOKEN_SETTING],
            )?;
            tx.commit()?;
            return Ok(());
        }
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![crate::publish_sheet::WEBHOOK_URL_SETTING, webhook_url],
        )?;
        if let Some(token) = token {
            transaction.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![crate::publish_sheet::WEBHOOK_TOKEN_SETTING, token],
            )?;
        }
        if let Some(enabled) = internal_reporting {
            transaction.execute(
                "INSERT INTO settings (key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![crate::publish_sheet::INTERNAL_REPORTING_SETTING,if enabled {"true"} else {"false"}],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// `assignment_id` is the primary key precisely so this cannot produce two rows for one
    /// post — the operator would see the same link twice in column D with no way to tell
    /// which to remove. An already-`sent` row is left alone: it is not owed again.
    pub fn queue_publish_sheet_row(
        &self,
        assignment_id: &str,
        campaign_id: &str,
        post_url: &str,
        poster: &str,
        partners: &[String],
    ) -> anyhow::Result<()> {
        let conn = self.conn()?;
        queue_sheet_row(
            &conn,
            assignment_id,
            campaign_id,
            post_url,
            poster,
            partners,
        )
    }

    /// Record a published carousel **and** its sheet obligation, in one transaction.
    ///
    /// # Why the two writes cannot be two statements
    ///
    /// The obligation is only durable if it commits with the fact it is about. Two separate
    /// connections leave a window: the assignment is marked `succeeded`, the process loses
    /// power, and the outbox row was never written. The post is live and undeletable, nothing
    /// on disk says the sheet is owed a link, and no sweep at restart has anything to find.
    ///
    /// Migration 17's own comment claimed this was already the case. It was not — the queue
    /// method opened its own connection — so this is the function that makes the claim true,
    /// and the one a publish path must call.
    ///
    /// **A failure here fails the recording, not the post.** The carousel is already out; the
    /// caller's job is to log this and move on, never to report the assignment as failed.
    pub fn record_publish_success_with_sheet_row(
        &self,
        assignment_id: &str,
        evidence_json: &str,
        campaign_id: &str,
        post_url: &str,
        poster: &str,
        partners: &[String],
    ) -> anyhow::Result<()> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "UPDATE publish_assignments SET state=?1,error_code=NULL,evidence_json=?2,\
             revision=revision+1,updated_at=?3 WHERE id=?4",
            params![
                crate::publish::PublishCampaignState::Succeeded.as_str(),
                evidence_json,
                Utc::now().to_rfc3339(),
                assignment_id
            ],
        )?;
        queue_sheet_row(
            &transaction,
            assignment_id,
            campaign_id,
            post_url,
            poster,
            partners,
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// The rows still owed to the sheet, oldest first.
    ///
    /// Includes `failed` as well as `pending`, which is the point of keeping them apart from
    /// `sent`: a push that failed is still owed, and the only thing `failed` adds is a
    /// message an operator can read. There is no attempt ceiling here — a row that stops
    /// being retried is a link nobody ever pastes, and the sheet is the operator's record of
    /// what went out.
    pub fn pending_publish_sheet_rows(&self, limit: usize) -> anyhow::Result<Vec<SheetOutboxRow>> {
        let conn = self.conn()?;
        // Clamped, because `as i64` on a very large `usize` wraps negative and SQLite reads a
        // negative LIMIT as *no limit* — so a caller asking for too much would get the whole
        // outbox materialised instead of an error.
        let limit = crate::publish_sheet::sweep_limit(limit);
        let mut statement = conn.prepare(
            "SELECT assignment_id,campaign_id,post_url,poster,partners_json,attempts,revision,\
                    last_error,(SELECT COALESCE(CASE WHEN json_valid(a.evidence_json) THEN json_extract(a.evidence_json,'$.post.submittedAt') END,CASE WHEN json_valid(a.evidence_json) THEN json_extract(a.evidence_json,'$.submittedAt') END,CASE WHEN json_valid(a.effect_intent) THEN json_extract(a.effect_intent,'$.submittedAt') END,a.created_at) FROM publish_assignments a WHERE a.id=publish_sheet_outbox.assignment_id)
             FROM publish_sheet_outbox
             WHERE state <> 'sent'
             ORDER BY created_at ASC
             LIMIT ?1",
        )?;
        let rows = statement
            .query_map(params![limit], |row| {
                let partners: String = row.get(4)?;
                Ok(SheetOutboxRow {
                    assignment_id: row.get(0)?,
                    campaign_id: row.get(1)?,
                    post_url: row.get(2)?,
                    poster: row.get(3)?,
                    // A row whose JSON cannot be parsed still travels, with no partner
                    // names, rather than stopping the queue behind it: the link in column D
                    // is the part the operator cannot reconstruct.
                    partners: serde_json::from_str(&partners).unwrap_or_default(),
                    attempts: row.get::<_, i64>(5)?.max(0) as u32,
                    revision: row.get(6)?,
                    last_error: row.get(7)?,
                    posted_at: row.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Read one still-owed row without depending on the global sweep window.
    pub fn pending_publish_sheet_row(
        &self,
        assignment_id: &str,
    ) -> anyhow::Result<Option<SheetOutboxRow>> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT assignment_id,campaign_id,post_url,poster,partners_json,attempts,revision,\
                    last_error,(SELECT COALESCE(CASE WHEN json_valid(a.evidence_json) THEN json_extract(a.evidence_json,'$.post.submittedAt') END,CASE WHEN json_valid(a.evidence_json) THEN json_extract(a.evidence_json,'$.submittedAt') END,CASE WHEN json_valid(a.effect_intent) THEN json_extract(a.effect_intent,'$.submittedAt') END,a.created_at) FROM publish_assignments a WHERE a.id=publish_sheet_outbox.assignment_id)
             FROM publish_sheet_outbox
             WHERE assignment_id=?1 AND state <> 'sent'",
            params![assignment_id],
            |row| {
                let partners: String = row.get(4)?;
                Ok(SheetOutboxRow {
                    assignment_id: row.get(0)?,
                    campaign_id: row.get(1)?,
                    post_url: row.get(2)?,
                    poster: row.get(3)?,
                    partners: serde_json::from_str(&partners).unwrap_or_default(),
                    attempts: row.get::<_, i64>(5)?.max(0) as u32,
                    revision: row.get(6)?,
                    last_error: row.get(7)?,
                    posted_at: row.get(8)?,
                })
            },
        )
        .optional()
        .map_err(anyhow::Error::from)
    }

    /// The sheet has this **version** of the row. Terminal — nothing re-sends a `sent` row.
    ///
    /// `revision` is not decoration: a completion that matched on id alone marked whatever the
    /// row held *now*, which after a concurrent re-queue is a different URL than the one that
    /// was actually delivered. That URL would then never travel and nothing would say so.
    /// Returns whether the row it named was still the row it delivered.
    #[cfg(test)]
    fn mark_publish_sheet_sent(&self, assignment_id: &str, revision: i64) -> anyhow::Result<bool> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE publish_sheet_outbox
             SET state='sent',attempts=attempts+1,last_error=NULL,updated_at=?2
             WHERE assignment_id=?1 AND revision=?3",
            params![assignment_id, Utc::now().to_rfc3339(), revision],
        )?;
        Ok(changed > 0)
    }

    /// Mark one delivered revision and replace its campaign projection in one transaction.
    ///
    /// A snapshot error rolls the outbox row back to pending/failed, so the sweeper can retry it.
    /// The campaign may have been deleted while its obligation deliberately survived; that case
    /// commits the Sheet row alone because there is no operation projection left to update.
    pub fn settle_publish_sheet_delivery(
        &self,
        assignment_id: &str,
        campaign_id: &str,
        revision: i64,
        input_digest: Option<&str>,
        target_snapshot: Option<&crate::ResolvedTargetSnapshot>,
    ) -> anyhow::Result<SheetOutboxSettlement> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = settle_delivery_on(
            &transaction,
            assignment_id,
            campaign_id,
            revision,
            input_digest,
            target_snapshot,
        )?;
        transaction.commit()?;
        Ok(result)
    }

    /// The push failed. Still owed; the message is for a person to read.
    ///
    /// Refuses to move a row that is already `sent`, and refuses to move a version other than
    /// the one that was delivered. Without the first guard a late error from a retry of an
    /// already-delivered row would reopen it, and the next sweep would paste the same link
    /// into column D a second time.
    pub fn mark_publish_sheet_failed(
        &self,
        assignment_id: &str,
        revision: i64,
        error: &str,
    ) -> anyhow::Result<bool> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE publish_sheet_outbox
             SET state='failed',attempts=attempts+1,last_error=?3,updated_at=?2
             WHERE assignment_id=?1 AND revision=?4 AND state <> 'sent'",
            params![assignment_id, Utc::now().to_rfc3339(), error, revision],
        )?;
        Ok(changed > 0)
    }
}

pub(super) fn settle_delivery_on(
    transaction: &Connection,
    assignment_id: &str,
    campaign_id: &str,
    revision: i64,
    input_digest: Option<&str>,
    target_snapshot: Option<&crate::ResolvedTargetSnapshot>,
) -> anyhow::Result<SheetOutboxSettlement> {
    let updated_at = Utc::now().to_rfc3339();
    let changed = transaction.execute(
        "UPDATE publish_sheet_outbox
             SET state='sent',attempts=attempts+1,last_error=NULL,updated_at=?4
             WHERE assignment_id=?1 AND campaign_id=?2 AND revision=?3 AND state <> 'sent'",
        params![assignment_id, campaign_id, revision, updated_at],
    )?;
    if changed == 0 {
        return Ok(SheetOutboxSettlement::StaleRevision);
    }

    let campaign_exists = transaction
        .query_row(
            "SELECT 1 FROM publish_campaigns WHERE id=?1",
            [campaign_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !campaign_exists {
        return Ok(SheetOutboxSettlement::DeliveredWithoutCampaign);
    }

    let input_digest = input_digest
        .context("publish execution input digest is required while its campaign still exists")?;
    let (status, retry_scope) = reconciled_sheet_delivery_status(transaction, campaign_id)?;
    let draft = crate::publish_runtime::PublishExecutionSnapshotDraft {
        input_digest: input_digest.to_string(),
        status,
        retry_scope,
        report_json: serde_json::json!({
            "campaignId": campaign_id,
            "status": status,
            "retryScope": retry_scope,
            "source": "sheet_delivery_reconciliation",
            "sheetEnabled": campaign_sheet_enabled(transaction, campaign_id)?,
            "targetSnapshot": target_snapshot,
        }),
    };
    // Validation and the actual upsert intentionally happen after the outbox UPDATE but
    // before commit. Any error here drops the transaction and makes the delivery retryable.
    let snapshot = super::publish::store_publish_execution_snapshot(
        transaction,
        campaign_id,
        &draft,
        &updated_at,
    )?;
    Ok(SheetOutboxSettlement::Delivered(snapshot))
}

/// The insert itself, so it can run inside a caller's transaction or on its own connection.
pub(super) fn queue_sheet_row(
    conn: &rusqlite::Connection,
    assignment_id: &str,
    campaign_id: &str,
    post_url: &str,
    poster: &str,
    partners: &[String],
) -> anyhow::Result<()> {
    super::publish_submission::record_canonical_identity(conn, assignment_id, post_url)?;
    // Publication proof releases the account even when Sheet reporting is disabled.
    conn.execute(
        "DELETE FROM publish_account_reservations WHERE assignment_id=?1 AND EXISTS(
          SELECT 1 FROM publish_assignments a WHERE a.id=?1 AND a.state='succeeded'
          AND json_valid(a.evidence_json)
          AND COALESCE(json_extract(a.evidence_json,'$.post.publicationVerified'),json_extract(a.evidence_json,'$.publicationVerified'))=1
          AND COALESCE(json_extract(a.evidence_json,'$.post.postUrl'),json_extract(a.evidence_json,'$.postUrl'))=?2)",
        params![assignment_id,post_url],
    )?;
    if !campaign_sheet_enabled(conn, campaign_id)? {
        return Ok(());
    }
    let delivery_target: Option<String> = conn.query_row(
        "SELECT CASE WHEN json_valid(request_json) AND json_extract(request_json,'$.sheetDelivery.version')=2
          THEN json_extract(request_json,'$.sheetDelivery') END FROM publish_campaigns WHERE id=?1",
        [campaign_id], |row|row.get(0),
    ).optional()?.flatten();
    if let Some(raw) = &delivery_target {
        let target: crate::publish_sheet::SheetDeliveryTarget = serde_json::from_str(raw)?;
        target.validate()?;
        let verified:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM publish_assignments WHERE id=?1 AND state='succeeded'
          AND json_valid(evidence_json)
          AND COALESCE(json_extract(evidence_json,'$.post.publicationVerified'),json_extract(evidence_json,'$.publicationVerified'))=1
          AND COALESCE(json_extract(evidence_json,'$.post.postUrl'),json_extract(evidence_json,'$.postUrl'))=?2)",params![assignment_id,post_url],|row|row.get(0))?;
        anyhow::ensure!(
            verified,
            "v2 Sheet delivery requires the verified canonical publication proof"
        );
        let existing: Option<(String,String,String)> = conn.query_row(
            "SELECT post_url,poster,partners_json FROM publish_sheet_outbox WHERE assignment_id=?1",
            [assignment_id], |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).optional()?;
        if let Some((url, by, old_partners)) = existing {
            anyhow::ensure!(
                url == post_url
                    && by == poster
                    && serde_json::from_str::<Vec<String>>(&old_partners)? == partners,
                "verified Sheet row identity is immutable"
            );
            return Ok(());
        }
    }
    let now = Utc::now().to_rfc3339();
    let posted_at: Option<String> = conn.query_row(
        "SELECT COALESCE(CASE WHEN json_valid(effect_intent) THEN json_extract(effect_intent,'$.submittedAt') END,
          CASE WHEN json_valid(evidence_json) THEN COALESCE(json_extract(evidence_json,'$.post.submittedAt'),json_extract(evidence_json,'$.submittedAt')) END)
         FROM publish_assignments WHERE id=?1", [assignment_id], |row|row.get(0),
    ).optional()?.flatten();
    if delivery_target.is_some() {
        anyhow::ensure!(
            posted_at
                .as_deref()
                .is_some_and(|at| chrono::DateTime::parse_from_rfc3339(at).is_ok()),
            "v2 Sheet delivery requires its immutable submission time"
        );
    }
    let report_metadata = if delivery_target.is_some() {
        super::publish_report::internal_report_on(conn, assignment_id)?
            .filter(|row| row.metadata.status == "Đã xác minh")
            .map(|row| serde_json::to_string(&row.metadata))
            .transpose()?
    } else {
        None
    };
    conn.execute(
        "INSERT INTO publish_sheet_outbox(
                 assignment_id,campaign_id,post_url,poster,partners_json,state,
                 attempts,created_at,updated_at,delivery_target_json,posted_at,report_metadata_json
             ) VALUES(?1,?2,?3,?4,?5,'pending',0,?6,?6,?7,?8,?9)
             ON CONFLICT(assignment_id) DO UPDATE SET
                 post_url=excluded.post_url,
                 poster=excluded.poster,
                 partners_json=excluded.partners_json,
                 state='pending',
                 last_error=NULL,
                 revision=publish_sheet_outbox.revision+1,
                 updated_at=excluded.updated_at
             WHERE publish_sheet_outbox.state <> 'sent'",
        params![
            assignment_id,
            campaign_id,
            post_url,
            poster,
            serde_json::to_string(partners)?,
            now,
            delivery_target,
            posted_at,
            report_metadata
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publish::{
        PublishBundle, PublishCampaignRequest, PublishCleanupPolicy, PublishImage,
        PublishMediaKind, PublishVisibility,
    };
    use std::path::PathBuf;

    fn fixture() -> (Database, PathBuf) {
        let path =
            std::env::temp_dir().join(format!("riviu-sheet-outbox-test-{}.db", Uuid::new_v4()));
        (Database::open(&path).expect("open fixture database"), path)
    }

    fn bundle(id: &str) -> PublishBundle {
        PublishBundle {
            id: id.into(),
            source_path: format!("/fixture/{id}"),
            name: id.into(),
            media_kind: PublishMediaKind::Image,
            images: vec![PublishImage {
                path: format!("/fixture/{id}/01.png"),
                file_name: "01.png".into(),
                order: 1,
                sha256: "a".repeat(64),
                byte_len: 3,
                width: 1,
                height: 1,
            }],
            video: None,
            caption_path: format!("/fixture/{id}/caption.txt"),
            caption: "caption".into(),
            caption_sha256: "b".repeat(64),
            total_bytes: 3,
            partners: Vec::new(),
        }
    }

    /// One campaign with one assignment, and that assignment's id.
    fn seed(db: &Database) -> (String, String) {
        seed_with_sheet(db, true)
    }

    fn seed_with_sheet(db: &Database, sheet_enabled: bool) -> (String, String) {
        // A fresh bundle id per campaign: `publish_bundles.id` is a primary key, so a fixture
        // that always says `bundle-a` can only ever make one campaign.
        let bundle_id = format!("bundle-{}", Uuid::new_v4());
        let request = PublishCampaignRequest {
            sheet_delivery: None,
            verification_contract_version: None,
            verification_builds: vec![],
            sheet_enabled,
            request_id: Uuid::new_v4().to_string(),
            source_root: "/fixture/root".into(),
            bundle_ids: vec![bundle_id.clone()],
            udids: vec![format!("phone-{bundle_id}")],
            run_at: None,
            visibility: PublishVisibility::Public,
            cleanup_policy: PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
            network: crate::SocialNetwork::TikTok,
            sound_policy: crate::publish::PublishSoundPolicy::Default,
            execution_confirmed: false,
            target_snapshot: None,
        };
        let campaign = db
            .create_publish_campaign(&request, &[bundle(&bundle_id)])
            .expect("create campaign");
        // `create_publish_campaign` hands back the *plan* — bundle, udid, ordinal — while the
        // row ids are assigned by the insert, so the id has to be read back.
        let assignment = db
            .get_publish_campaign(&campaign.id)
            .expect("read back")
            .expect("campaign exists")
            .assignments
            .into_iter()
            .next()
            .expect("one assignment")
            .id;
        (campaign.id, assignment)
    }

    fn review_fixture(age_minutes: i64) -> (Database, PathBuf, String, String, String) {
        let (db, path) = fixture();
        let (campaign, assignment) = seed(&db);
        db.update_publish_campaign_state(&campaign, crate::PublishCampaignState::Posting, None)
            .unwrap();
        db.update_publish_assignment_state(
            &assignment,
            crate::PublishCampaignState::Imported,
            None,
            None,
        )
        .unwrap();
        let intent = serde_json::json!({"effectIntent":"post","submittedAt":(Utc::now()-chrono::Duration::minutes(age_minutes)).to_rfc3339(),"expectedAccount":"fixture","importId":"keep-import"}).to_string();
        assert!(db
            .claim_publish_assignment_for_posting(&assignment, &intent)
            .unwrap());
        db.update_publish_assignment_state(
            &assignment,
            crate::PublishCampaignState::Verifying,
            Some("post_verification_pending"),
            Some(r#"{"state":"submitted","importId":"keep-import","cleanup":{"state":"kept"}}"#),
        )
        .unwrap();
        db.finish_publish_campaign(&campaign, PublishRunOutcome::AllPosted)
            .unwrap();
        (db, path, campaign, assignment, intent)
    }

    fn mark_scheduled(db: &Database, campaign: &str) {
        db.conn()
            .unwrap()
            .execute(
                "UPDATE publish_campaigns SET run_at='2026-09-10T20:00:00' WHERE id=?1",
                [campaign],
            )
            .unwrap();
    }

    #[test]
    fn scheduled_verification_waits_two_minutes_before_the_first_link_check() {
        let (db, path, campaign, _, intent) = review_fixture(0);
        mark_scheduled(&db, &campaign);
        let candidate = db.pending_publish_verifications(10).unwrap().remove(0);
        let intent: serde_json::Value = serde_json::from_str(&intent).unwrap();
        let submitted = DateTime::parse_from_rfc3339(intent["submittedAt"].as_str().unwrap())
            .unwrap()
            .with_timezone(&Utc);
        assert!(!candidate.is_due(submitted + chrono::Duration::seconds(119)));
        assert!(candidate.is_due(submitted + chrono::Duration::seconds(120)));
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn scheduled_verification_keeps_checking_after_thirty_minutes_and_restart() {
        let (db, path, campaign, assignment, intent) = review_fixture(60);
        mark_scheduled(&db, &campaign);
        assert!(db.expire_due_publish_verifications().unwrap().is_empty());
        for (code, delay) in [
            ("linkUnavailable", 300),
            ("readFailed", 300),
            ("readFailed", 600),
            ("readFailed", 1200),
            ("readFailed", 1200),
            ("linkUnavailable", 300),
        ] {
            let candidate = db.pending_publish_verifications(10).unwrap().remove(0);
            assert!(db
                .record_publish_verification_observation(
                    &candidate,
                    "TikTok is still processing",
                    code
                )
                .unwrap());
            let row = db.pending_publish_verifications(10).unwrap().remove(0);
            let e: serde_json::Value =
                serde_json::from_str(row.evidence_json.as_deref().unwrap()).unwrap();
            let status = &e["verificationStatus"];
            let checked =
                DateTime::parse_from_rfc3339(status["checkedAt"].as_str().unwrap()).unwrap();
            let next =
                DateTime::parse_from_rfc3339(status["nextCheckAt"].as_str().unwrap()).unwrap();
            assert_eq!((next - checked).num_seconds(), delay);
            assert_eq!(status["reviewAfterMinutes"], 240);
            assert!(!row.is_due(checked.into()));
            assert!(row.is_due(next.into()));
        }
        drop(db);
        let db = Database::open(&path).unwrap();
        db.interrupt_orphaned_publish_campaigns().unwrap();
        let row = db.pending_publish_verifications(10).unwrap().remove(0);
        assert_eq!(row.effect_intent.as_deref(), Some(intent.as_str()));
        assert!(!row.is_due(Utc::now()));
        assert!(!db
            .claim_publish_assignment_for_posting(&assignment, &intent)
            .unwrap());
        assert!(db.pending_publish_sheet_row(&assignment).unwrap().is_none());
        let url = "https://www.tiktok.com/@fixture/photo/1234567890123456789";
        let evidence = serde_json::json!({"postUrl":url,"publicationVerified":true}).to_string();
        assert!(db
            .record_verified_publish_with_sheet_row(&row, &evidence, url, "bot", &[])
            .unwrap());
        assert!(!db
            .record_verified_publish_with_sheet_row(&row, &evidence, url, "bot", &[])
            .unwrap());
        assert_eq!(db.pending_publish_sheet_rows(10).unwrap().len(), 1);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn scheduled_verification_stops_at_four_hours_and_retains_explicit_link_recovery() {
        for age in [239, 240] {
            let (db, path, campaign, assignment, intent) = review_fixture(age);
            mark_scheduled(&db, &campaign);
            let expired = db.expire_due_publish_verifications().unwrap();
            if age == 239 {
                assert!(expired.is_empty());
                assert_eq!(db.pending_publish_verifications(10).unwrap().len(), 1);
            } else {
                assert_eq!(expired, vec![campaign.clone()]);
                assert!(db.pending_publish_verifications(10).unwrap().is_empty());
                assert!(db.expire_due_publish_verifications().unwrap().is_empty());
                let manual = db
                    .publish_verifications_for_campaign(&campaign, 10)
                    .unwrap()
                    .remove(0);
                let e: serde_json::Value =
                    serde_json::from_str(manual.evidence_json.as_deref().unwrap()).unwrap();
                assert_eq!(e["verificationStatus"]["state"], "needsReview");
                assert_eq!(e["verificationStatus"]["reviewAfterMinutes"], 240);
                assert!(e["verificationStatus"]["reason"]
                    .as_str()
                    .unwrap()
                    .contains("Tự kiểm tra đã dừng"));
                assert_eq!(manual.effect_intent.as_deref(), Some(intent.as_str()));
                assert!(!db
                    .claim_publish_assignment_for_posting(&assignment, &intent)
                    .unwrap());
            }
            drop(db);
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn scheduled_initial_status_preserves_cleanup_and_durable_identity() {
        let (db, path, campaign, assignment, _) = review_fixture(0);
        let original =
            r#"{"post":{"importId":"keep","state":"submitted"},"cleanup":{"state":"kept"}}"#;
        assert_eq!(
            db.with_initial_scheduled_verification(&campaign, &assignment, original)
                .unwrap(),
            original
        );
        mark_scheduled(&db, &campaign);
        let output = db
            .with_initial_scheduled_verification(&campaign, &assignment, original)
            .unwrap();
        let e: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(e["post"]["importId"], "keep");
        assert_eq!(e["cleanup"]["state"], "kept");
        assert_eq!(e["verificationStatus"]["attempts"], 0);
        assert_eq!(e["verificationStatus"]["reviewAfterMinutes"], 240);
        assert!(e["verificationStatus"]["nextCheckAt"].as_str().is_some());
        assert!(db.pending_publish_sheet_row(&assignment).unwrap().is_none());
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn scheduled_verification_resumes_only_legacy_deadline_review() {
        let (db, path, campaign, assignment, intent) = review_fixture(31);
        assert_eq!(
            db.expire_due_publish_verifications().unwrap(),
            vec![campaign.clone()]
        );
        assert!(db.pending_publish_verifications(10).unwrap().is_empty());
        mark_scheduled(&db, &campaign);
        assert_eq!(db.pending_publish_verifications(10).unwrap().len(), 1);
        assert_eq!(
            db.expire_due_publish_verifications().unwrap(),
            vec![campaign.clone()]
        );
        let row = db.pending_publish_verifications(10).unwrap().remove(0);
        assert!(!row.is_due(Utc::now()));
        assert_eq!(row.effect_intent.as_deref(), Some(intent.as_str()));
        assert_eq!(
            db.get_publish_campaign(&campaign)
                .unwrap()
                .unwrap()
                .assignments[0]
                .state,
            crate::PublishCampaignState::Verifying
        );
        assert!(!db
            .claim_publish_assignment_for_posting(&assignment, &intent)
            .unwrap());
        // A review parked for another reason stays parked, including after startup.
        let mut e: serde_json::Value =
            serde_json::from_str(row.evidence_json.as_deref().unwrap()).unwrap();
        e["verificationStatus"]["state"] = serde_json::json!("needsReview");
        e["verificationStatus"]["cause"] = serde_json::json!("submissionIdentityMissing");
        db.update_publish_assignment_state(
            &assignment,
            crate::PublishCampaignState::Uncertain,
            Some("post_verification_needs_review"),
            Some(&e.to_string()),
        )
        .unwrap();
        assert!(db.pending_publish_verifications(10).unwrap().is_empty());
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn verification_review_deadline_stops_automatic_checks_and_preserves_effect_after_restart() {
        let (db, path, campaign, assignment, intent) = review_fixture(31);
        let candidate = db.pending_publish_verifications(10).unwrap().remove(0);
        assert!(db
            .record_publish_verification_pending(
                &candidate,
                "Observed draft; submitted post not identified"
            )
            .unwrap());
        let detail = db.get_publish_campaign(&campaign).unwrap().unwrap();
        assert_eq!(
            detail.assignments[0].state,
            crate::PublishCampaignState::Uncertain
        );
        assert_eq!(
            detail.assignments[0].error_code.as_deref(),
            Some("post_verification_needs_review")
        );
        assert_eq!(
            detail.assignments[0].effect_intent.as_deref(),
            Some(intent.as_str())
        );
        let evidence: serde_json::Value =
            serde_json::from_str(detail.assignments[0].evidence_json.as_deref().unwrap()).unwrap();
        assert_eq!(evidence["importId"], "keep-import");
        assert_eq!(evidence["cleanup"]["state"], "kept");
        assert_eq!(evidence["verificationStatus"]["state"], "needsReview");
        assert_eq!(
            detail.campaign.state,
            crate::PublishCampaignState::Uncertain
        );
        let snapshot = db
            .get_publish_execution_snapshot(&campaign)
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.status, crate::PublishExecutionStatus::Uncertain);
        assert_eq!(snapshot.retry_scope, crate::PublishRetryScope::LinkAndSheet);
        let summary = crate::operation::project_publish_summary(&detail, Some(&snapshot));
        assert_eq!(
            summary.state,
            crate::operation::OperationRunState::Uncertain
        );
        assert_eq!(
            summary.retry_scope,
            Some(crate::PublishRetryScope::LinkAndSheet)
        );
        assert!(!db
            .record_publish_verification_pending(&candidate, "stale")
            .unwrap());
        drop(db);
        let db = Database::open(&path).unwrap();
        db.interrupt_orphaned_publish_campaigns().unwrap();
        assert!(db.pending_publish_verifications(10).unwrap().is_empty());
        assert!(db.has_pending_publish_for_device(&candidate.udid).unwrap());
        assert!(!db
            .claim_publish_assignment_for_posting(&assignment, &intent)
            .unwrap());
        assert!(db.pending_publish_sheet_row(&assignment).unwrap().is_none());
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn verification_review_recent_failure_remains_pending_without_restarting_deadline() {
        let (db, path, campaign, assignment, intent) = review_fixture(1);
        let candidate = db.pending_publish_verifications(10).unwrap().remove(0);
        assert!(db
            .record_publish_verification_pending(&candidate, "temporary read failure")
            .unwrap());
        let detail = db.get_publish_campaign(&campaign).unwrap().unwrap();
        assert_eq!(
            detail.assignments[0].state,
            crate::PublishCampaignState::Verifying
        );
        assert_eq!(
            detail.assignments[0].effect_intent.as_deref(),
            Some(intent.as_str())
        );
        assert_eq!(db.pending_publish_verifications(10).unwrap().len(), 1);
        assert!(!db
            .claim_publish_assignment_for_posting(&assignment, &intent)
            .unwrap());
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn verification_backoff_survives_restart_and_resets_after_a_readable_observation() {
        let (db, path, campaign, _, intent) = review_fixture(1);
        for seconds in [30, 60, 120, 120] {
            let candidate = db.pending_publish_verifications(10).unwrap().remove(0);
            assert!(db
                .record_publish_verification_observation(
                    &candidate,
                    "root inaccessible",
                    "readFailed"
                )
                .unwrap());
            let candidate = db.pending_publish_verifications(10).unwrap().remove(0);
            let evidence: serde_json::Value =
                serde_json::from_str(candidate.evidence_json.as_deref().unwrap()).unwrap();
            let status = &evidence["verificationStatus"];
            let checked =
                DateTime::parse_from_rfc3339(status["checkedAt"].as_str().unwrap()).unwrap();
            let next =
                DateTime::parse_from_rfc3339(status["nextCheckAt"].as_str().unwrap()).unwrap();
            assert_eq!((next - checked).num_seconds(), seconds);
            assert!(!candidate.is_due(checked.into()));
            assert!(candidate.is_due(next.into()));
        }
        drop(db);
        let db = Database::open(&path).unwrap();
        let candidate = db.pending_publish_verifications(10).unwrap().remove(0);
        assert!(!candidate.is_due(Utc::now()));
        assert!(db
            .record_publish_verification_observation(&candidate, "upload pending", "postNotVisible")
            .unwrap());
        let detail = db.get_publish_campaign(&campaign).unwrap().unwrap();
        assert_eq!(
            detail.assignments[0].effect_intent.as_deref(),
            Some(intent.as_str())
        );
        let evidence: serde_json::Value =
            serde_json::from_str(detail.assignments[0].evidence_json.as_deref().unwrap()).unwrap();
        assert_eq!(evidence["verificationStatus"]["attempts"], 5);
        assert_eq!(evidence["verificationStatus"]["readFailures"], 0);
        assert_eq!(
            evidence["verificationStatus"]["reasonCode"],
            "postNotVisible"
        );
        assert!(db
            .pending_publish_sheet_row(&candidate.assignment_id)
            .unwrap()
            .is_none());
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn verification_review_expiry_ignores_device_presence_and_late_link_settles_once() {
        let (db, path, campaign, assignment, intent) = review_fixture(31);
        assert_eq!(
            db.expire_due_publish_verifications().unwrap(),
            vec![campaign.clone()]
        );
        assert!(db.expire_due_publish_verifications().unwrap().is_empty());
        assert!(db.pending_publish_verifications(10).unwrap().is_empty());
        let review = db
            .publish_verifications_for_campaign(&campaign, 10)
            .unwrap()
            .remove(0);
        assert_eq!(review.effect_intent.as_deref(), Some(intent.as_str()));
        let url = "https://www.tiktok.com/@fixture/photo/1234567890123456789";
        let mut proof: serde_json::Value =
            serde_json::from_str(review.evidence_json.as_deref().unwrap()).unwrap();
        proof["postUrl"] = serde_json::json!(url);
        proof["publicationVerified"] = serde_json::json!(true);
        assert!(db
            .record_verified_publish_with_sheet_row(&review, &proof.to_string(), url, "bot", &[])
            .unwrap());
        assert!(!db
            .record_verified_publish_with_sheet_row(&review, &proof.to_string(), url, "bot", &[])
            .unwrap());
        assert!(!db
            .record_publish_verification_pending(&review, "late failed observer")
            .unwrap());
        let row = db
            .get_publish_campaign(&campaign)
            .unwrap()
            .unwrap()
            .assignments
            .remove(0);
        assert_eq!(row.state, crate::PublishCampaignState::Succeeded);
        let evidence: serde_json::Value =
            serde_json::from_str(row.evidence_json.as_deref().unwrap()).unwrap();
        assert_eq!(evidence["verificationStatus"]["state"], "verified");
        assert!(row.error_code.is_none());
        assert_eq!(evidence["importId"], "keep-import");
        assert_eq!(db.pending_publish_sheet_rows(10).unwrap().len(), 1);
        assert!(!db.has_pending_publish_for_device(&review.udid).unwrap());
        assert!(!db
            .claim_publish_assignment_for_posting(&assignment, &intent)
            .unwrap());
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn verification_review_expiry_rolls_back_assignment_and_campaign_when_projection_fails() {
        let (db, path, campaign, _, _) = review_fixture(31);
        let before = db.get_publish_campaign(&campaign).unwrap().unwrap();
        let before_revision = db.publish_campaign_revision(&campaign).unwrap();
        let conn = db.conn().unwrap();
        conn.execute_batch("CREATE TRIGGER fail_review_projection BEFORE INSERT ON publish_execution_snapshots BEGIN SELECT RAISE(ABORT,'projection unavailable'); END;").unwrap();
        let candidate = db.pending_publish_verifications(10).unwrap().remove(0);
        assert!(db
            .record_publish_verification_pending(&candidate, "not identified")
            .is_err());
        let after = db.get_publish_campaign(&campaign).unwrap().unwrap();
        assert_eq!(
            db.pending_publish_verifications(10).unwrap()[0].revision,
            candidate.revision
        );
        assert_eq!(after.assignments[0].state, before.assignments[0].state);
        assert_eq!(
            db.publish_campaign_revision(&campaign).unwrap(),
            before_revision
        );
        assert_eq!(after.campaign.state, before.campaign.state);
        drop(conn);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn verification_review_due_transition_does_not_change_cancelled_assignment() {
        let (db, path, campaign, assignment, _) = review_fixture(31);
        let candidate = db.pending_publish_verifications(10).unwrap().remove(0);
        db.update_publish_assignment_state(
            &assignment,
            crate::PublishCampaignState::Cancelled,
            None,
            None,
        )
        .unwrap();
        assert!(!db
            .record_publish_verification_pending(&candidate, "expired")
            .unwrap());
        assert!(db.expire_due_publish_verifications().unwrap().is_empty());
        assert_eq!(
            db.get_publish_campaign(&campaign)
                .unwrap()
                .unwrap()
                .assignments[0]
                .state,
            crate::PublishCampaignState::Cancelled
        );
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn legacy_verification_without_submission_identity_requires_review_on_first_failure() {
        let (db, path) = fixture();
        let (campaign, assignment) = seed(&db);
        let prior =
            r#"{ "post":{"state":"posted","verdict":"Posted","importId":"historical-import"} }"#;
        db.update_publish_assignment_state(
            &assignment,
            crate::PublishCampaignState::Succeeded,
            None,
            Some(prior),
        )
        .unwrap();
        db.update_publish_campaign_state(&campaign, crate::PublishCampaignState::Succeeded, None)
            .unwrap();
        let candidate = db.pending_publish_verifications(10).unwrap().remove(0);
        assert!(db
            .record_publish_verification_pending(
                &candidate,
                "submission identity missing; review old post manually"
            )
            .unwrap());
        let detail = db.get_publish_campaign(&campaign).unwrap().unwrap();
        assert_eq!(
            detail.assignments[0].state,
            crate::PublishCampaignState::Uncertain
        );
        assert_eq!(
            detail.assignments[0].error_code.as_deref(),
            Some("post_verification_needs_review")
        );
        assert!(detail.assignments[0].effect_intent.is_none());
        let e: serde_json::Value =
            serde_json::from_str(detail.assignments[0].evidence_json.as_deref().unwrap()).unwrap();
        assert_eq!(e["priorEvidenceJson"], prior);
        assert_eq!(e["post"]["importId"], "historical-import");
        assert_eq!(
            e["verificationStatus"]["cause"],
            "submissionIdentityMissing"
        );
        assert!(e["verificationStatus"]["reason"]
            .as_str()
            .unwrap()
            .contains("Thiếu bằng chứng tài khoản hoặc thời điểm"));
        assert_eq!(
            e["verificationStatus"]["observationReason"],
            "submission identity missing; review old post manually"
        );
        assert!(e["verificationStatus"]["submittedAt"].is_null());
        assert!(e["verificationStatus"]["reviewAfterMinutes"].is_null());
        assert_eq!(
            detail.campaign.state,
            crate::PublishCampaignState::Uncertain
        );
        assert_eq!(
            db.get_publish_execution_snapshot(&campaign)
                .unwrap()
                .unwrap()
                .retry_scope,
            crate::PublishRetryScope::LinkAndSheet
        );
        drop(db);
        let db = Database::open(&path).unwrap();
        db.interrupt_orphaned_publish_campaigns().unwrap();
        assert!(db.pending_publish_verifications(10).unwrap().is_empty());
        assert_eq!(
            db.publish_verifications_for_campaign(&campaign, 10)
                .unwrap()
                .len(),
            1
        );
        assert!(db.has_pending_publish_for_device(&candidate.udid).unwrap());
        assert!(!db
            .claim_publish_assignment_for_posting(&assignment, r#"{"effectIntent":"post"}"#)
            .unwrap());
        assert!(db.pending_publish_sheet_rows(10).unwrap().is_empty());
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn legacy_verification_sweep_requires_complete_immutable_identity_without_inferred_age() {
        for identity in [
            serde_json::json!({"effectIntent":"post","expectedAccount":"fixture","submittedAt":"not-a-time"}),
            serde_json::json!({"effectIntent":"post","submittedAt":Utc::now().to_rfc3339()}),
            serde_json::json!({"effectIntent":"post","expectedAccount":"  ","submittedAt":Utc::now().to_rfc3339()}),
        ] {
            let (db, path, campaign, assignment, _) = review_fixture(1);
            let raw = identity.to_string();
            db.conn()
                .unwrap()
                .execute(
                    "UPDATE publish_assignments SET effect_intent=?1 WHERE id=?2",
                    params![raw, assignment],
                )
                .unwrap();
            assert_eq!(
                db.expire_due_publish_verifications().unwrap(),
                vec![campaign.clone()]
            );
            let row = db
                .get_publish_campaign(&campaign)
                .unwrap()
                .unwrap()
                .assignments
                .remove(0);
            assert_eq!(row.state, crate::PublishCampaignState::Uncertain);
            assert_eq!(row.effect_intent.as_deref(), Some(raw.as_str()));
            let e: serde_json::Value =
                serde_json::from_str(row.evidence_json.as_deref().unwrap()).unwrap();
            assert_eq!(
                e["verificationStatus"]["cause"],
                "submissionIdentityMissing"
            );
            assert!(e["verificationStatus"]["reviewAfterMinutes"].is_null());
            assert!(db.pending_publish_verifications(10).unwrap().is_empty());
            drop(db);
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn verification_survives_restart_and_atomically_settles_link_without_reposting() {
        use crate::PublishCampaignState as State;
        let (db, path) = fixture();
        let (campaign, assignment) = seed(&db);
        db.update_publish_campaign_state(&campaign, State::Posting, None)
            .unwrap();
        db.update_publish_assignment_state(&assignment, State::Imported, None, None)
            .unwrap();
        let submitted_at = Utc::now().to_rfc3339();
        let intent_owned = serde_json::json!({"effectIntent":"post","bundleId":"fixture","submittedAt":submitted_at,"expectedAccount":"fixture"}).to_string();
        let intent = intent_owned.as_str();
        assert!(db
            .claim_publish_assignment_for_posting(&assignment, intent)
            .unwrap());
        let prior = "{ \"state\": \"submitted\", \"importId\": \"import-1\" }";
        db.update_publish_assignment_state(
            &assignment,
            State::Verifying,
            Some("post_verification_pending"),
            Some(prior),
        )
        .unwrap();
        assert_eq!(
            db.finish_publish_campaign(&campaign, PublishRunOutcome::AllPosted)
                .unwrap(),
            Some(State::Verifying)
        );
        assert_eq!(
            db.reconciled_publish_execution_status(&campaign).unwrap(),
            (
                crate::PublishExecutionStatus::Partial,
                crate::PublishRetryScope::LinkAndSheet
            )
        );
        drop(db);
        let db = Database::open(&path).unwrap();
        db.interrupt_orphaned_publish_campaigns().unwrap();
        assert_eq!(
            db.publish_campaign_state(&campaign).unwrap(),
            Some(State::Verifying)
        );
        let candidates = db.pending_publish_verifications(10).unwrap();
        assert_eq!(candidates.len(), 1);
        let candidate = &candidates[0];
        assert_eq!(candidate.effect_intent.as_deref(), Some(intent));
        assert!(db.has_pending_publish_for_device(&candidate.udid).unwrap());
        assert!(!db
            .claim_publish_assignment_for_posting(&assignment, intent)
            .unwrap());
        assert!(db
            .record_publish_verification_pending(candidate, "Post is being processed")
            .unwrap());
        let url = "https://www.tiktok.com/@fixture/photo/1234567890123456789";
        let evidence = serde_json::json!({"postUrl":url,"publicationVerified":true}).to_string();
        assert!(
            !db.record_verified_publish_with_sheet_row(
                candidate,
                &evidence,
                url,
                "bot",
                &["partner".into()]
            )
            .unwrap(),
            "stale observation must lose CAS"
        );
        assert!(db.pending_publish_sheet_row(&assignment).unwrap().is_none());
        let refreshed = db.pending_publish_verifications(10).unwrap().remove(0);
        let foreign_url = "https://www.tiktok.com/@other/photo/1234567890123456789";
        let foreign_evidence =
            serde_json::json!({"postUrl":foreign_url,"publicationVerified":true}).to_string();
        assert!(db
            .record_verified_publish_with_sheet_row(
                &refreshed,
                &foreign_evidence,
                foreign_url,
                "bot",
                &[]
            )
            .is_err());
        assert!(db.pending_publish_sheet_row(&assignment).unwrap().is_none());
        assert!(db
            .record_verified_publish_with_sheet_row(
                &refreshed,
                &evidence,
                url,
                "bot",
                &["partner".into()]
            )
            .unwrap());
        assert!(!db
            .record_verified_publish_with_sheet_row(&refreshed, &evidence, url, "bot", &[])
            .unwrap());
        let detail = db.get_publish_campaign(&campaign).unwrap().unwrap();
        assert_eq!(detail.campaign.state, State::Succeeded);
        let stored: serde_json::Value =
            serde_json::from_str(detail.assignments[0].evidence_json.as_deref().unwrap()).unwrap();
        assert_eq!(stored["priorEvidenceJson"], prior);
        assert_eq!(stored["submittedAt"], submitted_at);
        assert_eq!(
            db.pending_publish_sheet_row(&assignment)
                .unwrap()
                .unwrap()
                .posted_at
                .as_deref(),
            Some(submitted_at.as_str())
        );
        let snapshot = db
            .get_publish_execution_snapshot(&campaign)
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.status, crate::PublishExecutionStatus::Partial);
        assert_eq!(snapshot.retry_scope, crate::PublishRetryScope::SheetOnly);
        assert_eq!(
            snapshot.report_json["source"],
            "post_verification_reconciliation"
        );
        assert_eq!(
            db.pending_publish_sheet_row(&assignment)
                .unwrap()
                .unwrap()
                .post_url,
            url
        );
        assert!(!db.has_pending_publish_for_device(&refreshed.udid).unwrap());
        assert!(db.pending_publish_verifications(10).unwrap().is_empty());
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn verification_excludes_active_posts_and_rolls_back_state_when_sheet_write_fails() {
        use crate::PublishCampaignState as State;
        let (db, path) = fixture();
        let (campaign, assignment) = seed(&db);
        db.update_publish_campaign_state(&campaign, State::Posting, None)
            .unwrap();
        db.update_publish_assignment_state(&assignment, State::Imported, None, None)
            .unwrap();
        assert!(db
            .claim_publish_assignment_for_posting(&assignment, r#"{"effectIntent":"post"}"#)
            .unwrap());
        assert!(
            db.pending_publish_verifications(10).unwrap().is_empty(),
            "active Posting is owned by the run"
        );
        db.update_publish_assignment_state(
            &assignment,
            State::Verifying,
            None,
            Some(r#"{"state":"submitted"}"#),
        )
        .unwrap();
        let candidate = db.pending_publish_verifications(10).unwrap().remove(0);
        let url = "https://www.tiktok.com/@fixture/photo/1234567890123456789";
        let evidence = serde_json::json!({"postUrl":url,"publicationVerified":true}).to_string();
        assert!(db
            .record_verified_publish_with_sheet_row(&candidate, &evidence, url, "", &[])
            .is_err());
        assert_eq!(
            db.get_publish_campaign(&campaign)
                .unwrap()
                .unwrap()
                .assignments[0]
                .state,
            State::Verifying
        );
        assert!(db.pending_publish_sheet_row(&assignment).unwrap().is_none());
        assert!(db
            .get_publish_execution_snapshot(&campaign)
            .unwrap()
            .is_none());
        db.update_publish_assignment_state(&assignment, State::Posting, None, None)
            .unwrap();
        assert!(!db
            .record_verified_publish_with_sheet_row(&candidate, &evidence, url, "bot", &[])
            .unwrap());
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn historical_feed_success_is_recovered_without_erasing_cancel_or_original_capture() {
        use crate::PublishCampaignState as State;
        let (db, path) = fixture();
        let (campaign, assignment) = seed(&db);
        let prior = "{\n  \"state\": \"posted\", \"verdict\": \"Posted\"\n}";
        db.update_publish_assignment_state(&assignment, State::Succeeded, None, Some(prior))
            .unwrap();
        db.update_publish_campaign_state(&campaign, State::Cancelled, None)
            .unwrap();
        let candidate = db.pending_publish_verifications(1).unwrap().remove(0);
        let url = "https://www.tiktok.com/@fixture/photo/1234567890123456789";
        let evidence = serde_json::json!({"postUrl":url,"publicationVerified":true}).to_string();
        assert!(db
            .record_verified_publish_with_sheet_row(&candidate, &evidence, url, "bot", &[])
            .unwrap());
        assert_eq!(
            db.publish_campaign_state(&campaign).unwrap(),
            Some(State::Cancelled)
        );
        let detail = db.get_publish_campaign(&campaign).unwrap().unwrap();
        let stored: serde_json::Value =
            serde_json::from_str(detail.assignments[0].evidence_json.as_deref().unwrap()).unwrap();
        assert_eq!(stored["priorEvidenceJson"], prior);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn orphaned_post_under_cancelled_campaign_stays_recoverable_and_never_reposts() {
        use crate::PublishCampaignState as State;
        let (db, path) = fixture();
        let (campaign, assignment) = seed(&db);
        db.update_publish_campaign_state(&campaign, State::Posting, None)
            .unwrap();
        db.update_publish_assignment_state(&assignment, State::Imported, None, None)
            .unwrap();
        let intent = r#"{"effectIntent":"post"}"#;
        assert!(db
            .claim_publish_assignment_for_posting(&assignment, intent)
            .unwrap());
        db.update_publish_campaign_state(&campaign, State::Cancelled, None)
            .unwrap();
        db.interrupt_orphaned_publish_campaigns().unwrap();
        assert_eq!(
            db.publish_campaign_state(&campaign).unwrap(),
            Some(State::Cancelled)
        );
        assert_eq!(
            db.get_publish_campaign(&campaign)
                .unwrap()
                .unwrap()
                .assignments[0]
                .state,
            State::Uncertain
        );
        assert_eq!(db.pending_publish_verifications(10).unwrap().len(), 1);
        assert!(!db
            .claim_publish_assignment_for_posting(&assignment, intent)
            .unwrap());
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn disabled_sheet_keeps_link_without_outbox_and_reconciles_complete_after_restart() {
        let (db, path) = fixture();
        let (campaign, assignment) = seed_with_sheet(&db, false);
        let link = "https://www.tiktok.com/@fixture/photo/1234567890123456789";
        let evidence = serde_json::json!({"post":{"postUrl":link}}).to_string();
        db.record_publish_success_with_sheet_row(
            &assignment,
            &evidence,
            &campaign,
            link,
            "bot",
            &[],
        )
        .unwrap();
        assert!(db.pending_publish_sheet_row(&assignment).unwrap().is_none());
        db.queue_publish_sheet_row(&assignment, &campaign, link, "bot", &[])
            .unwrap();
        assert!(db.pending_publish_sheet_row(&assignment).unwrap().is_none());
        drop(db);
        let db = Database::open(&path).unwrap();
        assert!(
            !db.publish_campaign_request(&campaign)
                .unwrap()
                .unwrap()
                .sheet_enabled
        );
        assert_eq!(
            db.reconciled_publish_execution_status(&campaign).unwrap(),
            (
                crate::PublishExecutionStatus::Complete,
                crate::PublishRetryScope::None
            )
        );
        assert_eq!(
            db.get_publish_campaign(&campaign)
                .unwrap()
                .unwrap()
                .assignments[0]
                .evidence_json
                .as_deref(),
            Some(evidence.as_str())
        );
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    /// The one row currently owed, and its version.
    fn owed(db: &Database) -> Vec<SheetOutboxRow> {
        db.pending_publish_sheet_rows(10).expect("read back")
    }

    fn set_publish_states(
        db: &Database,
        campaign_id: &str,
        assignment_id: &str,
        campaign_state: crate::publish::PublishCampaignState,
        assignment_state: crate::publish::PublishCampaignState,
        evidence: Option<&str>,
    ) {
        db.update_publish_assignment_state(assignment_id, assignment_state, None, evidence)
            .expect("set assignment state");
        db.update_publish_campaign_state(campaign_id, campaign_state, None)
            .expect("set campaign state");
    }

    #[test]
    fn reconciliation_status_covers_terminal_ambiguous_failed_and_legacy_rows() {
        use crate::publish::PublishCampaignState as CampaignState;
        use crate::publish_runtime::{
            PublishExecutionStatus as Status, PublishRetryScope as Scope,
        };

        let (db, path) = fixture();
        for campaign_state in [CampaignState::Cancelled, CampaignState::Missed] {
            let (campaign, assignment) = seed(&db);
            set_publish_states(
                &db,
                &campaign,
                &assignment,
                campaign_state,
                CampaignState::Ready,
                None,
            );
            assert_eq!(
                db.reconciled_publish_execution_status(&campaign)
                    .expect("reconcile terminal campaign"),
                (Status::Partial, Scope::None)
            );
        }

        for assignment_state in [CampaignState::Posting, CampaignState::Uncertain] {
            let (campaign, assignment) = seed(&db);
            set_publish_states(
                &db,
                &campaign,
                &assignment,
                CampaignState::Succeeded,
                assignment_state,
                None,
            );
            assert_eq!(
                db.reconciled_publish_execution_status(&campaign)
                    .expect("reconcile ambiguous assignment"),
                (Status::Uncertain, Scope::None)
            );
        }

        let (failed_campaign, failed_assignment) = seed(&db);
        set_publish_states(
            &db,
            &failed_campaign,
            &failed_assignment,
            CampaignState::FailedBeforeDispatch,
            CampaignState::FailedBeforeDispatch,
            None,
        );
        assert_eq!(
            db.reconciled_publish_execution_status(&failed_campaign)
                .expect("reconcile retryable failure"),
            (Status::Partial, Scope::FullPipeline)
        );

        // A legacy succeeded assignment may have a canonical link but no outbox row. It may
        // retry link/Sheet reconciliation, never the public Post.
        let (legacy_campaign, legacy_assignment) = seed(&db);
        let link = "https://www.tiktok.com/@fixture/video/7400000000000000001";
        set_publish_states(
            &db,
            &legacy_campaign,
            &legacy_assignment,
            CampaignState::Succeeded,
            CampaignState::Succeeded,
            Some(&serde_json::json!({"post": {"postUrl": link}}).to_string()),
        );
        assert_eq!(
            db.reconciled_publish_execution_status(&legacy_campaign)
                .expect("reconcile legacy row without outbox"),
            (Status::Partial, Scope::LinkAndSheet)
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn reconciliation_status_moves_pending_and_failed_sheet_rows_to_complete_only_when_sent() {
        use crate::publish::PublishCampaignState as CampaignState;
        use crate::publish_runtime::{
            PublishExecutionStatus as Status, PublishRetryScope as Scope,
        };

        let (db, path) = fixture();
        let (campaign, assignment) = seed(&db);
        let link = "https://www.tiktok.com/@fixture/video/7400000000000000001";
        db.record_publish_success_with_sheet_row(
            &assignment,
            &serde_json::json!({"post": {"postUrl": link}}).to_string(),
            &campaign,
            link,
            "bot",
            &[],
        )
        .expect("record post and outbox");
        db.update_publish_campaign_state(&campaign, CampaignState::Succeeded, None)
            .expect("settle campaign");
        assert_eq!(
            db.reconciled_publish_execution_status(&campaign)
                .expect("reconcile pending row"),
            (Status::Partial, Scope::SheetOnly)
        );

        let revision = owed(&db)[0].revision;
        assert!(db
            .mark_publish_sheet_failed(&assignment, revision, "fixture rejection")
            .expect("mark failed"));
        assert_eq!(
            db.reconciled_publish_execution_status(&campaign)
                .expect("reconcile failed row"),
            (Status::Partial, Scope::SheetOnly)
        );
        assert!(db
            .mark_publish_sheet_sent(&assignment, revision)
            .expect("mark sent"));
        assert_eq!(
            db.reconciled_publish_execution_status(&campaign)
                .expect("reconcile sent row"),
            (Status::Complete, Scope::None)
        );

        let _ = std::fs::remove_file(path);
    }

    /// A queued row comes back with the names in the order the workbook had them.
    #[test]
    fn a_queued_row_keeps_the_partner_order_the_workbook_had() {
        let (db, path) = fixture();
        let (campaign, assignment) = seed(&db);
        let partners = vec!["Quán B".to_string(), "Quán A".to_string()];
        db.queue_publish_sheet_row(
            &assignment,
            &campaign,
            "https://www.tiktok.com/@a/photo/1",
            "bot",
            &partners,
        )
        .expect("queue");

        let rows = owed(&db);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].post_url, "https://www.tiktok.com/@a/photo/1");
        assert_eq!(rows[0].poster, "bot");
        // Not sorted: the names go across columns K, L, M… in this order, so reordering them
        // here would silently rearrange the operator's sheet.
        assert_eq!(rows[0].partners, partners);
        assert_eq!(rows[0].attempts, 0);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn outbox_state_distinguishes_missing_pending_failed_and_sent() {
        let (db, path) = fixture();
        let (campaign, assignment) = seed(&db);
        assert_eq!(
            db.publish_sheet_outbox_state(&assignment)
                .expect("missing state"),
            None
        );

        db.queue_publish_sheet_row(&assignment, &campaign, "https://a/1", "bot", &[])
            .expect("queue");
        assert_eq!(
            db.publish_sheet_outbox_state(&assignment)
                .expect("pending state"),
            Some(SheetOutboxState::Pending)
        );
        let revision = owed(&db)[0].revision;
        assert!(db
            .mark_publish_sheet_failed(&assignment, revision, "offline")
            .expect("fail row"));
        assert_eq!(
            db.publish_sheet_outbox_state(&assignment)
                .expect("failed state"),
            Some(SheetOutboxState::Failed)
        );
        assert!(db
            .mark_publish_sheet_sent(&assignment, revision)
            .expect("send row"));
        assert_eq!(
            db.publish_sheet_outbox_state(&assignment)
                .expect("sent state"),
            Some(SheetOutboxState::Sent)
        );

        drop(db);
        std::fs::remove_file(path).expect("remove fixture database");
    }

    /// **A row the sheet already has is never re-opened, by any route.**
    ///
    /// Column D is pasted by hand nowhere — every link in it came from here — so a second
    /// write puts the same link on two rows and nothing in the sheet says which is the
    /// duplicate. Three routes could reopen it and all three are closed: re-queuing the
    /// assignment, a late failure from a retry, and the pending sweep picking it up again.
    #[test]
    fn a_row_the_sheet_already_has_is_never_sent_a_second_time() {
        let (db, path) = fixture();
        let (campaign, assignment) = seed(&db);
        db.queue_publish_sheet_row(&assignment, &campaign, "https://a/1", "bot", &[])
            .expect("queue");
        let revision = owed(&db)[0].revision;
        assert!(db
            .mark_publish_sheet_sent(&assignment, revision)
            .expect("sent"));
        assert!(
            owed(&db).is_empty(),
            "a sent row must not come back out of the queue"
        );
        assert!(
            db.pending_publish_sheet_row(&assignment)
                .expect("exact lookup")
                .is_none(),
            "an exact retry lookup must not resurrect a sent row"
        );

        // Route 1: the post path runs again and re-queues.
        db.queue_publish_sheet_row(&assignment, &campaign, "https://a/2", "bot", &[])
            .expect("re-queue");
        assert!(
            owed(&db).is_empty(),
            "re-queuing an already-sent assignment reopened it"
        );

        // Route 2: a late error arrives for a push that had in fact landed.
        assert!(!db
            .mark_publish_sheet_failed(&assignment, revision, "timeout")
            .expect("late failure"));
        assert!(
            owed(&db).is_empty(),
            "a late failure reopened a row the sheet already has"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_exact_retry_lookup_reads_only_the_named_assignment() {
        let (db, path) = fixture();
        let (campaign, first) = seed(&db);
        let (other_campaign, second) = seed(&db);
        db.queue_publish_sheet_row(&first, &campaign, "https://a/1", "bot", &[])
            .expect("first row");
        db.queue_publish_sheet_row(&second, &other_campaign, "https://a/2", "bot", &[])
            .expect("second row");

        let row = db
            .pending_publish_sheet_row(&second)
            .expect("exact lookup")
            .expect("second row remains owed");
        assert_eq!(row.assignment_id, second);
        assert_eq!(row.post_url, "https://a/2");
        let _ = std::fs::remove_file(path);
    }

    /// **A completion names the version it delivered, or it is refused.**
    ///
    /// The race, and it has a real consequence: a sweep reads the row, a re-queue replaces the
    /// URL, the sweep finishes and marks the row sent — and the *newer* link, the one nobody
    /// has pushed, is marked delivered. It never travels, and nothing anywhere says so.
    #[test]
    fn a_completion_for_a_stale_version_is_refused_and_the_newer_link_stays_owed() {
        let (db, path) = fixture();
        let (campaign, assignment) = seed(&db);
        db.queue_publish_sheet_row(&assignment, &campaign, "https://a/1", "bot", &[])
            .expect("queue");
        // What a sweep would have read.
        let in_flight = owed(&db)[0].clone();

        // And what a re-queue does to it while the request is out.
        db.queue_publish_sheet_row(&assignment, &campaign, "https://a/2", "bot", &[])
            .expect("re-queue");
        let current = owed(&db)[0].clone();
        assert_ne!(
            current.revision, in_flight.revision,
            "the version must move"
        );

        assert!(
            !db.mark_publish_sheet_sent(&assignment, in_flight.revision)
                .expect("stale completion"),
            "a completion for the old URL marked the new one delivered"
        );
        let still = owed(&db);
        assert_eq!(still.len(), 1, "the newer link stopped being owed");
        assert_eq!(still[0].post_url, "https://a/2");

        // The completion that names the current version does land.
        assert!(db
            .mark_publish_sheet_sent(&assignment, current.revision)
            .expect("current completion"));
        assert!(owed(&db).is_empty());
        let _ = std::fs::remove_file(path);
    }

    /// A failed push is still owed, and carries a message a person can read.
    #[test]
    fn a_failed_push_stays_in_the_queue_with_its_reason() {
        let (db, path) = fixture();
        let (campaign, assignment) = seed(&db);
        db.queue_publish_sheet_row(&assignment, &campaign, "https://a/1", "bot", &[])
            .expect("queue");
        let revision = owed(&db)[0].revision;
        assert!(db
            .mark_publish_sheet_failed(&assignment, revision, "webhook trả 500")
            .expect("failed"));

        let rows = owed(&db);
        assert_eq!(rows.len(), 1, "a failed push is still owed to the sheet");
        assert_eq!(rows[0].attempts, 1);
        assert_eq!(rows[0].last_error.as_deref(), Some("webhook trả 500"));

        // And it can still succeed afterwards, at the version it is now.
        assert!(db
            .mark_publish_sheet_sent(&assignment, rows[0].revision)
            .expect("sent"));
        assert!(owed(&db).is_empty());
        let _ = std::fs::remove_file(path);
    }

    /// **One assignment can owe the sheet at most one row, whatever the caller does.**
    #[test]
    fn re_queuing_replaces_the_pending_row_rather_than_adding_one() {
        let (db, path) = fixture();
        let (campaign, assignment) = seed(&db);
        db.queue_publish_sheet_row(&assignment, &campaign, "https://a/1", "bot", &["A".into()])
            .expect("queue");
        let revision = owed(&db)[0].revision;
        assert!(db
            .mark_publish_sheet_failed(&assignment, revision, "mạng đứt")
            .expect("failed"));
        db.queue_publish_sheet_row(&assignment, &campaign, "https://a/2", "bot", &["B".into()])
            .expect("re-queue");

        let rows = owed(&db);
        assert_eq!(rows.len(), 1, "a second row was created for one post");
        assert_eq!(rows[0].post_url, "https://a/2", "the newer link wins");
        assert_eq!(rows[0].partners, vec!["B".to_string()]);
        assert!(
            rows[0].last_error.is_none(),
            "re-queuing must clear the stale reason, or an operator reads an old failure"
        );
        let _ = std::fs::remove_file(path);
    }

    /// **One link, one row — even under a different assignment id.**
    ///
    /// The assignment key alone was not enough: a restored or recreated campaign hands a new
    /// id to an already-captured live link, both rows satisfy the primary key, and the same
    /// URL lands in column D twice. Both of them look entirely ordinary.
    #[test]
    fn the_same_link_cannot_be_owed_twice_under_two_assignment_ids() {
        let (db, path) = fixture();
        let (campaign, first) = seed(&db);
        let (other_campaign, second) = seed(&db);
        db.queue_publish_sheet_row(&first, &campaign, "https://a/1", "bot", &[])
            .expect("queue");
        let clash = db.queue_publish_sheet_row(&second, &other_campaign, "https://a/1", "bot", &[]);
        assert!(clash.is_err(), "the same link was owed twice");
        assert_eq!(owed(&db).len(), 1);
        let _ = std::fs::remove_file(path);
    }

    /// **Deleting the campaign must not delete the obligation.**
    ///
    /// The row is about a carousel that is live on a real account and cannot be taken down
    /// from here. Cascading it away removes the only record that the sheet is still owed a
    /// link — nothing can ever add it, and its absence is exactly what would encourage
    /// publishing the post again.
    #[test]
    fn an_obligation_outlives_the_campaign_it_came_from() {
        let (db, path) = fixture();
        let (campaign, assignment) = seed(&db);
        db.queue_publish_sheet_row(&assignment, &campaign, "https://a/1", "bot", &["A".into()])
            .expect("queue");
        let revision = owed(&db)[0].revision;
        assert!(db
            .mark_publish_sheet_failed(&assignment, revision, "webhook đang chết")
            .expect("failed"));

        db.conn()
            .expect("conn")
            .execute(
                "DELETE FROM publish_campaigns WHERE id=?1",
                params![&campaign],
            )
            .expect("the operator removes the campaign");

        let rows = owed(&db);
        assert_eq!(
            rows.len(),
            1,
            "the sheet obligation for a live post was cascaded away"
        );
        assert_eq!(rows[0].post_url, "https://a/1");
        assert_eq!(rows[0].partners, vec!["A".to_string()]);
        let _ = std::fs::remove_file(path);
    }

    /// **The post and its obligation commit together, or neither does.**
    ///
    /// Two separate statements leave a window: the assignment is marked `succeeded`, the
    /// process loses power, and nothing on disk says the sheet is owed a link for a post that
    /// is already live. Migration 17's comment claimed one transaction; it took two.
    #[test]
    fn recording_a_published_post_and_its_sheet_row_is_one_transaction() {
        let (db, path) = fixture();
        let (campaign, assignment) = seed(&db);
        db.record_publish_success_with_sheet_row(
            &assignment,
            r#"{"postUrl":"https://a/1"}"#,
            &campaign,
            "https://a/1",
            "bot",
            &["Quán A".into()],
        )
        .expect("record both");

        assert_eq!(
            db.get_publish_campaign(&campaign)
                .expect("read back")
                .expect("campaign exists")
                .assignments[0]
                .state,
            crate::publish::PublishCampaignState::Succeeded
        );
        assert_eq!(owed(&db)[0].post_url, "https://a/1");

        // And neither lands when the pair cannot: a link already owed under another
        // assignment makes the whole write fail, so the post is not recorded as succeeded on
        // the strength of half a transaction.
        let (other_campaign, second) = seed(&db);
        assert!(db
            .record_publish_success_with_sheet_row(
                &second,
                r#"{"postUrl":"https://a/1"}"#,
                &other_campaign,
                "https://a/1",
                "bot",
                &[],
            )
            .is_err());
        assert_ne!(
            db.get_publish_campaign(&other_campaign)
                .expect("read back")
                .expect("campaign exists")
                .assignments[0]
                .state,
            crate::publish::PublishCampaignState::Succeeded,
            "the assignment was marked succeeded while its obligation was rolled back"
        );
        let _ = std::fs::remove_file(path);
    }

    /// A row whose JSON is unreadable still travels, carrying the link.
    #[test]
    fn a_row_with_unreadable_partners_still_carries_its_link() {
        let (db, path) = fixture();
        let (campaign, assignment) = seed(&db);
        db.queue_publish_sheet_row(&assignment, &campaign, "https://a/1", "bot", &[])
            .expect("queue");
        db.conn()
            .expect("conn")
            .execute(
                "UPDATE publish_sheet_outbox SET partners_json='not json' WHERE assignment_id=?1",
                params![&assignment],
            )
            .expect("corrupt the column");

        let rows = owed(&db);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].post_url, "https://a/1");
        assert!(rows[0].partners.is_empty());
        let _ = std::fs::remove_file(path);
    }

    /// **Empty text is refused by the schema, not stored forever.**
    ///
    /// An empty `post_url` is a row that is eligible for every sweep and rejected by the
    /// script every time — a queue that never drains and never says why.
    #[test]
    fn a_blank_link_or_poster_cannot_be_queued_at_all() {
        let (db, path) = fixture();
        let (campaign, assignment) = seed(&db);
        assert!(db
            .queue_publish_sheet_row(&assignment, &campaign, "   ", "bot", &[])
            .is_err());
        assert!(db
            .queue_publish_sheet_row(&assignment, &campaign, "https://a/1", " ", &[])
            .is_err());
        assert!(owed(&db).is_empty());
        let _ = std::fs::remove_file(path);
    }

    /// An enormous limit does not become "no limit".
    ///
    /// `usize::MAX as i64` wraps negative, and SQLite reads a negative LIMIT as unbounded —
    /// so the guard against materialising the whole outbox would have been the thing that
    /// materialised it.
    #[test]
    fn an_absurd_limit_is_clamped_rather_than_wrapping_into_no_limit() {
        let (db, path) = fixture();
        let (campaign, assignment) = seed(&db);
        db.queue_publish_sheet_row(&assignment, &campaign, "https://a/1", "bot", &[])
            .expect("queue");
        assert_eq!(
            db.pending_publish_sheet_rows(usize::MAX)
                .expect("must not fail")
                .len(),
            1
        );
        assert!(db.pending_publish_sheet_rows(0).expect("read").is_empty());
        let _ = std::fs::remove_file(path);
    }
}
