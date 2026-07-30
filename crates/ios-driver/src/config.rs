use std::fmt;
use std::path::PathBuf;

use anyhow::bail;
use riviu_core::AgentSettings;

use crate::AgentArtifact;

#[derive(Clone, PartialEq, Eq)]
pub struct AgentToken(String);

impl AgentToken {
    pub fn new(value: impl Into<String>) -> anyhow::Result<Self> {
        let value = value.into();
        let value = value.trim();
        if value.is_empty() {
            bail!("agent token must not be empty");
        }
        Ok(Self(value.to_owned()))
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AgentToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AgentToken([REDACTED])")
    }
}

#[derive(Debug, Clone)]
pub struct UnifiedAgentConfig {
    pub token: AgentToken,
    pub artifact: AgentArtifact,
    pub settings: AgentSettings,
}

#[derive(Debug, Clone)]
pub enum DriverTarget {
    Mock,
    Real(UnifiedAgentConfig),
    LegacyStock,
}

#[derive(Debug, Clone)]
pub struct DriverConfig {
    pub sidecar_root: PathBuf,
    pub state_dir: PathBuf,
    pub target: DriverTarget,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn bundled_artifact() -> AgentArtifact {
        AgentArtifact::load(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sidecars/wda/agent-manifest.json"),
        )
        .expect("load bundled artifact")
    }

    #[test]
    fn agent_token_debug_is_redacted() {
        let token = AgentToken::new("fixture-token").unwrap();

        assert_eq!(format!("{token:?}"), "AgentToken([REDACTED])");
    }

    #[test]
    fn unified_config_uses_manifest_ports_bundle_and_ipa() {
        let artifact = bundled_artifact();
        let config = UnifiedAgentConfig {
            token: AgentToken::new("fixture-token").unwrap(),
            artifact: artifact.clone(),
            settings: AgentSettings::default(),
        };

        assert_eq!(config.artifact.manifest.bundle_id, "com.mrph.svc");
        assert_eq!(config.artifact.manifest.control_port, 8906);
        assert_eq!(config.artifact.manifest.mjpeg_port, 9093);
        assert_eq!(config.artifact.ipa_path, artifact.ipa_path);
        assert!(config
            .artifact
            .manifest
            .features
            .iter()
            .any(|feature| feature == "text"));
    }

    #[test]
    fn empty_agent_token_is_rejected() {
        assert!(AgentToken::new("").is_err());
        assert!(AgentToken::new("   ").is_err());
    }
}
