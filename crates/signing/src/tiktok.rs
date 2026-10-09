//! TikTok logins in the OS credential store, one entry per account.
//!
//! Each entry is `tiktok-login:<handle>` holding `{"v":1,"identifier":…,"password":…}`.
//! The secret never goes to SQLite: the desktop keeps a separate non-secret index (masked
//! identifier, timestamps) that it writes only after this store accepted the entry.
//!
//! Every type here that holds the secret wipes it on drop and prints it as `<redacted>`.

use std::fmt;

use anyhow::Context;
use riviu_core::tiktok_credentials::{
    canonical_tiktok_handle, classify_tiktok_identifier, mask_tiktok_identifier,
    TikTokCredentialIndexEntry, TikTokIdentifierKind,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::CredentialStore;

const ACCOUNT_PREFIX: &str = "tiktok-login:";
const FORMAT_VERSION: u8 = 1;

/// Windows Credential Manager's `CRED_MAX_CREDENTIAL_BLOB_SIZE`. `keyring` stores the value as
/// UTF-16, so the limit is on twice the UTF-16 length. Checked on every platform so that a
/// credential saved on a Mac also fits the Windows fleet controller.
pub const CREDENTIAL_BLOB_LIMIT_BYTES: usize = 2560;

/// A secret string: wiped from memory on drop, never printed, never serialized.
pub struct SecretText(Zeroizing<String>);

impl SecretText {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    /// The secret itself, for the one place that must hand it on.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretText(<redacted>)")
    }
}

/// A validated TikTok login, ready for the credential store.
pub struct TikTokLogin {
    handle: String,
    identifier: Zeroizing<String>,
    kind: TikTokIdentifierKind,
    password: SecretText,
}

impl TikTokLogin {
    /// Validate one login. The handle is canonicalized and the identifier trimmed; the
    /// password is kept byte for byte, because leading or trailing blanks can be part of it.
    ///
    /// Error messages name the handle (public) and never the identifier or the password.
    pub fn new(handle: &str, identifier: &str, password: SecretText) -> Result<Self, String> {
        let handle = canonical_tiktok_handle(handle)?;
        // An identifier that spells the handle is a username, even when it is all digits
        // and would otherwise read as a phone number.
        let (identifier, kind) = match canonical_tiktok_handle(identifier) {
            Ok(username) if username == handle => (username, TikTokIdentifierKind::Username),
            _ => classify_tiktok_identifier(identifier)?,
        };
        if password.expose().is_empty() {
            return Err("Thiếu mật khẩu".into());
        }
        if password.expose().chars().any(char::is_control) {
            return Err("Mật khẩu chứa ký tự điều khiển (xuống dòng, tab…)".into());
        }
        Ok(Self {
            handle,
            identifier: Zeroizing::new(identifier),
            kind,
            password,
        })
    }

    pub fn handle(&self) -> &str {
        &self.handle
    }

    pub fn identifier_kind(&self) -> TikTokIdentifierKind {
        self.kind
    }

    pub fn masked_identifier(&self) -> String {
        mask_tiktok_identifier(&self.identifier, self.kind)
    }

    /// The full identifier, for the login runner only. Never show or log it.
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    pub fn password(&self) -> &SecretText {
        &self.password
    }

    /// The non-secret row that records this login in SQLite.
    pub fn index_entry(&self, updated_at: String) -> TikTokCredentialIndexEntry {
        TikTokCredentialIndexEntry {
            handle: self.handle.clone(),
            identifier_kind: self.kind,
            masked_identifier: self.masked_identifier(),
            updated_at,
            cooldown_until: None,
            last_status: None,
        }
    }

    fn encode(&self) -> anyhow::Result<Zeroizing<String>> {
        #[derive(Serialize)]
        struct Stored<'a> {
            v: u8,
            identifier: &'a str,
            password: &'a str,
        }
        let value = Zeroizing::new(serde_json::to_string(&Stored {
            v: FORMAT_VERSION,
            identifier: &self.identifier,
            password: self.password.expose(),
        })?);
        if value.encode_utf16().count() * 2 > CREDENTIAL_BLOB_LIMIT_BYTES {
            anyhow::bail!(
                "Thông tin đăng nhập của @{} dài quá giới hạn {CREDENTIAL_BLOB_LIMIT_BYTES} byte \
                 của Windows Credential Manager",
                self.handle
            );
        }
        Ok(value)
    }

    fn decode(handle: &str, value: &str) -> anyhow::Result<Self> {
        #[derive(Deserialize)]
        struct Stored {
            v: u8,
            identifier: String,
            password: String,
        }
        // Deliberately not `?` on serde: its message can quote the input it failed on.
        let stored: Stored = serde_json::from_str(value).map_err(|_| {
            anyhow::anyhow!("Mục đăng nhập TikTok của @{handle} trong kho credential bị hỏng")
        })?;
        let identifier = Zeroizing::new(stored.identifier);
        let password = SecretText::new(stored.password);
        anyhow::ensure!(
            stored.v == FORMAT_VERSION,
            "Mục đăng nhập TikTok của @{handle} có định dạng {} chưa hỗ trợ",
            stored.v
        );
        Self::new(handle, &identifier, password).map_err(anyhow::Error::msg)
    }
}

impl fmt::Debug for TikTokLogin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TikTokLogin")
            .field("handle", &self.handle)
            .field("kind", &self.kind)
            .field("identifier", &self.masked_identifier())
            .field("password", &self.password)
            .finish()
    }
}

