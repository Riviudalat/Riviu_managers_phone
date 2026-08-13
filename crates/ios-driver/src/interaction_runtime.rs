use anyhow::Context;
use async_trait::async_trait;
use riviu_core::{AgentInstallProof, InstalledAgentIdentity};
use sha2::{Digest, Sha256};

use crate::{AgentInstallDecision, InstalledAppMetadata};

#[derive(Debug, Clone)]
pub(crate) struct InstallOnlyInspection {
    pub metadata: InstalledAppMetadata,
    pub identity: Option<InstalledAgentIdentity>,
}

#[async_trait]
pub(crate) trait InstallOnlyRuntime: Send {
    fn verify_artifact(&self) -> anyhow::Result<()>;

    fn artifact_sha256(&self) -> &str;

    fn install_decision(&self, installed: Option<&InstalledAppMetadata>) -> AgentInstallDecision;

    async fn inspect(&mut self, udid: &str) -> anyhow::Result<Option<InstallOnlyInspection>>;

    async fn uninstall(&mut self, udid: &str) -> anyhow::Result<()>;

    async fn install(&mut self, udid: &str) -> anyhow::Result<()>;

    async fn launch_auth(&mut self, udid: &str) -> anyhow::Result<()>;

    async fn protected_health(&mut self, udid: &str) -> anyhow::Result<bool>;
}

