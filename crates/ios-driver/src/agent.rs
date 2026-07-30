use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SUPPORTED_AGENT_PROTOCOL: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentManifest {
    pub artifact_id: String,
    pub artifact_version: String,
    pub bundle_id: String,
    pub bundle_version: String,
    pub bundle_build: String,
    pub payload_app: String,
    pub signer_identity: String,
    pub protocol_version: u32,
    pub ipa: String,
    pub sha256: String,
    pub control_port: u16,
    pub mjpeg_port: u16,
    pub logical_width: u32,
    pub logical_height: u32,
    pub features: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentArtifact {
    pub manifest: AgentManifest,
    pub ipa_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledAppMetadata {
    pub bundle_id: String,
    pub version: Option<String>,
    pub build: Option<String>,
    pub application_type: Option<String>,
    pub path: Option<String>,
    pub signer_identity: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentInstallDecision {
    Reuse,
    InstallMissing,
    RepairVersionMismatch,
    ReportRepairRequired,
}

pub fn decide_install(
    manifest: &AgentManifest,
    installed: Option<&InstalledAppMetadata>,
    auto_repair: bool,
) -> AgentInstallDecision {
    let Some(installed) = installed else {
        return if auto_repair {
            AgentInstallDecision::InstallMissing
        } else {
            AgentInstallDecision::ReportRepairRequired
        };
    };

    let installed_payload = installed
        .path
        .as_deref()
        .and_then(|path| path.rsplit(['/', '\\']).find(|part| !part.is_empty()));
    let matches = installed.bundle_id == manifest.bundle_id
        && installed.version.as_deref() == Some(manifest.bundle_version.as_str())
        && installed.build.as_deref() == Some(manifest.bundle_build.as_str())
        && installed_payload == Some(manifest.payload_app.as_str())
        && installed.signer_identity.as_deref() == Some(manifest.signer_identity.as_str());
    if matches {
        AgentInstallDecision::Reuse
    } else if auto_repair {
        AgentInstallDecision::RepairVersionMismatch
    } else {
        AgentInstallDecision::ReportRepairRequired
    }
}

impl AgentArtifact {
    pub fn load(manifest_path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let manifest_path = manifest_path.as_ref();
        let bytes = std::fs::read(manifest_path).with_context(|| {
            format!("failed to read agent manifest {}", manifest_path.display())
        })?;
        let manifest: AgentManifest = serde_json::from_slice(&bytes).with_context(|| {
            format!("failed to parse agent manifest {}", manifest_path.display())
        })?;
        validate_manifest(&manifest)?;

        let relative_ipa = Path::new(&manifest.ipa);
        if relative_ipa.is_absolute()
            || relative_ipa.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            bail!("agent manifest IPA path must be a safe relative path");
        }

        let manifest_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
        let ipa_path = manifest_dir.join(relative_ipa);
        let metadata = std::fs::metadata(&ipa_path)
            .with_context(|| format!("agent artifact is missing: {}", ipa_path.display()))?;
        if !metadata.is_file() {
            bail!("agent artifact is not a file: {}", ipa_path.display());
        }

        Ok(Self { manifest, ipa_path })
    }

    pub fn verify_checksum(&self) -> anyhow::Result<()> {
        let mut ipa = File::open(&self.ipa_path).with_context(|| {
            format!("failed to open agent artifact {}", self.ipa_path.display())
        })?;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = ipa.read(&mut buffer).with_context(|| {
                format!("failed to read agent artifact {}", self.ipa_path.display())
            })?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }

        let actual = format!("{:x}", hasher.finalize());
        if !actual.eq_ignore_ascii_case(&self.manifest.sha256) {
            bail!(
                "agent artifact checksum mismatch for {}",
                self.manifest.artifact_id
            );
        }
        Ok(())
    }
}

fn validate_manifest(manifest: &AgentManifest) -> anyhow::Result<()> {
    for (name, value) in [
        ("artifactId", manifest.artifact_id.as_str()),
        ("artifactVersion", manifest.artifact_version.as_str()),
        ("bundleId", manifest.bundle_id.as_str()),
        ("bundleVersion", manifest.bundle_version.as_str()),
        ("bundleBuild", manifest.bundle_build.as_str()),
        ("payloadApp", manifest.payload_app.as_str()),
        ("signerIdentity", manifest.signer_identity.as_str()),
        ("ipa", manifest.ipa.as_str()),
        ("sha256", manifest.sha256.as_str()),
    ] {
        if value.trim().is_empty() {
            bail!("agent manifest field {name} must not be blank");
        }
    }

    if manifest.protocol_version != SUPPORTED_AGENT_PROTOCOL {
        bail!(
            "unsupported agent protocol {}; supported protocol is {}",
            manifest.protocol_version,
            SUPPORTED_AGENT_PROTOCOL
        );
    }
    if manifest.control_port == 0 || manifest.mjpeg_port == 0 {
        bail!("agent manifest ports must be non-zero");
    }
    if manifest.logical_width == 0 || manifest.logical_height == 0 {
        bail!("agent manifest logical dimensions must be non-zero");
    }
    if manifest.sha256.len() != 64 || !manifest.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("agent manifest sha256 checksum must be 64 ASCII hexadecimal characters");
    }
    if manifest.features.is_empty()
        || manifest
            .features
            .iter()
            .any(|feature| feature.trim().is_empty())
    {
        bail!("agent manifest features must contain non-blank names");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: PathBuf,
        manifest_path: PathBuf,
    }

    impl Fixture {
        fn new(manifest: Value, create_ipa: bool) -> Self {
            let id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
            let root =
                std::env::temp_dir().join(format!("riviu-agent-test-{}-{id}", std::process::id()));
            fs::create_dir_all(&root).expect("create fixture directory");
            if create_ipa {
                fs::write(root.join("fixture.ipa"), b"fixture IPA bytes")
                    .expect("write fixture IPA");
            }
            let manifest_path = root.join("agent-manifest.json");
            fs::write(
                &manifest_path,
                serde_json::to_vec_pretty(&manifest).expect("serialize fixture manifest"),
            )
            .expect("write fixture manifest");
            Self {
                root,
                manifest_path,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn bundled_manifest_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sidecars/wda/agent-manifest.json")
    }

    fn valid_manifest() -> Value {
        json!({
            "artifactId": "fixture-agent",
            "artifactVersion": "1.2.3",
            "bundleId": "com.example.fixture",
            "bundleVersion": "1.0",
            "bundleBuild": "1",
            "payloadApp": "FixtureAgent.app",
            "signerIdentity": "iPhone Distribution: Fixture Org",
            "protocolVersion": SUPPORTED_AGENT_PROTOCOL,
            "ipa": "fixture.ipa",
            "sha256": "ca76e81375b2f7d536df7a57de0b937a400f88489e74e986e537a66d3f6029b8",
            "controlPort": 8906,
            "mjpegPort": 9093,
            "logicalWidth": 375,
            "logicalHeight": 667,
            "features": ["stream", "text"]
        })
    }

    #[test]
    fn bundled_manifest_matches_the_bundled_ipa() {
        let artifact = AgentArtifact::load(bundled_manifest_path()).expect("load bundled agent");

        assert_eq!(artifact.manifest.bundle_id, "com.mrph.svc");
        assert_eq!(artifact.manifest.protocol_version, SUPPORTED_AGENT_PROTOCOL);
        assert!(artifact
            .manifest
            .features
            .iter()
            .any(|feature| feature == "text"));
        artifact
            .verify_checksum()
            .expect("bundled IPA checksum must match its manifest");
    }

    #[test]
    fn checksum_mismatch_is_rejected_before_install() {
        let mut artifact =
            AgentArtifact::load(bundled_manifest_path()).expect("load bundled agent");
        artifact.manifest.sha256 = "00".repeat(32);

        let error = artifact
            .verify_checksum()
            .expect_err("checksum mismatch must be rejected");

        assert!(error.to_string().to_ascii_lowercase().contains("checksum"));
    }

    #[test]
    fn checksum_comparison_is_case_insensitive() {
        let mut artifact =
            AgentArtifact::load(bundled_manifest_path()).expect("load bundled agent");
        artifact.manifest.sha256.make_ascii_uppercase();

        artifact
            .verify_checksum()
            .expect("uppercase checksum must match the bundled IPA");
    }

    #[test]
    fn unsupported_protocol_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["protocolVersion"] = json!(SUPPORTED_AGENT_PROTOCOL + 1);
        let fixture = Fixture::new(manifest, true);

        let error = AgentArtifact::load(&fixture.manifest_path)
            .expect_err("unsupported protocol must be rejected");

        assert!(error.to_string().contains("protocol"));
    }

    #[test]
    fn unsafe_manifest_ipa_paths_are_rejected() {
        let absolute = std::env::temp_dir().join("outside-riviu-agent.ipa");
        for ipa in [
            absolute.to_string_lossy().into_owned(),
            "../fixture.ipa".into(),
        ] {
            let mut manifest = valid_manifest();
            manifest["ipa"] = json!(ipa);
            let fixture = Fixture::new(manifest, true);

            assert!(AgentArtifact::load(&fixture.manifest_path).is_err());
        }
    }

    #[test]
    fn blank_fields_and_features_are_rejected() {
        for field in [
            "artifactId",
            "artifactVersion",
            "bundleId",
            "bundleVersion",
            "bundleBuild",
            "payloadApp",
            "signerIdentity",
            "ipa",
            "sha256",
        ] {
            let mut manifest = valid_manifest();
            manifest[field] = json!("   ");
            let fixture = Fixture::new(manifest, true);
            assert!(
                AgentArtifact::load(&fixture.manifest_path).is_err(),
                "{field}"
            );
        }

        for features in [json!([]), json!(["stream", "  "])] {
            let mut manifest = valid_manifest();
            manifest["features"] = features;
            let fixture = Fixture::new(manifest, true);
            assert!(AgentArtifact::load(&fixture.manifest_path).is_err());
        }
    }

    #[test]
    fn invalid_protocol_ports_dimensions_and_checksums_are_rejected() {
        for protocol in [0, SUPPORTED_AGENT_PROTOCOL + 1] {
            let mut manifest = valid_manifest();
            manifest["protocolVersion"] = json!(protocol);
            let fixture = Fixture::new(manifest, true);
            assert!(AgentArtifact::load(&fixture.manifest_path).is_err());
        }

        for field in ["controlPort", "mjpegPort", "logicalWidth", "logicalHeight"] {
            let mut manifest = valid_manifest();
            manifest[field] = json!(0);
            let fixture = Fixture::new(manifest, true);
            assert!(
                AgentArtifact::load(&fixture.manifest_path).is_err(),
                "{field}"
            );
        }

        for checksum in [
            "00",
            "zz76e81375b2f7d536df7a57de0b937a400f88489e74e986e537a66d3f6029b8",
        ] {
            let mut manifest = valid_manifest();
            manifest["sha256"] = json!(checksum);
            let fixture = Fixture::new(manifest, true);
            assert!(AgentArtifact::load(&fixture.manifest_path).is_err());
        }
    }

    #[test]
    fn missing_ipa_is_rejected() {
        let fixture = Fixture::new(valid_manifest(), false);

        let error =
            AgentArtifact::load(&fixture.manifest_path).expect_err("missing IPA must be rejected");

        assert!(error.to_string().contains("artifact"));
    }

    #[test]
    fn lifecycle_missing_install_is_installed_only_when_auto_repair_is_enabled() {
        let artifact = AgentArtifact::load(bundled_manifest_path()).expect("load bundled agent");

        assert_eq!(
            decide_install(&artifact.manifest, None, true),
            AgentInstallDecision::InstallMissing
        );
        assert_eq!(
            decide_install(&artifact.manifest, None, false),
            AgentInstallDecision::ReportRepairRequired
        );
    }

    #[test]
    fn lifecycle_matching_install_is_reused() {
        let artifact = AgentArtifact::load(bundled_manifest_path()).expect("load bundled agent");
        let installed = InstalledAppMetadata {
            bundle_id: artifact.manifest.bundle_id.clone(),
            version: Some(artifact.manifest.bundle_version.clone()),
            build: Some(artifact.manifest.bundle_build.clone()),
            application_type: Some("User".to_string()),
            path: Some(format!(
                "/private/var/containers/Bundle/Application/FIXTURE/{}",
                artifact.manifest.payload_app
            )),
            signer_identity: Some(artifact.manifest.signer_identity.clone()),
        };

        assert_eq!(
            decide_install(&artifact.manifest, Some(&installed), true),
            AgentInstallDecision::Reuse
        );
    }

    #[test]
    fn lifecycle_mismatched_install_is_repaired_or_reported() {
        let artifact = AgentArtifact::load(bundled_manifest_path()).expect("load bundled agent");
        let installed = InstalledAppMetadata {
            bundle_id: artifact.manifest.bundle_id.clone(),
            version: Some("0.9".to_string()),
            build: Some("7".to_string()),
            application_type: Some("User".to_string()),
            path: Some(format!(
                "/private/var/containers/Bundle/Application/FIXTURE/{}",
                artifact.manifest.payload_app
            )),
            signer_identity: Some(artifact.manifest.signer_identity.clone()),
        };

        assert_eq!(
            decide_install(&artifact.manifest, Some(&installed), true),
            AgentInstallDecision::RepairVersionMismatch
        );
        assert_eq!(
            decide_install(&artifact.manifest, Some(&installed), false),
            AgentInstallDecision::ReportRepairRequired
        );
    }

    #[test]
    fn lifecycle_rejects_same_bundle_version_with_wrong_payload_or_signer() {
        let artifact = AgentArtifact::load(bundled_manifest_path()).expect("load bundled agent");
        let mut installed = InstalledAppMetadata {
            bundle_id: artifact.manifest.bundle_id.clone(),
            version: Some(artifact.manifest.bundle_version.clone()),
            build: Some(artifact.manifest.bundle_build.clone()),
            application_type: Some("User".to_string()),
            path: Some(
                "/private/var/containers/Bundle/Application/FIXTURE/csc-native-ios.app".to_string(),
            ),
            signer_identity: Some("iPhone Distribution: Wuhan Fixture".to_string()),
        };

        assert_eq!(
            decide_install(&artifact.manifest, Some(&installed), true),
            AgentInstallDecision::RepairVersionMismatch
        );

        installed.path = Some(format!(
            "/private/var/containers/Bundle/Application/FIXTURE/{}",
            artifact.manifest.payload_app
        ));
        assert_eq!(
            decide_install(&artifact.manifest, Some(&installed), true),
            AgentInstallDecision::RepairVersionMismatch
        );
    }
}
