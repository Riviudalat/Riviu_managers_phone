//! Publish campaigns and the analytics rollup over them.

use super::*;

impl Database {
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
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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
                    ordinal: narrow(row.get::<_, i64>(2)?, "ordinal")?,
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
                    // Not a `narrow`: this is `enumerate()` over a list the caller just
                    // built, not a value read back out of a column. There is no stored
                    // number here that could disagree with the type.
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
}
