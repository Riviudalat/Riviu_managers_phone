//! "Tài khoản TikTok": per-account TikTok logins kept in the OS credential store, and the
//! dedicated "Cài lại TikTok 45.7.3 + đăng nhập" command that uses them.
//!
//! The operator types a username and a password into the device panel; nothing is imported
//! from a file. The rules every path here keeps:
//!
//! - The identifier and the password go to `state.secrets` (Windows Credential Manager) and
//!   nowhere else. SQLite gets only the non-secret index: handle, identifier kind, masked
//!   identifier, timestamps and the last login outcome.
//! - The credential store is written **first** and the index second, so an index row never
//!   describes a write that did not happen. A failure between the two leaves an orphan entry
//!   that the list shows as "Chưa có" and the next save overwrites.
//! - Nothing returns, logs or puts into an error a password or a full identifier.
//! - None of this is reachable from the Local API or the Riviu MCP adapter.

use riviu_core::db::Database;
use riviu_core::tiktok_credentials::{
    canonical_tiktok_handle, TikTokCredentialIndexEntry, TikTokCredentialPresence,
    TikTokCredentialSummary,
};
use riviu_signing::{CredentialStore, SecretText, TikTokLogin};
use tauri::State;

use crate::{command_error::CommandError, state::AppState};

mod relogin;

pub use relogin::TikTokReloginReport;

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Every saved login, with a fresh check that the OS credential store still holds it.
#[tauri::command]
pub async fn tiktok_credentials_list(
    state: State<'_, AppState>,
) -> Result<Vec<TikTokCredentialSummary>, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let secrets = state.secrets.clone();
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || list_credentials(&secrets, &db))
        .await
        .map_err(CommandError::operation)?
        .map_err(CommandError::operation)
}

/// Save or replace one account's login, typed by the operator. The username is also the
/// handle the login is stored under. Returns the masked summary, never the secret.
#[tauri::command]
pub fn tiktok_credential_set(
    state: State<'_, AppState>,
    username: String,
    password: String,
) -> Result<TikTokCredentialSummary, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let login = TikTokLogin::new(&username, &username, SecretText::new(password))
        .map_err(CommandError::invalid_argument)?;
    let entry = store_login(&state.secrets, &state.db, &login).map_err(CommandError::operation)?;
    Ok(TikTokCredentialSummary::new(
        entry,
        TikTokCredentialPresence::Stored,
    ))
}

/// Delete one account's login from the credential store, then its index row.
#[tauri::command]
pub fn tiktok_credential_clear(
    state: State<'_, AppState>,
    handle: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let handle = canonical_tiktok_handle(&handle).map_err(CommandError::invalid_argument)?;
    clear_login(&state.secrets, &state.db, &handle).map_err(CommandError::operation)
}

/// "Cài lại TikTok 45.7.3 + đăng nhập" on one Android phone: preflight, log out, uninstall,
/// install the library's verified 45.7.3 APK, first-run screens, log in once with the saved
/// login of the phone's assigned nick, close the security sheet, prove the handle.
///
/// One Repair lease covers every stage. Each stage's outcome is written to the op log as it
/// happens; a device-level stop is a report, not an error.
#[tauri::command]
pub async fn tiktok_reinstall_and_login(
    state: State<'_, AppState>,
    udid: String,
    app_id: String,
) -> Result<TikTokReloginReport, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    relogin::run(&state, udid, app_id).await
}

fn list_credentials(
    secrets: &CredentialStore,
    db: &Database,
) -> anyhow::Result<Vec<TikTokCredentialSummary>> {
    Ok(db
        .list_tiktok_credential_index()?
        .into_iter()
        .map(|entry| {
            let presence = match secrets.tiktok_login(&entry.handle) {
                Ok(Some(_)) => TikTokCredentialPresence::Stored,
                Ok(None) => TikTokCredentialPresence::Missing,
                Err(error) => {
                    log::warn!(
                        "không đọc được đăng nhập TikTok của @{}: {error:#}",
                        entry.handle
                    );
                    TikTokCredentialPresence::Unreadable
                }
            };
            TikTokCredentialSummary::new(entry, presence)
        })
        .collect())
}

