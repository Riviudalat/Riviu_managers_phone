use super::*;
use crate::{OperationDeviceLog, OperationDeviceLogEntry, OperationRunKind};

impl Database {
    /// Read only the selected run AND device. The ring keyed by UDID alone is not history.
    pub fn operation_device_log(
        &self,
        kind: OperationRunKind,
        source_id: &str,
        udid: &str,
    ) -> anyhow::Result<OperationDeviceLog> {
        let query = match kind {
            OperationRunKind::Nurture => "
                SELECT CAST(sequence AS TEXT),recorded_at,'nurture',
                  COALESCE(json_extract(status_json,'$.phase'),''),
                  json_extract(status_json,'$.lastMessage'),
                  json_extract(status_json,'$.cleanupError')
                FROM nurture_run_status_events WHERE run_id=?1 AND udid=?2",
            OperationRunKind::Interaction => "
                SELECT CAST(sequence AS TEXT),recorded_at,action,state,text,detail
                FROM operation_device_events WHERE source_kind='interaction' AND source_id=?1 AND udid=?2
                UNION ALL
                SELECT a.id,a.created_at,'evidence',a.kind,NULL,NULL FROM interaction_artifacts a
                JOIN interaction_assignments i ON i.id=a.assignment_id AND i.campaign_id=a.campaign_id
                WHERE a.campaign_id=?1 AND i.actor_udid=?2",
            OperationRunKind::Publish => "
                SELECT CAST(sequence AS TEXT),recorded_at,action,state,text,detail
                FROM operation_device_events WHERE source_kind='publish' AND source_id=?1 AND udid=?2",
            OperationRunKind::Flow => "
                SELECT a.id,a.updated_at,a.action_kind,a.state,NULL,a.error_json
                FROM flow_node_attempts a JOIN flow_device_runs d ON d.id=a.device_run_id
                WHERE d.run_id=?1 AND d.udid=?2",
            OperationRunKind::AppInstall | OperationRunKind::MaterialTransfer => "
                SELECT i.udid,NULL,b.kind,i.state,i.detail,i.error_code
                FROM library_batch_items i JOIN library_batches b ON b.id=i.batch_id
                WHERE i.batch_id=?1 AND i.udid=?2",
            // These sources do not store per-device step times. Do not copy fleet events
            // into a device log or manufacture a timeline from the polling clock.
            OperationRunKind::Script | OperationRunKind::Orchestration => {
                return Ok(OperationDeviceLog { entries: vec![], truncated: false });
            }
        };
        let connection = self.conn()?;
        let mut statement = connection.prepare(&format!(
            "SELECT * FROM ({query}) ORDER BY 2 DESC,1 DESC LIMIT 501"
        ))?;
        let mut entries = statement
            .query_map(params![source_id, udid], |row| {
                Ok(OperationDeviceLogEntry {
                    id: row.get(0)?,
                    at: row.get(1)?,
                    action: row.get(2)?,
                    state: row.get(3)?,
                    text: row.get(4)?,
                    detail: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let truncated = entries.len() > 500;
        entries.truncate(500);
        entries.reverse();
        Ok(OperationDeviceLog { entries, truncated })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_timeline_preserves_transitions_and_rolls_back_with_source() {
        let path =
            std::env::temp_dir().join(format!("operation-publish-log-{}.sqlite", Uuid::new_v4()));
        let db = Database::open(&path).unwrap();
        let mut conn = db.conn().unwrap();
        conn.execute_batch("INSERT INTO publish_campaigns
            (id,request_id,source_root,request_json,state,created_at,updated_at)
            VALUES ('campaign','request','fixture','{}','queued','2026-09-07T00:00:00Z','2026-09-07T00:00:00Z');
            INSERT INTO publish_bundles
            (id,campaign_id,ordinal,name,source_path,caption,caption_sha256,manifest_json,created_at)
            VALUES ('bundle','campaign',0,'fixture','fixture','caption',
            'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','{}','2026-09-07T00:00:00Z');
            INSERT INTO publish_assignments
            (id,campaign_id,bundle_id,ordinal,udid,state,created_at,updated_at)
            VALUES ('assignment','campaign','bundle',0,'phone-a','queued','2026-09-07T00:00:00Z','2026-09-07T00:00:00Z');
            UPDATE publish_assignments SET state='posting',updated_at='2026-09-07T00:01:00Z' WHERE id='assignment';").unwrap();
        {
            let tx = conn.transaction().unwrap();
            tx.execute(
                "UPDATE publish_assignments SET state='succeeded' WHERE id='assignment'",
                [],
            )
            .unwrap();
            assert_eq!(
                tx.query_row(
                    "SELECT COUNT(*) FROM operation_device_events WHERE state='succeeded'",
                    [],
                    |row| row.get::<_, u32>(0)
                )
                .unwrap(),
                1
            );
            tx.rollback().unwrap();
        }
        let read = db
            .operation_device_log(OperationRunKind::Publish, "campaign", "phone-a")
            .unwrap();
        assert_eq!(
            read.entries
                .iter()
                .map(|row| row.state.as_str())
                .collect::<Vec<_>>(),
            ["queued", "posting"]
        );
        assert_eq!(read.entries[1].at.as_deref(), Some("2026-09-07T00:01:00Z"));
        assert!(db
            .operation_device_log(OperationRunKind::Publish, "campaign", "phone-b")
            .unwrap()
            .entries
            .is_empty());
        assert!(db
            .operation_device_log(OperationRunKind::Publish, "different", "phone-a")
            .unwrap()
            .entries
            .is_empty());
        drop(conn);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn device_log_is_scoped_to_exact_nurture_run_and_device() {
        let path = std::env::temp_dir().join(format!("operation-log-{}.sqlite", Uuid::new_v4()));
        let db = Database::open(&path).unwrap();
        let run = Uuid::new_v4();
        let other_run = Uuid::new_v4();
        for id in [run, other_run] {
            let initial = ["phone-a", "phone-b"].map(|udid| {
                let mut status = crate::NurtureSessionStatus::new(udid);
                status.run_id = Some(id);
                status.run_size = 2;
                status
            });
            db.create_nurture_run(id, &["phone-a".into(), "phone-b".into()], &initial)
                .unwrap();
            for udid in ["phone-a", "phone-b"] {
                let mut status = crate::NurtureSessionStatus::new(udid);
                status.run_id = Some(id);
                status.last_message = format!("{id}:{udid}");
                db.append_nurture_status(&status).unwrap();
            }
        }
        let rows = db
            .operation_device_log(OperationRunKind::Nurture, &run.to_string(), "phone-a")
            .unwrap();
        assert!(!rows.truncated);
        assert!(rows
            .entries
            .iter()
            .any(|row| row.text.as_deref() == Some(&format!("{run}:phone-a"))));
        assert!(rows.entries.iter().all(|row| !row
            .text
            .as_deref()
            .unwrap_or_default()
            .contains("phone-b")
            && !row
                .text
                .as_deref()
                .unwrap_or_default()
                .contains(&other_run.to_string())));
        assert!(rows.entries.iter().all(|row| row
            .at
            .as_ref()
            .is_some_and(|at| chrono::DateTime::parse_from_rfc3339(at).is_ok())));
        for kind in [
            OperationRunKind::Interaction,
            OperationRunKind::Publish,
            OperationRunKind::Flow,
            OperationRunKind::AppInstall,
            OperationRunKind::MaterialTransfer,
        ] {
            assert!(db
                .operation_device_log(kind, "missing", "phone-a")
                .unwrap()
                .entries
                .is_empty());
        }
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}
