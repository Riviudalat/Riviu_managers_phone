//! The job queue, saved scripts, the key/value settings row, and the operation log.

use super::*;

impl Database {
    /// Called once before the new queue admits workers. Persisted intents cannot
    /// be replayed after the worker that owned them has disappeared.
    pub fn recover_script_jobs(&self) -> anyhow::Result<usize> {
        use crate::StepStatus;
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut stmt = tx.prepare(
            "SELECT id, steps_json FROM jobs j WHERE status IN ('\"queued\"','\"running\"')
             OR EXISTS (SELECT 1 FROM operation_device_events e
                WHERE e.source_kind='script' AND e.source_id=j.id AND e.state='running'
                AND e.sequence=(SELECT MAX(last.sequence) FROM operation_device_events last
                    WHERE last.source_kind='script' AND last.source_id=e.source_id
                    AND last.udid=e.udid AND json_extract(last.detail,'$.stepIndex')=json_extract(e.detail,'$.stepIndex')))",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        for (id, raw) in &rows {
            // Only devices that actually wrote a running step get a recovery
            // event. Never manufacture events for the remaining roster.
            let recovered = tx.execute(
                "INSERT INTO operation_device_events
                (source_kind,source_id,udid,action,state,recorded_at,text,detail)
                SELECT 'script',e.source_id,e.udid,e.action,'uncertain',?2,
                'Worker lost after script intent; do not replay',
                json_set(COALESCE(e.detail,'{}'),'$.recovered',json('true'),'$.elapsedMs',NULL)
                FROM operation_device_events e WHERE e.source_kind='script' AND e.source_id=?1
                AND e.state='running' AND e.sequence=(SELECT MAX(last.sequence)
                FROM operation_device_events last WHERE last.source_kind='script'
                AND last.source_id=e.source_id AND last.udid=e.udid
                AND json_extract(last.detail,'$.stepIndex')=json_extract(e.detail,'$.stepIndex'))",
                params![id, Utc::now().to_rfc3339()],
            )?;
            let mut steps: Vec<JobStepRecord> = serde_json::from_str(raw)?;
            // A stale aggregate may have lost an intent; recover each ledger step
            // independently before considering the legacy aggregate-only states.
            let mut statement = tx.prepare("SELECT json_extract(detail,'$.stepIndex'),MIN(udid) FROM operation_device_events WHERE source_kind='script' AND source_id=?1 AND state='uncertain' AND sequence IN (SELECT MAX(sequence) FROM operation_device_events WHERE source_kind='script' AND source_id=?1 GROUP BY udid,json_extract(detail,'$.stepIndex')) GROUP BY json_extract(detail,'$.stepIndex')")?;
            let indices = statement
                .query_map([id], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(statement);
            for (index, udid) in indices {
                if let Some(step) = usize::try_from(index)
                    .ok()
                    .and_then(|index| steps.get_mut(index))
                {
                    step.status = StepStatus::Uncertain;
                    step.error = Some(format!(
                        "{udid}: worker lost after script intent; do not replay"
                    ));
                    step.artifact_path = None;
                }
            }
            let uncertain = recovered > 0
                || steps
                    .iter()
                    .any(|s| matches!(s.status, StepStatus::Running | StepStatus::Uncertain));
            for step in &mut steps {
                match step.status {
                    StepStatus::Running => {
                        step.status = StepStatus::Uncertain;
                        step.error = Some(
                            "Worker lost after intent; effect may have occurred. Do not replay."
                                .into(),
                        );
                    }
                    StepStatus::Pending => step.status = StepStatus::Skipped,
                    _ => {}
                }
            }
            let status = if uncertain {
                JobStatus::Uncertain
            } else {
                JobStatus::Cancelled
            };
            tx.execute(
                "UPDATE jobs SET status=?2,steps_json=?3,error=CASE WHEN error IS NULL THEN ?4 ELSE error || '; ' || ?4 END,updated_at=?5 WHERE id=?1",
                params![
                    id,
                    serde_json::to_string(&status)?,
                    serde_json::to_string(&steps)?,
                    "Script interrupted by application restart; automatic replay disabled",
                    Utc::now().to_rfc3339()
                ],
            )?;
        }
        tx.commit()?;
        Ok(rows.len())
    }

    pub fn save_job(&self, job: &JobRecord) -> anyhow::Result<()> {
        let conn = self.conn()?;
        save_job_on(&conn, job)
    }

    /// The per-device timeline and its source checkpoint commit together.
    pub fn save_job_device_step(
        &self,
        job: &JobRecord,
        udid: &str,
        index: usize,
        session_id: &str,
        elapsed_ms: u64,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            job.udids.iter().any(|id| id == udid),
            "Device outside script snapshot"
        );
        let step = job.steps.get(index).context("Script step missing")?;
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = serde_json::to_value(&step.status)?;
        tx.execute("INSERT INTO operation_device_events
            (source_kind,source_id,udid,action,state,recorded_at,text,detail)
            VALUES ('script',?1,?2,?3,?4,?5,?6,?7)", params![
            job.id.to_string(), udid, step.action, state.as_str().context("Script state missing")?,
            job.updated_at.to_rfc3339(), step.error,
            serde_json::json!({"stepIndex":index,"sessionId":session_id,"elapsedMs":elapsed_ms,"artifact":step.artifact_path}).to_string(),
        ])?;
        let mut merged = job.clone();
        merge_job_ledger_on(&tx, &mut merged, false)?;
        save_job_on(&tx, &merged)?;
        tx.commit()?;
        Ok(())
    }

    /// Reconcile reporting debt without inserting an event or saving a private
    /// worker's aggregate. The current ledger remains the evidence owner.
    pub fn save_job_reporting_error(&self, job: &JobRecord) -> anyhow::Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut merged = job.clone();
        merge_job_ledger_on(&tx, &mut merged, false)?;
        save_job_on(&tx, &merged)?;
        tx.commit()?;
        Ok(())
    }

    /// Every worker has drained. Missing observations mean never started;
    /// outstanding durable intents remain uncertain, including failed reports.
    pub fn settle_script_job(&self, job: &mut JobRecord) -> anyhow::Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        merge_job_ledger_on(&tx, job, true)?;
        if job
            .steps
            .iter()
            .any(|step| step.status == crate::StepStatus::Uncertain)
        {
            job.status = JobStatus::Uncertain;
        }
        save_job_on(&tx, job)?;
        tx.commit()?;
        Ok(())
    }

    pub fn list_jobs(&self, limit: usize) -> anyhow::Result<Vec<JobRecord>> {
        let conn = self.conn()?;
        list_jobs_on(&conn, limit)
    }
}

