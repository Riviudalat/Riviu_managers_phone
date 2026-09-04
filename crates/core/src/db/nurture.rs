//! Nurture: the session profile, the agent and stream settings beside it, and the ledger
//! of what each comment cost.
//!
//! `resolve_api_key` lives here rather than with the other secrets because it is read on
//! every settings load — the engine re-reads the profile mid-session, so the key has to come
//! back every time or a live refresh blanks it.

use super::*;

/// One durable Nuoi TikTok invocation and the latest exact status observed for each target.
///
/// The event table keeps every transition; this read model deliberately returns only the last
/// event per device because that is what the Operations page projects. `target_udids` remains
/// separate so a device that never got past admission cannot disappear from the denominator.
#[derive(Debug, Clone)]
pub struct NurtureRunHistory {
    pub run_id: Uuid,
    pub target_udids: Vec<String>,
    pub statuses: Vec<crate::types::NurtureSessionStatus>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Database {
    /// Persist a run and its first per-device states before any worker is spawned.
    pub fn create_nurture_run(
        &self,
        run_id: Uuid,
        target_udids: &[String],
        initial_statuses: &[crate::types::NurtureSessionStatus],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(!target_udids.is_empty(), "nurture target list is empty");
        anyhow::ensure!(
            target_udids.len() == initial_statuses.len(),
            "nurture initial status count does not match its target list"
        );
        let targets = target_udids
            .iter()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        anyhow::ensure!(
            targets.len() == target_udids.len(),
            "nurture target list contains a duplicate device"
        );
        let status_udids = initial_statuses
            .iter()
            .map(|status| status.udid.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        anyhow::ensure!(
            status_udids.len() == initial_statuses.len()
                && status_udids.iter().all(|udid| targets.contains(udid)),
            "nurture initial statuses do not cover the exact target list"
        );
        for status in initial_statuses {
            anyhow::ensure!(
                status.run_id == Some(run_id),
                "nurture initial status carries a different run ID"
            );
            anyhow::ensure!(
                status.run_size as usize == target_udids.len(),
                "nurture initial status carries a different run size"
            );
        }

        let now = Utc::now();
        let target_json = serde_json::to_string(target_udids)?;
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run_id_text = run_id.to_string();
        let now_text = now.to_rfc3339();
        transaction.execute(
            "INSERT INTO nurture_runs
             (id,target_udids_json,target_count,created_at,updated_at)
             VALUES(?1,?2,?3,?4,?4)",
            params![
                run_id_text,
                target_json,
                target_udids.len() as i64,
                now_text
            ],
        )?;
        for status in initial_statuses {
            let recorded_at = status.updated_at.unwrap_or(now);
            transaction.execute(
                "INSERT INTO nurture_run_status_events
                 (run_id,udid,status_json,recorded_at) VALUES(?1,?2,?3,?4)",
                params![
                    run_id_text,
                    status.udid,
                    serde_json::to_string(status)?,
                    recorded_at.to_rfc3339()
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Append the exact status accepted by the runtime. Previous transitions are immutable.
    pub fn append_nurture_status(
        &self,
        status: &crate::types::NurtureSessionStatus,
    ) -> anyhow::Result<()> {
        let run_id = status
            .run_id
            .context("a persisted nurture status requires run_id")?;
        let now = status.updated_at.unwrap_or_else(Utc::now);
        let run_id_text = run_id.to_string();
        let now_text = now.to_rfc3339();
        let payload = serde_json::to_string(status)?;
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let targets_json = transaction
            .query_row(
                "SELECT target_udids_json FROM nurture_runs WHERE id=?1",
                [run_id_text.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .with_context(|| format!("nurture run {run_id} does not exist"))?;
        let targets: Vec<String> = serde_json::from_str(&targets_json)
            .context("invalid JSON in nurture run target snapshot")?;
        anyhow::ensure!(
            targets.iter().any(|target| target == &status.udid),
            "nurture status device is outside its immutable target snapshot"
        );
        transaction.execute(
            "INSERT INTO nurture_run_status_events
             (run_id,udid,status_json,recorded_at) VALUES(?1,?2,?3,?4)",
            params![run_id_text, status.udid, payload, now_text],
        )?;
        transaction.execute(
            "UPDATE nurture_runs SET updated_at=?2 WHERE id=?1",
            params![run_id_text, now_text],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn list_nurture_runs(&self, limit: usize) -> anyhow::Result<Vec<NurtureRunHistory>> {
        let conn = self.conn()?;
        let rows = {
            let mut statement = conn.prepare(
                "SELECT id,target_udids_json,created_at,updated_at
                 FROM nurture_runs ORDER BY updated_at DESC,id ASC LIMIT ?1",
            )?;
            let rows = statement
                .query_map([limit.clamp(1, 200) as i64], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        rows.into_iter()
            .map(|(run_id, targets_json, created_at, updated_at)| {
                self.read_nurture_run(
                    &conn,
                    Uuid::parse_str(&run_id).context("invalid nurture run UUID")?,
                    targets_json,
                    DateTime::parse_from_rfc3339(&created_at)
                        .context("invalid nurture run created_at")?
                        .with_timezone(&Utc),
                    DateTime::parse_from_rfc3339(&updated_at)
                        .context("invalid nurture run updated_at")?
                        .with_timezone(&Utc),
                )
            })
            .collect()
    }

    pub fn get_nurture_run(&self, run_id: Uuid) -> anyhow::Result<Option<NurtureRunHistory>> {
        let conn = self.conn()?;
        let row = conn
            .query_row(
                "SELECT target_udids_json,created_at,updated_at FROM nurture_runs WHERE id=?1",
                [run_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        row.map(|(targets_json, created_at, updated_at)| {
            self.read_nurture_run(
                &conn,
                run_id,
                targets_json,
                DateTime::parse_from_rfc3339(&created_at)
                    .context("invalid nurture run created_at")?
                    .with_timezone(&Utc),
                DateTime::parse_from_rfc3339(&updated_at)
                    .context("invalid nurture run updated_at")?
                    .with_timezone(&Utc),
            )
        })
        .transpose()
    }

    /// Read the latest worker-owned rows which still claim to be running.
    ///
    /// Startup uses this before admitting new commands so it can force-stop the exact app on
    /// each affected device and append the resulting process proof. Merely changing these rows
    /// to `failed` would leave TikTok running after a desktop crash.
    pub fn list_orphaned_nurture_statuses(
        &self,
    ) -> anyhow::Result<Vec<crate::types::NurtureSessionStatus>> {
        let conn = self.conn()?;
        let rows = {
            let mut statement = conn.prepare(
                "SELECT event.run_id,event.status_json
                 FROM nurture_run_status_events event
                 WHERE json_extract(event.status_json,'$.running')=1 AND NOT EXISTS (
                   SELECT 1 FROM nurture_run_status_events newer
                   WHERE newer.run_id=event.run_id AND newer.udid=event.udid
                     AND newer.sequence>event.sequence
                 )
                 ORDER BY event.run_id ASC,event.udid ASC",
            )?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        rows.into_iter()
            .map(|(run_id, payload)| {
                let run_id = Uuid::parse_str(&run_id).context("invalid nurture run UUID")?;
                let mut status: crate::types::NurtureSessionStatus = serde_json::from_str(&payload)
                    .context("invalid JSON in orphaned nurture status")?;
                if let Some(status_run_id) = status.run_id {
                    anyhow::ensure!(
                        status_run_id == run_id,
                        "orphaned nurture status carries a different run ID"
                    );
                } else {
                    status.run_id = Some(run_id);
                }
                Ok(status)
            })
            .collect()
    }

    fn read_nurture_run(
        &self,
        conn: &Connection,
        run_id: Uuid,
        targets_json: String,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> anyhow::Result<NurtureRunHistory> {
        let target_udids: Vec<String> = serde_json::from_str(&targets_json)
            .context("invalid JSON in nurture run target snapshot")?;
        let payloads = {
            let mut statement = conn.prepare(
                "SELECT event.status_json
                 FROM nurture_run_status_events event
                 WHERE event.run_id=?1 AND NOT EXISTS (
                   SELECT 1 FROM nurture_run_status_events newer
                   WHERE newer.run_id=event.run_id AND newer.udid=event.udid
                     AND newer.sequence>event.sequence
                 )
                 ORDER BY event.udid ASC",
            )?;
            let payloads = statement
                .query_map([run_id.to_string()], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            payloads
        };
        let statuses = payloads
            .into_iter()
            .map(|payload| {
                serde_json::from_str(&payload).context("invalid JSON in nurture status history")
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(NurtureRunHistory {
            run_id,
            target_udids,
            statuses,
            created_at,
            updated_at,
        })
    }

    /// Close worker-owned sessions left running by a process exit without rewriting history.
    pub fn interrupt_orphaned_nurture_sessions(&self) -> anyhow::Result<usize> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let rows = {
            let mut statement = transaction.prepare(
                "SELECT event.run_id,event.status_json
                 FROM nurture_run_status_events event
                 WHERE json_extract(event.status_json,'$.running')=1 AND NOT EXISTS (
                   SELECT 1 FROM nurture_run_status_events newer
                   WHERE newer.run_id=event.run_id AND newer.udid=event.udid
                     AND newer.sequence>event.sequence
                 )",
            )?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let now = Utc::now();
        let now_text = now.to_rfc3339();
        for (run_id, payload) in &rows {
            let mut status: crate::types::NurtureSessionStatus =
                serde_json::from_str(payload).context("invalid JSON in orphaned nurture status")?;
            status.finish(crate::nurture::Outcome::Partial);
            status.last_message = "Phiên Nuôi bị gián đoạn khi ứng dụng khởi động lại".to_string();
            if status.cleanup_state == crate::types::NurtureCleanupState::Pending {
                status.cleanup_state = crate::types::NurtureCleanupState::Failed;
                status.cleanup_error = Some("nurture_worker_lost_on_restart".to_string());
            }
            status.updated_at = Some(now);
            transaction.execute(
                "INSERT INTO nurture_run_status_events
                 (run_id,udid,status_json,recorded_at) VALUES(?1,?2,?3,?4)",
                params![
                    run_id,
                    status.udid,
                    serde_json::to_string(&status)?,
                    now_text
                ],
            )?;
            transaction.execute(
                "UPDATE nurture_runs SET updated_at=?2 WHERE id=?1",
                params![run_id, now_text],
            )?;
        }
        transaction.commit()?;
        Ok(rows.len())
    }

    pub fn get_nurture_settings(&self) -> anyhow::Result<crate::types::NurtureSettings> {
        match self.get_setting("nurture.settings")? {
            Some(raw) => {
                let mut settings: crate::types::NurtureSettings = serde_json::from_str(&raw)
                    .context("invalid JSON in stored setting nurture.settings")?;
                let need_v2 = self.get_setting(NURTURE_SETTINGS_MIGRATION_V2)?.is_none();
                let need_v3 = self.get_setting(NURTURE_SETTINGS_MIGRATION_V3)?.is_none();
                if need_v2 {
                    settings.migrate_legacy_defaults();
                }
                if need_v3 {
                    settings.adopt_openrouter_luna_if_still_shipped_deepseek();
                }
                if need_v2 || need_v3 {
                    // Re-serializing also drops obsolete risk-guard keys that
                    // were accepted by the old profile schema.
                    self.save_nurture_settings(&settings)?;
                }
                self.resolve_api_key(&mut settings)?;
                Ok(settings)
            }
            None => Ok(crate::types::NurtureSettings::default()),
        }
    }
    /// Put the API key back on the settings the engine is about to use.
    ///
    /// The key does not live in the settings blob any more — see [`SecretStore`] — so every read
    /// has to re-attach it. This is called from [`Self::get_nurture_settings`] rather than from
    /// the command layer **because the engine re-reads its settings mid-session**
    /// (`nurture::run_session` refreshes from the database on every post), and a key attached
    /// only at the command layer would come back empty on the first live refresh: commenting
    /// would stop part-way through a run, with an "API key đang trống" that the operator could
    /// not act on because the key *is* configured.
    ///
    /// Also migrates: a blob still carrying a key from before this existed is moved into the
    /// store and blanked, once.
    fn resolve_api_key(&self, settings: &mut crate::types::NurtureSettings) -> anyhow::Result<()> {
        let Some(store) = self.secrets.as_ref() else {
            return Ok(());
        };
        if !settings.api_key.is_empty() {
            // Legacy row: the key is still in SQLite. **One call does the whole migration,
            // and that is the fix.**
            //
            // This used to `mem::take` the key, save the now-blank settings, and then write
            // the real key back to the store — three steps with a window in the middle where
            // the key was absent from *both* places. If that second store write failed
            // (locked keyring, revoked access, a transient DPAPI error) the only copy was
            // gone, and after a restart there was nothing left to recover from. The comment
            // here explained the ordering as necessary; it was necessary only because of the
            // `mem::take`.
            //
            // `save_nurture_settings` already performs exactly this migration when the key is
            // present: it writes `settings.api_key` to the secret store, then persists a blob
            // with the key field cleared. So leaving the key in place and calling it once
            // stores the key before anything blanks it, and there is no window at all.
            //
            // Found by an independent review on 27/08/2026.
            self.save_nurture_settings(settings)?;
            return Ok(());
        }
        if let Some(key) = store.get_secret(SECRET_AI_API_KEY)? {
            settings.api_key = key;
        }
        Ok(())
    }
    /// The settings blob and the two migration markers that say it has been brought forward.
    ///
    /// One transaction, because the three belong together: this was three `set_setting`
    /// calls, each opening **its own connection**, so a failure after the first saved the
    /// blob without its markers and the next read re-ran the migrations over it. Near
    /// harmless in practice — `migrate_legacy_defaults` only replaces values still equal to
    /// the old defaults — but it is three writes that must land together, and one connection
    /// is cheaper than three.
    pub fn save_nurture_settings(
        &self,
        settings: &crate::types::NurtureSettings,
    ) -> anyhow::Result<()> {
        // The API key goes to the secret store, and the blob keeps an empty string in its
        // place. Faithful rather than clever: an empty key here really does clear the stored
        // one, so "leave it unchanged" is a decision for the caller that owns the form, not a
        // silent rule buried in the database layer.
        let payload = match self.secrets.as_ref() {
            Some(store) => {
                store.set_secret(SECRET_AI_API_KEY, &settings.api_key)?;
                let mut without = settings.clone();
                without.api_key.clear();
                serde_json::to_string(&without)?
            }
            None => serde_json::to_string(settings)?,
        };
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (key, value) in [
            ("nurture.settings", payload.as_str()),
            (NURTURE_SETTINGS_MIGRATION_V2, "2026-08-06-human-v2"),
            (
                NURTURE_SETTINGS_MIGRATION_V3,
                NURTURE_SETTINGS_MIGRATION_V3_VALUE,
            ),
        ] {
            transaction.execute(
                "INSERT INTO settings (key, value) VALUES (?1,?2)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, value],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }
    pub fn get_agent_settings(&self) -> anyhow::Result<crate::types::AgentSettings> {
        match self.get_setting("agent.settings.v1")? {
            Some(raw) => serde_json::from_str(&raw)
                .context("invalid JSON in stored setting agent.settings.v1"),
            None => Ok(crate::types::AgentSettings::default()),
        }
    }
    pub fn save_agent_settings(
        &self,
        settings: &crate::types::AgentSettings,
    ) -> anyhow::Result<()> {
        self.set_setting("agent.settings.v1", &serde_json::to_string(settings)?)
    }
    /// The operator's stream quality and frame rate, or the defaults.
    ///
    /// No migration: the `settings` key/value table has existed since the first one, and
    /// both other persisted settings blobs already live in it. A new schema object here
    /// would buy nothing and drag the migration ledger tests with it — the "migration" is a
    /// new key string.
    ///
    /// Strict about malformed JSON, like its neighbours: a stored value that cannot be read
    /// is a fact worth surfacing, not something to quietly replace with defaults. The
    /// startup caller decides whether that is fatal; here it is reported.
    pub fn get_stream_settings(&self) -> anyhow::Result<crate::types::StreamSettings> {
        match self.get_setting("stream.settings.v1")? {
            Some(raw) => serde_json::from_str(&raw)
                .context("invalid JSON in stored setting stream.settings.v1"),
            None => Ok(crate::types::StreamSettings::default()),
        }
    }
    pub fn save_stream_settings(
        &self,
        settings: &crate::types::StreamSettings,
    ) -> anyhow::Result<()> {
        self.set_setting("stream.settings.v1", &serde_json::to_string(settings)?)
    }
    pub fn add_nurture_comment_attempt(
        &self,
        attempt: &crate::types::NurtureCommentAttempt,
    ) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"
            INSERT INTO nurture_comment_attempts
              (id, udid, outcome, source, model, base_url_host, prompt_tokens,
               completion_tokens, cost_usd, preview, caption_preview, frame_sha256,
               context_confidence, relevance, evidence_support, distinct_frames,
               carousel_slides, created_at)
            VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)
            "#,
            params![
                attempt.id,
                attempt.udid,
                attempt.outcome,
                attempt.source,
                attempt.model,
                attempt.base_url_host,
                attempt.prompt_tokens as i64,
                attempt.completion_tokens as i64,
                attempt.cost_usd,
                attempt.preview,
                attempt.caption_preview,
                attempt.frame_sha256,
                attempt.context_confidence.map(i64::from),
                attempt.relevance.map(i64::from),
                attempt.evidence_support.map(i64::from),
                attempt.distinct_frames.map(i64::from),
                attempt.carousel_slides.map(i64::from),
                attempt.created_at,
            ],
        )?;
        Ok(())
    }
    /// Record how a comment attempt ended.
    ///
    /// **Reports a miss, because a zero-row `UPDATE` used to be indistinguishable from
    /// success.** The insert side logs a warning and carries on when it fails, so the caller
    /// could hold an `attempt_id` for a row that was never written; this `UPDATE` then matched
    /// nothing, returned `Ok(())`, and the session went on believing the audit trail had been
    /// closed out. Token counts, cost, evidence and the final outcome were simply absent, for
    /// a comment that had actually been posted and paid for.
    ///
    /// Found by an independent review on 27/08/2026.
    pub fn update_nurture_comment_attempt_outcome(
        &self,
        id: &str,
        outcome: &str,
    ) -> anyhow::Result<()> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE nurture_comment_attempts SET outcome=?2 WHERE id=?1",
            params![id, outcome],
        )?;
        anyhow::ensure!(
            changed > 0,
            "không có dòng comment attempt nào mang id {id} — dòng audit chưa từng được ghi"
        );
        Ok(())
    }
    pub fn list_nurture_comment_attempts(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<crate::types::NurtureCommentAttempt>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,udid,outcome,source,model,base_url_host,prompt_tokens,
                    completion_tokens,cost_usd,preview,caption_preview,frame_sha256,
                    context_confidence,relevance,evidence_support,distinct_frames,
                    carousel_slides,created_at
             FROM nurture_comment_attempts ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(crate::types::NurtureCommentAttempt {
                id: row.get(0)?,
                udid: row.get(1)?,
                outcome: row.get(2)?,
                source: row.get(3)?,
                model: row.get(4)?,
                base_url_host: row.get(5)?,
                prompt_tokens: narrow(row.get::<_, i64>(6)?, "prompt_tokens")?,
                completion_tokens: narrow(row.get::<_, i64>(7)?, "completion_tokens")?,
                cost_usd: row.get(8)?,
                preview: row.get(9)?,
                caption_preview: row.get(10)?,
                frame_sha256: row.get(11)?,
                context_confidence: row
                    .get::<_, Option<i64>>(12)?
                    .map(|v| narrow(v, "context_confidence"))
                    .transpose()?,
                relevance: row
                    .get::<_, Option<i64>>(13)?
                    .map(|v| narrow(v, "relevance"))
                    .transpose()?,
                evidence_support: row
                    .get::<_, Option<i64>>(14)?
                    .map(|v| narrow(v, "evidence_support"))
                    .transpose()?,
                distinct_frames: row
                    .get::<_, Option<i64>>(15)?
                    .map(|v| narrow(v, "distinct_frames"))
                    .transpose()?,
                carousel_slides: row
                    .get::<_, Option<i64>>(16)?
                    .map(|v| narrow(v, "carousel_slides"))
                    .transpose()?,
                created_at: row.get(17)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    /// Tokens and comment counts, today and all time.
    ///
    /// **Reads `nurture_comment_attempts`, not `nurture_comment_costs`, and that is the fix
    /// that made this function mean anything.** The costs table has exactly one writer — the
    /// iOS/pixel `do_comment` — so on a fourteen-phone Android fleet it is empty and this
    /// summary reported zero for every run. The attempts table is written by both paths.
    ///
    /// Counting only `outcome = 'sent'` for the comment tallies, but summing tokens over
    /// **every** attempt: a comment the verification gate rejected still burned up to four
    /// API calls, and recording that as free was how the most expensive failure mode became
    /// invisible.
    pub fn nurture_cost_summary(&self) -> anyhow::Result<crate::types::NurtureCostSummary> {
        let conn = self.conn()?;
        let today = Utc::now().format("%Y-%m-%d").to_string();
        const QUERY: &str = "SELECT COALESCE(SUM(prompt_tokens),0),              COALESCE(SUM(completion_tokens),0),              COALESCE(SUM(CASE WHEN outcome = 'sent' THEN 1 ELSE 0 END),0)              FROM nurture_comment_attempts";
        let total: (i64, i64, i64) =
            conn.query_row(QUERY, [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        let today_row: (i64, i64, i64) = conn.query_row(
            &format!("{QUERY} WHERE created_at LIKE ?1"),
            params![format!("{today}%")],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        Ok(crate::types::NurtureCostSummary {
            today_prompt_tokens: today_row.0.max(0) as u64,
            today_completion_tokens: today_row.1.max(0) as u64,
            total_prompt_tokens: total.0.max(0) as u64,
            total_completion_tokens: total.1.max(0) as u64,
            today_comments: narrow(today_row.2, "today_comments")?,
            total_comments: narrow(total.2, "total_comments")?,
        })
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;
    use crate::types::{NurtureCleanupState, NurturePhase, NurtureSessionStatus};

    fn fixture() -> (Database, PathBuf) {
        let path =
            std::env::temp_dir().join(format!("riviu-nurture-history-test-{}.db", Uuid::new_v4()));
        (Database::open(&path).expect("open fixture database"), path)
    }

    fn queued(run_id: Uuid, udid: &str, run_size: u32) -> NurtureSessionStatus {
        NurtureSessionStatus {
            running: true,
            run_id: Some(run_id),
            run_size,
            phase: NurturePhase::Queued,
            updated_at: Some(Utc::now()),
            last_message: "queued".into(),
            ..NurtureSessionStatus::new(udid)
        }
    }

    #[test]
    fn run_history_keeps_the_target_snapshot_and_appends_each_exact_status() {
        let (database, path) = fixture();
        let run_id = Uuid::new_v4();
        let targets = vec!["phone-b".to_string(), "phone-a".to_string()];
        let initial = vec![queued(run_id, "phone-b", 2), queued(run_id, "phone-a", 2)];
        database
            .create_nurture_run(run_id, &targets, &initial)
            .expect("create durable run");

        let mut changed = initial[1].clone();
        changed.phase = NurturePhase::Watching;
        changed.videos_done = 7;
        changed.last_message = "watching card 8".into();
        changed.updated_at = Some(Utc::now());
        database
            .append_nurture_status(&changed)
            .expect("append transition");

        let history = database
            .get_nurture_run(run_id)
            .expect("read history")
            .expect("run exists");
        assert_eq!(history.target_udids, targets, "target order is provenance");
        assert_eq!(history.statuses.len(), 2);
        assert_eq!(
            serde_json::to_value(
                history
                    .statuses
                    .iter()
                    .find(|status| status.udid == "phone-a")
                    .expect("phone-a status")
            )
            .expect("serialize restored status"),
            serde_json::to_value(&changed).expect("serialize expected status"),
            "the exact accepted status must survive serialization"
        );
        assert_eq!(
            database
                .conn()
                .expect("inspect events")
                .query_row(
                    "SELECT COUNT(*) FROM nurture_run_status_events WHERE run_id=?1",
                    [run_id.to_string()],
                    |row| row.get::<_, i64>(0),
                )
                .expect("count events"),
            3,
            "two initial states plus one transition must all remain"
        );

        drop(database);
        std::fs::remove_file(path).expect("remove fixture database");
    }

    #[test]
    fn restart_reconciliation_appends_one_partial_terminal_state() {
        let (database, path) = fixture();
        let run_id = Uuid::new_v4();
        let targets = vec!["phone-a".to_string()];
        let initial = vec![queued(run_id, "phone-a", 1)];
        database
            .create_nurture_run(run_id, &targets, &initial)
            .expect("create durable run");

        let orphaned = database
            .list_orphaned_nurture_statuses()
            .expect("list worker-owned rows before reconciliation");
        assert_eq!(orphaned.len(), 1);
        assert_eq!(orphaned[0].run_id, Some(run_id));
        assert_eq!(orphaned[0].udid, "phone-a");

        assert_eq!(
            database
                .interrupt_orphaned_nurture_sessions()
                .expect("reconcile orphan"),
            1
        );
        let recovered = database
            .get_nurture_run(run_id)
            .expect("read recovered run")
            .expect("run exists")
            .statuses
            .into_iter()
            .next()
            .expect("device status");
        assert!(!recovered.running);
        assert_eq!(recovered.phase, NurturePhase::Finished);
        assert_eq!(recovered.outcome, Some(crate::nurture::Outcome::Partial));
        assert_eq!(recovered.cleanup_state, NurtureCleanupState::Failed);
        assert_eq!(
            recovered.cleanup_error.as_deref(),
            Some("nurture_worker_lost_on_restart")
        );
        assert_eq!(
            database
                .interrupt_orphaned_nurture_sessions()
                .expect("second reconciliation is idempotent"),
            0
        );
        assert_eq!(
            database
                .conn()
                .expect("inspect events")
                .query_row(
                    "SELECT COUNT(*) FROM nurture_run_status_events WHERE run_id=?1",
                    [run_id.to_string()],
                    |row| row.get::<_, i64>(0),
                )
                .expect("count events"),
            2,
            "recovery appends once and never rewrites the running evidence"
        );

        drop(database);
        std::fs::remove_file(path).expect("remove fixture database");
    }
}
