use std::path::PathBuf;

use riviu_core::db::Database;
use riviu_core::AgentSettings;
use riviu_ios_driver::{AgentArtifact, AgentToken, DriverConfig, DriverTarget, UnifiedAgentConfig};
use riviu_signing::CredentialStore;

pub struct ResolvedAgentRuntime {
    pub driver_config: DriverConfig,
    pub settings: AgentSettings,
    pub token_configured: bool,
}

/// The desktop can run an Android fleet even when its independent iOS runtime
/// cannot be resolved. Database failures remain fatal; iOS-only token,
/// manifest, artifact, and checksum failures are carried as a degraded result.
pub struct BootstrapAgentRuntimeResolution {
    pub runtime: Option<ResolvedAgentRuntime>,
    pub settings: AgentSettings,
    pub degraded_reason: Option<String>,
    pub degraded_driver_config: DriverConfig,
}

pub fn resolve_desktop_agent_runtime_for_bootstrap_with_candidate(
    sidecar_root: PathBuf,
    state_dir: PathBuf,
    database: &Database,
    credentials: &CredentialStore,
    legacy_token: Option<&str>,
    mock_requested: bool,
    prefer_candidate: bool,
) -> anyhow::Result<BootstrapAgentRuntimeResolution> {
    // Settings belong to the desktop, not to either platform. Read them before
    // attempting the iOS-only artifact path so an Android-only boot retains the
    // operator's configuration.
    let settings = database.get_agent_settings()?;
    let degraded_driver_config = DriverConfig {
        sidecar_root: sidecar_root.clone(),
        state_dir: state_dir.clone(),
        target: DriverTarget::LegacyStock,
    };
    match resolve_desktop_agent_runtime_with_candidate(
        sidecar_root,
        state_dir,
        database,
        credentials,
        legacy_token,
        mock_requested,
        prefer_candidate,
    ) {
        Ok(runtime) => Ok(BootstrapAgentRuntimeResolution {
            settings,
            runtime: Some(runtime),
            degraded_reason: None,
            degraded_driver_config,
        }),
        Err(error) => Ok(BootstrapAgentRuntimeResolution {
            runtime: None,
            settings,
            degraded_reason: Some(format!("iOS agent runtime unavailable: {error:#}")),
            degraded_driver_config,
        }),
    }
}

/// Convenience wrapper that pins `candidate = false`. Production resolves the
/// candidate explicitly through `..._with_candidate`, so this exists for tests.
#[cfg(test)]
pub fn resolve_desktop_agent_runtime(
    sidecar_root: PathBuf,
    state_dir: PathBuf,
    database: &Database,
    credentials: &CredentialStore,
    legacy_token: Option<&str>,
    mock_requested: bool,
) -> anyhow::Result<ResolvedAgentRuntime> {
    resolve_desktop_agent_runtime_with_candidate(
        sidecar_root,
        state_dir,
        database,
        credentials,
        legacy_token,
        mock_requested,
        false,
    )
}