fn account(handle: &str) -> String {
    format!("{ACCOUNT_PREFIX}{handle}")
}

impl CredentialStore {
    /// Write (or replace) one TikTok login. Validated before anything is written.
    pub fn set_tiktok_login(&self, login: &TikTokLogin) -> anyhow::Result<()> {
        let value = login.encode()?;
        self.set(&account(login.handle()), &value)
    }

    /// Read one TikTok login back. `Ok(None)` means the store answered and has no entry; an
    /// entry that cannot be decoded is an error, not an absence.
    pub fn tiktok_login(&self, handle: &str) -> anyhow::Result<Option<TikTokLogin>> {
        let handle = canonical_tiktok_handle(handle).map_err(anyhow::Error::msg)?;
        let Some(value) = self.get(&account(&handle))? else {
            return Ok(None);
        };
        let value = Zeroizing::new(value);
        TikTokLogin::decode(&handle, &value)
            .map(Some)
            .with_context(|| format!("không đọc được đăng nhập TikTok của @{handle}"))
    }

    /// Delete one TikTok login. Deleting an entry that is not there succeeds.
    pub fn clear_tiktok_login(&self, handle: &str) -> anyhow::Result<()> {
        let handle = canonical_tiktok_handle(handle).map_err(anyhow::Error::msg)?;
        self.delete(&account(&handle))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::CredentialBackend;

    #[derive(Clone, Default)]
    struct MemoryBackend(Arc<Mutex<HashMap<String, String>>>);

    impl CredentialBackend for MemoryBackend {
        fn get(&self, account: &str) -> anyhow::Result<Option<String>> {
            Ok(self.0.lock().expect("values").get(account).cloned())
        }
        fn set(&self, account: &str, value: &str) -> anyhow::Result<()> {
            self.0
                .lock()
                .expect("values")
                .insert(account.into(), value.into());
            Ok(())
        }
        fn delete(&self, account: &str) -> anyhow::Result<()> {
            self.0.lock().expect("values").remove(account);
            Ok(())
        }
    }

    const FAKE: &str = " fake-not-a-real-pass ";

    fn login(handle: &str, identifier: &str, password: &str) -> Result<TikTokLogin, String> {
        TikTokLogin::new(handle, identifier, SecretText::new(password.into()))
    }

    #[test]
    fn a_login_round_trips_under_its_own_account_with_the_password_untrimmed() {
        let backend = MemoryBackend::default();
        let store = CredentialStore::new(Arc::new(backend.clone()));
        let saved = login(" @Riviu.Demo ", " demo@example.com ", FAKE).expect("valid");
        store.set_tiktok_login(&saved).expect("write");

        let raw = backend
            .get("tiktok-login:riviu.demo")
            .unwrap()
            .expect("stored under the canonical handle");
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"v": 1, "identifier": "demo@example.com", "password": FAKE})
        );

        let read = store.tiktok_login("RIVIU.DEMO").unwrap().expect("present");
        assert_eq!(read.password().expose(), FAKE);
        assert_eq!(read.identifier(), "demo@example.com");
        assert_eq!(read.masked_identifier(), "de***@example.com");

        store.clear_tiktok_login("riviu.demo").expect("clear");
        assert!(store.tiktok_login("riviu.demo").unwrap().is_none());
        store
            .clear_tiktok_login("riviu.demo")
            .expect("clearing twice is fine");
    }

    #[test]
    fn nothing_that_prints_a_login_shows_the_secret_or_the_identifier() {
        let saved = login("riviu.demo", "demo@example.com", FAKE).unwrap();
        let printed = format!("{saved:?}");
        assert!(!printed.contains(FAKE.trim()), "{printed}");
        assert!(!printed.contains("demo@example.com"), "{printed}");
        assert!(printed.contains("<redacted>"));

        for error in [
            login("riviu.demo", "demo@example.com", "").unwrap_err(),
            login("riviu.demo", "demo@example.com", "line\nbreak").unwrap_err(),
            login("riviu.demo", "not an id", FAKE).unwrap_err(),
        ] {
            assert!(!error.contains("demo@example.com") && !error.contains("break"));
        }
    }

    #[test]
    fn a_value_over_the_windows_blob_limit_is_refused_before_any_write() {
        let backend = MemoryBackend::default();
        let store = CredentialStore::new(Arc::new(backend.clone()));
        // 1250 UTF-16 units of password plus the JSON envelope crosses 2560 bytes.
        let long = "x".repeat(1250);
        let error = store
            .set_tiktok_login(&login("riviu.demo", "demo@example.com", &long).unwrap())
            .unwrap_err()
            .to_string();
        assert!(error.contains("2560"), "{error}");
        assert!(!error.contains(&long));
        assert!(backend.get("tiktok-login:riviu.demo").unwrap().is_none());

        let fits = "x".repeat(1200);
        store
            .set_tiktok_login(&login("riviu.demo", "demo@example.com", &fits).unwrap())
            .expect("under the limit");
    }

    #[test]
    fn a_corrupt_entry_is_an_error_that_does_not_quote_it() {
        let backend = MemoryBackend::default();
        let store = CredentialStore::new(Arc::new(backend.clone()));
        backend
            .set("tiktok-login:riviu.demo", "{\"v\":1,\"password\":\"leak-me")
            .unwrap();
        let error = format!("{:#}", store.tiktok_login("riviu.demo").unwrap_err());
        assert!(!error.contains("leak-me"), "{error}");
    }
}
