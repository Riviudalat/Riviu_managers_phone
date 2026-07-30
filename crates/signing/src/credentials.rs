use std::sync::Arc;

use anyhow::Context;

const SERVICE: &str = "riviu-managers-phone";
pub(crate) const AGENT_TOKEN_ACCOUNT: &str = "agent-auth-token";
pub(crate) const APPLE_EMAIL_ACCOUNT: &str = "apple-id-email";
pub(crate) const APPLE_PASSWORD_ACCOUNT: &str = "apple-id-password";

pub trait CredentialBackend: Send + Sync {
    fn get(&self, account: &str) -> anyhow::Result<Option<String>>;
    fn set(&self, account: &str, value: &str) -> anyhow::Result<()>;
    fn delete(&self, account: &str) -> anyhow::Result<()>;
}

struct SystemCredentialBackend;

impl SystemCredentialBackend {
    fn entry(account: &str) -> anyhow::Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, account)
            .with_context(|| format!("failed to open OS credential account {account}"))
    }
}

impl CredentialBackend for SystemCredentialBackend {
    fn get(&self, account: &str) -> anyhow::Result<Option<String>> {
        match Self::entry(account)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error)
                .with_context(|| format!("failed to read OS credential account {account}")),
        }
    }

    fn set(&self, account: &str, value: &str) -> anyhow::Result<()> {
        Self::entry(account)?
            .set_password(value)
            .with_context(|| format!("failed to write OS credential account {account}"))
    }

    fn delete(&self, account: &str) -> anyhow::Result<()> {
        match Self::entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error)
                .with_context(|| format!("failed to delete OS credential account {account}")),
        }
    }
}

#[derive(Clone)]
pub struct CredentialStore {
    backend: Arc<dyn CredentialBackend>,
}

impl CredentialStore {
    pub fn new(backend: Arc<dyn CredentialBackend>) -> Self {
        Self { backend }
    }

    pub fn system() -> anyhow::Result<Self> {
        Ok(Self::new(Arc::new(SystemCredentialBackend)))
    }

    pub fn agent_token_or_create(&self, legacy_env: Option<&str>) -> anyhow::Result<String> {
        let explicit = legacy_env.map(str::trim).filter(|value| !value.is_empty());
        if let Some(token) = explicit {
            if self.get(AGENT_TOKEN_ACCOUNT)?.as_deref() != Some(token) {
                self.set(AGENT_TOKEN_ACCOUNT, token)?;
            }
            return Ok(token.to_owned());
        }

        self.get(AGENT_TOKEN_ACCOUNT)?.ok_or_else(|| {
            anyhow::anyhow!(
                "agent auth token is not configured; set RIVIU_RTMMO_TOKEN for the first launch"
            )
        })
    }

    pub fn has_agent_token(&self) -> anyhow::Result<bool> {
        Ok(self
            .get(AGENT_TOKEN_ACCOUNT)?
            .is_some_and(|token| !token.is_empty()))
    }

    pub(crate) fn get(&self, account: &str) -> anyhow::Result<Option<String>> {
        self.backend.get(account)
    }

    pub(crate) fn set(&self, account: &str, value: &str) -> anyhow::Result<()> {
        self.backend.set(account, value)
    }

    pub(crate) fn delete(&self, account: &str) -> anyhow::Result<()> {
        self.backend.delete(account)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct MemoryBackend {
        values: Arc<Mutex<HashMap<String, String>>>,
        fail_get: Arc<Mutex<bool>>,
    }

    impl CredentialBackend for MemoryBackend {
        fn get(&self, account: &str) -> anyhow::Result<Option<String>> {
            if *self.fail_get.lock().expect("fail flag") {
                anyhow::bail!("fixture backend read failed");
            }
            Ok(self.values.lock().expect("values").get(account).cloned())
        }

        fn set(&self, account: &str, value: &str) -> anyhow::Result<()> {
            self.values
                .lock()
                .expect("values")
                .insert(account.to_string(), value.to_string());
            Ok(())
        }

        fn delete(&self, account: &str) -> anyhow::Result<()> {
            self.values.lock().expect("values").remove(account);
            Ok(())
        }
    }

    fn fixture() -> (MemoryBackend, CredentialStore) {
        let backend = MemoryBackend::default();
        let store = CredentialStore::new(Arc::new(backend.clone()));
        (backend, store)
    }

    #[test]
    fn explicit_environment_token_replaces_a_stale_stored_value() {
        let (backend, store) = fixture();
        backend.set(AGENT_TOKEN_ACCOUNT, "stored-token").unwrap();

        assert_eq!(
            store.agent_token_or_create(Some("legacy-token")).unwrap(),
            "legacy-token"
        );
        assert_eq!(
            backend.get(AGENT_TOKEN_ACCOUNT).unwrap().as_deref(),
            Some("legacy-token")
        );
    }

    #[test]
    fn first_run_migrates_a_legacy_token_once() {
        let (backend, store) = fixture();

        assert_eq!(
            store.agent_token_or_create(Some(" legacy-token ")).unwrap(),
            "legacy-token"
        );
        assert_eq!(store.agent_token_or_create(None).unwrap(), "legacy-token");
        assert_eq!(
            backend.get(AGENT_TOKEN_ACCOUNT).unwrap().as_deref(),
            Some("legacy-token")
        );
    }

    #[test]
    fn first_run_requires_the_artifact_token_instead_of_generating_one() {
        let (backend, store) = fixture();

        let error = store
            .agent_token_or_create(Some("   "))
            .expect_err("missing artifact token must fail");

        assert!(error.to_string().contains("RIVIU_RTMMO_TOKEN"));
        assert_eq!(backend.get(AGENT_TOKEN_ACCOUNT).unwrap(), None);
    }

    #[test]
    fn apple_id_and_agent_token_use_distinct_accounts() {
        assert_ne!(AGENT_TOKEN_ACCOUNT, APPLE_EMAIL_ACCOUNT);
        assert_ne!(AGENT_TOKEN_ACCOUNT, APPLE_PASSWORD_ACCOUNT);
        assert_ne!(APPLE_EMAIL_ACCOUNT, APPLE_PASSWORD_ACCOUNT);
    }

    #[test]
    fn backend_errors_are_not_reported_as_missing_credentials() {
        let (backend, store) = fixture();
        *backend.fail_get.lock().expect("fail flag") = true;

        let error = store
            .has_agent_token()
            .expect_err("backend error must propagate");

        assert!(error.to_string().contains("fixture backend read failed"));
    }
}