pub fn resolve_desktop_agent_runtime_with_candidate(
    sidecar_root: PathBuf,
    state_dir: PathBuf,
    database: &Database,
    credentials: &CredentialStore,
    legacy_token: Option<&str>,
    mock_requested: bool,
    prefer_candidate: bool,
) -> anyhow::Result<ResolvedAgentRuntime> {
    let settings = database.get_agent_settings()?;
    if mock_requested {
        return Ok(ResolvedAgentRuntime {
            driver_config: DriverConfig {
                sidecar_root,
                state_dir,
                target: DriverTarget::Mock,
            },
            settings,
            token_configured: false,
        });
    }

    let backend_override = std::env::var("RIVIU_AGENT_MODE")
        .or_else(|_| std::env::var("RIVIU_WDA_BACKEND"))
        .unwrap_or_default();
    let backend_override = backend_override.trim().to_ascii_lowercase();
    let default_mode = option_env!("RIVIU_DEFAULT_AGENT_MODE").unwrap_or(if prefer_candidate {
        "candidate"
    } else {
        "rt-mmo"
    });
    let effective_mode = if backend_override.is_empty() || backend_override == "auto" {
        default_mode
    } else {
        backend_override.as_str()
    };
    let explicit_manifest = std::env::var_os("RIVIU_AGENT_MANIFEST").map(PathBuf::from);
    let candidate_manifest = sidecar_root.join("wda").join("candidate-manifest.json");
    let text_manifest = sidecar_root.join("wda").join("text-manifest.json");
    let full_requested = effective_mode == "full";
    let text_requested = matches!(effective_mode, "text" | "candidate-text");
    let use_candidate = explicit_manifest.is_some()
        || full_requested
        || text_requested
        || matches!(effective_mode, "candidate" | "riviu-agent")
        || (prefer_candidate && effective_mode == "candidate" && candidate_manifest.is_file());
    let manifest_path = explicit_manifest.unwrap_or_else(|| {
        if text_requested && !full_requested {
            text_manifest
        } else if use_candidate {
            candidate_manifest
        } else {
            sidecar_root.join("wda").join("agent-manifest.json")
        }
    });
    let (token, token_configured) = if use_candidate {
        let explicit_token = std::env::var("RIVIU_AGENT_TOKEN").ok();
        if effective_mode == "full" {
            // Full owns a short-lived local agent session. Avoid an interactive
            // Keychain read/write during bootstrap; production candidate mode
            // keeps the persistent Keychain-backed credential path below.
            (
                credentials.candidate_agent_token_ephemeral(explicit_token.as_deref())?,
                true,
            )
        } else {
            let token = credentials.candidate_agent_token_or_create(explicit_token.as_deref())?;
            (token, credentials.has_candidate_agent_token()?)
        }
    } else {
        let token = credentials.agent_token_or_create(legacy_token)?;
        (token, credentials.has_agent_token()?)
    };
    let artifact = AgentArtifact::load(manifest_path)?;
    artifact.verify_checksum()?;

    let unified = UnifiedAgentConfig {
        token: AgentToken::new(token)?,
        artifact,
        settings: settings.clone(),
    };
    Ok(ResolvedAgentRuntime {
        driver_config: DriverConfig {
            sidecar_root,
            state_dir,
            target: DriverTarget::Real(unified),
        },
        settings,
        token_configured,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use riviu_signing::CredentialBackend;
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct MemoryBackend {
        values: Arc<Mutex<HashMap<String, String>>>,
    }

    impl CredentialBackend for MemoryBackend {
        fn get(&self, account: &str) -> anyhow::Result<Option<String>> {
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

    struct Fixture {
        db: Database,
        db_path: PathBuf,
        credentials: CredentialStore,
    }

    impl Fixture {
        fn new() -> Self {
            let db_path = std::env::temp_dir().join(format!(
                "riviu-agent-runtime-test-{}.db",
                uuid::Uuid::new_v4()
            ));
            let backend = MemoryBackend::default();
            Self {
                db: Database::open(&db_path).expect("open fixture database"),
                db_path,
                credentials: CredentialStore::new(Arc::new(backend)),
            }
        }

        fn try_resolve(&self, legacy_token: Option<&str>) -> anyhow::Result<ResolvedAgentRuntime> {
            resolve_desktop_agent_runtime(
                bundled_sidecar_root(),
                std::env::temp_dir().join("riviu-agent-runtime-state"),
                &self.db,
                &self.credentials,
                legacy_token,
                false,
            )
        }

        fn resolve(&self, legacy_token: Option<&str>) -> ResolvedAgentRuntime {
            self.try_resolve(legacy_token)
                .expect("resolve desktop agent runtime")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.db_path);
        }
    }

    fn bundled_sidecar_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .join("sidecars")
            .canonicalize()
            .expect("bundled sidecar root")
    }

    #[test]
    fn new_install_resolves_the_bundled_unified_artifact() {
        let fixture = Fixture::new();

        let runtime = fixture.resolve(Some("artifact-token"));

        let DriverTarget::Real(config) = runtime.driver_config.target else {
            panic!("desktop must select the unified real agent")
        };
        assert_eq!(config.artifact.manifest.bundle_id, "com.mrph.svc");
        assert_eq!(config.artifact.manifest.control_port, 8906);
        assert_eq!(config.artifact.manifest.mjpeg_port, 9093);
        assert!(runtime.token_configured);
    }

    #[test]
    fn new_install_without_the_artifact_token_is_rejected() {
        let fixture = Fixture::new();

        let error = fixture
            .try_resolve(None)
            .err()
            .expect("missing first-run token must fail");

        assert!(error.to_string().contains("RIVIU_RTMMO_TOKEN"));
    }

    #[test]
    fn mock_runtime_needs_neither_production_token_nor_agent_artifact() {
        let fixture = Fixture::new();
        let runtime = resolve_desktop_agent_runtime(
            std::env::temp_dir().join("missing-riviu-sidecars"),
            std::env::temp_dir().join("riviu-agent-runtime-mock-state"),
            &fixture.db,
            &fixture.credentials,
            None,
            true,
        )
        .expect("resolve isolated mock runtime");

        assert!(matches!(runtime.driver_config.target, DriverTarget::Mock));
        assert!(!runtime.token_configured);
    }

    #[test]
    fn explicit_token_can_replace_a_previous_import() {
        let fixture = Fixture::new();

        let _ = fixture.resolve(Some("legacy-token"));

        assert_eq!(
            fixture
                .credentials
                .agent_token_or_create(Some("replacement-token"))
                .expect("read imported token"),
            "replacement-token"
        );
    }

    #[test]
    fn stock_backend_environment_does_not_switch_the_desktop_product() {
        static ENV_LOCK: Mutex<()> = Mutex::new(());
        let _guard = ENV_LOCK.lock().expect("environment lock");
        let previous = std::env::var_os("RIVIU_WDA_BACKEND");
        std::env::set_var("RIVIU_WDA_BACKEND", "stock");
        let fixture = Fixture::new();

        let runtime = fixture.resolve(Some("artifact-token"));

        match previous {
            Some(value) => std::env::set_var("RIVIU_WDA_BACKEND", value),
            None => std::env::remove_var("RIVIU_WDA_BACKEND"),
        }
        assert!(matches!(
            runtime.driver_config.target,
            DriverTarget::Real(_)
        ));
    }
}
