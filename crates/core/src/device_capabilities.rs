use std::cmp::Ordering;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use crate::stream_budget::StreamStopProof;

pub const MAX_INTERACTION_CLIPBOARD_BYTES: usize = 64 * 1024;
pub const OPEN_URL_TIMEOUT_MS: u32 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActiveTransport {
    LegacyUsbmuxTransport,
    RsdTransport,
    Mock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScreenOrientation {
    Portrait,
    PortraitUpsideDown,
    LandscapeLeft,
    LandscapeRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClipboardAccessMode {
    TargetBackgroundSafe,
    AgentForegroundRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RouteScope {
    Sessionless,
    Session,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RouteMethod {
    Get,
    Post,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtectedRouteContract {
    pub contract_id: String,
    pub method: RouteMethod,
    pub scope: RouteScope,
    pub path: String,
    pub auth_header_name: String,
    pub body_schema_id: String,
    pub request_timeout_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualifiedGeometry {
    pub logical_width: f64,
    pub logical_height: f64,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub scale_x: f64,
    pub scale_y: f64,
    pub orientation: ScreenOrientation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstalledAgentIdentity {
    pub bundle_id: String,
    pub version: String,
    pub build: String,
    pub executable_name: String,
    pub signer_identity_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstalledTargetIdentity {
    pub bundle_id: String,
    pub version: String,
    pub build: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceCapabilitySnapshot {
    pub installed_agent: InstalledAgentIdentity,
    pub selected_artifact_sha256: String,
    pub agent_version: String,
    pub protocol_version: u32,
    pub driver_adapter_version: String,
    pub transport: ActiveTransport,
    pub product_type: String,
    pub ios_version: String,
    pub target_app: InstalledTargetIdentity,
    pub protected_auth_ready: bool,
    pub geometry: Option<QualifiedGeometry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceControllerCapabilities {
    pub snapshot: DeviceCapabilitySnapshot,
    pub ui: UiCapabilities,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiCapabilities {
    pub open_url: Option<OpenUrlCapability>,
    pub clipboard: Option<ClipboardCapability>,
    pub target_identity_copy_link: Option<TargetIdentityCapability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenUrlCapability {
    pub route: ProtectedRouteContract,
    pub target_bundle_id: String,
    pub live_report_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClipboardCapability {
    pub mode: ClipboardAccessMode,
    pub set_route: ProtectedRouteContract,
    pub get_route: ProtectedRouteContract,
    pub maximum_decoded_bytes: u32,
    pub live_report_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TargetIdentityCapability {
    pub open_url_contract_id: String,
    pub clipboard_contract_id: String,
    pub share_detector_version: String,
    pub copy_link_detector_version: String,
    pub detector_set_sha256: String,
    pub layout_id: String,
    pub geometry: QualifiedGeometry,
    pub live_report_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceQualificationBase {
    pub installed_agent: InstalledAgentIdentity,
    pub selected_artifact_sha256: String,
    pub agent_version: String,
    pub protocol_version: u32,
    pub driver_adapter_version: String,
    pub transport: ActiveTransport,
    pub product_type: String,
    pub ios_min_inclusive: String,
    pub ios_max_inclusive: String,
    pub target_app: InstalledTargetIdentity,
    pub geometry: QualifiedGeometry,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceCapabilityQualification {
    pub qualification_id: String,
    pub environment: String,
    pub base: DeviceQualificationBase,
    pub ui: UiCapabilities,
    pub clipboard_contract_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct DeviceCapabilityRegistry {
    qualifications: Vec<DeviceCapabilityQualification>,
}

impl DeviceCapabilityRegistry {
    pub fn try_new(
        qualifications: Vec<DeviceCapabilityQualification>,
    ) -> Result<Self, CapabilityValidationError> {
        for qualification in &qualifications {
            qualification.validate()?;
        }
        Ok(Self { qualifications })
    }

    pub fn empty() -> Self {
        Self::default()
    }

    pub fn qualifications(&self) -> &[DeviceCapabilityQualification] {
        &self.qualifications
    }

    pub fn negotiate(&self, snapshot: &DeviceCapabilitySnapshot) -> DeviceControllerCapabilities {
        let mut matches = self
            .qualifications
            .iter()
            .filter(|qualification| qualification.matches(snapshot));
        let first = matches.next();
        let ui = match (first, matches.next()) {
            (Some(qualification), None) => qualification.ui.clone(),
            _ => UiCapabilities::default(),
        };
        DeviceControllerCapabilities {
            snapshot: snapshot.clone(),
            ui,
        }
    }
}

impl DeviceCapabilityQualification {
    pub fn validate(&self) -> Result<(), CapabilityValidationError> {
        require_nonempty("qualificationId", &self.qualification_id)?;
        require_nonempty("environment", &self.environment)?;
        self.base.validate()?;

        if let Some(open_url) = &self.ui.open_url {
            open_url.route.validate("openUrl.route")?;
            if open_url.route.request_timeout_ms != OPEN_URL_TIMEOUT_MS {
                return Err(invalid("openUrl.route.requestTimeoutMs"));
            }
            require_nonempty("openUrl.targetBundleId", &open_url.target_bundle_id)?;
            require_sha256("openUrl.liveReportSha256", &open_url.live_report_sha256)?;
            if open_url.target_bundle_id != self.base.target_app.bundle_id {
                return Err(invalid("openUrl.targetBundleId"));
            }
        }

        if let Some(clipboard) = &self.ui.clipboard {
            clipboard.set_route.validate("clipboard.setRoute")?;
            clipboard.get_route.validate("clipboard.getRoute")?;
            if clipboard.maximum_decoded_bytes as usize != MAX_INTERACTION_CLIPBOARD_BYTES {
                return Err(invalid("clipboard.maximumDecodedBytes"));
            }
            require_sha256("clipboard.liveReportSha256", &clipboard.live_report_sha256)?;
        }

        if let Some(identity) = &self.ui.target_identity_copy_link {
            let open_url = self
                .ui
                .open_url
                .as_ref()
                .ok_or_else(|| invalid("targetIdentityCopyLink.openUrlContractId"))?;
            let clipboard = self
                .ui
                .clipboard
                .as_ref()
                .ok_or_else(|| invalid("targetIdentityCopyLink.clipboardContractId"))?;
            let clipboard_contract_id = self
                .clipboard_contract_id
                .as_deref()
                .ok_or_else(|| invalid("clipboardContractId"))?;
            if identity.open_url_contract_id != open_url.route.contract_id
                || identity.clipboard_contract_id != clipboard_contract_id
            {
                return Err(invalid("targetIdentityCopyLink.contractIds"));
            }
            require_nonempty(
                "targetIdentityCopyLink.shareDetectorVersion",
                &identity.share_detector_version,
            )?;
            require_nonempty(
                "targetIdentityCopyLink.copyLinkDetectorVersion",
                &identity.copy_link_detector_version,
            )?;
            require_nonempty("targetIdentityCopyLink.layoutId", &identity.layout_id)?;
            require_sha256(
                "targetIdentityCopyLink.detectorSetSha256",
                &identity.detector_set_sha256,
            )?;
            require_sha256(
                "targetIdentityCopyLink.liveReportSha256",
                &identity.live_report_sha256,
            )?;
            identity.geometry.validate()?;
            if identity.geometry != self.base.geometry
                || identity.live_report_sha256 != open_url.live_report_sha256
                || identity.live_report_sha256 != clipboard.live_report_sha256
            {
                return Err(invalid("targetIdentityCopyLink.evidenceBinding"));
            }
        }
        Ok(())
    }

    fn matches(&self, snapshot: &DeviceCapabilitySnapshot) -> bool {
        if !snapshot.protected_auth_ready {
            return false;
        }
        let Some(geometry) = snapshot.geometry.as_ref() else {
            return false;
        };
        self.base.installed_agent == snapshot.installed_agent
            && self.base.selected_artifact_sha256 == snapshot.selected_artifact_sha256
            && self.base.agent_version == snapshot.agent_version
            && self.base.protocol_version == snapshot.protocol_version
            && self.base.driver_adapter_version == snapshot.driver_adapter_version
            && self.base.transport == snapshot.transport
            && self.base.product_type == snapshot.product_type
            && self.base.target_app == snapshot.target_app
            && self.base.geometry == *geometry
            && version_in_range(
                &snapshot.ios_version,
                &self.base.ios_min_inclusive,
                &self.base.ios_max_inclusive,
            )
    }
}

impl DeviceQualificationBase {
    fn validate(&self) -> Result<(), CapabilityValidationError> {
        self.installed_agent.validate()?;
        self.target_app.validate()?;
        require_sha256(
            "base.selectedArtifactSha256",
            &self.selected_artifact_sha256,
        )?;
        for (field, value) in [
            ("base.agentVersion", self.agent_version.as_str()),
            (
                "base.driverAdapterVersion",
                self.driver_adapter_version.as_str(),
            ),
            ("base.productType", self.product_type.as_str()),
            ("base.iosMinInclusive", self.ios_min_inclusive.as_str()),
            ("base.iosMaxInclusive", self.ios_max_inclusive.as_str()),
        ] {
            require_nonempty(field, value)?;
        }
        if self.protocol_version == 0
            || parse_version(&self.ios_min_inclusive).is_none()
            || parse_version(&self.ios_max_inclusive).is_none()
            || compare_versions(&self.ios_min_inclusive, &self.ios_max_inclusive)
                == Some(Ordering::Greater)
        {
            return Err(invalid("base.versionRange"));
        }
        self.geometry.validate()
    }
}

impl InstalledAgentIdentity {
    fn validate(&self) -> Result<(), CapabilityValidationError> {
        for (field, value) in [
            ("installedAgent.bundleId", self.bundle_id.as_str()),
            ("installedAgent.version", self.version.as_str()),
            ("installedAgent.build", self.build.as_str()),
            (
                "installedAgent.executableName",
                self.executable_name.as_str(),
            ),
        ] {
            require_nonempty(field, value)?;
        }
        require_sha256(
            "installedAgent.signerIdentitySha256",
            &self.signer_identity_sha256,
        )
    }
}

impl InstalledTargetIdentity {
    fn validate(&self) -> Result<(), CapabilityValidationError> {
        for (field, value) in [
            ("targetApp.bundleId", self.bundle_id.as_str()),
            ("targetApp.version", self.version.as_str()),
            ("targetApp.build", self.build.as_str()),
        ] {
            require_nonempty(field, value)?;
        }
        Ok(())
    }
}

impl ProtectedRouteContract {
    fn validate(&self, prefix: &str) -> Result<(), CapabilityValidationError> {
        for (suffix, value) in [
            ("contractId", self.contract_id.as_str()),
            ("path", self.path.as_str()),
            ("authHeaderName", self.auth_header_name.as_str()),
            ("bodySchemaId", self.body_schema_id.as_str()),
        ] {
            require_nonempty(&format!("{prefix}.{suffix}"), value)?;
        }
        if !self.path.starts_with('/')
            || self.auth_header_name.len() > 64
            || !self.auth_header_name.starts_with("X-")
            || !self.auth_header_name.ends_with("-Token")
            || !self
                .auth_header_name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || self.request_timeout_ms == 0
            || self.request_timeout_ms > OPEN_URL_TIMEOUT_MS
        {
            return Err(invalid(prefix));
        }
        Ok(())
    }
}

impl QualifiedGeometry {
    fn validate(&self) -> Result<(), CapabilityValidationError> {
        if self.logical_width != 375.0
            || self.logical_height != 667.0
            || self.pixel_width == 0
            || self.pixel_height == 0
            || !self.scale_x.is_finite()
            || !self.scale_y.is_finite()
            || self.scale_x <= 0.0
            || self.scale_y <= 0.0
            || self.orientation != ScreenOrientation::Portrait
            || ((self.logical_width * self.scale_x) - self.pixel_width as f64).abs() > 0.01
            || ((self.logical_height * self.scale_y) - self.pixel_height as f64).abs() > 0.01
        {
            return Err(invalid("geometry"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentInstallProof {
    pub installed: InstalledAgentIdentity,
    pub artifact_sha256: String,
    pub protected_auth_ready: bool,
    pub session_created: bool,
    pub stream_started: bool,
}

impl AgentInstallProof {
    pub fn validate_install_only(&self) -> Result<(), CapabilityValidationError> {
        self.installed.validate()?;
        require_sha256("artifactSha256", &self.artifact_sha256)?;
        if !self.protected_auth_ready || self.session_created || self.stream_started {
            return Err(invalid("installOnlyLifecycle"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid interaction capability field {field}")]
pub struct CapabilityValidationError {
    pub field: String,
}

pub fn validate_clipboard_read_limit(maximum_decoded_bytes: usize) -> anyhow::Result<()> {
    if maximum_decoded_bytes > MAX_INTERACTION_CLIPBOARD_BYTES {
        anyhow::bail!("clipboard read limit exceeds {MAX_INTERACTION_CLIPBOARD_BYTES} bytes");
    }
    Ok(())
}

fn invalid(field: impl Into<String>) -> CapabilityValidationError {
    CapabilityValidationError {
        field: field.into(),
    }
}

fn require_nonempty(field: &str, value: &str) -> Result<(), CapabilityValidationError> {
    if value.trim().is_empty() || value != value.trim() {
        return Err(invalid(field));
    }
    Ok(())
}

fn require_sha256(field: &str, value: &str) -> Result<(), CapabilityValidationError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(field));
    }
    Ok(())
}

fn parse_version(value: &str) -> Option<Vec<u64>> {
    let parts: Vec<_> = value.split('.').collect();
    if parts.is_empty() || parts.len() > 4 || parts.iter().any(|part| part.is_empty()) {
        return None;
    }
    parts
        .into_iter()
        .map(|part| part.parse::<u64>().ok())
        .collect()
}

fn compare_versions(left: &str, right: &str) -> Option<Ordering> {
    let mut left = parse_version(left)?;
    let mut right = parse_version(right)?;
    let length = left.len().max(right.len());
    left.resize(length, 0);
    right.resize(length, 0);
    Some(left.cmp(&right))
}

fn version_in_range(value: &str, minimum: &str, maximum: &str) -> bool {
    matches!(
        (
            compare_versions(value, minimum),
            compare_versions(value, maximum)
        ),
        (
            Some(Ordering::Equal | Ordering::Greater),
            Some(Ordering::Equal | Ordering::Less)
        )
    )
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use async_trait::async_trait;
    use serde_json::json;

    use super::*;
    use crate::{
        DeviceDriver, DeviceInfo, InteractionSessionKind, StreamStartProof, SwipeGesture, TapPoint,
        UiSession, UnsupportedCapability,
    };

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const SHA_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const SHA_D: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

    fn route(contract_id: &str, path: &str, schema: &str) -> ProtectedRouteContract {
        ProtectedRouteContract {
            contract_id: contract_id.into(),
            method: RouteMethod::Post,
            scope: RouteScope::Sessionless,
            path: path.into(),
            auth_header_name: "X-Riviu-Token".into(),
            body_schema_id: schema.into(),
            request_timeout_ms: 10_000,
        }
    }

    fn geometry() -> QualifiedGeometry {
        QualifiedGeometry {
            logical_width: 375.0,
            logical_height: 667.0,
            pixel_width: 750,
            pixel_height: 1334,
            scale_x: 2.0,
            scale_y: 2.0,
            orientation: ScreenOrientation::Portrait,
        }
    }

    fn fixture_snapshot() -> DeviceCapabilitySnapshot {
        DeviceCapabilitySnapshot {
            installed_agent: InstalledAgentIdentity {
                bundle_id: "com.fixture.agent".into(),
                version: "1.0".into(),
                build: "10".into(),
                executable_name: "FixtureRunner".into(),
                signer_identity_sha256: SHA_B.into(),
            },
            selected_artifact_sha256: SHA_A.into(),
            agent_version: "fixture-agent-1".into(),
            protocol_version: 2,
            driver_adapter_version: "interaction-v1".into(),
            transport: ActiveTransport::LegacyUsbmuxTransport,
            product_type: "iPhone10,1".into(),
            ios_version: "16.7.15".into(),
            target_app: InstalledTargetIdentity {
                bundle_id: "com.ss.iphone.ugc.Ame".into(),
                version: "35.0.0".into(),
                build: "350001".into(),
            },
            protected_auth_ready: true,
            geometry: Some(geometry()),
        }
    }

    fn fixture_qualification() -> DeviceCapabilityQualification {
        DeviceCapabilityQualification {
            qualification_id: "fixture-g0".into(),
            environment: "FIXTURE_ONLY".into(),
            base: DeviceQualificationBase {
                installed_agent: fixture_snapshot().installed_agent,
                selected_artifact_sha256: SHA_A.into(),
                agent_version: "fixture-agent-1".into(),
                protocol_version: 2,
                driver_adapter_version: "interaction-v1".into(),
                transport: ActiveTransport::LegacyUsbmuxTransport,
                product_type: "iPhone10,1".into(),
                ios_min_inclusive: "16.7.15".into(),
                ios_max_inclusive: "16.7.15".into(),
                target_app: fixture_snapshot().target_app,
                geometry: geometry(),
            },
            ui: UiCapabilities {
                open_url: Some(OpenUrlCapability {
                    route: route("fixture-open-url-v1", "/fixture/url", "open-url-body-v1"),
                    target_bundle_id: "com.ss.iphone.ugc.Ame".into(),
                    live_report_sha256: SHA_D.into(),
                }),
                clipboard: Some(ClipboardCapability {
                    mode: ClipboardAccessMode::TargetBackgroundSafe,
                    set_route: route(
                        "fixture-clipboard-set-v1",
                        "/fixture/clipboard/set",
                        "clipboard-set-base64-v1",
                    ),
                    get_route: route(
                        "fixture-clipboard-get-v1",
                        "/fixture/clipboard/get",
                        "clipboard-get-base64-v1",
                    ),
                    maximum_decoded_bytes: 65_536,
                    live_report_sha256: SHA_D.into(),
                }),
                target_identity_copy_link: Some(TargetIdentityCapability {
                    open_url_contract_id: "fixture-open-url-v1".into(),
                    clipboard_contract_id: "fixture-clipboard-v1".into(),
                    share_detector_version: "share-v1".into(),
                    copy_link_detector_version: "copy-link-v1".into(),
                    detector_set_sha256: SHA_C.into(),
                    layout_id: "iphone8-portrait-v1".into(),
                    geometry: geometry(),
                    live_report_sha256: SHA_D.into(),
                }),
            },
            clipboard_contract_id: Some("fixture-clipboard-v1".into()),
        }
    }

    fn fixture_registry() -> DeviceCapabilityRegistry {
        DeviceCapabilityRegistry::try_new(vec![fixture_qualification()]).unwrap()
    }

    fn assert_denied(snapshot: DeviceCapabilitySnapshot) {
        let negotiated = fixture_registry().negotiate(&snapshot);
        assert_eq!(negotiated.snapshot, snapshot);
        assert_eq!(negotiated.ui, UiCapabilities::default());
    }

    #[test]
    fn capability_requires_every_runtime_dimension_to_match() {
        let registry = fixture_registry();
        let actual = fixture_snapshot();
        assert!(registry.negotiate(&actual).ui.open_url.is_some());
        assert!(registry.negotiate(&actual).ui.clipboard.is_some());
        assert!(registry
            .negotiate(&actual)
            .ui
            .target_identity_copy_link
            .is_some());

        let mutations: Vec<Box<dyn Fn(&mut DeviceCapabilitySnapshot)>> = vec![
            Box::new(|v| v.selected_artifact_sha256 = SHA_D.into()),
            Box::new(|v| v.installed_agent.bundle_id.push_str(".changed")),
            Box::new(|v| v.installed_agent.version.push_str(".changed")),
            Box::new(|v| v.installed_agent.build.push('1')),
            Box::new(|v| v.installed_agent.executable_name.push_str("Changed")),
            Box::new(|v| v.installed_agent.signer_identity_sha256 = SHA_D.into()),
            Box::new(|v| v.agent_version.push_str(".changed")),
            Box::new(|v| v.protocol_version += 1),
            Box::new(|v| v.driver_adapter_version.push_str(".changed")),
            Box::new(|v| v.transport = ActiveTransport::RsdTransport),
            Box::new(|v| v.product_type = "iPhone99,1".into()),
            Box::new(|v| v.ios_version = "16.7.14".into()),
            Box::new(|v| v.target_app.bundle_id.push_str(".changed")),
            Box::new(|v| v.target_app.version.push_str(".changed")),
            Box::new(|v| v.target_app.build.push('1')),
            Box::new(|v| v.protected_auth_ready = false),
            Box::new(|v| v.geometry = None),
            Box::new(|v| v.geometry.as_mut().unwrap().logical_width = 376.0),
            Box::new(|v| v.geometry.as_mut().unwrap().logical_height = 668.0),
            Box::new(|v| v.geometry.as_mut().unwrap().pixel_width = 751),
            Box::new(|v| v.geometry.as_mut().unwrap().pixel_height = 1335),
            Box::new(|v| v.geometry.as_mut().unwrap().scale_x = 3.0),
            Box::new(|v| v.geometry.as_mut().unwrap().scale_y = 3.0),
            Box::new(|v| {
                v.geometry.as_mut().unwrap().orientation = ScreenOrientation::LandscapeLeft
            }),
        ];

        for mutate in mutations {
            let mut changed = actual.clone();
            mutate(&mut changed);
            assert_denied(changed);
        }
    }

    #[test]
    fn ambiguous_matching_qualifications_fail_closed() {
        let entry = fixture_qualification();
        let mut duplicate = entry.clone();
        duplicate.qualification_id = "fixture-g0-duplicate".into();
        let registry = DeviceCapabilityRegistry::try_new(vec![entry, duplicate]).unwrap();
        assert_eq!(
            registry.negotiate(&fixture_snapshot()).ui,
            UiCapabilities::default()
        );
    }

    #[test]
    fn static_contract_drift_is_rejected_before_negotiation() {
        let mut cases: Vec<DeviceCapabilityQualification> = Vec::new();

        let mut bad = fixture_qualification();
        bad.base.selected_artifact_sha256 = "ABC".into();
        cases.push(bad);

        let mut bad = fixture_qualification();
        bad.ui.open_url.as_mut().unwrap().route.request_timeout_ms = 9_999;
        cases.push(bad);

        let mut bad = fixture_qualification();
        bad.ui.open_url.as_mut().unwrap().route.auth_header_name = "Bearer fixture-secret".into();
        cases.push(bad);

        let mut bad = fixture_qualification();
        bad.ui.clipboard.as_mut().unwrap().maximum_decoded_bytes = 65_535;
        cases.push(bad);

        let mut bad = fixture_qualification();
        bad.ui
            .target_identity_copy_link
            .as_mut()
            .unwrap()
            .open_url_contract_id = "wrong".into();
        cases.push(bad);

        let mut bad = fixture_qualification();
        bad.base.geometry.logical_width = 390.0;
        cases.push(bad);

        let mut bad = fixture_qualification();
        bad.base.geometry.scale_x = f64::NAN;
        cases.push(bad);

        let mut bad = fixture_qualification();
        bad.base.geometry.orientation = ScreenOrientation::LandscapeRight;
        cases.push(bad);

        let mut bad = fixture_qualification();
        bad.ui
            .target_identity_copy_link
            .as_mut()
            .unwrap()
            .detector_set_sha256 = SHA_A.to_uppercase();
        cases.push(bad);

        for entry in cases {
            assert!(DeviceCapabilityRegistry::try_new(vec![entry]).is_err());
        }
    }

    #[test]
    fn route_and_capability_types_serialize_without_secret_values() {
        assert_eq!(
            serde_json::to_value(ActiveTransport::LegacyUsbmuxTransport).unwrap(),
            json!("legacyUsbmuxTransport")
        );
        assert_eq!(
            serde_json::to_value(ClipboardAccessMode::AgentForegroundRequired).unwrap(),
            json!("agentForegroundRequired")
        );
        let value = serde_json::to_value(route(
            "fixture-open-url-v1",
            "/fixture/url",
            "open-url-body-v1",
        ))
        .unwrap();
        assert_eq!(value["requestTimeoutMs"], json!(10_000));
        assert_eq!(value["authHeaderName"], json!("X-Riviu-Token"));
        assert!(value.get("token").is_none());
    }

    #[test]
    fn install_only_proof_rejects_session_or_stream_side_effects() {
        let mut proof = AgentInstallProof {
            installed: fixture_snapshot().installed_agent,
            artifact_sha256: SHA_A.into(),
            protected_auth_ready: true,
            session_created: false,
            stream_started: false,
        };
        assert!(proof.validate_install_only().is_ok());
        proof.session_created = true;
        assert!(proof.validate_install_only().is_err());
        proof.session_created = false;
        proof.stream_started = true;
        assert!(proof.validate_install_only().is_err());
        proof.stream_started = false;
        proof.protected_auth_ready = false;
        assert!(proof.validate_install_only().is_err());
    }

    struct UnsupportedDriver;

    #[async_trait]
    impl DeviceDriver for UnsupportedDriver {
        async fn list_devices(&self) -> anyhow::Result<Vec<DeviceInfo>> {
            Ok(Vec::new())
        }
        async fn refresh_device(&self, _udid: &str) -> anyhow::Result<DeviceInfo> {
            anyhow::bail!("unused")
        }
        async fn install_app(&self, _udid: &str, _path: &Path) -> anyhow::Result<()> {
            Ok(())
        }
        async fn uninstall_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn screenshot(&self, _udid: &str, _dest: &Path) -> anyhow::Result<PathBuf> {
            anyhow::bail!("unused")
        }
        async fn syslog_tail(&self, _udid: &str, _lines: usize) -> anyhow::Result<String> {
            Ok(String::new())
        }
        async fn launch_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn terminate_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn reboot(&self, _udid: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn start_ui_session(&self, _udid: &str) -> anyhow::Result<Box<dyn UiSession>> {
            Ok(Box::new(UnsupportedSession))
        }
        async fn ensure_stream(&self, _udid: &str) -> anyhow::Result<String> {
            Ok(String::new())
        }
        async fn prepare_device(&self, _udid: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    struct UnsupportedSession;

    #[async_trait]
    impl UiSession for UnsupportedSession {
        async fn tap(&self, _point: TapPoint) -> anyhow::Result<()> {
            Ok(())
        }
        async fn swipe(&self, _gesture: SwipeGesture) -> anyhow::Result<()> {
            Ok(())
        }
        async fn type_text(&self, _text: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn home(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn find_and_tap(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn assert_visible(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
    }

    fn assert_unsupported(error: anyhow::Error, capability: &'static str) {
        assert_eq!(
            error
                .downcast_ref::<UnsupportedCapability>()
                .expect("typed unsupported capability")
                .capability,
            capability
        );
    }

    #[tokio::test]
    async fn new_driver_and_session_methods_default_to_typed_unsupported() {
        let driver = UnsupportedDriver;
        assert_unsupported(
            driver
                .inspect_interaction_device("fixture")
                .await
                .unwrap_err(),
            "inspectInteractionDevice",
        );
        assert_unsupported(
            driver
                .repair_agent_install_only("fixture")
                .await
                .unwrap_err(),
            "repairAgentInstallOnly",
        );
        assert_unsupported(
            driver.stop_owned_stream("fixture").await.unwrap_err(),
            "stopOwnedStream",
        );
        assert_unsupported(
            driver
                .start_stream_after_session("fixture")
                .await
                .unwrap_err(),
            "startStreamAfterSession",
        );
        let error = match driver
            .start_interaction_session(
                "fixture",
                "com.ss.iphone.ugc.Ame",
                InteractionSessionKind::Ordinary,
            )
            .await
        {
            Ok(_) => panic!("unsupported driver unexpectedly created a session"),
            Err(error) => error,
        };
        assert_unsupported(error, "startInteractionSession");

        let session = UnsupportedSession;
        assert_unsupported(
            session
                .open_url("https://example.invalid")
                .await
                .unwrap_err(),
            "openUrl",
        );
        assert_unsupported(
            session
                .set_clipboard("text/plain", b"fixture")
                .await
                .unwrap_err(),
            "setClipboard",
        );
        assert_unsupported(
            session.get_clipboard(65_536).await.unwrap_err(),
            "getClipboard",
        );
        assert_unsupported(
            session.active_app_identity().await.unwrap_err(),
            "activeAppIdentity",
        );
        assert!(validate_clipboard_read_limit(65_536).is_ok());
        assert!(validate_clipboard_read_limit(65_537).is_err());
        let _: Option<StreamStartProof> = None;
    }
}