/// The one write path for a login: credential store first, index second. A new secret is
/// the operator's acknowledgement of the previous login outcome, so it clears it.
fn store_login(
    secrets: &CredentialStore,
    db: &Database,
    login: &TikTokLogin,
) -> anyhow::Result<TikTokCredentialIndexEntry> {
    secrets.set_tiktok_login(login)?;
    let mut entry = db.upsert_tiktok_credential_index(&login.index_entry(now()))?;
    if entry.last_status.is_some() {
        db.set_tiktok_credential_status(&entry.handle, None)?;
        entry.last_status = None;
    }
    if let Err(error) = db.log_op(
        "tiktok.credential.set",
        &format!("@{} ({})", entry.handle, entry.masked_identifier),
    ) {
        log::warn!("không ghi được nhật ký lưu đăng nhập TikTok: {error:#}");
    }
    Ok(entry)
}

fn clear_login(secrets: &CredentialStore, db: &Database, handle: &str) -> anyhow::Result<()> {
    secrets.clear_tiktok_login(handle)?;
    db.delete_tiktok_credential_index(handle)?;
    if let Err(error) = db.log_op("tiktok.credential.clear", &format!("@{handle}")) {
        log::warn!("không ghi được nhật ký xóa đăng nhập TikTok: {error:#}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use riviu_signing::CredentialBackend;

    use super::*;

    /// In-memory credential store; `fail_set_after` refuses writes once that many succeeded.
    #[derive(Clone, Default)]
    struct Memory {
        values: Arc<Mutex<HashMap<String, String>>>,
        fail_set_after: Arc<Mutex<Option<usize>>>,
    }

    impl CredentialBackend for Memory {
        fn get(&self, account: &str) -> anyhow::Result<Option<String>> {
            Ok(self.values.lock().expect("values").get(account).cloned())
        }
        fn set(&self, account: &str, value: &str) -> anyhow::Result<()> {
            let mut values = self.values.lock().expect("values");
            if let Some(limit) = *self.fail_set_after.lock().expect("limit") {
                if values.len() >= limit {
                    anyhow::bail!("fixture credential store refused the write");
                }
            }
            values.insert(account.into(), value.into());
            Ok(())
        }
        fn delete(&self, account: &str) -> anyhow::Result<()> {
            self.values.lock().expect("values").remove(account);
            Ok(())
        }
    }

    struct Fixture {
        memory: Memory,
        secrets: CredentialStore,
        db: Database,
        path: std::path::PathBuf,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn fixture() -> Fixture {
        let memory = Memory::default();
        let path = std::env::temp_dir().join(format!(
            "riviu-tiktok-credential-commands-{}.db",
            uuid::Uuid::new_v4()
        ));
        Fixture {
            secrets: CredentialStore::new(Arc::new(memory.clone())),
            memory,
            db: Database::open(&path).expect("open fixture database"),
            path,
        }
    }

    const FAKE_A: &str = " fake-pass-a ";

    #[test]
    fn a_failed_credential_write_leaves_no_index_row() {
        let f = fixture();
        *f.memory.fail_set_after.lock().unwrap() = Some(0);
        let login = TikTokLogin::new(
            "riviu.demo",
            "demo@example.com",
            SecretText::new(FAKE_A.into()),
        )
        .unwrap();

        assert!(store_login(&f.secrets, &f.db, &login).is_err());
        assert!(f.db.list_tiktok_credential_index().unwrap().is_empty());
    }

    #[test]
    fn the_list_reports_a_missing_secret_as_missing_and_never_returns_it() {
        let f = fixture();
        for handle in ["kept.one", "lost.one"] {
            let login =
                TikTokLogin::new(handle, "demo@example.com", SecretText::new(FAKE_A.into()))
                    .unwrap();
            store_login(&f.secrets, &f.db, &login).unwrap();
        }
        f.memory
            .values
            .lock()
            .unwrap()
            .remove("tiktok-login:lost.one");

        let listed = list_credentials(&f.secrets, &f.db).unwrap();
        let presence: Vec<_> = listed
            .iter()
            .map(|row| (row.handle.as_str(), row.presence))
            .collect();
        assert_eq!(
            presence,
            [
                ("kept.one", TikTokCredentialPresence::Stored),
                ("lost.one", TikTokCredentialPresence::Missing)
            ]
        );
        let json = serde_json::to_string(&listed).unwrap();
        assert!(!json.contains(FAKE_A.trim()) && !json.contains("demo@example.com"));
        assert!(json.contains("de***@example.com"));

        clear_login(&f.secrets, &f.db, "kept.one").unwrap();
        assert!(f.memory.values.lock().unwrap().is_empty());
        assert_eq!(f.db.list_tiktok_credential_index().unwrap().len(), 1);
    }
}
