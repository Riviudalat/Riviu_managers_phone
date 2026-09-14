use std::collections::BTreeSet;

use anyhow::{ensure, Context};
use rusqlite::{params, OptionalExtension, TransactionBehavior};

use super::Database;

const SECRET_PREFIX: &str = "flow-connector-";
const SECRET_INDEX: &str = "flow.connector.secret.names";

impl Database {
    pub fn flow_connector_root(&self) -> std::path::PathBuf {
        self.path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("flow-data")
    }

    pub fn flow_connector_secret(&self, name: &str) -> anyhow::Result<Option<String>> {
        crate::flow::validate_flow_variable_name(name).map_err(anyhow::Error::msg)?;
        let store = self
            .secrets
            .as_ref()
            .context("OS credential store is not attached")?;
        Ok(store
            .get_secret(&format!("{SECRET_PREFIX}{name}"))?
            .filter(|value| !value.is_empty()))
    }

    /// Only names are persisted in SQLite; a blank value removes the reference.
    pub fn set_flow_connector_secret(&self, name: &str, value: &str) -> anyhow::Result<()> {
        crate::flow::validate_flow_variable_name(name).map_err(anyhow::Error::msg)?;
        ensure!(
            value.len() <= 8192 && !value.chars().any(char::is_control),
            "Invalid connector credential"
        );
        let store = self
            .secrets
            .as_ref()
            .context("OS credential store is not attached")?;
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let raw: Option<String> = tx
            .query_row(
                "SELECT value FROM settings WHERE key=?1",
                [SECRET_INDEX],
                |row| row.get(0),
            )
            .optional()?;
        let mut names: BTreeSet<String> = raw
            .map(|s| serde_json::from_str(&s))
            .transpose()?
            .unwrap_or_default();
        if value.is_empty() {
            names.remove(name);
        } else {
            names.insert(name.into());
        }
        ensure!(
            names.len() <= 100,
            "At most 100 connector credentials may be stored"
        );
        store.set_secret(&format!("{SECRET_PREFIX}{name}"), value)?;
        tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![SECRET_INDEX,serde_json::to_string(&names)?])?;
        tx.commit()?;
        Ok(())
    }

    pub fn list_flow_connector_secrets(&self) -> anyhow::Result<Vec<String>> {
        let names: BTreeSet<String> = self
            .get_setting(SECRET_INDEX)?
            .map(|s| serde_json::from_str(&s))
            .transpose()?
            .unwrap_or_default();
        names
            .into_iter()
            .filter_map(|name| match self.flow_connector_secret(&name) {
                Ok(Some(_)) => Some(Ok(name)),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            })
            .collect()
    }
}
