use std::path::{Path, PathBuf};

use anyhow::Context;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::types::{JobRecord, JobStatus, JobStepRecord, StepStatus};

mod flow_runs;
mod flows;
mod migrations;

pub use flow_runs::{AttemptTransitionPatch, FlowStateConflict};
pub(crate) use flow_runs::{FlowAttemptExecutionContext, FlowRecoveryRunContext};

pub struct Database {
    path: PathBuf,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let db = Self { path };
        db.migrate()?;
        Ok(db)
    }

    pub fn default_path() -> anyhow::Result<PathBuf> {
        let base = dirs::data_dir().context("no data dir")?;
        Ok(base.join("riviu-managers-phone").join("riviu.db"))
    }

    fn conn(&self) -> anyhow::Result<Connection> {
        let connection = Connection::open(&self.path)?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(connection)
    }

    fn migrate(&self) -> anyhow::Result<()> {
        let mut conn = self.conn()?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        migrations::run(&mut conn)
    }

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
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn get_device_meta(&self, udid: &str) -> anyhow::Result<crate::types::DeviceMeta> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT udid, notes, tags_json, group_id, proxy_id FROM device_meta WHERE udid = ?1",
        )?;
        let mut rows = stmt.query(params![udid])?;
        if let Some(row) = rows.next()? {
            let tags_json: String = row.get(2)?;
            Ok(crate::types::DeviceMeta {
                udid: row.get(0)?,
                notes: row.get(1)?,
                tags: serde_json::from_str(&tags_json).unwrap_or_default(),
                group_id: row.get(3)?,
                proxy_id: row.get(4)?,
            })
        } else {
            Ok(crate::types::DeviceMeta {
                udid: udid.to_string(),
                notes: String::new(),
                tags: vec![],
                group_id: None,
                proxy_id: None,
            })
        }
    }

    pub fn upsert_device_meta(&self, meta: &crate::types::DeviceMeta) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"INSERT INTO device_meta (udid, notes, tags_json, group_id, proxy_id)
               VALUES (?1,?2,?3,?4,?5)
               ON CONFLICT(udid) DO UPDATE SET
                 notes=excluded.notes, tags_json=excluded.tags_json,
                 group_id=excluded.group_id, proxy_id=excluded.proxy_id"#,
            params![
                meta.udid,
                meta.notes,
                serde_json::to_string(&meta.tags)?,
                meta.group_id,
                meta.proxy_id
            ],
        )?;
        Ok(())
    }

    pub fn list_groups(&self) -> anyhow::Result<Vec<crate::types::DeviceGroup>> {
        let conn = self.conn()?;
        let mut stmt =
            conn.prepare("SELECT id, name, color, created_at FROM groups ORDER BY name")?;
        let groups: Vec<(String, String, String, String)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .filter_map(|r| r.ok())
            .collect();
        let mut out = Vec::new();
        for (id, name, color, created_at) in groups {
            let mut mstmt = conn.prepare("SELECT udid FROM group_members WHERE group_id = ?1")?;
            let udids: Vec<String> = mstmt
                .query_map(params![id], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect();
            out.push(crate::types::DeviceGroup {
                id,
                name,
                color,
                udids,
                created_at,
            });
        }
        Ok(out)
    }

    pub fn upsert_group(&self, group: &crate::types::DeviceGroup) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"INSERT INTO groups (id, name, color, created_at) VALUES (?1,?2,?3,?4)
               ON CONFLICT(id) DO UPDATE SET name=excluded.name, color=excluded.color"#,
            params![group.id, group.name, group.color, group.created_at],
        )?;
        conn.execute(
            "DELETE FROM group_members WHERE group_id = ?1",
            params![group.id],
        )?;
        for udid in &group.udids {
            conn.execute(
                "INSERT OR IGNORE INTO group_members (group_id, udid) VALUES (?1,?2)",
                params![group.id, udid],
            )?;
        }
        Ok(())
    }

    pub fn delete_group(&self, id: &str) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM group_members WHERE group_id = ?1", params![id])?;
        conn.execute("DELETE FROM groups WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn list_proxies(&self) -> anyhow::Result<Vec<crate::types::ProxyConfig>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, proxy_type, host, port, username, password, notes FROM proxies ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(crate::types::ProxyConfig {
                id: row.get(0)?,
                name: row.get(1)?,
                proxy_type: row.get(2)?,
                host: row.get(3)?,
                port: row.get::<_, i64>(4)? as u16,
                username: row.get(5)?,
                password: row.get(6)?,
                notes: row.get(7)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn upsert_proxy(&self, p: &crate::types::ProxyConfig) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"INSERT INTO proxies (id, name, proxy_type, host, port, username, password, notes)
               VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
               ON CONFLICT(id) DO UPDATE SET
                 name=excluded.name, proxy_type=excluded.proxy_type, host=excluded.host,
                 port=excluded.port, username=excluded.username, password=excluded.password,
                 notes=excluded.notes"#,
            params![
                p.id,
                p.name,
                p.proxy_type,
                p.host,
                p.port as i64,
                p.username,
                p.password,
                p.notes
            ],
        )?;
        Ok(())
    }

    pub fn delete_proxy(&self, id: &str) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM proxies WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn list_materials(&self) -> anyhow::Result<Vec<crate::types::MaterialItem>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, path, kind, size, created_at FROM materials ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(crate::types::MaterialItem {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                kind: row.get(3)?,
                size: row.get::<_, i64>(4)? as u64,
                created_at: row.get(5)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn add_material(&self, item: &crate::types::MaterialItem) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO materials (id, name, path, kind, size, created_at) VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                item.id,
                item.name,
                item.path,
                item.kind,
                item.size as i64,
                item.created_at
            ],
        )?;
        Ok(())
    }

    pub fn delete_material(&self, id: &str) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM materials WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn list_apps_library(&self) -> anyhow::Result<Vec<crate::types::AppLibraryItem>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, path, bundle_id, version, created_at FROM apps_library ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(crate::types::AppLibraryItem {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
                bundle_id: row.get(3)?,
                version: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn add_app_library(&self, item: &crate::types::AppLibraryItem) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO apps_library (id, name, path, bundle_id, version, created_at) VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                item.id,
                item.name,
                item.path,
                item.bundle_id,
                item.version,
                item.created_at
            ],
        )?;
        Ok(())
    }

    pub fn delete_app_library(&self, id: &str) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM apps_library WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn list_schedules(&self) -> anyhow::Result<Vec<crate::types::ScheduleItem>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, script_name, udids_json, every_minutes, enabled, last_run_at, next_run_at FROM schedules ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| {
            let udids_json: String = row.get(3)?;
            Ok(crate::types::ScheduleItem {
                id: row.get(0)?,
                name: row.get(1)?,
                script_name: row.get(2)?,
                udids: serde_json::from_str(&udids_json).unwrap_or_default(),
                every_minutes: row.get::<_, i64>(4)? as u32,
                enabled: row.get::<_, i64>(5)? != 0,
                last_run_at: row.get(6)?,
                next_run_at: row.get(7)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn upsert_schedule(&self, s: &crate::types::ScheduleItem) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"INSERT INTO schedules (id, name, script_name, udids_json, every_minutes, enabled, last_run_at, next_run_at)
               VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
               ON CONFLICT(id) DO UPDATE SET
                 name=excluded.name, script_name=excluded.script_name, udids_json=excluded.udids_json,
                 every_minutes=excluded.every_minutes, enabled=excluded.enabled,
                 last_run_at=excluded.last_run_at, next_run_at=excluded.next_run_at"#,
            params![
                s.id,
                s.name,
                s.script_name,
                serde_json::to_string(&s.udids)?,
                s.every_minutes as i64,
                if s.enabled { 1 } else { 0 },
                s.last_run_at,
                s.next_run_at
            ],
        )?;
        Ok(())
    }

    pub fn delete_schedule(&self, id: &str) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM schedules WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn list_publish_tasks(&self) -> anyhow::Result<Vec<crate::types::PublishTask>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, script_name, material_ids_json, udids_json, status, created_at FROM publish_tasks ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            let mid: String = row.get(3)?;
            let uid: String = row.get(4)?;
            Ok(crate::types::PublishTask {
                id: row.get(0)?,
                name: row.get(1)?,
                script_name: row.get(2)?,
                material_ids: serde_json::from_str(&mid).unwrap_or_default(),
                udids: serde_json::from_str(&uid).unwrap_or_default(),
                status: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn add_publish_task(&self, t: &crate::types::PublishTask) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO publish_tasks (id, name, script_name, material_ids_json, udids_json, status, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                t.id,
                t.name,
                t.script_name,
                serde_json::to_string(&t.material_ids)?,
                serde_json::to_string(&t.udids)?,
                t.status,
                t.created_at
            ],
        )?;
        Ok(())
    }

    pub fn update_publish_status(&self, id: &str, status: &str) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE publish_tasks SET status = ?1 WHERE id = ?2",
            params![status, id],
        )?;
        Ok(())
    }

    pub fn list_users(&self) -> anyhow::Result<Vec<crate::types::LocalUser>> {
        let conn = self.conn()?;
        let mut stmt =
            conn.prepare("SELECT id, email, role, created_at FROM users ORDER BY email")?;
        let rows = stmt.query_map([], |row| {
            Ok(crate::types::LocalUser {
                id: row.get(0)?,
                email: row.get(1)?,
                role: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn register_user(
        &self,
        email: &str,
        password: &str,
        role: &str,
    ) -> anyhow::Result<crate::types::LocalUser> {
        let conn = self.conn()?;
        let user = crate::types::LocalUser {
            id: Uuid::new_v4().to_string(),
            email: email.to_string(),
            role: role.to_string(),
            created_at: Utc::now().to_rfc3339(),
        };
        conn.execute(
            "INSERT INTO users (id, email, password_hash, role, created_at) VALUES (?1,?2,?3,?4,?5)",
            params![user.id, user.email, password, user.role, user.created_at],
        )?;
        Ok(user)
    }

    pub fn login_user(
        &self,
        email: &str,
        password: &str,
    ) -> anyhow::Result<Option<crate::types::LocalUser>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, email, role, created_at, password_hash FROM users WHERE email = ?1",
        )?;
        let mut rows = stmt.query(params![email])?;
        if let Some(row) = rows.next()? {
            let hash: String = row.get(4)?;
            if hash != password {
                return Ok(None);
            }
            Ok(Some(crate::types::LocalUser {
                id: row.get(0)?,
                email: row.get(1)?,
                role: row.get(2)?,
                created_at: row.get(3)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn guest_user(&self) -> anyhow::Result<crate::types::LocalUser> {
        let users = self.list_users()?;
        if let Some(u) = users.into_iter().find(|u| u.email == "guest@local") {
            Ok(u)
        } else {
            self.register_user("guest@local", "guest", "admin")
        }
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

    pub fn get_nurture_settings(&self) -> anyhow::Result<crate::types::NurtureSettings> {
        match self.get_setting("nurture.settings")? {
            Some(raw) => Ok(serde_json::from_str(&raw).unwrap_or_default()),
            None => Ok(crate::types::NurtureSettings::default()),
        }
    }

    pub fn save_nurture_settings(
        &self,
        settings: &crate::types::NurtureSettings,
    ) -> anyhow::Result<()> {
        self.set_setting("nurture.settings", &serde_json::to_string(settings)?)
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

    pub fn add_nurture_comment_cost(
        &self,
        cost: &crate::types::NurtureCommentCost,
    ) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"
            INSERT INTO nurture_comment_costs
              (id, udid, model, base_url_host, prompt_tokens, completion_tokens, usd, preview, created_at)
            VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
            "#,
            params![
                cost.id,
                cost.udid,
                cost.model,
                cost.base_url_host,
                cost.prompt_tokens as i64,
                cost.completion_tokens as i64,
                cost.usd,
                cost.preview,
                cost.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn list_nurture_comment_costs(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<crate::types::NurtureCommentCost>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, udid, model, base_url_host, prompt_tokens, completion_tokens, usd, preview, created_at
             FROM nurture_comment_costs ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(crate::types::NurtureCommentCost {
                id: row.get(0)?,
                udid: row.get(1)?,
                model: row.get(2)?,
                base_url_host: row.get(3)?,
                prompt_tokens: row.get::<_, i64>(4)? as u32,
                completion_tokens: row.get::<_, i64>(5)? as u32,
                usd: row.get(6)?,
                preview: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn nurture_cost_summary(&self) -> anyhow::Result<crate::types::NurtureCostSummary> {
        let conn = self.conn()?;
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let total: (f64, i64) = conn.query_row(
            "SELECT COALESCE(SUM(usd),0), COUNT(*) FROM nurture_comment_costs",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let today_row: (f64, i64) = conn.query_row(
            "SELECT COALESCE(SUM(usd),0), COUNT(*) FROM nurture_comment_costs WHERE created_at LIKE ?1",
            params![format!("{today}%")],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(crate::types::NurtureCostSummary {
            today_usd: today_row.0,
            total_usd: total.0,
            today_comments: today_row.1 as u32,
            total_comments: total.1 as u32,
        })
    }
}

struct JobRow {
    id: String,
    script_name: String,
    udids_json: String,
    status: String,
    created_at: String,
    updated_at: String,
    steps_json: String,
    error: Option<String>,
}

impl JobRow {
    fn into_job(self) -> anyhow::Result<JobRecord> {
        Ok(JobRecord {
            id: Uuid::parse_str(&self.id)?,
            script_name: self.script_name,
            udids: serde_json::from_str(&self.udids_json)?,
            status: serde_json::from_str(&self.status).unwrap_or(JobStatus::Failed),
            created_at: DateTime::parse_from_rfc3339(&self.created_at)?.with_timezone(&Utc),
            updated_at: DateTime::parse_from_rfc3339(&self.updated_at)?.with_timezone(&Utc),
            steps: serde_json::from_str::<Vec<JobStepRecord>>(&self.steps_json).unwrap_or_default(),
            error: self.error,
        })
    }
}

pub fn step_label(status: &StepStatus) -> &'static str {
    match status {
        StepStatus::Pending => "pending",
        StepStatus::Running => "running",
        StepStatus::Succeeded => "succeeded",
        StepStatus::Failed => "failed",
        StepStatus::Skipped => "skipped",
    }
}

#[cfg(test)]
mod agent_settings_tests {
    use super::*;
    use crate::types::AgentSettings;

    fn fixture() -> (Database, PathBuf) {
        let path =
            std::env::temp_dir().join(format!("riviu-agent-settings-test-{}.db", Uuid::new_v4()));
        (Database::open(&path).expect("open fixture database"), path)
    }

    #[test]
    fn agent_settings_round_trip_without_secret_fields() {
        let (db, path) = fixture();
        let settings = AgentSettings { auto_repair: false };

        db.save_agent_settings(&settings).expect("save settings");

        assert_eq!(db.get_agent_settings().expect("load settings"), settings);
        let raw = db
            .get_setting("agent.settings.v1")
            .expect("read raw setting")
            .expect("stored setting");
        assert_eq!(raw, r#"{"autoRepair":false}"#);
        assert!(!raw.to_ascii_lowercase().contains("token"));
        std::fs::remove_file(path).expect("remove fixture database");
    }

    #[test]
    fn invalid_agent_settings_json_is_not_silently_defaulted() {
        let (db, path) = fixture();
        db.set_setting("agent.settings.v1", "{not-json")
            .expect("store malformed fixture");

        let error = db
            .get_agent_settings()
            .expect_err("malformed settings must fail");

        assert!(error.to_string().contains("agent.settings.v1"));
        std::fs::remove_file(path).expect("remove fixture database");
    }
}
