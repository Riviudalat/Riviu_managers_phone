//! A campaign owns one execution token; every worker write also checks the child revision.
use super::publish_sheet::queue_sheet_row;
use super::*;

#[derive(Debug, Clone)]
pub struct PublishPipelineRun {
    pub campaign_id: String,
    pub token: String,
}

impl Database {
    /// The approval gate in front of every fresh claim (PLAN §3): the schedule timer and the
    /// manual, scheduled and orchestration execution paths all call it before
    /// [`Self::claim_publish_pipeline`], and dispatcher admission repeats the same per-phone
    /// check atomically. It reads the frozen approval and the current account of every phone
    /// that has not crossed the effect boundary, and refuses the whole receiver set when any
    /// one is stale: a smaller, silently re-chosen set is never dispatched. Submitted and
    /// uncertain rows are not fresh work and are not looked at here.
    pub fn publish_approval_refusal(&self, campaign_id: &str) -> anyhow::Result<Option<String>> {
        let conn = self.conn()?;
        let Some(raw) = conn
            .query_row(
                "SELECT request_json FROM publish_campaigns WHERE id=?1",
                [campaign_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        else {
            return Ok(None);
        };
        let request: crate::PublishCampaignRequest = serde_json::from_str(&raw)?;
        let mut statement = conn.prepare(
            "SELECT a.udid,COALESCE((SELECT handle FROM device_meta WHERE udid=a.udid),'')
             FROM publish_assignments a WHERE a.campaign_id=?1 AND a.effect_intent IS NULL
             AND a.state IN ('queued','scheduled','ready','imported','failed_before_dispatch')
             ORDER BY a.ordinal",
        )?;
        let fresh = statement
            .query_map([campaign_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        if fresh.is_empty() {
            return Ok(None);
        }
        if request.approval.is_none() {
            return Ok(crate::publish::publish_approval_refusal(&request, "", ""));
        }
        let refusals: Vec<String> = fresh
            .iter()
            .filter_map(|(udid, handle)| {
                crate::publish::publish_approval_refusal(&request, udid, handle)
            })
            .collect();
        Ok((!refusals.is_empty()).then(|| refusals.join("; ")))
    }

    /// Settle a refusal from [`Self::publish_approval_refusal`] before any device work: the
    /// fresh rows and the campaign become `failed_before_dispatch` with the reason kept in
    /// `error_code`. A later retry is checked against the same frozen approval and refused
    /// again; only a newly reviewed campaign can carry changed inputs. Rows past the effect
    /// boundary are not touched.
    pub fn refuse_publish_for_approval(
        &self,
        campaign_id: &str,
        reason: &str,
    ) -> anyhow::Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = Utc::now().to_rfc3339();
        let code = format!("publish_approval_changed: {reason}");
        tx.execute(
            "UPDATE publish_assignments SET state='failed_before_dispatch',error_code=?2,revision=revision+1,updated_at=?3
             WHERE campaign_id=?1 AND effect_intent IS NULL
             AND state IN ('queued','scheduled','ready','imported','failed_before_dispatch')",
            params![campaign_id, code, now],
        )?;
        tx.execute(
            "UPDATE publish_campaigns SET state='failed_before_dispatch',error_code=?2,revision=revision+1,updated_at=?3
             WHERE id=?1 AND state IN ('queued','scheduled','ready','imported','failed_before_dispatch')",
            params![campaign_id, code, now],
        )?;
        // The approval is invalidated: its account locks end with it.
        release_settled_account_locks(&tx, Some(campaign_id), None)?;
        tx.commit()?;
        Ok(())
    }

    /// Effect admission (PLAN §3, review R2-F2). Called with the device lease held and after
    /// every capacity wait, immediately before the first transfer, app-session or composer
    /// effect. One short transaction compares the frozen approval with the current account of
    /// every phone in this campaign that has not crossed the effect boundary; nothing is held
    /// across device I/O. A passing admission stays valid after it returns because every phone
    /// of the set is account-locked until the set settles (`lock_campaign_accounts`, enforced by
    /// `ensure_account_edit_allowed`), so this comparison is the backstop for a write that
    /// bypassed the account writers. On any change the whole remaining pre-dispatch set is
    /// settled review-required and the caller gets a terminal refusal, so the old approval is
    /// never retried. A row already past the effect boundary only reconciles and is admitted.
    pub fn admit_publish_effect(
        &self,
        assignment_id: &str,
    ) -> anyhow::Result<Result<(), crate::publish_recovery::RecoveryFailure>> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (campaign_id, raw, intent): (String, String, Option<String>) = tx.query_row(
            "SELECT a.campaign_id,c.request_json,a.effect_intent FROM publish_assignments a
             JOIN publish_campaigns c ON c.id=a.campaign_id WHERE a.id=?1",
            [assignment_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if intent.is_some() {
            return Ok(Ok(()));
        }
        let request: crate::PublishCampaignRequest = serde_json::from_str(&raw)?;
        let remaining = {
            let mut statement = tx.prepare(
                "SELECT a.udid,COALESCE((SELECT handle FROM device_meta WHERE udid=a.udid),'')
                 FROM publish_assignments a WHERE a.campaign_id=?1 AND a.effect_intent IS NULL
                 AND a.state NOT IN ('cancelled','missed','succeeded','uncertain','verifying')
                 ORDER BY a.ordinal",
            )?;
            let rows = statement
                .query_map([&campaign_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        let refusals: Vec<String> = if request.approval.is_none() {
            crate::publish::publish_approval_refusal(&request, "", "")
                .into_iter()
                .collect()
        } else {
            remaining
                .iter()
                .filter_map(|(udid, handle)| {
                    crate::publish::publish_approval_refusal(&request, udid, handle)
                })
                .collect()
        };
        if refusals.is_empty() {
            tx.commit()?;
            return Ok(Ok(()));
        }
        let message = format!("publish_approval_changed: {}", refusals.join("; "));
        let now = Utc::now();
        // Siblings waiting for their own step stop here too: queued jobs finish, recovery
        // records fail, rows become failed_before_dispatch. This assignment is settled by the
        // caller's terminal failure; rows past the effect boundary are untouched.
        tx.execute(
            "UPDATE publish_dispatch_jobs SET state='finished',reason='publish_approval_changed',
             finished_at_ms=?2,revision=revision+1 WHERE state='queued' AND assignment_id<>?3 AND assignment_id IN
             (SELECT id FROM publish_assignments WHERE campaign_id=?1 AND effect_intent IS NULL)",
            params![campaign_id, now.timestamp_millis(), assignment_id],
        )?;
        tx.execute(
            "UPDATE publish_recovery_state SET payload=json_set(payload,'$.state','failed','$.lastError',?2,
             '$.nextRetryAt',NULL,'$.reconnectDeadline',NULL) WHERE assignment_id<>?3 AND assignment_id IN
             (SELECT id FROM publish_assignments WHERE campaign_id=?1 AND effect_intent IS NULL
              AND state IN ('queued','scheduled','ready','imported','failed_before_dispatch'))",
            params![campaign_id, message, assignment_id],
        )?;
        tx.execute(
            "UPDATE publish_assignments SET state='failed_before_dispatch',error_code=?2,revision=revision+1,updated_at=?3
             WHERE campaign_id=?1 AND id<>?4 AND effect_intent IS NULL
             AND state IN ('queued','scheduled','ready','imported','failed_before_dispatch')",
            params![campaign_id, message, now.to_rfc3339(), assignment_id],
        )?;
        tx.commit()?;
        Ok(Err(crate::publish_recovery::RecoveryFailure::new(
            "publish_approval_changed",
            crate::publish_recovery::FailureKind::Terminal,
            message,
        )))
    }

    pub fn claim_publish_pipeline(
        &self,
        campaign_id: &str,
    ) -> anyhow::Result<Option<PublishPipelineRun>> {
        self.ensure_publish_recovery_ready()?;
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let request: Option<String> = tx
            .query_row(
                "SELECT request_json FROM publish_campaigns WHERE id=?1",
                [campaign_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(request) = request {
            serde_json::from_str::<crate::PublishCampaignRequest>(&request)?
                .network
                .ensure_implemented()?;
        }
        let now = Utc::now().to_rfc3339();
        let token = Uuid::new_v4().to_string();
        let admitted_at = chrono::Local::now().naive_local();
        let changed=tx.execute("UPDATE publish_campaigns SET state='posting',error_code=NULL,revision=revision+1,updated_at=?2
            WHERE id=?1 AND state IN ('queued','scheduled','ready','imported','failed_before_dispatch','verifying')
            AND NOT EXISTS(SELECT 1 FROM publish_start_requests s WHERE s.campaign_id=?1 AND s.state='uncertain')
            AND (run_at IS NULL OR datetime(run_at)<=datetime(?3))
            AND (run_at IS NULL OR datetime(?3)<=datetime(run_at,'+30 seconds')
                OR EXISTS(SELECT 1 FROM publish_dispatch_jobs j WHERE j.campaign_id=?1 AND j.started_at_ms IS NOT NULL))
            AND NOT EXISTS(SELECT 1 FROM publish_pipeline_runs r WHERE r.campaign_id=?1)
            AND NOT EXISTS(SELECT 1 FROM publish_assignments a WHERE a.campaign_id=?1 AND a.state IN ('posting','uncertain'))
            AND EXISTS(SELECT 1 FROM publish_assignments a WHERE a.campaign_id=?1 AND a.effect_intent IS NULL AND a.state IN ('queued','scheduled','ready','imported','failed_before_dispatch')
                AND a.error_code IS NOT ?4)",
            params![campaign_id,now,admitted_at.format("%Y-%m-%dT%H:%M:%S").to_string(),super::PUBLISH_DEVICE_NOT_RELEASED])?;
        if changed == 0 {
            return Ok(None);
        }
        tx.execute(
            "INSERT INTO publish_pipeline_runs(campaign_id,token,created_at) VALUES(?1,?2,?3)",
            params![campaign_id, token, now],
        )?;
        Self::enqueue_publish_jobs(
            &tx,
            &PublishPipelineRun {
                campaign_id: campaign_id.into(),
                token: token.clone(),
            },
            Utc::now().timestamp_millis(),
        )?;
        tx.execute("UPDATE publish_start_requests SET state='queued',stage='queued',revision=revision+1,updated_at=?2
            WHERE campaign_id=?1 AND state='preparing'",params![campaign_id,now])?;
        // Fresh work re-enters the approved set, so its phones are account-locked again.
        lock_campaign_accounts(&tx, campaign_id)?;
        pipeline_event(&tx, campaign_id, &now)?;
        tx.commit()?;
        Ok(Some(PublishPipelineRun {
            campaign_id: campaign_id.into(),
            token,
        }))
    }

    pub fn has_active_publish_pipeline(&self, campaign_id: &str) -> anyhow::Result<bool> {
        self.conn()?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM publish_pipeline_runs WHERE campaign_id=?1)",
                [campaign_id],
                |r| r.get(0),
            )
            .map_err(Into::into)
    }

    pub fn publish_pipeline_current(&self, run: &PublishPipelineRun) -> anyhow::Result<bool> {
        self.conn()?.query_row("SELECT EXISTS(SELECT 1 FROM publish_pipeline_runs r JOIN publish_campaigns c ON c.id=r.campaign_id WHERE r.campaign_id=?1 AND r.token=?2 AND c.state='posting')",params![run.campaign_id,run.token],|r|r.get(0)).map_err(Into::into)
    }

    pub fn publish_assignment_revision(&self, assignment: &str) -> anyhow::Result<i64> {
        self.conn()?
            .query_row(
                "SELECT revision FROM publish_assignments WHERE id=?1",
                [assignment],
                |r| r.get(0),
            )
            .map_err(Into::into)
    }

    /// The expected previous state and null intent prevent a transfer from reopening a Post.
    #[allow(clippy::too_many_arguments)]
    pub fn advance_publish_pipeline_assignment(
        &self,
        run: &PublishPipelineRun,
        assignment: &str,
        expected_revision: i64,
        expected_state: crate::PublishCampaignState,
        next_state: crate::PublishCampaignState,
        error: Option<&str>,
        evidence: Option<&str>,
    ) -> anyhow::Result<bool> {
        anyhow::ensure!(
            matches!(
                next_state,
                crate::PublishCampaignState::Transferring
                    | crate::PublishCampaignState::Imported
                    | crate::PublishCampaignState::FailedBeforeDispatch
            ),
            "invalid pre-Post pipeline transition"
        );
        use crate::PublishCampaignState as S;
        anyhow::ensure!(
            matches!(
                (&expected_state, &next_state),
                (
                    S::Queued | S::Scheduled | S::Ready | S::FailedBeforeDispatch,
                    S::Transferring | S::FailedBeforeDispatch
                ) | (S::Transferring, S::Imported | S::FailedBeforeDispatch)
                    | (S::Imported, S::Imported | S::FailedBeforeDispatch)
            ),
            "invalid pipeline state edge"
        );
        let changed=self.conn()?.execute("UPDATE publish_assignments SET state=?1,error_code=?2,evidence_json=COALESCE(?3,evidence_json),revision=revision+1,updated_at=?4
            WHERE id=?5 AND campaign_id=?6 AND revision=?7 AND state=?8 AND effect_intent IS NULL
            AND EXISTS(SELECT 1 FROM publish_pipeline_runs r JOIN publish_campaigns c ON c.id=r.campaign_id WHERE r.campaign_id=?6 AND r.token=?9 AND c.state='posting')",
            params![next_state.as_str(),error,evidence,Utc::now().to_rfc3339(),assignment,run.campaign_id,expected_revision,expected_state.as_str(),run.token])?;
        Ok(changed == 1)
    }

    pub fn claim_pipeline_post(
        &self,
        run: &PublishPipelineRun,
        assignment: &str,
        revision: i64,
        intent: &str,
    ) -> anyhow::Result<bool> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Read under the write lock: see `publish_shutdown` for why this is the exit barrier.
        if self.publish_shutdown_started() {
            return Ok(false);
        }
        let changed=tx.execute("UPDATE publish_assignments SET state='posting',error_code=NULL,effect_intent=?1,evidence_json=?1,revision=revision+1,updated_at=?2
            WHERE id=?3 AND campaign_id=?4 AND revision=?5 AND state='imported' AND effect_intent IS NULL
            AND EXISTS(SELECT 1 FROM publish_pipeline_runs r JOIN publish_campaigns c ON c.id=r.campaign_id WHERE r.campaign_id=?4 AND r.token=?6 AND c.state='posting')",
            params![intent,Utc::now().to_rfc3339(),assignment,run.campaign_id,revision,run.token])?;
        if changed == 1 {
            super::publish_submission::validate_submission_claim(&tx, assignment, intent)?;
        }
        tx.commit()?;
        Ok(changed == 1)
    }

    /// Cancellation blocks a new Post, but cannot discard the result of an already dispatched one.
    #[allow(clippy::too_many_arguments)]
    pub fn settle_pipeline_post(
        &self,
        run: &PublishPipelineRun,
        assignment: &str,
        revision: i64,
        state: crate::PublishCampaignState,
        error: Option<&str>,
        evidence: &str,
        link: Option<&str>,
        poster: &str,
        partners: &[String],
    ) -> anyhow::Result<bool> {
        use crate::PublishCampaignState as S;
        anyhow::ensure!(
            matches!(
                state,
                S::Succeeded | S::Verifying | S::Uncertain | S::FailedBeforeDispatch
            ),
            "invalid post outcome state"
        );
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed=tx.execute("UPDATE publish_assignments SET state=?1,error_code=?2,evidence_json=?3,revision=revision+1,updated_at=?4 WHERE id=?5 AND campaign_id=?6 AND revision=?7 AND state IN ('imported','posting')
            AND ((state='imported' AND ?1='failed_before_dispatch' AND effect_intent IS NULL) OR (state='posting' AND effect_intent IS NOT NULL AND ?1 IN ('succeeded','verifying','uncertain')))
            AND EXISTS(SELECT 1 FROM publish_pipeline_runs r WHERE r.campaign_id=?6 AND r.token=?8)",params![state.as_str(),error,evidence,Utc::now().to_rfc3339(),assignment,run.campaign_id,revision,run.token])?;
        if changed == 0 {
            return Ok(false);
        }
        if let Some(url) = link {
            anyhow::ensure!(
                state == crate::PublishCampaignState::Succeeded,
                "only verified publication queues Sheet"
            );
            queue_sheet_row(&tx, assignment, &run.campaign_id, url, poster, partners)?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// Only the coordinator removes its token, after joining every worker (including panics).
    pub fn finish_publish_pipeline(&self, run: &PublishPipelineRun) -> anyhow::Result<bool> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "DELETE FROM publish_pipeline_runs WHERE campaign_id=?1 AND token=?2",
            params![run.campaign_id, run.token],
        )?;
        if changed == 0 {
            return Ok(false);
        }
        let now = Utc::now().to_rfc3339();
        tx.execute("UPDATE publish_assignments SET state=CASE WHEN effect_intent IS NULL THEN 'failed_before_dispatch' ELSE 'uncertain' END,error_code='publish_worker_lost',revision=revision+1,updated_at=?2 WHERE campaign_id=?1 AND state IN ('transferring','posting')",params![run.campaign_id,now])?;
        let rows: Vec<String> = tx
            .prepare("SELECT state FROM publish_assignments WHERE campaign_id=?1")?
            .query_map([&run.campaign_id], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let state = if rows.iter().any(|s| s == "uncertain") {
            "uncertain"
        } else if rows.iter().any(|s| s == "verifying") {
            "verifying"
        } else if !rows.is_empty() && rows.iter().all(|s| s == "missed") {
            "missed"
        } else if !rows.is_empty() && rows.iter().all(|s| s == "succeeded") {
            "succeeded"
        } else {
            "failed_before_dispatch"
        };
        let error = match state {
            "succeeded" => None,
            "verifying" => Some("post_verification_pending"),
            "uncertain" => Some("post_or_cleanup_failed"),
            "missed" => Some("schedule_capacity_deadline"),
            _ => Some("publish_assignment_failed"),
        };
        if tx.execute("UPDATE publish_campaigns SET state=?1,error_code=?2,revision=revision+1,updated_at=?3 WHERE id=?4 AND state='posting'",params![state,error,now,run.campaign_id])?==1{pipeline_event(&tx,&run.campaign_id,&now)?;}
        release_settled_account_locks(&tx, Some(&run.campaign_id), None)?;
        tx.commit()?;
        Ok(true)
    }
}
fn pipeline_event(tx: &rusqlite::Transaction<'_>, campaign: &str, now: &str) -> anyhow::Result<()> {
    tx.execute("INSERT INTO publish_events(campaign_id,revision,kind,payload_json,created_at) SELECT id,revision,'state',json_object('state',state,'errorCode',error_code,'source','per_device_pipeline'),?2 FROM publish_campaigns WHERE id=?1",params![campaign,now])?;
    Ok(())
}

/// Durable account locks (PLAN §3, review R3): from the moment an approval is accepted until its
/// set settles, every phone in it keeps the account the approval bound, and every account writer
/// refuses a change (`fleet::ensure_account_edit_allowed`). The approval validators
/// (`publish_approval_refusal`, `admit_publish_effect`) keep comparing as the backstop.
///
/// Rows live in `settings` under `publish.account-lock.{kind}.{key}.{udid}`, valued
/// `{udid, owner, kind, account, label}` plus `generation` for a preparation:
/// - `preparing`, keyed by the preparation's generation, owner = request ID: taken in Start's
///   acceptance transaction, or when creation begins without a Start, before the approval is
///   compared with the phones. It binds only while its [`PublishPreparationOwner`] is alive in
///   this process (review R4-F1) and the Start, if any, has neither settled nor created its
///   campaign. The owner exists before the lock can be written and its drop ends the lock, so a
///   cancelled await, a failed release or a failed Start settlement leaves no binding row; each
///   preparation releases only its own generation.
/// - `campaign`, keyed by the campaign ID: taken in the transaction that inserts a campaign
///   carrying an approval, and again whenever fresh work re-enters the set (claim, assignment
///   retry, recovery requeue). It binds while the campaign is not cancelled or missed and one of
///   its rows can still start fresh work (no effect intent, and an active state or an open
///   dispatch job); a row with an effect intent only reconciles under the approval.
///
/// A row that no longer binds is ignored, and deleted by the next writer check, at pipeline
/// finish, cancel and Start settlement, and by the startup sweep.
const ACCOUNT_LOCK_PREFIX: &str = "publish.account-lock.";

/// Settings row `l` is a lock (`?2` is the prefix). Other settings values are not JSON, so a
/// query reads `l.value` only behind this test, inside a `CASE`.
const LOCK_ROW: &str = "substr(l.key,1,length(?2))=?2 AND json_valid(l.value)";

/// Whether the lock in settings row `l` still binds; `?1` is the JSON array of the
/// preparation generations alive in this process ([`active_preparations`]).
const LIVE_ACCOUNT_LOCK: &str = "COALESCE(CASE json_extract(l.value,'$.kind')
    WHEN 'campaign' THEN EXISTS(SELECT 1 FROM publish_assignments a JOIN publish_campaigns c ON c.id=a.campaign_id
        WHERE c.id=json_extract(l.value,'$.owner') AND c.state NOT IN ('cancelled','missed')
        AND a.effect_intent IS NULL
        AND (a.state IN ('queued','scheduled','preparing','ready','transferring','imported','posting')
            OR EXISTS(SELECT 1 FROM publish_dispatch_jobs j WHERE j.assignment_id=a.id
                AND j.state IN ('queued','running','paused'))))
    ELSE json_extract(l.value,'$.generation') IN (SELECT value FROM json_each(?1))
        AND NOT EXISTS(SELECT 1 FROM publish_start_requests s
        WHERE s.request_id=json_extract(l.value,'$.owner')
        AND (s.campaign_id IS NOT NULL OR s.state NOT IN ('accepted','preparing')))
    END,0)";

/// Settings key of the generation that owns an accepted Start's preparation.
const START_OWNER_PREFIX: &str = "publish.start-owner.";

/// The preparation generations alive in this process. A fresh process has none, so every
/// earlier process's preparation lock and Start owner is inactive after a restart.
static ACTIVE_PREPARATIONS: std::sync::LazyLock<
    parking_lot::Mutex<std::collections::HashSet<String>>,
> = std::sync::LazyLock::new(Default::default);

/// The owner of one preparation's account lock (review R4-F1). Register it before the lock is
/// written and keep it for exactly as long as the preparation (or its Start's settlement) runs:
/// its drop, on any exit including a cancelled future or an unwinding task, is what ends the
/// lock, whether or not the rows could be deleted.
#[derive(Debug)]
pub struct PublishPreparationOwner {
    generation: String,
}

impl PublishPreparationOwner {
    pub fn register() -> Self {
        let generation = Uuid::new_v4().to_string();
        ACTIVE_PREPARATIONS.lock().insert(generation.clone());
        Self { generation }
    }

    pub fn generation(&self) -> &str {
        &self.generation
    }
}

impl Drop for PublishPreparationOwner {
    fn drop(&mut self) {
        ACTIVE_PREPARATIONS.lock().remove(&self.generation);
    }
}

fn preparation_active(generation: &str) -> bool {
    ACTIVE_PREPARATIONS.lock().contains(generation)
}

/// `?1` of [`LIVE_ACCOUNT_LOCK`]: a snapshot taken inside the caller's transaction. An owner
/// registers before its lock is written, so a lock the transaction can see has its owner in the
/// snapshot until that owner drops.
fn active_preparations() -> String {
    serde_json::Value::Array(
        ACTIVE_PREPARATIONS
            .lock()
            .iter()
            .cloned()
            .map(serde_json::Value::String)
            .collect(),
    )
    .to_string()
}

fn account_lock_key(kind: &str, key: &str, udid: &str) -> String {
    format!("{ACCOUNT_LOCK_PREFIX}{kind}.{key}.{udid}")
}

fn current_account(conn: &Connection, udid: &str) -> anyhow::Result<String> {
    let handle: String = conn.query_row(
        "SELECT COALESCE((SELECT handle FROM device_meta WHERE udid=?1),'')",
        [udid],
        |row| row.get(0),
    )?;
    Ok(crate::publish::normalize_publish_account(&handle))
}

/// Lock `udids` for the preparation `generation` of `request`, which has no campaign yet,
/// recording each current account for the backstop before the first owner effect. Refused when
/// the owner already dropped (its caller was cancelled before this write ran), so no lock is
/// ever written without a live owner.
pub(super) fn lock_preparing_accounts(
    conn: &Connection,
    generation: &str,
    request: &str,
    udids: &[String],
) -> anyhow::Result<()> {
    anyhow::ensure!(
        preparation_active(generation),
        "Lượt chuẩn bị đã dừng trước khi khóa tài khoản; chưa khóa máy nào"
    );
    let label = format!(
        "lượt đăng {} đang bắt đầu",
        request.get(..8).unwrap_or(request)
    );
    for udid in udids {
        let value = serde_json::json!({
            "udid": udid, "owner": request, "kind": "preparing", "label": label,
            "account": current_account(conn, udid)?, "generation": generation,
        });
        conn.execute(
            "INSERT OR IGNORE INTO settings(key,value) VALUES(?1,?2)",
            params![
                account_lock_key("preparing", generation, udid),
                value.to_string()
            ],
        )?;
    }
    Ok(())
}

/// Record which generation owns the accepted Start `request` (same transaction as acceptance).
pub(super) fn record_start_owner(
    conn: &Connection,
    request: &str,
    generation: &str,
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO settings(key,value) VALUES(?1,?2)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![
            format!("{START_OWNER_PREFIX}{request}"),
            serde_json::json!({ "generation": generation }).to_string()
        ],
    )?;
    Ok(())
}

pub(super) fn forget_start_owner(conn: &Connection, request: &str) -> anyhow::Result<()> {
    conn.execute(
        "DELETE FROM settings WHERE key=?1",
        [format!("{START_OWNER_PREFIX}{request}")],
    )?;
    Ok(())
}

/// A `publish_start_requests` row whose campaign already waits for its hour. Its preparation
/// ended when that campaign was created, and only the Start's own `preparing -> queued` write
/// can be missing (drop audit schedule_restart#4, 16/10/2026): settling it `uncertain` kept the
/// scheduler from ever claiming the campaign, which was then missed 30 s after its hour.
pub(super) const START_CAMPAIGN_WAITS: &str = "EXISTS(SELECT 1 FROM publish_campaigns c
    WHERE c.id=publish_start_requests.campaign_id AND c.run_at IS NOT NULL AND c.state='scheduled')";

/// A Start still `accepted`/`preparing` whose owner is no longer alive will never be settled by
/// it: its task ended, or its terminal write failed every retry (review R4-F1). Settle it the way
/// a restart does, `uncertain`, which needs a person and is never claimed or posted again, and
/// drop the owner records that no longer name a live generation. Its preparation locks are then
/// inactive and released with the other settled locks. One whose campaign waits for its hour
/// ([`START_CAMPAIGN_WAITS`]) is settled `queued` instead, as its own last write would have.
pub(super) fn reconcile_orphaned_publish_starts(conn: &Connection) -> anyhow::Result<()> {
    let active = active_preparations();
    let error = serde_json::json!({
        "code": "PublishStartInterrupted",
        "message": "Lượt chuẩn bị đã dừng mà chưa ghi được kết quả; kiểm tra lượt cũ, không tự đăng lại",
    })
    .to_string();
    let orphan = "substr(o.key,1,length(?2))=?2 AND CASE WHEN json_valid(o.value)
        THEN COALESCE(json_extract(o.value,'$.generation') NOT IN (SELECT value FROM json_each(?1)),1)
        ELSE 1 END";
    conn.execute(
        &format!(
            "UPDATE publish_start_requests SET state='queued',stage='queued',error_json=NULL,
             revision=revision+1,updated_at=?3 WHERE state='preparing' AND {START_CAMPAIGN_WAITS}
             AND request_id IN (SELECT substr(o.key,length(?2)+1) FROM settings o WHERE {orphan})"
        ),
        params![active, START_OWNER_PREFIX, Utc::now().to_rfc3339()],
    )?;
    conn.execute(
        &format!(
            "UPDATE publish_start_requests SET state='uncertain',stage='interrupted',error_json=?3,
             revision=revision+1,updated_at=?4 WHERE state IN ('accepted','preparing')
             AND request_id IN (SELECT substr(o.key,length(?2)+1) FROM settings o WHERE {orphan})"
        ),
        params![active, START_OWNER_PREFIX, error, Utc::now().to_rfc3339()],
    )?;
    conn.execute(
        &format!("DELETE FROM settings WHERE key IN (SELECT o.key FROM settings o WHERE {orphan})"),
        params![active, START_OWNER_PREFIX],
    )?;
    Ok(())
}

/// Lock every phone of `campaign` that can still start fresh work, with the account its
/// approval bound. A campaign without an approval cannot dispatch fresh work and takes none.
pub(super) fn lock_campaign_accounts(conn: &Connection, campaign: &str) -> anyhow::Result<()> {
    let (raw, run_at): (String, Option<String>) = conn.query_row(
        "SELECT request_json,run_at FROM publish_campaigns WHERE id=?1",
        [campaign],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    // An unreadable legacy request has no approval either; the validators refuse its fresh work.
    let Some(approval) = serde_json::from_str::<crate::PublishCampaignRequest>(&raw)
        .ok()
        .and_then(|request| request.approval)
    else {
        return Ok(());
    };
    let short = campaign.get(..8).unwrap_or(campaign);
    let rows = {
        let mut statement = conn.prepare(
            "SELECT a.udid,COALESCE(b.name,a.bundle_id) FROM publish_assignments a
             LEFT JOIN publish_bundles b ON b.campaign_id=a.campaign_id AND b.id=a.bundle_id
             WHERE a.campaign_id=?1 AND a.effect_intent IS NULL
             AND a.state IN ('queued','scheduled','preparing','ready','transferring','imported','posting','failed_before_dispatch')",
        )?;
        let rows = statement
            .query_map([campaign], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    for (udid, bundle) in rows {
        let label = match run_at.as_deref() {
            Some(at) => format!(
                "lịch đăng {short} lúc {} (bài \"{bundle}\")",
                at.replace('T', " ")
            ),
            None => format!("lượt đăng {short} (bài \"{bundle}\")"),
        };
        let value = serde_json::json!({
            "udid": udid, "owner": campaign, "kind": "campaign", "label": label,
            "account": approval.accounts.get(&udid).cloned().unwrap_or_default(),
        });
        conn.execute(
            "INSERT OR IGNORE INTO settings(key,value) VALUES(?1,?2)",
            params![
                account_lock_key("campaign", campaign, &udid),
                value.to_string()
            ],
        )?;
    }
    Ok(())
}

/// Delete the locks that no longer bind, optionally only one owner's or one phone's.
pub(super) fn release_settled_account_locks(
    conn: &Connection,
    owner: Option<&str>,
    udid: Option<&str>,
) -> anyhow::Result<()> {
    conn.execute(
        &format!(
            "DELETE FROM settings WHERE key IN (SELECT l.key FROM settings l
             WHERE CASE WHEN {LOCK_ROW} THEN
                 (?3 IS NULL OR json_extract(l.value,'$.owner')=?3)
                 AND (?4 IS NULL OR json_extract(l.value,'$.udid')=?4)
                 AND NOT {LIVE_ACCOUNT_LOCK}
             ELSE 0 END)"
        ),
        params![active_preparations(), ACCOUNT_LOCK_PREFIX, owner, udid],
    )?;
    Ok(())
}

/// The approval that keeps `udid`'s account locked now, named for the operator, if any.
pub(super) fn live_account_lock(conn: &Connection, udid: &str) -> anyhow::Result<Option<String>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT json_extract(l.value,'$.label') FROM settings l
                 WHERE CASE WHEN {LOCK_ROW} THEN
                     json_extract(l.value,'$.udid')=?3 AND {LIVE_ACCOUNT_LOCK}
                 ELSE 0 END ORDER BY l.key LIMIT 1"
            ),
            params![active_preparations(), ACCOUNT_LOCK_PREFIX, udid],
            |row| row.get(0),
        )
        .optional()?)
}

impl Database {
    /// A creation locks its phones under its own `generation` before it reads their accounts.
    /// Its owner must already be registered; a Start's lock from its acceptance is separate.
    pub fn lock_publish_preparation_accounts(
        &self,
        generation: &str,
        request: &str,
        udids: &[String],
    ) -> anyhow::Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        lock_preparing_accounts(&tx, generation, request, udids)?;
        tx.commit()?;
        Ok(())
    }

    /// Delete exactly this generation's preparation rows. Another call with the same request ID
    /// keeps its own; if this delete fails, the owner's drop still ends the lock.
    pub fn release_publish_preparation_accounts(&self, generation: &str) -> anyhow::Result<()> {
        self.conn()?.execute(
            "DELETE FROM settings WHERE substr(key,1,length(?1))=?1",
            [format!("{ACCOUNT_LOCK_PREFIX}preparing.{generation}.")],
        )?;
        Ok(())
    }

    /// Whether this accepted Start still waits for an owner that is gone (review R4-F1).
    pub fn publish_start_orphaned(&self, request: &str) -> anyhow::Result<bool> {
        let generation: Option<String> = self
            .conn()?
            .query_row(
                "SELECT json_extract(o.value,'$.generation') FROM settings o
                 JOIN publish_start_requests s ON s.request_id=?2
                 WHERE o.key=?1 || ?2 AND json_valid(o.value) AND s.state IN ('accepted','preparing')",
                params![START_OWNER_PREFIX, request],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        Ok(generation.is_some_and(|generation| !preparation_active(&generation)))
    }

    /// Whether this Start's campaign already waits for its hour ([`START_CAMPAIGN_WAITS`]).
    pub fn publish_start_campaign_waits(&self, request: &str) -> anyhow::Result<bool> {
        Ok(self.conn()?.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM publish_start_requests
                 WHERE request_id=?1 AND {START_CAMPAIGN_WAITS})"
            ),
            [request],
            |row| row.get(0),
        )?)
    }

