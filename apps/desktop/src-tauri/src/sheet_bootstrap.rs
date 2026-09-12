//! Local installer connection. Never selects a spreadsheet for the operator.
use anyhow::Context;
use riviu_core::db::Database;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Connection {
    webhook_url: String,
    token: String,
}

pub(crate) fn apply(db: &Database, sidecars: &Path) -> anyhow::Result<()> {
    let saved = db.publish_sheet_delivery_settings()?;
    let file = sidecars.join("publish-sheet/connection.json");
    if saved.webhook_url.is_empty() && saved.token.is_empty() && file.is_file() {
        let bytes = std::fs::read(file).context("Không đọc được cấu hình kết nối Sheet đi kèm")?;
        let connection: Connection =
            serde_json::from_slice(&bytes).context("Cấu hình kết nối Sheet chưa hợp lệ")?;
        anyhow::ensure!(
            riviu_core::publish_sheet::is_acceptable_webhook(&connection.webhook_url)
                && !connection.token.trim().is_empty(),
            "Kết nối Sheet đi kèm thiếu webhook hoặc token"
        );
        db.set_publish_sheet_config_with_reporting(
            &connection.webhook_url,
            Some(&connection.token),
            Some(true),
        )?;
    }
    if db
        .get_setting("publish_sheet_default_link_removed_v1")?
        .is_none()
    {
        if let Some(url) = db.get_setting(riviu_core::publish_sheet::SHEET_URL_SETTING)? {
            let normalized = tauri::Url::parse(&url)
                .ok()
                .and_then(|u| u.path_segments().map(|s| s.collect::<Vec<_>>().join("/")))
                .unwrap_or_default();
            // Digest identifies only the previously shipped default, never a newly chosen table.
            let digest = format!("{:x}", Sha256::digest(normalized.as_bytes()));
            let tab_is_default =
                riviu_core::publish_sheet::SheetDeliveryTarget::from_sheet_url(&url, true)
                    .is_ok_and(|target| target.sheet_gid == 0);
            if digest == LEGACY_PATH_DIGEST && tab_is_default {
                db.set_setting(riviu_core::publish_sheet::SHEET_URL_SETTING, "")?;
            }
        }
        db.set_setting("publish_sheet_default_link_removed_v1", "1")?;
    }
    Ok(())
}

const LEGACY_PATH_DIGEST: &str = "e178a970e8af67436378d474c62830776dcb9bd6d1e9159e75048f2dd89d4de7";

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connection_restores_without_setting_link_and_later_user_link_persists() {
        let dir = std::env::temp_dir().join(format!("sheet-bootstrap-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("publish-sheet")).unwrap();
        std::fs::write(dir.join("publish-sheet/connection.json"),r#"{"webhookUrl":"https://script.google.com/macros/s/fixture/exec","token":"fixture-token"}"#).unwrap();
        let db = Database::open(dir.join("test.db")).unwrap();
        apply(&db, &dir).unwrap();
        let config = db.publish_sheet_delivery_settings().unwrap();
        assert_eq!(config.token, "fixture-token");
        assert!(db
            .get_setting(riviu_core::publish_sheet::SHEET_URL_SETTING)
            .unwrap()
            .unwrap_or_default()
            .is_empty());
        db.set_setting(
            riviu_core::publish_sheet::SHEET_URL_SETTING,
            "https://docs.google.com/spreadsheets/d/chosen/edit#gid=0",
        )
        .unwrap();
        apply(&db, &dir).unwrap();
        assert!(db
            .get_setting(riviu_core::publish_sheet::SHEET_URL_SETTING)
            .unwrap()
            .unwrap()
            .contains("chosen"));
        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn legacy_link_is_cleared_once_without_erasing_credentials_or_chosen_link() {
        let dir = std::env::temp_dir().join(format!("sheet-reset-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::open(dir.join("test.db")).unwrap();
        let old="https://docs.google.com/spreadsheets/d/1HUcp3DMPTLfQVjhxRnXa_4xvyXd3-EF3LAbYsl0Wwxc/edit?gid=0#gid=0";
        db.set_publish_sheet_config_with_reporting(
            "https://script.google.com/macros/s/fixture/exec",
            Some("keep-token"),
            Some(true),
        )
        .unwrap();
        db.set_setting(riviu_core::publish_sheet::SHEET_URL_SETTING, old)
            .unwrap();
        apply(&db, &dir).unwrap();
        assert_eq!(
            db.get_setting(riviu_core::publish_sheet::SHEET_URL_SETTING)
                .unwrap()
                .as_deref(),
            Some("")
        );
        assert_eq!(
            db.publish_sheet_delivery_settings().unwrap().token,
            "keep-token"
        );
        db.set_setting(riviu_core::publish_sheet::SHEET_URL_SETTING, old)
            .unwrap();
        apply(&db, &dir).unwrap();
        assert_eq!(
            db.get_setting(riviu_core::publish_sheet::SHEET_URL_SETTING)
                .unwrap()
                .as_deref(),
            Some(old)
        );
        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
