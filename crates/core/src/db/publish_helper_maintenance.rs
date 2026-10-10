use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::Database;

/// Exact submitted bindings; raw intent/account/clipboard content never crosses IPC.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperMaintenanceSubmittedBinding {
    pub assignment_id: String,
    pub campaign_id: String,
    pub publication_id: Option<String>,
    pub assignment_revision: i64,
    pub campaign_revision: i64,
    pub state: String,
    pub campaign_state: String,
    pub intent_sha256: Option<String>,
}

impl Database {
    /// One read statement freezes a consistent, ordered set for exactly one serial.
    /// Include all retained intents, even terminal rows, and ambiguous submitted rows.
    /// This method does not mutate the publication ledger or Sheet outbox.
    pub fn helper_maintenance_submitted_bindings(
        &self,
        udid: &str,
    ) -> anyhow::Result<Vec<HelperMaintenanceSubmittedBinding>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            "SELECT a.id,a.campaign_id,a.publication_id,a.revision,c.revision,
                    a.state,c.state,a.effect_intent
             FROM publish_assignments a JOIN publish_campaigns c ON c.id=a.campaign_id
             WHERE a.udid=?1 AND (a.effect_intent IS NOT NULL
                 OR a.state IN ('posting','submitted','verifying','uncertain','succeeded'))
             ORDER BY a.campaign_id,a.id",
        )?;
        let rows = statement.query_map([udid], |row| {
            let intent: Option<String> = row.get(7)?;
            Ok(HelperMaintenanceSubmittedBinding {
                assignment_id: row.get(0)?,
                campaign_id: row.get(1)?,
                publication_id: row.get(2)?,
                assignment_revision: row.get(3)?,
                campaign_revision: row.get(4)?,
                state: row.get(5)?,
                campaign_state: row.get(6)?,
                intent_sha256: intent.map(|value| format!("{:x}", Sha256::digest(value.as_bytes()))),
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// The first durable record on `udid` that keeps automatic helper reconcile behind the
    /// operator, named in Vietnamese, or `None` when every effect there is final.
    ///
    /// Blocking means intent recorded and no final receipt: an uncertain or in-flight
    /// Post, Like/Save/Comment/Follow, cleanup tap, Flow effect node or claimed Sheet write.
    /// Rows last written before `since` (the last settled helper maintenance on this phone)
    /// were already archived by that supersession and do not block again. An unparsable
    /// timestamp is treated as recent: unknown is never proof of settlement.
    pub fn helper_auto_reconcile_blocker(
        &self,
        udid: &str,
        since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> anyhow::Result<Option<String>> {
        let conn = self.conn()?;
        let recent = |updated_at: &str| {
            since.is_none_or(|since| {
                chrono::DateTime::parse_from_rfc3339(updated_at)
                    .map_or(true, |at| at.with_timezone(&chrono::Utc) >= since)
            })
        };
        let ledgers: [(&str, &str); 5] = [
            ("bài đăng",
             "SELECT id,state,updated_at FROM publish_assignments
              WHERE udid=?1 AND state IN ('posting','verifying','uncertain') ORDER BY updated_at,id"),
            ("lượt tương tác",
             "SELECT id,state,updated_at FROM interaction_assignments
              WHERE actor_udid=?1 AND state IN ('sending','uncertain') ORDER BY updated_at,id"),
            ("thao tác TikTok",
             "SELECT id,action_kind||' '||state,updated_at FROM tiktok_action_runs
              WHERE device_udid=?1 AND state IN ('armed','uncertain') ORDER BY updated_at,id"),
            ("lượt gỡ tương tác",
             "SELECT id,action_kind||' '||state,updated_at FROM public_cleanup_runs
              WHERE device_udid=?1 AND state IN ('armed','uncertain') ORDER BY updated_at,id"),
            ("bước Flow",
             "SELECT n.id,n.action_kind||' '||n.state,n.updated_at
              FROM flow_node_attempts n JOIN flow_device_runs d ON d.id=n.device_run_id
              WHERE d.udid=?1 AND n.side_effect_class<>'none'
                AND n.state IN ('intentCommitted','effectDispatched','verifying','uncertain')
              ORDER BY n.updated_at,n.id"),
        ];
        for (label, sql) in ledgers {
            let mut statement = conn.prepare(sql)?;
            let rows = statement.query_map([udid], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            for row in rows {
                let (id, state, updated_at) = row?;
                if recent(&updated_at) {
                    return Ok(Some(format!("{label} {id} đang ở trạng thái {state}")));
                }
            }
        }
        // A claimed Sheet write may have reached Google; its claim carries no timestamp row.
        let sheet: Option<String> = conn
            .query_row(
                "SELECT s.assignment_id FROM publish_sheet_sync_state s
                 JOIN publish_assignments a ON a.id=s.assignment_id
                 WHERE a.udid=?1 AND s.claim_token IS NOT NULL ORDER BY s.assignment_id LIMIT 1",
                [udid],
                |row| row.get(0),
            )
            .optional()?;
        Ok(sheet.map(|id| format!("ghi Sheet của bài đăng {id} chưa xác nhận")))
    }
}
