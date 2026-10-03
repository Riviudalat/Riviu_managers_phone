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
}
