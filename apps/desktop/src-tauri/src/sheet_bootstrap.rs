//! Migrate a retired default link without importing installer-side connections.
use riviu_core::db::Database;
use sha2::{Digest, Sha256};
use std::path::Path;

pub(crate) fn apply(db: &Database, _sidecars: &Path) -> anyhow::Result<()> {
    // Only explicit operator configuration may supply webhook credentials.
    // A leftover connection beside a raw build must never enroll a clean profile.
    migrate_legacy_link(db, LEGACY_PATH_DIGEST)
}

fn migrate_legacy_link(db: &Database, legacy_path_digest: &str) -> anyhow::Result<()> {
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
            if digest == legacy_path_digest && tab_is_default {
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
    fn clean_profile_never_imports_valid_or_malformed_sidecar_connection() {
        let dir = std::env::temp_dir().join(format!("sheet-bootstrap-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("publish-sheet")).unwrap();
        for (index, content) in [r#"{"webhookUrl":"https://script.google.com/macros/s/fixture/exec","token":"fixture-token"}"#, "malformed JSON"].into_iter().enumerate() {
            std::fs::write(dir.join("publish-sheet/connection.json"),content).unwrap();
            let db=Database::open(dir.join(format!("clean-{index}.db"))).unwrap();
            apply(&db,&dir).unwrap();
            let config=db.publish_sheet_delivery_settings().unwrap();
            assert!(config.webhook_url.is_empty());
            assert!(config.token.is_empty());
            assert!(!config.internal_reporting);
            assert!(db.get_setting(riviu_core::publish_sheet::SHEET_URL_SETTING).unwrap().unwrap_or_default().is_empty());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn existing_operator_connection_and_chosen_link_survive_startup() {
        let dir = std::env::temp_dir().join(format!("sheet-existing-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("publish-sheet")).unwrap();
        std::fs::write(
            dir.join("publish-sheet/connection.json"),
            r#"{"webhookUrl":"https://fixture.invalid/other","token":"other"}"#,
        )
        .unwrap();
        let db = Database::open(dir.join("test.db")).unwrap();
        db.set_publish_sheet_config_with_reporting(
            "https://fixture.invalid/chosen",
            Some("operator-token"),
            Some(true),
        )
        .unwrap();
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
        let config = db.publish_sheet_delivery_settings().unwrap();
        assert_eq!(config.webhook_url, "https://fixture.invalid/chosen");
        assert_eq!(config.token, "operator-token");
        assert!(config.internal_reporting);
        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn legacy_link_is_cleared_once_without_erasing_credentials_or_chosen_link() {
        let dir = std::env::temp_dir().join(format!("sheet-reset-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::open(dir.join("test.db")).unwrap();
        let old = "https://docs.google.com/spreadsheets/d/retired-fixture/edit?gid=0#gid=0";
        let legacy_digest = format!(
            "{:x}",
            Sha256::digest(b"spreadsheets/d/retired-fixture/edit")
        );
        db.set_publish_sheet_config_with_reporting(
            "https://script.google.com/macros/s/fixture/exec",
            Some("keep-token"),
            Some(true),
        )
        .unwrap();
        db.set_setting(riviu_core::publish_sheet::SHEET_URL_SETTING, old)
            .unwrap();
        migrate_legacy_link(&db, &legacy_digest).unwrap();
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
        migrate_legacy_link(&db, &legacy_digest).unwrap();
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
