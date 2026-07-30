use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use parking_lot::Mutex;
use riviu_core::{AgentInstallProof, InstalledAgentIdentity, InteractionSessionKind};
use sha2::{Digest, Sha256};

use crate::{AgentInstallDecision, InstalledAppMetadata};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InteractionLifecyclePhase {
    Stopped,
    SessionStarting {
        token: u64,
        kind: InteractionSessionKind,
    },
    SessionReady {
        token: u64,
        kind: InteractionSessionKind,
    },
    StreamStarting {
        token: u64,
        kind: InteractionSessionKind,
    },
    Streaming {
        token: u64,
        kind: InteractionSessionKind,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InteractionLifecycleState {
    generation: u64,
    phase: InteractionLifecyclePhase,
}

#[derive(Default)]
struct InteractionLifecycleMap {
    next_token: u64,
    devices: HashMap<String, InteractionLifecycleState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InteractionSessionReservation {
    udid: String,
    generation: u64,
    token: u64,
    kind: InteractionSessionKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InteractionStreamReservation {
    udid: String,
    generation: u64,
    token: u64,
    kind: InteractionSessionKind,
}

impl InteractionStreamReservation {
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }
}

/// Driver-local proof that the explicit interaction lifecycle is being
/// followed for one device. The token never crosses the driver boundary.
#[derive(Clone, Default)]
pub(crate) struct InteractionLifecycleRegistry {
    state: Arc<Mutex<InteractionLifecycleMap>>,
}

impl InteractionLifecycleRegistry {
    pub(crate) fn record_stopped(&self, udid: &str, generation: u64) {
        self.state.lock().devices.insert(
            udid.to_string(),
            InteractionLifecycleState {
                generation,
                phase: InteractionLifecyclePhase::Stopped,
            },
        );
    }

    pub(crate) fn begin_session(
        &self,
        udid: &str,
        generation: u64,
        kind: InteractionSessionKind,
    ) -> anyhow::Result<InteractionSessionReservation> {
        let mut state = self.state.lock();
        let Some(current) = state.devices.get(udid).copied() else {
            anyhow::bail!(
                "interaction session requires a stop_owned_stream reservation for {udid}"
            );
        };
        if current.generation != generation || current.phase != InteractionLifecyclePhase::Stopped {
            anyhow::bail!(
                "interaction session requires the current stop_owned_stream reservation for {udid}"
            );
        }

        state.next_token = state
            .next_token
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("interaction lifecycle token space exhausted"))?;
        let token = state.next_token;
        state.devices.insert(
            udid.to_string(),
            InteractionLifecycleState {
                generation,
                phase: InteractionLifecyclePhase::SessionStarting { token, kind },
            },
        );
        Ok(InteractionSessionReservation {
            udid: udid.to_string(),
            generation,
            token,
            kind,
        })
    }

    pub(crate) fn complete_session(
        &self,
        reservation: &InteractionSessionReservation,
    ) -> anyhow::Result<()> {
        let mut state = self.state.lock();
        let Some(current) = state.devices.get_mut(&reservation.udid) else {
            anyhow::bail!("interaction session reservation is no longer active");
        };
        let expected = InteractionLifecyclePhase::SessionStarting {
            token: reservation.token,
            kind: reservation.kind,
        };
        if current.generation != reservation.generation || current.phase != expected {
            anyhow::bail!("interaction session reservation is stale");
        }
        current.phase = InteractionLifecyclePhase::SessionReady {
            token: reservation.token,
            kind: reservation.kind,
        };
        Ok(())
    }

    pub(crate) fn reserve_stream(
        &self,
        udid: &str,
        generation: u64,
    ) -> anyhow::Result<InteractionStreamReservation> {
        let mut state = self.state.lock();
        let Some(current) = state.devices.get_mut(udid) else {
            anyhow::bail!("interaction stream requires an approved session reservation");
        };
        let InteractionLifecyclePhase::SessionReady { token, kind } = current.phase else {
            anyhow::bail!("interaction stream requires an approved session reservation");
        };
        if current.generation != generation {
            anyhow::bail!("interaction stream session reservation has a stale generation");
        }
        current.phase = InteractionLifecyclePhase::StreamStarting { token, kind };
        Ok(InteractionStreamReservation {
            udid: udid.to_string(),
            generation,
            token,
            kind,
        })
    }

    pub(crate) fn complete_stream(
        &self,
        reservation: &InteractionStreamReservation,
    ) -> anyhow::Result<()> {
        let mut state = self.state.lock();
        let Some(current) = state.devices.get_mut(&reservation.udid) else {
            anyhow::bail!("interaction stream reservation is no longer active");
        };
        let expected = InteractionLifecyclePhase::StreamStarting {
            token: reservation.token,
            kind: reservation.kind,
        };
        if current.generation != reservation.generation || current.phase != expected {
            anyhow::bail!("interaction stream reservation is stale");
        }
        current.phase = InteractionLifecyclePhase::Streaming {
            token: reservation.token,
            kind: reservation.kind,
        };
        Ok(())
    }

    pub(crate) fn clear(&self, udid: &str) {
        self.state.lock().devices.remove(udid);
    }

    #[cfg(test)]
    pub(crate) fn has_session_reservation(&self, udid: &str) -> bool {
        self.state.lock().devices.get(udid).is_some_and(|state| {
            matches!(
                state.phase,
                InteractionLifecyclePhase::SessionReady { .. }
                    | InteractionLifecyclePhase::StreamStarting { .. }
                    | InteractionLifecyclePhase::Streaming { .. }
            )
        })
    }
}

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

    #[test]
    fn interaction_lifecycle_requires_stop_then_session_then_stream() {
        let lifecycle = InteractionLifecycleRegistry::default();

        assert!(lifecycle
            .begin_session("fixture", 1, riviu_core::InteractionSessionKind::Ordinary)
            .unwrap_err()
            .to_string()
            .contains("stop_owned_stream"));

        lifecycle.record_stopped("fixture", 1);
        let session = lifecycle
            .begin_session("fixture", 1, riviu_core::InteractionSessionKind::Ordinary)
            .expect("session reservation");
        assert!(lifecycle.reserve_stream("fixture", 1).is_err());
        lifecycle
            .complete_session(&session)
            .expect("approved session");
        let stream = lifecycle
            .reserve_stream("fixture", 1)
            .expect("stream reservation");
        assert_eq!(stream.generation(), 1);
        lifecycle.complete_stream(&stream).expect("running stream");
    }

    #[test]
    fn interaction_lifecycle_rejects_stale_generation_and_clears_failed_transition() {
        let lifecycle = InteractionLifecycleRegistry::default();
        lifecycle.record_stopped("fixture", 4);
        let session = lifecycle
            .begin_session("fixture", 4, riviu_core::InteractionSessionKind::FreshText)
            .expect("session reservation");
        lifecycle
            .complete_session(&session)
            .expect("approved session");

        assert!(lifecycle.reserve_stream("fixture", 5).is_err());
        assert!(lifecycle.has_session_reservation("fixture"));
        lifecycle.clear("fixture");
        assert!(!lifecycle.has_session_reservation("fixture"));
        assert!(lifecycle.reserve_stream("fixture", 4).is_err());
    }

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
