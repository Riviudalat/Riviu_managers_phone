//! Versioned records for the operator's accounts, network profiles and saved tasks.
use anyhow::{ensure, Context};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OperatorRecordKind {
    Account,
    Network,
    SavedTask,
}
impl OperatorRecordKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Account => "account",
            Self::Network => "network",
            Self::SavedTask => "savedTask",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperatorRecord {
    pub id: Uuid,
    pub kind: OperatorRecordKind,
    pub name: String,
    pub revision: u64,
    pub data: Value,
    pub archived: bool,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperatorRecordInput {
    pub id: Uuid,
    pub kind: OperatorRecordKind,
    pub name: String,
    pub expected_revision: Option<u64>,
    pub data: Value,
}
impl OperatorRecordInput {
    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(!self.id.is_nil(), "Record ID is required");
        ensure!(
            !self.name.trim().is_empty() && self.name.len() <= 180,
            "Name must contain 1-180 characters"
        );
        ensure!(
            self.expected_revision != Some(0),
            "Revision must be positive"
        );
        ensure!(self.data.is_object(), "Record data must be an object");
        ensure!(
            serde_json::to_vec(&self.data)?.len() <= 262_144,
            "Record data exceeds 256 KiB"
        );
        crate::validate_automation_config(&self.data)?;
        match self.kind {
            OperatorRecordKind::Account => {
                ensure!(
                    !self.data["username"]
                        .as_str()
                        .unwrap_or_default()
                        .trim()
                        .is_empty(),
                    "Account username is required"
                );
                ensure!(
                    matches!(
                        self.data["platform"].as_str(),
                        Some("tiktok" | "instagram" | "threads" | "other")
                    ),
                    "Invalid account platform"
                );
            }
            OperatorRecordKind::Network => {
                ensure!(
                    matches!(
                        self.data["protocol"].as_str(),
                        Some("http" | "https" | "socks5" | "router")
                    ),
                    "Invalid network protocol"
                );
                let host = self.data["host"]
                    .as_str()
                    .context("Network host is required")?;
                ensure!(
                    !host.is_empty()
                        && host.len() <= 253
                        && host.chars().all(|c| c.is_ascii_alphanumeric()
                            || matches!(c, '.' | '-' | ':' | '[' | ']')),
                    "Invalid network host"
                );
                ensure!(
                    self.data["port"]
                        .as_u64()
                        .is_some_and(|port| (1..=65535).contains(&port)),
                    "Network port must be 1-65535"
                );
            }
            OperatorRecordKind::SavedTask => {
                let app_id = self.data["appId"]
                    .as_str()
                    .context("Task appId is required")?;
                Uuid::parse_str(app_id).context("Invalid task appId")?;
                ensure!(
                    self.data["appRevision"]
                        .as_u64()
                        .is_some_and(|revision| revision > 0),
                    "Task must pin an app revision"
                );
                serde_json::from_value::<crate::TargetRef>(self.data["target"].clone())
                    .context("Task target is required")?;
            }
        }
        Ok(())
    }
}
