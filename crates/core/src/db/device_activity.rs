use super::*;
use crate::device_work::DeviceWorkOwner;

/// A held lease, never a queued owner. The caller rechecks its token after reading.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DeviceActivityScope {
    pub udid: String,
    pub owner: DeviceWorkOwner,
    pub since: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceActivityProgress {
    pub udid: String,
    pub operation_id: String,
    pub kind: crate::OperationRunKind,
    pub action: String,
    pub state: String,
    pub updated_at: String,
}

impl Database {
    /// Read fresh progress for uniquely attributable active work in one batch.
    pub fn device_activity_progress(
        &self,
        scopes: &[DeviceActivityScope],
    ) -> anyhow::Result<Vec<DeviceActivityProgress>> {
        // Oversized/ambiguous input yields generic busy, never a partial attribution.
        if scopes.is_empty() || scopes.len() > 1000 {
            return Ok(Vec::new());
        }
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            "WITH scopes AS (
                SELECT json_extract(value,'$.udid') udid,
                       json_extract(value,'$.owner') owner,
                       json_extract(value,'$.since') since FROM json_each(?1)
            ), candidates AS (
                SELECT DISTINCT s.udid,s.since,'publish' kind,c.id source_id
                FROM scopes s JOIN publish_assignments a ON a.udid=s.udid
                JOIN publish_campaigns c ON c.id=a.campaign_id
                WHERE s.owner='script'
                  AND a.state IN ('preparing','ready','transferring','imported','posting','verifying')
                  AND c.state IN ('preparing','ready','transferring','imported','posting','verifying')
                UNION
                SELECT DISTINCT s.udid,s.since,'interaction',c.id
                FROM scopes s JOIN interaction_assignments a ON a.actor_udid=s.udid
                JOIN interaction_campaigns c ON c.id=a.campaign_id
                WHERE s.owner='interaction' AND c.state='running'
                  AND a.state IN ('preparing','ready','sending')
                UNION
                SELECT s.udid,s.since,'flow',r.id
                FROM scopes s JOIN flow_device_runs d ON d.udid=s.udid
                JOIN flow_runs r ON r.id=d.run_id
                WHERE s.owner='script' AND r.state IN ('queued','running') AND d.state IN ('preflight','running')
                UNION
                SELECT s.udid,s.since,'script',j.id
                FROM scopes s JOIN jobs j ON json_extract(j.status,'$')='running'
                WHERE s.owner='script' AND EXISTS (SELECT 1 FROM json_each(j.udids_json) WHERE value=s.udid)
                LIMIT 1001
            ), unique_source AS (
                SELECT udid,since,kind,source_id FROM candidates
                WHERE (SELECT COUNT(*) FROM candidates)<=1000
                GROUP BY udid HAVING COUNT(*)=1
            ), progress AS (
                SELECT c.udid,c.kind,c.source_id,c.since,e.action,e.state,e.recorded_at at
                FROM unique_source c JOIN operation_device_events e ON e.sequence=(
                    SELECT MAX(sequence) FROM operation_device_events
                    WHERE source_kind=c.kind AND source_id=c.source_id AND udid=c.udid
                ) WHERE c.kind<>'flow'
                UNION ALL
                SELECT c.udid,c.kind,c.source_id,c.since,a.action_kind,a.state,a.updated_at
                FROM unique_source c JOIN flow_device_runs d ON d.run_id=c.source_id AND d.udid=c.udid
                JOIN flow_node_attempts a ON a.id=(
                    SELECT id FROM flow_node_attempts WHERE device_run_id=d.id
                    ORDER BY updated_at DESC,rowid DESC LIMIT 1
                ) WHERE c.kind='flow'
            ) SELECT udid,kind,source_id,action,state,at FROM progress
              WHERE julianday(at)>=julianday(since)",
        )?;
        let rows = statement.query_map([serde_json::to_string(scopes)?], |row| {
            let kind: String = row.get(1)?;
            let source: String = row.get(2)?;
            Ok(DeviceActivityProgress {
                udid: row.get(0)?,
                operation_id: format!("{kind}:{source}"),
                kind: match kind.as_str() {
                    "publish" => crate::OperationRunKind::Publish,
                    "interaction" => crate::OperationRunKind::Interaction,
                    "flow" => crate::OperationRunKind::Flow,
                    _ => crate::OperationRunKind::Script,
                },
                action: row.get(3)?,
                state: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?;
        // SQLite date functions round sub-milliseconds; preserve the exact lease boundary.
        Ok(rows
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .filter(|row| {
                scopes.iter().any(|scope| {
                    scope.udid == row.udid
                        && DateTime::parse_from_rfc3339(&row.updated_at)
                            .is_ok_and(|at| at >= scope.since)
                })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_activity_binds_current_source_and_clears_terminal_or_stale_progress() {
        let path = std::env::temp_dir().join(format!("device-activity-{}.sqlite", Uuid::new_v4()));
        let db = Database::open(&path).unwrap();
        let conn = db.conn().unwrap();
        conn.execute_batch("INSERT INTO publish_campaigns
            (id,request_id,source_root,request_json,state,created_at,updated_at)
            VALUES ('campaign','request','fixture','{}','posting','2026-09-07T00:00:00Z','2026-09-07T00:02:00Z');
            INSERT INTO publish_bundles
            (id,campaign_id,ordinal,name,source_path,caption,caption_sha256,manifest_json,created_at)
            VALUES ('bundle','campaign',0,'fixture','fixture','PRIVATE CAPTION',
            'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','{}','2026-09-07T00:00:00Z');
            INSERT INTO publish_assignments
            (id,campaign_id,bundle_id,ordinal,udid,state,created_at,updated_at)
            VALUES ('assignment','campaign','bundle',0,'phone-a','posting','2026-09-07T00:00:00Z','2026-09-07T00:02:00Z');
            INSERT INTO operation_device_events(source_kind,source_id,udid,action,state,recorded_at,text)
            VALUES ('publish','campaign','phone-a','publishStep','opening_sounds','2026-09-07T00:02:01Z','PRIVATE CAPTION'),
                   ('publish','old-campaign','phone-a','publishStep','finished','2026-09-07T00:02:03Z','stale run'),
                   ('publish','campaign','phone-b','publishStep','finished','2026-09-07T00:02:04Z','other phone');").unwrap();
        let scope = DeviceActivityScope {
            udid: "phone-a".into(),
            owner: DeviceWorkOwner::Script,
            since: "2026-09-07T00:01:00Z".parse().unwrap(),
        };
        let rows = db.device_activity_progress(&[scope.clone()]).unwrap();
        assert_eq!(
            rows.len(),
            1,
            "current held work must expose its actual progress"
        );
        assert_eq!(rows[0].operation_id, "publish:campaign");
        assert_eq!(rows[0].kind, crate::OperationRunKind::Publish);
        assert_eq!(rows[0].state, "opening_sounds");
        assert_eq!(rows[0].updated_at, "2026-09-07T00:02:01Z");
        assert!(
            db.device_activity_progress(&[]).unwrap().is_empty(),
            "idle clears activity"
        );
        assert!(
            db.device_activity_progress(&[DeviceActivityScope {
                owner: DeviceWorkOwner::Interaction,
                ..scope.clone()
            }])
            .unwrap()
            .is_empty(),
            "a different owner cannot inherit publish progress"
        );
        assert!(
            db.device_activity_progress(&[DeviceActivityScope {
                since: "2026-09-07T00:03:00Z".parse().unwrap(),
                ..scope.clone()
            }])
            .unwrap()
            .is_empty(),
            "a new same-kind lease cannot inherit old progress"
        );

        let job = crate::JobRecord {
            id: Uuid::new_v4(),
            script_name: "private script title".into(),
            udids: vec!["phone-a".into()],
            status: crate::JobStatus::Running,
            created_at: scope.since,
            updated_at: scope.since,
            steps: vec![],
            error: None,
        };
        db.save_job(&job).unwrap();
        assert!(
            db.device_activity_progress(&[scope.clone()])
                .unwrap()
                .is_empty(),
            "two active sources under shared Script ownership are ambiguous"
        );
        let mut finished_job = job;
        finished_job.status = crate::JobStatus::Succeeded;
        db.save_job(&finished_job).unwrap();
        conn.execute(
            "UPDATE publish_assignments SET state='succeeded' WHERE id='assignment'",
            [],
        )
        .unwrap();
        assert!(
            db.device_activity_progress(&[scope]).unwrap().is_empty(),
            "a terminal device must clear even while its campaign remains active"
        );
        // Like-only keeps its assignment preparing/sending until action settlement
        // and control cleanup; confirmed belongs to the action, not the assignment.
        conn.execute_batch("INSERT INTO interaction_campaigns
            (id,request_id,request_json,state,message_count,created_at,updated_at)
            VALUES ('interaction','interaction-request','{}','running',2,'2026-09-07T00:00:00Z','2026-09-07T00:02:00Z');
            INSERT INTO interaction_targets
            (id,campaign_id,line_no,original_url,normalized_url,target_key,content_id,kind,created_at)
            VALUES ('target','interaction',1,'fixture','fixture','target','content','photo','2026-09-07T00:00:00Z');
            INSERT INTO interaction_assignments
            (id,campaign_id,target_id,message_ordinal,actor_udid,state,created_at,updated_at)
            VALUES ('like-only','interaction','target',0,'phone-a','sending','2026-09-07T00:00:00Z','2026-09-07T00:02:00Z');
            INSERT INTO tiktok_action_runs
            (id,owner_kind,owner_id,device_udid,campaign_id,assignment_id,action_kind,state,created_at,updated_at)
            VALUES ('like','interaction','like-only','phone-a','interaction','like-only','like','armed','2026-09-07T00:00:00Z','2026-09-07T00:02:01Z');").unwrap();
        let interaction = DeviceActivityScope {
            udid: "phone-a".into(),
            owner: DeviceWorkOwner::Interaction,
            since: "2026-09-07T00:01:00Z".parse().unwrap(),
        };
        let rows = db.device_activity_progress(&[interaction.clone()]).unwrap();
        assert_eq!(
            (rows[0].action.as_str(), rows[0].state.as_str()),
            ("like", "armed")
        );
        conn.execute("UPDATE tiktok_action_runs SET state='confirmed',updated_at='2026-09-07T00:02:02Z' WHERE id='like'", []).unwrap();
        assert_eq!(
            db.device_activity_progress(&[interaction.clone()]).unwrap()[0].state,
            "confirmed"
        );
        conn.execute("UPDATE interaction_assignments SET state='succeeded',updated_at='2026-09-07T00:02:03Z' WHERE id='like-only'", []).unwrap();
        assert!(db
            .device_activity_progress(&[interaction])
            .unwrap()
            .is_empty());
        drop(conn);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}
