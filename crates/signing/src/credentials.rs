use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;

const SERVICE: &str = "riviu-managers-phone";
pub(crate) const AGENT_TOKEN_ACCOUNT: &str = "agent-auth-token";
pub(crate) const CANDIDATE_AGENT_TOKEN_ACCOUNT: &str = "agent-candidate-auth-token";
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
    /// How long a *candidate* keychain backend gets to answer before it is passed over.
    ///
    /// This runs while choosing a backend, not while using one, so the cost of waiting is
    /// paid on a path the operator is sitting in front of. A keyring service that has not
    /// answered in two seconds is wedged — on Windows that is a stalled Credential Manager —
    /// and the right move is the next candidate, not a longer wait.
    const CANDIDATE_KEYCHAIN_TIMEOUT: Duration = Duration::from_secs(2);

    pub fn new(backend: Arc<dyn CredentialBackend>) -> Self {
        Self { backend }
    }

    pub fn system() -> anyhow::Result<Self> {
        Ok(Self::new(Arc::new(SystemCredentialBackend)))
    }

    /// Read one application secret by name.
    ///
    /// Namespaced under `app-secret:` so a caller cannot reach — or collide with — the four
    /// purpose-built accounts above (the agent tokens and the Apple ID pair), each of which has
    /// its own reasoning and its own test.
    pub fn app_secret(&self, name: &str) -> anyhow::Result<Option<String>> {
        self.backend.get(&Self::app_secret_account(name))
    }

    /// Write one application secret by name. An empty value clears it.
    pub fn set_app_secret(&self, name: &str, value: &str) -> anyhow::Result<()> {
        let account = Self::app_secret_account(name);
        if value.is_empty() {
            return self.backend.delete(&account);
        }
        self.backend.set(&account, value)
    }

    fn app_secret_account(name: &str) -> String {
        format!("app-secret:{name}")
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

    /// Candidate Agent authentication is local to this desktop and can be
    /// generated on first launch. Keep it in a separate Keychain account so a
    /// production RT-MMO credential is never silently reused with protocol v2.
    pub fn candidate_agent_token_or_create(
        &self,
        explicit_env: Option<&str>,
    ) -> anyhow::Result<String> {
        let explicit = explicit_env
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(token) = explicit {
            if self.candidate_get()?.as_deref() != Some(token) {
                self.candidate_set(token)?;
            }
            return Ok(token.to_owned());
        }

        if let Some(token) = self.candidate_get()? {
            if !token.trim().is_empty() {
                return Ok(token);
            }
        }

        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        // A locked/unavailable macOS keychain must not block the desktop
        // startup path. The token remains valid for this process and the
        // best-effort write can complete on a later launch.
        self.candidate_set(&token)?;
        Ok(token)
    }

    /// Generate the Full desktop credential without touching the interactive
    /// macOS Keychain. The Full bundle owns its candidate agent lifecycle, so
    /// an in-memory token is sufficient for that process and avoids a login
    /// keychain prompt during every bootstrap.
    pub fn candidate_agent_token_ephemeral(
        &self,
        explicit_env: Option<&str>,
    ) -> anyhow::Result<String> {
        if let Some(token) = explicit_env
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Ok(token.to_owned());
        }
        Ok(format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        ))
    }

    pub fn has_candidate_agent_token(&self) -> anyhow::Result<bool> {
        Ok(self
            .candidate_get()?
            .is_some_and(|token| !token.trim().is_empty()))
    }

    fn candidate_get(&self) -> anyhow::Result<Option<String>> {
        let backend = self.backend.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let _ = sender.send(backend.get(CANDIDATE_AGENT_TOKEN_ACCOUNT));
        });
        match receiver.recv_timeout(Self::CANDIDATE_KEYCHAIN_TIMEOUT) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                anyhow::bail!("candidate credential worker disconnected")
            }
        }
    }

    fn candidate_set(&self, value: &str) -> anyhow::Result<()> {
        let backend = self.backend.clone();
        let value = value.to_owned();
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let _ = sender.send(backend.set(CANDIDATE_AGENT_TOKEN_ACCOUNT, &value));
        });
        match receiver.recv_timeout(Self::CANDIDATE_KEYCHAIN_TIMEOUT) {
            Ok(result) => result,
            // Keep the generated token in memory when Keychain is waiting for
            // user interaction. A later launch retries persistence.
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(()),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                anyhow::bail!("candidate credential worker disconnected")
            }
        }
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
    fn ephemeral_candidate_token_does_not_touch_the_backend() {
        let (backend, store) = fixture();

        let token = store
            .candidate_agent_token_ephemeral(None)
            .expect("generate candidate token");

        assert_eq!(token.len(), 64);
        assert_eq!(backend.get(CANDIDATE_AGENT_TOKEN_ACCOUNT).unwrap(), None);
    }

    #[test]
    fn app_secrets_cannot_collide_with_the_purpose_built_accounts() {
        let (backend, store) = fixture();
        store
            .set_app_secret("nurture-ai-api-key", "sk-x")
            .expect("set");
        assert_eq!(
            store.app_secret("nurture-ai-api-key").unwrap().as_deref(),
            Some("sk-x")
        );
        // Not stored under a bare name, so nothing can reach the agent/Apple accounts by
        // choosing a clever `name`.
        assert!(backend.get("nurture-ai-api-key").unwrap().is_none());
        assert!(backend
            .get("app-secret:nurture-ai-api-key")
            .unwrap()
            .is_some());
        // Empty clears rather than storing an empty string.
        store
            .set_app_secret("nurture-ai-api-key", "")
            .expect("clear");
        assert!(store.app_secret("nurture-ai-api-key").unwrap().is_none());
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