    /// Settle every Start whose owner is gone and release the locks that no longer bind.
    pub fn reconcile_orphaned_publish_starts(&self) -> anyhow::Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        reconcile_orphaned_publish_starts(&tx)?;
        release_settled_account_locks(&tx, None, None)?;
        tx.commit()?;
        Ok(())
    }

    /// Backstop before Start's first owner effect (review R3-F1): each selected phone must still
    /// hold this preparation generation's binding lock with the account recorded when it was
    /// taken. The writers already refuse while the lock binds; this catches anything that
    /// bypassed them.
    pub fn publish_account_lock_drift(
        &self,
        generation: &str,
        udids: &[String],
    ) -> anyhow::Result<Option<String>> {
        let conn = self.conn()?;
        let shown = |account: &str| {
            if account.is_empty() {
                "chưa gắn".to_owned()
            } else {
                format!("@{account}")
            }
        };
        let mut refusals = Vec::new();
        for udid in udids {
            let lock: Option<(String, bool)> = conn
                .query_row(
                    &format!(
                        "SELECT COALESCE(json_extract(l.value,'$.account'),''),{LIVE_ACCOUNT_LOCK}
                         FROM settings l WHERE l.key=?2"
                    ),
                    params![
                        active_preparations(),
                        account_lock_key("preparing", generation, udid)
                    ],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let current = current_account(&conn, udid)?;
            match lock {
                Some((locked, true)) if locked == current => {}
                Some((locked, true)) => refusals.push(format!(
                    "{udid}: tài khoản đã đổi từ {} sang {} trong lúc chuẩn bị; chưa đổi tác vụ cũ, xem lại rồi tạo lượt mới",
                    shown(&locked),
                    shown(&current)
                )),
                _ => refusals.push(format!(
                    "{udid}: khóa tài khoản của lượt này không còn; chưa đổi tác vụ cũ, xem lại rồi bắt đầu lại"
                )),
            }
        }
        Ok((!refusals.is_empty()).then(|| refusals.join("; ")))
    }
}

#[cfg(test)]
#[path = "publish_pipeline_tests.rs"]
pub(super) mod tests;
