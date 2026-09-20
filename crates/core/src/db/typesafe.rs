use super::*;
use crate::typesafe::{Client, Settings};

const SETTINGS_KEY: &str = "typesafe.settings";
const CREDENTIAL_KEY: &str = "typesafe-api-key";

impl Database {
    pub fn typesafe_settings(&self) -> anyhow::Result<Settings> {
        let mut settings: Settings = self
            .get_setting(SETTINGS_KEY)?
            .map(|raw| serde_json::from_str(&raw))
            .transpose()?
            .unwrap_or_default();
        settings.has_api_key = self
            .secrets
            .as_ref()
            .map(|store| store.get_secret(CREDENTIAL_KEY))
            .transpose()?
            .flatten()
            .is_some_and(|key| !key.trim().is_empty());
        Ok(settings)
    }

    pub fn update_typesafe_settings(
        &self,
        enabled: bool,
        expected_revision: u64,
    ) -> anyhow::Result<Settings> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let raw: Option<String> = tx
            .query_row(
                "SELECT value FROM settings WHERE key=?1",
                [SETTINGS_KEY],
                |r| r.get(0),
            )
            .optional()?;
        let prior: Settings = raw
            .map(|v| serde_json::from_str(&v))
            .transpose()?
            .unwrap_or_default();
        anyhow::ensure!(
            prior.revision == expected_revision,
            "TypeSafeSettingsConflict: reload before saving"
        );
        let next = Settings {
            enabled,
            revision: expected_revision
                .checked_add(1)
                .context("TypeSafe revision overflow")?,
            has_api_key: false,
        };
        tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![SETTINGS_KEY, serde_json::to_string(&next)?])?;
        tx.commit()?;
        self.typesafe_settings()
    }

    pub fn update_typesafe_credential(&self, key: &str) -> anyhow::Result<Settings> {
        anyhow::ensure!(
            key.len() <= 512 && !key.contains(['\r', '\n']),
            "Invalid TypeSafe credential format"
        );
        self.secrets
            .as_ref()
            .context("OS credential store unavailable")?
            .set_secret(CREDENTIAL_KEY, key.trim())?;
        self.typesafe_settings()
    }

    pub fn typesafe_client(&self) -> anyhow::Result<Client> {
        let key = self
            .secrets
            .as_ref()
            .context("OS credential store unavailable")?
            .get_secret(CREDENTIAL_KEY)?
            .unwrap_or_default();
        Ok(Client::new(key))
    }
}
