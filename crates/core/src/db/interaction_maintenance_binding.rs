//! Read-only identity for an operator's retained-session maintenance.
use super::*;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionMaintenanceBinding {
    pub campaign_id: String,
    pub assignment_id: String,
    pub udid: String,
    pub revision: i64,
    pub intent_sha256: String,
}

impl Database {
    pub fn interaction_maintenance_binding(
        &self,
        campaign: &str,
        assignment: &str,
    ) -> anyhow::Result<InteractionMaintenanceBinding> {
        let conn = self.conn()?;
        let row: (String, String, String, i64, Option<String>, Option<String>, Option<String>) = conn.query_row(
            "SELECT a.actor_udid,a.state,c.state,a.revision,a.effect_intent,a.evidence_json,a.error_code
             FROM interaction_assignments a JOIN interaction_campaigns c ON c.id=a.campaign_id
             WHERE a.id=?1 AND a.campaign_id=?2",
            params![assignment,campaign],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)),
        )?;
        anyhow::ensure!(matches!(row.1.as_str(), "uncertain" | "failed")
            && matches!(row.2.as_str(), "partial" | "failed" | "cancelled"),
            "interaction still active or not a retained recovery obligation");
        let actions: Vec<(String,String,i64,Option<String>,Option<String>,Option<String>)> = conn.prepare(
            "SELECT action_kind,state,revision,effect_intent,evidence_json,error_code
             FROM tiktok_action_runs WHERE assignment_id=?1 ORDER BY action_kind,id")?
            .query_map([assignment], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?
            .collect::<Result<_,_>>()?;
        anyhow::ensure!(!actions.is_empty() && actions.iter().all(|a|
            matches!(a.1.as_str(), "confirmed" | "no_op" | "failed_before_effect" | "uncertain")),
            "interaction action worker still active or ledger missing");
        let digest = crate::frame_sha256(&serde_json::to_vec(&serde_json::json!({
            "campaignId":campaign,"assignmentId":assignment,"assignment":row,"actions":actions
        }))?);
        Ok(InteractionMaintenanceBinding {
            campaign_id: campaign.into(), assignment_id: assignment.into(),
            udid: row.0, revision: row.3, intent_sha256: digest,
        })
    }
}