pub(crate) async fn repair_install_only_locked(
    runtime: &mut (impl InstallOnlyRuntime + ?Sized),
    udid: &str,
) -> anyhow::Result<AgentInstallProof> {
    runtime
        .verify_artifact()
        .context("selected Agent artifact failed integrity verification")?;
    let artifact_sha256 = runtime.artifact_sha256().to_ascii_lowercase();

    let mut inspected = runtime.inspect(udid).await?;
    let decision = runtime.install_decision(inspected.as_ref().map(|value| &value.metadata));
    let changed = match decision {
        AgentInstallDecision::Reuse => false,
        AgentInstallDecision::InstallMissing => {
            runtime.install(udid).await?;
            true
        }
        AgentInstallDecision::RepairVersionMismatch => {
            runtime.uninstall(udid).await?;
            runtime.install(udid).await?;
            true
        }
        AgentInstallDecision::ReportRepairRequired => {
            anyhow::bail!("install-only repair did not select a repairable Agent state")
        }
    };

    if changed {
        inspected = runtime.inspect(udid).await?;
    }
    let inspected = inspected.context("selected Agent is missing after install-only repair")?;
    if runtime.install_decision(Some(&inspected.metadata)) != AgentInstallDecision::Reuse {
        anyhow::bail!("installed Agent metadata does not match the selected artifact");
    }
    let identity = inspected
        .identity
        .context("installed Agent identity is unavailable after install-only repair")?;
    if identity.bundle_id != inspected.metadata.bundle_id
        || Some(identity.version.as_str()) != inspected.metadata.version.as_deref()
        || Some(identity.build.as_str()) != inspected.metadata.build.as_deref()
        || inspected
            .metadata
            .signer_identity
            .as_deref()
            .map(|signer| format!("{:x}", Sha256::digest(signer.as_bytes())))
            .as_deref()
            != Some(identity.signer_identity_sha256.as_str())
    {
        anyhow::bail!("installed Agent identity disagrees with inspected metadata");
    }

    runtime.launch_auth(udid).await?;
    if !runtime.protected_health(udid).await? {
        anyhow::bail!("protected Agent auth probe failed");
    }

    let proof = AgentInstallProof {
        installed: identity,
        artifact_sha256,
        protected_auth_ready: true,
        session_created: false,
        stream_started: false,
    };
    proof
        .validate_install_only()
        .context("invalid install-only Agent proof")?;
    Ok(proof)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use parking_lot::Mutex;

    use crate::{AgentInstallDecision, InstalledAppMetadata};

    const ARTIFACT_SHA256: &str =
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SIGNER_SHA256: &str = "b2fe399af3af0d10c303ae38c65cf9683e60d12c4223dab6fbdade63a82185bd";

    #[derive(Debug, Clone, Copy)]
    enum RuntimeFault {
        None,
        Auth,
        ProtectedHealth,
    }

    struct FixtureRuntime {
        calls: Mutex<Vec<&'static str>>,
        installed: Mutex<Option<InstallOnlyInspection>>,
        fault: RuntimeFault,
        session_creates: AtomicUsize,
        stream_starts: AtomicUsize,
    }

    impl FixtureRuntime {
        fn missing() -> Self {
            Self::with_install(None, RuntimeFault::None)
        }

        fn matching(fault: RuntimeFault) -> Self {
            Self::with_install(Some(matching_inspection()), fault)
        }

        fn mismatched() -> Self {
            let mut inspection = matching_inspection();
            inspection.metadata.version = Some("0.9".into());
            inspection.identity = None;
            Self::with_install(Some(inspection), RuntimeFault::None)
        }

        fn with_install(installed: Option<InstallOnlyInspection>, fault: RuntimeFault) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                installed: Mutex::new(installed),
                fault,
                session_creates: AtomicUsize::new(0),
                stream_starts: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> Vec<&'static str> {
            self.calls.lock().clone()
        }

        fn session_creates(&self) -> usize {
            self.session_creates.load(Ordering::Relaxed)
        }

        fn stream_starts(&self) -> usize {
            self.stream_starts.load(Ordering::Relaxed)
        }
    }

    #[async_trait]
    impl InstallOnlyRuntime for FixtureRuntime {
        fn verify_artifact(&self) -> anyhow::Result<()> {
            self.calls.lock().push("verify_artifact");
            Ok(())
        }

        fn artifact_sha256(&self) -> &str {
            ARTIFACT_SHA256
        }

        fn install_decision(
            &self,
            installed: Option<&InstalledAppMetadata>,
        ) -> AgentInstallDecision {
            match installed {
                None => AgentInstallDecision::InstallMissing,
                Some(installed)
                    if installed.version.as_deref() == Some("1.0")
                        && installed.build.as_deref() == Some("1") =>
                {
                    AgentInstallDecision::Reuse
                }
                Some(_) => AgentInstallDecision::RepairVersionMismatch,
            }
        }

        async fn inspect(&mut self, _udid: &str) -> anyhow::Result<Option<InstallOnlyInspection>> {
            self.calls.lock().push("inspect");
            Ok(self.installed.lock().clone())
        }

        async fn uninstall(&mut self, _udid: &str) -> anyhow::Result<()> {
            self.calls.lock().push("uninstall");
            *self.installed.lock() = None;
            Ok(())
        }

        async fn install(&mut self, _udid: &str) -> anyhow::Result<()> {
            self.calls.lock().push("install");
            *self.installed.lock() = Some(matching_inspection());
            Ok(())
        }

        async fn launch_auth(&mut self, _udid: &str) -> anyhow::Result<()> {
            self.calls.lock().push("launch_auth");
            if matches!(self.fault, RuntimeFault::Auth) {
                anyhow::bail!("fixture protected auth failure");
            }
            Ok(())
        }

        async fn protected_health(&mut self, _udid: &str) -> anyhow::Result<bool> {
            self.calls.lock().push("protected_health");
            Ok(!matches!(self.fault, RuntimeFault::ProtectedHealth))
        }
    }

    fn matching_metadata() -> InstalledAppMetadata {
        InstalledAppMetadata {
            bundle_id: "com.fixture.agent".into(),
            version: Some("1.0".into()),
            build: Some("1".into()),
            application_type: Some("User".into()),
            path: Some("/private/FixtureAgent.app".into()),
            signer_identity: Some("Fixture Signer".into()),
        }
    }

    fn matching_inspection() -> InstallOnlyInspection {
        InstallOnlyInspection {
            metadata: matching_metadata(),
            identity: Some(riviu_core::InstalledAgentIdentity {
                bundle_id: "com.fixture.agent".into(),
                version: "1.0".into(),
                build: "1".into(),
                executable_name: "FixtureAgent".into(),
                signer_identity_sha256: SIGNER_SHA256.into(),
            }),
        }
    }

    #[tokio::test]
    async fn install_only_repair_never_proves_session_or_stream() {
        let mut runtime = FixtureRuntime::missing();

        let proof = repair_install_only_locked(&mut runtime, "fixture")
            .await
            .expect("install-only repair");

        assert_eq!(
            runtime.calls(),
            [
                "verify_artifact",
                "inspect",
                "install",
                "inspect",
                "launch_auth",
                "protected_health",
            ]
        );
        assert_eq!(runtime.session_creates(), 0);
        assert_eq!(runtime.stream_starts(), 0);
        assert!(proof.protected_auth_ready);
        assert!(!proof.session_created);
        assert!(!proof.stream_started);
        proof.validate_install_only().expect("valid proof");
    }

    #[tokio::test]
    async fn install_only_replaces_only_a_mismatched_install() {
        let mut runtime = FixtureRuntime::mismatched();

        repair_install_only_locked(&mut runtime, "fixture")
            .await
            .expect("repair mismatched install");

        assert_eq!(
            runtime.calls(),
            [
                "verify_artifact",
                "inspect",
                "uninstall",
                "install",
                "inspect",
                "launch_auth",
                "protected_health",
            ]
        );
    }

    #[tokio::test]
    async fn install_only_auth_faults_never_trigger_reinstall() {
        for fault in [RuntimeFault::Auth, RuntimeFault::ProtectedHealth] {
            let mut runtime = FixtureRuntime::matching(fault);

            let result = repair_install_only_locked(&mut runtime, "fixture").await;

            assert!(result.is_err());
            assert!(!runtime.calls().contains(&"uninstall"));
            assert!(!runtime.calls().contains(&"install"));
            assert_eq!(runtime.session_creates(), 0);
            assert_eq!(runtime.stream_starts(), 0);
        }
    }

    #[tokio::test]
    async fn install_only_rejects_identity_that_disagrees_with_inspected_metadata() {
        let mut inspection = matching_inspection();
        inspection.identity.as_mut().expect("identity").build = "2".into();
        let mut runtime = FixtureRuntime::with_install(Some(inspection), RuntimeFault::None);

        let error = repair_install_only_locked(&mut runtime, "fixture")
            .await
            .expect_err("incoherent inspection must fail closed");

        assert!(error.to_string().contains("identity"));
        assert!(!runtime.calls().contains(&"launch_auth"));
        assert!(!runtime.calls().contains(&"uninstall"));
        assert!(!runtime.calls().contains(&"install"));
    }

    #[tokio::test]
    async fn install_only_rejects_signer_hash_that_disagrees_with_inspected_metadata() {
        let mut inspection = matching_inspection();
        inspection
            .identity
            .as_mut()
            .expect("identity")
            .signer_identity_sha256 = "c".repeat(64);
        let mut runtime = FixtureRuntime::with_install(Some(inspection), RuntimeFault::None);

        let error = repair_install_only_locked(&mut runtime, "fixture")
            .await
            .expect_err("incoherent signer proof must fail closed");

        assert!(error.to_string().contains("identity"));
        assert!(!runtime.calls().contains(&"launch_auth"));
    }
}
