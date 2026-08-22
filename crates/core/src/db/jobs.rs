//! The job queue, saved scripts, the key/value settings row, and the operation log.

use super::*;

impl Database {
    pub fn save_job(&self, job: &JobRecord) -> anyhow::Result<()> {
        let conn = self.conn()?;
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
    pub fn list_jobs(&self, limit: usize) -> anyhow::Result<Vec<JobRecord>> {
        let conn = self.conn()?;
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
