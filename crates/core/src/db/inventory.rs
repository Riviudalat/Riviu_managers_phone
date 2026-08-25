//! The lists the farm pages edit: proxies, media, the app library and the schedule.
//!
//! All the same shape — list, upsert, delete against one table — which is why they sit
//! together rather than in four files of thirty lines.

use super::*;

impl Database {
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
                port: narrow(row.get::<_, i64>(4)?, "port")?,
                username: row.get(5)?,
                password: row.get(6)?,
                notes: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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
            "SELECT id, name, script_name, udids_json, every_minutes, enabled, last_run_at, next_run_at, last_error FROM schedules ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| {
            let udids_json: String = row.get(3)?;
            Ok(crate::types::ScheduleItem {
                id: row.get(0)?,
                name: row.get(1)?,
                script_name: row.get(2)?,
                udids: serde_json::from_str(&udids_json).unwrap_or_default(),
                every_minutes: narrow(row.get::<_, i64>(4)?, "every_minutes")?,
                enabled: row.get::<_, i64>(5)? != 0,
                last_run_at: row.get(6)?,
                next_run_at: row.get(7)?,
                last_error: row.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn upsert_schedule(&self, s: &crate::types::ScheduleItem) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            r#"INSERT INTO schedules (id, name, script_name, udids_json, every_minutes, enabled, last_run_at, next_run_at, last_error)
               VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
               ON CONFLICT(id) DO UPDATE SET
                 name=excluded.name, script_name=excluded.script_name, udids_json=excluded.udids_json,
                 every_minutes=excluded.every_minutes, enabled=excluded.enabled,
                 last_run_at=excluded.last_run_at, next_run_at=excluded.next_run_at,
                 last_error=excluded.last_error"#,
            params![
                s.id,
                s.name,
                s.script_name,
                serde_json::to_string(&s.udids)?,
                s.every_minutes as i64,
                if s.enabled { 1 } else { 0 },
                s.last_run_at,
                s.next_run_at,
                s.last_error
            ],
        )?;
        Ok(())
    }
    pub fn delete_schedule(&self, id: &str) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM schedules WHERE id = ?1", params![id])?;
        Ok(())
    }
}
