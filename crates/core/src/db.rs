use std::path::{Path, PathBuf};

use anyhow::Context;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
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

const NURTURE_SETTINGS_MIGRATION_V2: &str = "nurture.settings.migration.v2";

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
            "INSERT INTO publish_dispatch(campaign_id,state,owner,claimed_at,updated_at) VALUES (?1,?2,NULL,NULL,?3)",
            params![campaign_id, state.as_str(), now],
        )?;
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
                    ordinal: row.get::<_, i64>(2)? as u32,
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
            "UPDATE publish_dispatch SET state=?1,updated_at=?2 WHERE campaign_id=?3",
            params![state.as_str(), now, id],
        )?;
        transaction.execute(
            "INSERT INTO publish_events(campaign_id,revision,kind,payload_json,created_at) VALUES (?1,?2,'state',?3,?4)",
            params![id, revision, payload.to_string(), now],
        )?;
        transaction.commit()?;
        Ok(())
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

    pub fn cancel_publish_campaign(&self, id: &str) -> anyhow::Result<()> {
        self.update_publish_campaign_state(
            id,
            crate::publish::PublishCampaignState::Cancelled,
            None,
        )
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
            Some(raw) => {
                let mut settings: crate::types::NurtureSettings = serde_json::from_str(&raw)
                    .context("invalid JSON in stored setting nurture.settings")?;
                if self.get_setting(NURTURE_SETTINGS_MIGRATION_V2)?.is_none() {
                    settings.migrate_legacy_defaults();
                    // Re-serializing also drops obsolete risk-guard keys that
                    // were accepted by the old profile schema.
                    self.save_nurture_settings(&settings)?;
                }
                Ok(settings)
            }
            None => Ok(crate::types::NurtureSettings::default()),
        }
    }

    pub fn save_nurture_settings(
        &self,
        settings: &crate::types::NurtureSettings,
    ) -> anyhow::Result<()> {
        self.set_setting("nurture.settings", &serde_json::to_string(settings)?)?;
        self.set_setting(NURTURE_SETTINGS_MIGRATION_V2, "2026-08-06-human-v2")
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

    pub fn add_nurture_comment_attempt(
        &self,
        attempt: &crate::types::NurtureCommentAttempt,
    ) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"
            INSERT INTO nurture_comment_attempts
              (id, udid, outcome, source, model, base_url_host, prompt_tokens,
               completion_tokens, usd, preview, caption_preview, frame_sha256,
               context_confidence, relevance, evidence_support, created_at)
            VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
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
                attempt.usd,
                attempt.preview,
                attempt.caption_preview,
                attempt.frame_sha256,
                attempt.context_confidence.map(i64::from),
                attempt.relevance.map(i64::from),
                attempt.evidence_support.map(i64::from),
                attempt.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn update_nurture_comment_attempt_outcome(
        &self,
        id: &str,
        outcome: &str,
    ) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE nurture_comment_attempts SET outcome=?2 WHERE id=?1",
            params![id, outcome],
        )?;
        Ok(())
    }

    pub fn list_nurture_comment_attempts(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<crate::types::NurtureCommentAttempt>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id,udid,outcome,source,model,base_url_host,prompt_tokens,
                    completion_tokens,usd,preview,caption_preview,frame_sha256,
                    context_confidence,relevance,evidence_support,created_at
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
                prompt_tokens: row.get::<_, i64>(6)? as u32,
                completion_tokens: row.get::<_, i64>(7)? as u32,
                usd: row.get(8)?,
                preview: row.get(9)?,
                caption_preview: row.get(10)?,
                frame_sha256: row.get(11)?,
                context_confidence: row.get::<_, Option<i64>>(12)?.map(|v| v as u8),
                relevance: row.get::<_, Option<i64>>(13)?.map(|v| v as u8),
                evidence_support: row.get::<_, Option<i64>>(14)?.map(|v| v as u8),
                created_at: row.get(15)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
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

    pub fn create_interaction_campaign(
        &self,
        request: &crate::interaction::ThreadCampaignRequest,
        plan: &crate::interaction::ThreadPlan,
    ) -> anyhow::Result<String> {
        request.validate().map_err(|error| anyhow::anyhow!(error))?;
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let campaign_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        transaction.execute(
            "INSERT INTO interaction_campaigns
             (id,request_id,request_json,state,message_count,revision,created_at,updated_at)
             VALUES (?1,?2,?3,'queued',?4,0,?5,?5)",
            params![
                campaign_id,
                request.request_id,
                serde_json::to_string(request)?,
                i64::from(request.message_count),
                now,
            ],
        )?;
        for (ordinal, udid) in request.actor_udids.iter().enumerate() {
            transaction.execute(
                "INSERT INTO interaction_campaign_actors
                 (campaign_id,actor_ordinal,udid) VALUES (?1,?2,?3)",
                params![campaign_id, ordinal as i64, udid],
            )?;
        }

        let mut target_ids = std::collections::HashMap::new();
        for (line_index, target) in request.targets.iter().enumerate() {
            let target_id = Uuid::new_v4().to_string();
            let kind = match target.kind {
                crate::interaction::TikTokPostKind::Video => "video",
                crate::interaction::TikTokPostKind::Photo => "photo",
            };
            transaction.execute(
                "INSERT INTO interaction_targets
                 (id,campaign_id,line_no,original_url,normalized_url,target_key,content_id,kind,created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    target_id,
                    campaign_id,
                    (line_index + 1) as i64,
                    target.original_url,
                    target.normalized_url,
                    target.target_key,
                    target.content_id,
                    kind,
                    now,
                ],
            )?;
            target_ids.insert(target.target_key.clone(), target_id);
        }

        let mut assignment_ids = std::collections::HashMap::new();
        for assignment in &plan.assignments {
            let assignment_id = Uuid::new_v4().to_string();
            let target_id = target_ids
                .get(&assignment.target_key)
                .ok_or_else(|| anyhow::anyhow!("plan target is missing from request"))?;
            let parent_id = assignment.parent_ordinal.and_then(|ordinal| {
                assignment_ids
                    .get(&(assignment.target_key.clone(), ordinal))
                    .cloned()
            });
            transaction.execute(
                "INSERT INTO interaction_assignments
                 (id,campaign_id,target_id,message_ordinal,actor_udid,parent_assignment_id,created_at,updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?7)",
                params![
                    assignment_id,
                    campaign_id,
                    target_id,
                    i64::from(assignment.ordinal),
                    assignment.actor_udid,
                    parent_id,
                    now,
                ],
            )?;
            assignment_ids.insert(
                (assignment.target_key.clone(), assignment.ordinal),
                assignment_id,
            );
        }
        transaction.execute(
            "INSERT INTO interaction_dispatch(campaign_id,state,updated_at) VALUES(?1,'queued',?2)",
            params![campaign_id, now],
        )?;
        transaction.commit()?;
        Ok(campaign_id)
    }

    pub fn list_interaction_campaigns(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<crate::interaction::InteractionCampaignSummary>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            "SELECT c.id,c.request_id,c.state,c.message_count,c.updated_at,
                    (SELECT COUNT(*) FROM interaction_targets t WHERE t.campaign_id=c.id),
                    (SELECT COUNT(*) FROM interaction_assignments a WHERE a.campaign_id=c.id AND a.state='succeeded'),
                    (SELECT COUNT(*) FROM interaction_assignments a WHERE a.campaign_id=c.id AND a.state IN ('failed','uncertain','skipped_parent'))
             FROM interaction_campaigns c ORDER BY c.updated_at DESC LIMIT ?1",
        )?;
        let rows = statement.query_map(params![limit as i64], interaction_summary_from_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn get_interaction_campaign(
        &self,
        campaign_id: &str,
    ) -> anyhow::Result<Option<crate::interaction::InteractionCampaignDetail>> {
        let conn = self.conn()?;
        let summary = conn
            .query_row(
                "SELECT c.id,c.request_id,c.state,c.message_count,c.updated_at,
                        (SELECT COUNT(*) FROM interaction_targets t WHERE t.campaign_id=c.id),
                        (SELECT COUNT(*) FROM interaction_assignments a WHERE a.campaign_id=c.id AND a.state='succeeded'),
                        (SELECT COUNT(*) FROM interaction_assignments a WHERE a.campaign_id=c.id AND a.state IN ('failed','uncertain','skipped_parent'))
                 FROM interaction_campaigns c WHERE c.id=?1",
                params![campaign_id],
                interaction_summary_from_row,
            )
            .optional()?;
        let Some(summary) = summary else {
            return Ok(None);
        };
        let mut statement = conn.prepare(
            "SELECT a.id,t.target_key,a.message_ordinal,a.actor_udid,a.parent_assignment_id,
                    a.state,a.prepared_json,a.error_code
             FROM interaction_assignments a
             JOIN interaction_targets t ON t.id=a.target_id
             WHERE a.campaign_id=?1 ORDER BY t.line_no,a.message_ordinal",
        )?;
        let rows = statement.query_map(params![campaign_id], |row| {
            let state: String = row.get(5)?;
            let prepared_json: Option<String> = row.get(6)?;
            let prepared_text = prepared_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                .and_then(|value| {
                    value
                        .get("text")
                        .and_then(|text| text.as_str())
                        .map(str::to_string)
                });
            Ok(crate::interaction::InteractionAssignmentRecord {
                id: row.get(0)?,
                target_key: row.get(1)?,
                ordinal: row.get::<_, i64>(2)? as u8,
                actor_udid: row.get(3)?,
                parent_assignment_id: row.get(4)?,
                state: interaction_message_state(&state),
                prepared_text,
                error_code: row.get(7)?,
            })
        })?;
        Ok(Some(crate::interaction::InteractionCampaignDetail {
            summary,
            assignments: rows.collect::<Result<Vec<_>, _>>()?,
        }))
    }

    pub fn get_interaction_campaign_request(
        &self,
        campaign_id: &str,
    ) -> anyhow::Result<
        Option<(
            crate::interaction::ThreadCampaignRequest,
            crate::interaction::ThreadPlan,
        )>,
    > {
        let conn = self.conn()?;
        let raw = conn
            .query_row(
                "SELECT request_json FROM interaction_campaigns WHERE id=?1",
                params![campaign_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        let request = serde_json::from_str::<crate::interaction::ThreadCampaignRequest>(&raw)?;
        let plan = crate::interaction::plan_threads(&request)
            .map_err(|error| anyhow::anyhow!("persisted interaction plan invalid: {error}"))?;
        Ok(Some((request, plan)))
    }

    pub fn update_interaction_campaign_state(
        &self,
        campaign_id: &str,
        state: crate::interaction::ThreadCampaignState,
        error_code: Option<&str>,
    ) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE interaction_campaigns SET state=?1,revision=revision+1,error_code=?2,updated_at=?3 WHERE id=?4",
            params![interaction_campaign_state_label(state), error_code, Utc::now().to_rfc3339(), campaign_id],
        )?;
        Ok(())
    }

    pub fn prepare_interaction_assignment(
        &self,
        assignment_id: &str,
        prepared: &crate::interaction::PreparedThreadMessage,
    ) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE interaction_assignments
             SET prepared_json=?1,state='ready',revision=revision+1,updated_at=?2
             WHERE id=?3 AND effect_intent IS NULL",
            params![
                serde_json::to_string(prepared)?,
                Utc::now().to_rfc3339(),
                assignment_id
            ],
        )?;
        Ok(())
    }

    pub fn update_interaction_assignment_state(
        &self,
        assignment_id: &str,
        state: crate::interaction::ThreadMessageState,
        error_code: Option<&str>,
        effect_intent: Option<&str>,
        evidence_json: Option<&str>,
    ) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE interaction_assignments SET state=?1,error_code=?2,effect_intent=COALESCE(?3,effect_intent),evidence_json=COALESCE(?4,evidence_json),revision=revision+1,updated_at=?5 WHERE id=?6",
            params![
                interaction_message_state_label(state),
                error_code,
                effect_intent,
                evidence_json,
                Utc::now().to_rfc3339(),
                assignment_id,
            ],
        )?;
        Ok(())
    }

    /// Saved frames for a campaign, newest first. Rows without a
    /// `relative_path` predate evidence storage and have no file behind them.
    pub fn list_interaction_artifacts(
        &self,
        campaign_id: &str,
    ) -> anyhow::Result<Vec<crate::interaction::InteractionArtifactRecord>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            "SELECT id,assignment_id,kind,relative_path,sha256,created_at
             FROM interaction_artifacts WHERE campaign_id=?1 ORDER BY created_at DESC",
        )?;
        let rows = statement.query_map(params![campaign_id], |row| {
            Ok(crate::interaction::InteractionArtifactRecord {
                id: row.get(0)?,
                assignment_id: row.get(1)?,
                kind: row.get(2)?,
                relative_path: row.get(3)?,
                sha256: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// One saved frame by id, for reading its bytes back.
    pub fn get_interaction_artifact(
        &self,
        artifact_id: &str,
    ) -> anyhow::Result<Option<crate::interaction::InteractionArtifactRecord>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                "SELECT id,assignment_id,kind,relative_path,sha256,created_at
                 FROM interaction_artifacts WHERE id=?1",
                params![artifact_id],
                |row| {
                    Ok(crate::interaction::InteractionArtifactRecord {
                        id: row.get(0)?,
                        assignment_id: row.get(1)?,
                        kind: row.get(2)?,
                        relative_path: row.get(3)?,
                        sha256: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                },
            )
            .optional()?)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_interaction_artifact(
        &self,
        campaign_id: &str,
        target_key: &str,
        assignment_id: Option<&str>,
        kind: &str,
        metadata_json: &str,
        sha256: &str,
        relative_path: Option<&str>,
    ) -> anyhow::Result<String> {
        let conn = self.conn()?;
        let target_id: String = conn.query_row(
            "SELECT id FROM interaction_targets WHERE campaign_id=?1 AND target_key=?2",
            params![campaign_id, target_key],
            |row| row.get(0),
        )?;
        let id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO interaction_artifacts
             (id,campaign_id,target_id,assignment_id,kind,metadata_json,relative_path,sha256,created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                id,
                campaign_id,
                target_id,
                assignment_id,
                kind,
                metadata_json,
                relative_path,
                sha256,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(id)
    }
}

fn publish_state_from_str(value: &str) -> crate::publish::PublishCampaignState {
    match value {
        "queued" => crate::publish::PublishCampaignState::Queued,
        "scheduled" => crate::publish::PublishCampaignState::Scheduled,
        "preparing" => crate::publish::PublishCampaignState::Preparing,
        "ready" => crate::publish::PublishCampaignState::Ready,
        "transferring" => crate::publish::PublishCampaignState::Transferring,
        "imported" => crate::publish::PublishCampaignState::Imported,
        "posting" => crate::publish::PublishCampaignState::Posting,
        "verifying" => crate::publish::PublishCampaignState::Verifying,
        "succeeded" => crate::publish::PublishCampaignState::Succeeded,
        "failed_before_dispatch" => crate::publish::PublishCampaignState::FailedBeforeDispatch,
        "uncertain" => crate::publish::PublishCampaignState::Uncertain,
        "cancelled" => crate::publish::PublishCampaignState::Cancelled,
        "missed" => crate::publish::PublishCampaignState::Missed,
        _ => crate::publish::PublishCampaignState::Uncertain,
    }
}

fn interaction_campaign_state_label(
    state: crate::interaction::ThreadCampaignState,
) -> &'static str {
    match state {
        crate::interaction::ThreadCampaignState::Queued => "queued",
        crate::interaction::ThreadCampaignState::Running => "running",
        crate::interaction::ThreadCampaignState::Succeeded => "succeeded",
        crate::interaction::ThreadCampaignState::Partial => "partial",
        crate::interaction::ThreadCampaignState::Failed => "failed",
        crate::interaction::ThreadCampaignState::Cancelled => "cancelled",
    }
}

fn interaction_message_state_label(state: crate::interaction::ThreadMessageState) -> &'static str {
    match state {
        crate::interaction::ThreadMessageState::Queued => "queued",
        crate::interaction::ThreadMessageState::Preparing => "preparing",
        crate::interaction::ThreadMessageState::Ready => "ready",
        crate::interaction::ThreadMessageState::Sending => "sending",
        crate::interaction::ThreadMessageState::Succeeded => "succeeded",
        crate::interaction::ThreadMessageState::Failed => "failed",
        crate::interaction::ThreadMessageState::Uncertain => "uncertain",
        crate::interaction::ThreadMessageState::SkippedParent => "skipped_parent",
    }
}

fn interaction_message_state(value: &str) -> crate::interaction::ThreadMessageState {
    match value {
        "preparing" => crate::interaction::ThreadMessageState::Preparing,
        "ready" => crate::interaction::ThreadMessageState::Ready,
        "sending" => crate::interaction::ThreadMessageState::Sending,
        "succeeded" => crate::interaction::ThreadMessageState::Succeeded,
        "failed" => crate::interaction::ThreadMessageState::Failed,
        "uncertain" => crate::interaction::ThreadMessageState::Uncertain,
        "skipped_parent" => crate::interaction::ThreadMessageState::SkippedParent,
        _ => crate::interaction::ThreadMessageState::Queued,
    }
}

fn interaction_summary_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<crate::interaction::InteractionCampaignSummary> {
    let state: String = row.get(2)?;
    Ok(crate::interaction::InteractionCampaignSummary {
        id: row.get(0)?,
        request_id: row.get(1)?,
        state: match state.as_str() {
            "running" => crate::interaction::ThreadCampaignState::Running,
            "succeeded" => crate::interaction::ThreadCampaignState::Succeeded,
            "partial" => crate::interaction::ThreadCampaignState::Partial,
            "failed" => crate::interaction::ThreadCampaignState::Failed,
            "cancelled" => crate::interaction::ThreadCampaignState::Cancelled,
            _ => crate::interaction::ThreadCampaignState::Queued,
        },
        message_count: row.get::<_, i64>(3)? as u8,
        updated_at: row.get(4)?,
        target_count: row.get::<_, i64>(5)? as u32,
        succeeded_messages: row.get::<_, i64>(6)? as u32,
        failed_messages: row.get::<_, i64>(7)? as u32,
    })
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

#[cfg(test)]
mod nurture_settings_migration_tests {
    use super::*;
    use crate::types::NurtureSettings;

    fn fixture() -> (Database, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "riviu-nurture-settings-migration-test-{}.db",
            Uuid::new_v4()
        ));
        (Database::open(&path).expect("open fixture database"), path)
    }

    #[test]
    fn stored_legacy_profile_is_migrated_once_and_obsolete_keys_are_removed() {
        let (db, path) = fixture();
        let legacy = serde_json::json!({
            "baseUrl": "https://api.deepseek.com",
            "model": "custom-model",
            "apiKey": "fixture-key",
            "inputPricePer1m": 1.25,
            "outputPricePer1m": 10.0,
            "bundleId": "com.ss.iphone.ugc.Ame",
            "numVideos": 50,
            "numRounds": 1,
            "likeProb": 40,
            "commentProb": 25,
            "followProb": 5,
            "frenzyProb": 8,
            "watchMin": 5.0,
            "watchMax": 20.0,
            "persona": "custom-persona",
            "fatigue": true,
            "timeOfDay": true,
            "pauseSwipe": true,
            "nightStart": 0,
            "nightEnd": 0,
            "recoverDelayMin": 2,
            "recoverDelayMax": 4,
            "staggerDelayMin": 5,
            "staggerDelayMax": 15,
            "commentLang": "vi",
            "aiDirections": "custom",
            "maxCommentWords": 12,
            "riskGuardEnabled": true,
            "riskMaxLikes": 10,
            "scheduleEnabled": false,
            "scheduleEveryMinutes": 60,
            "scheduleDurationMinutes": 20,
            "scheduleUdids": ["fixture-device"]
        });
        db.set_setting("nurture.settings", &legacy.to_string())
            .expect("store legacy profile");

        let migrated = db.get_nurture_settings().expect("load migrated profile");
        assert_eq!(migrated.num_videos, 120);
        assert_eq!(migrated.like_prob, 35);
        assert_eq!(migrated.comment_prob, 0);
        assert_eq!(migrated.follow_prob, 3);
        assert_eq!(migrated.frenzy_prob, 6);
        assert_eq!((migrated.watch_min, migrated.watch_max), (3.0, 18.0));
        assert_eq!(migrated.schedule_every_minutes, 240);
        assert_eq!(migrated.schedule_duration_minutes, 150);
        assert_eq!(migrated.api_key, "fixture-key");
        assert_eq!(migrated.model, "custom-model");
        assert_eq!(migrated.persona, "custom-persona");
        assert_eq!(migrated.schedule_udids, vec!["fixture-device"]);

        let raw = db
            .get_setting("nurture.settings")
            .expect("read normalized profile")
            .expect("normalized profile exists");
        assert!(!raw.contains("riskGuard"));
        assert_eq!(
            db.get_setting(NURTURE_SETTINGS_MIGRATION_V2)
                .expect("read migration marker")
                .as_deref(),
            Some("2026-08-06-human-v2")
        );

        // A second read does not reapply migration or alter the normalized
        // profile, which keeps manual edits stable after the first launch.
        let second = db
            .get_nurture_settings()
            .expect("reload normalized profile");
        assert_eq!(second.num_videos, migrated.num_videos);
        assert_eq!(second.comment_prob, migrated.comment_prob);
        assert_eq!(second.api_key, migrated.api_key);
        assert_eq!(second.schedule_udids, migrated.schedule_udids);
        std::fs::remove_file(path).expect("remove fixture database");
    }

    #[test]
    fn saving_a_new_profile_marks_it_as_already_migrated() {
        let (db, path) = fixture();
        let settings = NurtureSettings::default();
        db.save_nurture_settings(&settings).expect("save profile");
        assert_eq!(
            db.get_setting(NURTURE_SETTINGS_MIGRATION_V2)
                .expect("read migration marker")
                .as_deref(),
            Some("2026-08-06-human-v2")
        );
        std::fs::remove_file(path).expect("remove fixture database");
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    use crate::interaction::{
        plan_threads, PreparedThreadMessage, ResolvedTikTokTarget, ThreadCampaignRequest,
        ThreadMessageState, ThreadMode, TikTokPostKind,
    };

    fn fixture() -> (Database, PathBuf) {
        let path =
            std::env::temp_dir().join(format!("riviu-interaction-test-{}.db", Uuid::new_v4()));
        (Database::open(&path).expect("open fixture database"), path)
    }

    fn request() -> ThreadCampaignRequest {
        ThreadCampaignRequest {
            request_id: "interaction-db-1".into(),
            targets: vec![ResolvedTikTokTarget {
                original_url: "https://www.tiktok.com/@creator/video/123".into(),
                normalized_url: "https://www.tiktok.com/@creator/video/123".into(),
                target_key: "content:123".into(),
                content_id: "123".into(),
                author: "creator".into(),
                kind: TikTokPostKind::Video,
            }],
            actor_udids: vec!["actor-a".into(), "actor-b".into()],
            message_count: 2,
            instruction: "tự nhiên".into(),
            max_words: 12,
            // Both default on the wire; a fixture spells them out so the shape stays
            // visible and a new field cannot be forgotten silently.
            manual_comments: Vec::new(),
            like_target: false,

            mode: ThreadMode::Threaded,
        }
    }

    #[test]
    fn interaction_campaign_persists_plan_prepared_text_and_evidence_atomically() {
        let (db, path) = fixture();
        let request = request();
        let plan = plan_threads(&request).expect("plan");
        let campaign_id = db
            .create_interaction_campaign(&request, &plan)
            .expect("create campaign");
        let detail = db
            .get_interaction_campaign(&campaign_id)
            .expect("get campaign")
            .expect("campaign exists");
        assert_eq!(detail.assignments.len(), 2);
        let first = &plan.assignments[0];
        let prepared = PreparedThreadMessage::new(first, "  món này   nhìn ngon quá ");
        db.prepare_interaction_assignment(&detail.assignments[0].id, &prepared)
            .expect("prepare");
        db.update_interaction_assignment_state(
            &detail.assignments[0].id,
            ThreadMessageState::Sending,
            None,
            Some("post_comment"),
            None,
        )
        .expect("intent");
        db.update_interaction_assignment_state(
            &detail.assignments[0].id,
            ThreadMessageState::Succeeded,
            None,
            None,
            Some(r#"{"armedFrameSha256":"a","clearedFrameSha256":"b"}"#),
        )
        .expect("evidence");
        db.add_interaction_artifact(
            &campaign_id,
            "content:123",
            Some(&detail.assignments[0].id),
            "comment-root-evidence",
            r#"{"fixture":true}"#,
            "fixture-sha",
            Some("campaign/assignment/attempt/artifact.jpeg"),
        )
        .expect("artifact");
        let loaded = db
            .get_interaction_campaign_request(&campaign_id)
            .expect("request")
            .expect("request exists");
        assert_eq!(loaded.0.request_id, request.request_id);
        let updated = db
            .get_interaction_campaign(&campaign_id)
            .expect("updated")
            .expect("updated exists");
        assert_eq!(updated.assignments[0].state, ThreadMessageState::Succeeded);
        assert_eq!(
            updated.assignments[0].prepared_text.as_deref(),
            Some("món này nhìn ngon quá")
        );
        std::fs::remove_file(path).expect("remove fixture database");
    }

    /// A message that was meant to post and did not has to show up in the
    /// campaign totals.
    ///
    /// `skipped_parent` was counted in neither bucket. It is not a rare state:
    /// one message whose identity cannot be read makes every later message in
    /// that thread `skipped_parent`, so a six-message campaign could report
    /// "1 succeeded, 0 failed" with five silently dropped — the one number an
    /// operator reads to decide whether anything went wrong.
    #[test]
    fn a_skipped_parent_message_is_counted_as_not_delivered() {
        let (db, path) = fixture();
        let request = request();
        let plan = plan_threads(&request).expect("plan");
        let campaign_id = db
            .create_interaction_campaign(&request, &plan)
            .expect("create campaign");
        let detail = db
            .get_interaction_campaign(&campaign_id)
            .expect("detail")
            .expect("campaign exists");

        db.update_interaction_assignment_state(
            &detail.assignments[0].id,
            ThreadMessageState::Succeeded,
            None,
            None,
            None,
        )
        .expect("mark succeeded");
        db.update_interaction_assignment_state(
            &detail.assignments[1].id,
            ThreadMessageState::SkippedParent,
            Some("parent_identity_not_confirmed"),
            None,
            None,
        )
        .expect("mark skipped");

        let summary = db
            .get_interaction_campaign(&campaign_id)
            .expect("summary")
            .expect("campaign exists")
            .summary;
        assert_eq!(summary.succeeded_messages, 1);
        assert_eq!(
            summary.failed_messages, 1,
            "a skipped message is a message that did not post"
        );

        let listed = db
            .list_interaction_campaigns(10)
            .expect("list")
            .into_iter()
            .find(|item| item.id == campaign_id)
            .expect("campaign listed");
        assert_eq!(
            listed.failed_messages, 1,
            "the list view must agree with the detail view"
        );
        std::fs::remove_file(path).expect("remove fixture database");
    }
}

#[cfg(test)]
mod publish_tests {
    use super::*;
    use crate::publish::{
        PublishBundle, PublishCampaignRequest, PublishCleanupPolicy, PublishImage,
        PublishMediaKind, PublishVisibility,
    };

    fn fixture() -> (Database, PathBuf) {
        let path = std::env::temp_dir().join(format!("riviu-publish-test-{}.db", Uuid::new_v4()));
        (Database::open(&path).expect("open fixture database"), path)
    }

    fn bundle(id: &str, ordinal: usize) -> PublishBundle {
        PublishBundle {
            id: id.into(),
            source_path: format!("/fixture/{id}"),
            name: format!("bundle-{ordinal}"),
            media_kind: PublishMediaKind::Image,
            images: vec![PublishImage {
                path: format!("/fixture/{id}/01.png"),
                file_name: "01.png".into(),
                order: 1,
                sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                byte_len: 3,
                width: 1,
                height: 1,
            }],
            caption_path: format!("/fixture/{id}/caption.txt"),
            caption: format!("caption {ordinal}"),
            caption_sha256: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .into(),
            total_bytes: 3,
        }
    }

    #[test]
    fn publish_campaign_persists_mapping_hash_manifest_and_revision_events() {
        let (db, path) = fixture();
        let request = PublishCampaignRequest {
            request_id: "publish-db-1".into(),
            source_root: "/fixture/root".into(),
            bundle_ids: vec!["bundle-a".into(), "bundle-b".into()],
            udids: vec!["phone-a".into(), "phone-b".into()],
            run_at: None,
            visibility: PublishVisibility::Public,
            cleanup_policy: PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
        };
        let campaign = db
            .create_publish_campaign(&request, &[bundle("bundle-a", 0), bundle("bundle-b", 1)])
            .expect("create campaign");
        assert_eq!(campaign.state, crate::publish::PublishCampaignState::Queued);
        assert_eq!(campaign.assignments[1].udid, "phone-b");
        assert_eq!(
            db.list_publish_campaigns(10).expect("list")[0]
                .assignments
                .len(),
            2
        );

        let detail = db
            .get_publish_campaign(&campaign.id)
            .expect("get campaign")
            .expect("campaign exists");
        assert_eq!(detail.bundles.len(), 2);
        assert_eq!(detail.bundles[0].caption_sha256.len(), 64);
        assert_eq!(detail.assignments.len(), 2);
        assert_eq!(detail.events.len(), 1);

        db.update_publish_campaign_state(
            &campaign.id,
            crate::publish::PublishCampaignState::Ready,
            None,
        )
        .expect("state event");
        let updated = db
            .get_publish_campaign(&campaign.id)
            .expect("reload")
            .expect("campaign exists");
        assert_eq!(
            updated.campaign.state,
            crate::publish::PublishCampaignState::Ready
        );
        assert_eq!(updated.events.len(), 2);
        std::fs::remove_file(path).expect("remove fixture database");
    }

    #[test]
    fn publish_campaign_rejects_duplicate_device_mapping() {
        let (db, path) = fixture();
        let request = PublishCampaignRequest {
            request_id: "publish-db-duplicate".into(),
            source_root: "/fixture/root".into(),
            bundle_ids: vec!["bundle-a".into(), "bundle-b".into()],
            udids: vec!["phone-a".into(), "phone-a".into()],
            run_at: None,
            visibility: PublishVisibility::Public,
            cleanup_policy: PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
        };
        let error = db
            .create_publish_campaign(&request, &[bundle("bundle-a", 0), bundle("bundle-b", 1)])
            .expect_err("duplicate UDID must be rejected");
        assert!(error.to_string().contains("duplicate UDID"));
        std::fs::remove_file(path).expect("remove fixture database");
    }
}