fn merge_job_ledger_on(
    conn: &Connection,
    job: &mut JobRecord,
    terminal: bool,
) -> anyhow::Result<()> {
    use crate::StepStatus;
    let mut latest = std::collections::BTreeMap::new();
    let mut statement = conn.prepare(
        "SELECT e.udid,json_extract(e.detail,'$.stepIndex'),e.state,e.text,json_extract(e.detail,'$.artifact')
         FROM operation_device_events e JOIN (
            SELECT MAX(sequence) sequence FROM operation_device_events
            WHERE source_kind='script' AND source_id=?1
            GROUP BY udid,json_extract(detail,'$.stepIndex')
         ) newest ON newest.sequence=e.sequence",
    )?;
    let rows = statement.query_map([job.id.to_string()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;
    for row in rows {
        let (id, index, state, error, artifact) = row?;
        let mut status = serde_json::from_value::<StepStatus>(serde_json::Value::String(state))?;
        if terminal && status == StepStatus::Running {
            status = StepStatus::Uncertain;
        }
        latest.insert((usize::try_from(index)?, id), (status, error, artifact));
    }
    for (index, step) in job.steps.iter_mut().enumerate() {
        step.status = StepStatus::aggregate(job.udids.iter().map(|id| {
            latest
                .get(&(index, id.clone()))
                .map(|row| row.0.clone())
                .unwrap_or(if terminal {
                    StepStatus::Skipped
                } else {
                    StepStatus::Pending
                })
        }));
        // Pick one deterministic device whose observation owns the aggregate
        // state. Its diagnostic and artifact stay together, with device identity.
        let selected = latest
            .iter()
            .find(|((i, id), row)| *i == index && job.udids.contains(id) && row.0 == step.status);
        step.error = selected
            .and_then(|((_, id), row)| row.1.as_ref().map(|error| format!("{id}: {error}")));
        step.artifact_path = selected.and_then(|(_, row)| row.2.clone());
        if let Some(((_, id), _)) = selected {
            if step.status == StepStatus::Uncertain && step.error.is_none() {
                step.error = Some(format!("{id}: outstanding script intent; do not replay"));
            }
        }
    }
    let previous: Option<String> = conn
        .query_row(
            "SELECT error FROM jobs WHERE id=?1",
            [job.id.to_string()],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    if let Some(previous) = previous {
        if !job
            .error
            .as_deref()
            .is_some_and(|error| error.contains(&previous))
        {
            job.error = Some(match job.error.take() {
                Some(error) => format!("{previous}; {error}"),
                None => previous,
            });
        }
    }
    Ok(())
}

fn save_job_on(conn: &Connection, job: &JobRecord) -> anyhow::Result<()> {
    conn.execute(
            r#"
            INSERT INTO jobs (id, script_name, udids_json, status, created_at, updated_at, steps_json, error)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(id) DO UPDATE SET
              status=excluded.status,
              updated_at=excluded.updated_at,
              steps_json=excluded.steps_json,
              error=excluded.error
            "#,
            params![
                job.id.to_string(),
                job.script_name,
                serde_json::to_string(&job.udids)?,
                serde_json::to_string(&job.status)?,
                job.created_at.to_rfc3339(),
                job.updated_at.to_rfc3339(),
                serde_json::to_string(&job.steps)?,
                job.error,
            ],
        )?;
    Ok(())
}
fn list_jobs_on(conn: &Connection, limit: usize) -> anyhow::Result<Vec<JobRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, script_name, udids_json, status, created_at, updated_at, steps_json, error
             FROM jobs ORDER BY created_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok(JobRow {
            id: row.get(0)?,
            script_name: row.get(1)?,
            udids_json: row.get(2)?,
            status: row.get(3)?,
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
            steps_json: row.get(6)?,
            error: row.get(7)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?.into_job()?);
    }
    Ok(out)
}

impl Database {
    pub fn get_job(&self, id: Uuid) -> anyhow::Result<Option<JobRecord>> {
        let conn = self.conn()?;
        let row = conn
            .query_row(
                "SELECT id, script_name, udids_json, status, created_at, updated_at, steps_json, error
                 FROM jobs WHERE id = ?1",
                params![id.to_string()],
                |row| {
                    Ok(JobRow {
                        id: row.get(0)?,
                        script_name: row.get(1)?,
                        udids_json: row.get(2)?,
                        status: row.get(3)?,
                        created_at: row.get(4)?,
                        updated_at: row.get(5)?,
                        steps_json: row.get(6)?,
                        error: row.get(7)?,
                    })
                },
            )
            .optional()?;
        row.map(JobRow::into_job).transpose()
    }
    pub fn save_script(&self, name: &str, body_json: &str) -> anyhow::Result<()> {
        let conn = self.conn()?;
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            r#"
            INSERT INTO scripts (id, name, body_json, updated_at)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(name) DO UPDATE SET body_json=excluded.body_json, updated_at=excluded.updated_at
            "#,
            params![id, name, body_json, now],
        )?;
        Ok(())
    }
    pub fn list_scripts(&self) -> anyhow::Result<Vec<(String, String)>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT name, body_json FROM scripts ORDER BY name")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
    pub fn get_script(&self, name: &str) -> anyhow::Result<Option<String>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT body_json FROM scripts WHERE name = ?1")?;
        let mut rows = stmt.query(params![name])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }
    pub fn set_setting(&self, key: &str, value: &str) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
    pub fn get_setting(&self, key: &str) -> anyhow::Result<Option<String>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }
    pub fn log_op(&self, action: &str, detail: &str) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO op_logs (id, action, detail, created_at) VALUES (?1,?2,?3,?4)",
            params![
                Uuid::new_v4().to_string(),
                action,
                detail,
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }
    pub fn list_op_logs(&self, limit: usize) -> anyhow::Result<Vec<crate::types::OpLog>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, action, detail, created_at FROM op_logs ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(crate::types::OpLog {
                id: row.get(0)?,
                action: row.get(1)?,
                detail: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
