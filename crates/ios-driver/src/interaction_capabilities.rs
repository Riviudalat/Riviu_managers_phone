use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Context;
use riviu_core::{
    ClipboardAccessMode, ClipboardCapability, DeviceCapabilityQualification,
    DeviceCapabilityRegistry, DeviceQualificationBase, InstalledAgentIdentity,
    InstalledTargetIdentity, OpenUrlCapability, ProtectedRouteContract, QualifiedGeometry,
    RouteMethod, RouteScope, TargetIdentityCapability, UiCapabilities,
};
use serde::{Deserialize, Serialize};

const REGISTRY_SCHEMA_VERSION: u32 = 1;
const DRIVER_ADAPTER_VERSION: &str = "interaction-v1";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RegistryDocument {
    schema_version: u32,
    driver_adapter_version: String,
    qualifications: Vec<RawQualification>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawQualification {
    qualification_id: String,
    environment: String,
    base: RawBase,
    #[serde(default)]
    open_url: Option<RawOpenUrl>,
    #[serde(default)]
    clipboard: Option<RawClipboard>,
    #[serde(default)]
    target_identity_copy_link: Option<RawTargetIdentity>,
    live_report_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawBase {
    agent_artifact_sha256: String,
    agent_bundle_id: String,
    agent_bundle_version: String,
    agent_bundle_build: String,
    agent_executable_name: String,
    agent_signer_identity_sha256: String,
    agent_version: String,
    protocol_version: u32,
    transport: riviu_core::ActiveTransport,
    product_type: String,
    ios_min_inclusive: String,
    ios_max_inclusive: String,
    tiktok_bundle_id: String,
    tiktok_version: String,
    tiktok_build: String,
    geometry: QualifiedGeometry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawOpenUrl {
    contract_id: String,
    method: RouteMethod,
    scope: RouteScope,
    path: String,
    auth_header_name: String,
    body_schema_id: String,
    request_timeout_ms: u32,
    target_bundle_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawClipboard {
    contract_id: String,
    mode: ClipboardAccessMode,
    set_route: RawRoute,
    get_route: RawRoute,
    maximum_decoded_bytes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRoute {
    method: RouteMethod,
    scope: RouteScope,
    path: String,
    auth_header_name: String,
    body_schema_id: String,
    request_timeout_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawTargetIdentity {
    open_url_contract_id: String,
    clipboard_contract_id: String,
    share_detector_version: String,
    copy_link_detector_version: String,
    detector_set_sha256: String,
    layout_id: String,
}

pub fn load_production_registry(path: &Path) -> anyhow::Result<DeviceCapabilityRegistry> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("read interaction capability registry {}", path.display()))?;
    let document: RegistryDocument =
        serde_json::from_slice(&bytes).context("parse interaction capability registry")?;
    if document
        .qualifications
        .iter()
        .any(|entry| entry.environment != "LIVE_MAC_DEVICE")
    {
        anyhow::bail!(
            "production interaction capability registry contains a non-live qualification"
        );
    }
    build_registry(document)
}

#[cfg(test)]
pub(crate) fn parse_registry_bytes(bytes: &[u8]) -> anyhow::Result<DeviceCapabilityRegistry> {
    let document: RegistryDocument =
        serde_json::from_slice(bytes).context("parse interaction capability registry")?;
    build_registry(document)
}

fn build_registry(document: RegistryDocument) -> anyhow::Result<DeviceCapabilityRegistry> {
    if document.schema_version != REGISTRY_SCHEMA_VERSION
        || document.driver_adapter_version != DRIVER_ADAPTER_VERSION
    {
        anyhow::bail!("unsupported interaction capability registry version");
    }

    reject_duplicates_and_overlaps(&document.qualifications)?;
    let qualifications = document
        .qualifications
        .into_iter()
        .map(|entry| convert_entry(entry, &document.driver_adapter_version))
        .collect::<anyhow::Result<Vec<_>>>()?;
    DeviceCapabilityRegistry::try_new(qualifications)
        .map_err(anyhow::Error::new)
        .context("validate interaction capability registry")
}

fn convert_entry(
    entry: RawQualification,
    driver_adapter_version: &str,
) -> anyhow::Result<DeviceCapabilityQualification> {
    if !is_lower_sha256(&entry.live_report_sha256) {
        anyhow::bail!("interaction qualification live report SHA-256 is invalid");
    }
    if entry.clipboard.as_ref().is_some_and(|clipboard| {
        clipboard.contract_id.trim().is_empty()
            || clipboard.contract_id != clipboard.contract_id.trim()
    }) {
        anyhow::bail!("interaction clipboard contract id is blank or non-canonical");
    }
    let base = DeviceQualificationBase {
        installed_agent: InstalledAgentIdentity {
            bundle_id: entry.base.agent_bundle_id.clone(),
            version: entry.base.agent_bundle_version.clone(),
            build: entry.base.agent_bundle_build.clone(),
            executable_name: entry.base.agent_executable_name.clone(),
            signer_identity_sha256: entry.base.agent_signer_identity_sha256.clone(),
        },
        selected_artifact_sha256: entry.base.agent_artifact_sha256.clone(),
        agent_version: entry.base.agent_version.clone(),
        protocol_version: entry.base.protocol_version,
        driver_adapter_version: driver_adapter_version.to_string(),
        transport: entry.base.transport,
        product_type: entry.base.product_type.clone(),
        ios_min_inclusive: entry.base.ios_min_inclusive.clone(),
        ios_max_inclusive: entry.base.ios_max_inclusive.clone(),
        target_app: InstalledTargetIdentity {
            bundle_id: entry.base.tiktok_bundle_id.clone(),
            version: entry.base.tiktok_version.clone(),
            build: entry.base.tiktok_build.clone(),
        },
        geometry: entry.base.geometry.clone(),
    };

    let open_url = entry
        .open_url
        .map(|route| {
            if route.method != RouteMethod::Post
                || route.body_schema_id != "open-url-body-v1"
                || route.request_timeout_ms != 10_000
            {
                anyhow::bail!("unsupported open URL route contract");
            }
            Ok(OpenUrlCapability {
                route: ProtectedRouteContract {
                    contract_id: route.contract_id,
                    method: route.method,
                    scope: route.scope,
                    path: route.path,
                    auth_header_name: route.auth_header_name,
                    body_schema_id: route.body_schema_id,
                    request_timeout_ms: route.request_timeout_ms,
                },
                target_bundle_id: route.target_bundle_id,
                live_report_sha256: entry.live_report_sha256.clone(),
            })
        })
        .transpose()?;

    let clipboard_contract_id = entry
        .clipboard
        .as_ref()
        .map(|clipboard| clipboard.contract_id.clone());
    let clipboard = entry
        .clipboard
        .map(|clipboard| {
            if clipboard.set_route.method != RouteMethod::Post
                || clipboard.get_route.method != RouteMethod::Post
                || clipboard.set_route.body_schema_id != "clipboard-set-base64-v1"
                || clipboard.get_route.body_schema_id != "clipboard-get-base64-v1"
            {
                anyhow::bail!("unsupported clipboard route contract");
            }
            let set_route = convert_route(
                format!("{}:set", clipboard.contract_id),
                clipboard.set_route,
            );
            let get_route = convert_route(
                format!("{}:get", clipboard.contract_id),
                clipboard.get_route,
            );
            Ok(ClipboardCapability {
                mode: clipboard.mode,
                set_route,
                get_route,
                maximum_decoded_bytes: clipboard.maximum_decoded_bytes,
                live_report_sha256: entry.live_report_sha256.clone(),
            })
        })
        .transpose()?;

    let target_identity_copy_link =
        entry
            .target_identity_copy_link
            .map(|identity| TargetIdentityCapability {
                open_url_contract_id: identity.open_url_contract_id,
                clipboard_contract_id: identity.clipboard_contract_id,
                share_detector_version: identity.share_detector_version,
                copy_link_detector_version: identity.copy_link_detector_version,
                detector_set_sha256: identity.detector_set_sha256,
                layout_id: identity.layout_id,
                geometry: base.geometry.clone(),
                live_report_sha256: entry.live_report_sha256.clone(),
            });

    Ok(DeviceCapabilityQualification {
        qualification_id: entry.qualification_id,
        environment: entry.environment,
        base,
        ui: UiCapabilities {
            open_url,
            clipboard,
            target_identity_copy_link,
        },
        clipboard_contract_id,
    })
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn convert_route(contract_id: String, route: RawRoute) -> ProtectedRouteContract {
    ProtectedRouteContract {
        contract_id,
        method: route.method,
        scope: route.scope,
        path: route.path,
        auth_header_name: route.auth_header_name,
        body_schema_id: route.body_schema_id,
        request_timeout_ms: route.request_timeout_ms,
    }
}

fn reject_duplicates_and_overlaps(entries: &[RawQualification]) -> anyhow::Result<()> {
    let mut ids = HashSet::new();
    let mut families: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for entry in entries {
        if !ids.insert(entry.qualification_id.as_str()) {
            anyhow::bail!("duplicate interaction qualification id");
        }
        let mut family = serde_json::to_value(&entry.base)?;
        let object = family
            .as_object_mut()
            .context("interaction qualification base must be an object")?;
        object.remove("iosMinInclusive");
        object.remove("iosMaxInclusive");
        let key = serde_json::to_string(&family)?;
        let ranges = families.entry(key).or_default();
        if ranges.iter().any(|(minimum, maximum)| {
            version_ranges_overlap(
                minimum.as_str(),
                maximum.as_str(),
                &entry.base.ios_min_inclusive,
                &entry.base.ios_max_inclusive,
            )
        }) {
            anyhow::bail!("ambiguous overlapping interaction qualifications");
        }
        ranges.push((
            entry.base.ios_min_inclusive.clone(),
            entry.base.ios_max_inclusive.clone(),
        ));
    }
    Ok(())
}

fn version_ranges_overlap(a_min: &str, a_max: &str, b_min: &str, b_max: &str) -> bool {
    let Some(a_min) = parse_version(a_min) else {
        return true;
    };
    let Some(a_max) = parse_version(a_max) else {
        return true;
    };
    let Some(b_min) = parse_version(b_min) else {
        return true;
    };
    let Some(b_max) = parse_version(b_max) else {
        return true;
    };
    a_min <= b_max && b_min <= a_max
}

fn parse_version(value: &str) -> Option<[u64; 4]> {
    let parts: Vec<_> = value.split('.').collect();
    if parts.is_empty() || parts.len() > 4 || parts.iter().any(|part| part.is_empty()) {
        return None;
    }
    let mut version = [0; 4];
    for (index, part) in parts.into_iter().enumerate() {
        version[index] = part.parse::<u64>().ok()?;
    }
    Some(version)
}

#[cfg(test)]
mod tests {
    use riviu_core::{
        ActiveTransport, DeviceCapabilitySnapshot, InstalledAgentIdentity, InstalledTargetIdentity,
        QualifiedGeometry, ScreenOrientation,
    };
    use serde_json::{json, Value};

    use super::*;

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const SHA_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const SHA_D: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

    fn registry_value() -> Value {
        json!({
            "schemaVersion": 1,
            "driverAdapterVersion": "interaction-v1",
            "qualifications": [{
                "qualificationId": "fixture-g0",
                "environment": "FIXTURE_ONLY",
                "base": {
                    "agentArtifactSha256": SHA_A,
                    "agentBundleId": "com.fixture.agent",
                    "agentBundleVersion": "fixture-bundle-version",
                    "agentBundleBuild": "fixture-bundle-build",
                    "agentExecutableName": "FixtureRunner",
                    "agentSignerIdentitySha256": SHA_B,
                    "agentVersion": "fixture-1",
                    "protocolVersion": 1,
                    "transport": "legacyUsbmuxTransport",
                    "productType": "iPhone10,1",
                    "iosMinInclusive": "16.7.15",
                    "iosMaxInclusive": "16.7.15",
                    "tiktokBundleId": "com.ss.iphone.ugc.Ame",
                    "tiktokVersion": "fixture-version",
                    "tiktokBuild": "fixture-build",
                    "geometry": {
                        "logicalWidth": 375.0,
                        "logicalHeight": 667.0,
                        "pixelWidth": 750,
                        "pixelHeight": 1334,
                        "scaleX": 2.0,
                        "scaleY": 2.0,
                        "orientation": "portrait"
                    }
                },
                "openUrl": {
                    "contractId": "fixture-open-url-v1",
                    "method": "post",
                    "scope": "sessionless",
                    "path": "/fixture/url",
                    "authHeaderName": "X-Fixture-Token",
                    "bodySchemaId": "open-url-body-v1",
                    "requestTimeoutMs": 10000,
                    "targetBundleId": "com.ss.iphone.ugc.Ame"
                },
                "clipboard": {
                    "contractId": "fixture-clipboard-v1",
                    "mode": "targetBackgroundSafe",
                    "setRoute": {
                        "method": "post",
                        "scope": "sessionless",
                        "path": "/fixture/clipboard/set",
                        "authHeaderName": "X-Fixture-Token",
                        "bodySchemaId": "clipboard-set-base64-v1",
                        "requestTimeoutMs": 10000
                    },
                    "getRoute": {
                        "method": "post",
                        "scope": "sessionless",
                        "path": "/fixture/clipboard/get",
                        "authHeaderName": "X-Fixture-Token",
                        "bodySchemaId": "clipboard-get-base64-v1",
                        "requestTimeoutMs": 10000
                    },
                    "maximumDecodedBytes": 65536
                },
                "targetIdentityCopyLink": {
                    "openUrlContractId": "fixture-open-url-v1",
                    "clipboardContractId": "fixture-clipboard-v1",
                    "shareDetectorVersion": "share-v1",
                    "copyLinkDetectorVersion": "copy-link-v1",
                    "detectorSetSha256": SHA_C,
                    "layoutId": "iphone8-portrait-v1"
                },
                "liveReportSha256": SHA_D
            }]
        })
    }

    fn snapshot() -> DeviceCapabilitySnapshot {
        DeviceCapabilitySnapshot {
            installed_agent: InstalledAgentIdentity {
                bundle_id: "com.fixture.agent".into(),
                version: "fixture-bundle-version".into(),
                build: "fixture-bundle-build".into(),
                executable_name: "FixtureRunner".into(),
                signer_identity_sha256: SHA_B.into(),
            },
            selected_artifact_sha256: SHA_A.into(),
            agent_version: "fixture-1".into(),
            protocol_version: 1,
            driver_adapter_version: "interaction-v1".into(),
            transport: ActiveTransport::LegacyUsbmuxTransport,
            product_type: "iPhone10,1".into(),
            ios_version: "16.7.15".into(),
            target_app: InstalledTargetIdentity {
                bundle_id: "com.ss.iphone.ugc.Ame".into(),
                version: "fixture-version".into(),
                build: "fixture-build".into(),
            },
            protected_auth_ready: true,
            geometry: Some(QualifiedGeometry {
                logical_width: 375.0,
                logical_height: 667.0,
                pixel_width: 750,
                pixel_height: 1334,
                scale_x: 2.0,
                scale_y: 2.0,
                orientation: ScreenOrientation::Portrait,
            }),
        }
    }

    #[test]
    fn interaction_capability_registry_loads_empty_production_and_exact_fixture() {
        let empty = parse_registry_bytes(
            br#"{"schemaVersion":1,"driverAdapterVersion":"interaction-v1","qualifications":[]}"#,
        )
        .expect("empty registry");
        assert!(empty.qualifications().is_empty());

        let fixture = parse_registry_bytes(&serde_json::to_vec(&registry_value()).unwrap())
            .expect("fixture registry");
        let negotiated = fixture.negotiate(&snapshot());
        assert!(negotiated.ui.open_url.is_some());
        assert!(negotiated.ui.clipboard.is_some());
        assert!(negotiated.ui.target_identity_copy_link.is_some());
    }

    #[test]
    fn interaction_capability_registry_rejects_unknown_secret_and_ambiguous_entries() {
        let mut unknown = registry_value();
        unknown["extra"] = json!(true);
        assert!(parse_registry_bytes(&serde_json::to_vec(&unknown).unwrap()).is_err());

        let mut secret = registry_value();
        secret["qualifications"][0]["openUrl"]["token"] = json!("fixture-secret");
        assert!(parse_registry_bytes(&serde_json::to_vec(&secret).unwrap()).is_err());

        let mut malformed = registry_value();
        malformed["qualifications"][0]["base"]["agentArtifactSha256"] = json!("ABC");
        assert!(parse_registry_bytes(&serde_json::to_vec(&malformed).unwrap()).is_err());

        let mut unknown_mode = registry_value();
        unknown_mode["qualifications"][0]["clipboard"]["mode"] = json!("unknown");
        assert!(parse_registry_bytes(&serde_json::to_vec(&unknown_mode).unwrap()).is_err());

        let mut duplicate = registry_value();
        let second = duplicate["qualifications"][0].clone();
        duplicate["qualifications"]
            .as_array_mut()
            .unwrap()
            .push(second);
        assert!(parse_registry_bytes(&serde_json::to_vec(&duplicate).unwrap()).is_err());

        let mut overlap = registry_value();
        let mut second = overlap["qualifications"][0].clone();
        second["qualificationId"] = json!("fixture-overlap");
        overlap["qualifications"]
            .as_array_mut()
            .unwrap()
            .push(second);
        assert!(parse_registry_bytes(&serde_json::to_vec(&overlap).unwrap()).is_err());
    }

    #[test]
    fn interaction_capability_registry_rejects_invalid_contract_dimensions() {
        for mutation in [
            ("/qualifications/0/base/transport", json!("unknown")),
            (
                "/qualifications/0/base/geometry/orientation",
                json!("unknown"),
            ),
            ("/qualifications/0/base/geometry/pixelWidth", json!(0)),
            ("/qualifications/0/liveReportSha256", json!(null)),
            ("/qualifications/0/openUrl/requestTimeoutMs", json!(9999)),
            (
                "/qualifications/0/clipboard/maximumDecodedBytes",
                json!(65537),
            ),
            (
                "/qualifications/0/targetIdentityCopyLink/openUrlContractId",
                json!("different-contract"),
            ),
        ] {
            let mut value = registry_value();
            *value.pointer_mut(mutation.0).expect("fixture path") = mutation.1;
            assert!(
                parse_registry_bytes(&serde_json::to_vec(&value).unwrap()).is_err(),
                "accepted invalid field {}",
                mutation.0
            );
        }

        let mut report_hash_without_ui = registry_value();
        report_hash_without_ui["qualifications"][0]
            .as_object_mut()
            .expect("qualification object")
            .retain(|key, _| {
                !matches!(
                    key.as_str(),
                    "openUrl" | "clipboard" | "targetIdentityCopyLink"
                )
            });
        report_hash_without_ui["qualifications"][0]["liveReportSha256"] = json!("not-a-hash");
        assert!(
            parse_registry_bytes(&serde_json::to_vec(&report_hash_without_ui).unwrap()).is_err(),
            "root evidence hash must be validated even without UI capabilities"
        );

        let mut blank_clipboard_contract = registry_value();
        blank_clipboard_contract["qualifications"][0]
            .as_object_mut()
            .expect("qualification object")
            .remove("targetIdentityCopyLink");
        blank_clipboard_contract["qualifications"][0]["clipboard"]["contractId"] = json!("");
        assert!(
            parse_registry_bytes(&serde_json::to_vec(&blank_clipboard_contract).unwrap()).is_err(),
            "clipboard contract id must remain non-empty without identity capability"
        );
    }

    #[test]
    fn interaction_capability_registry_normalizes_ios_versions_for_overlap_checks() {
        let mut overlap = registry_value();
        overlap["qualifications"][0]["base"]["iosMaxInclusive"] = json!("16.7");
        let mut second = overlap["qualifications"][0].clone();
        second["qualificationId"] = json!("fixture-overlap-normalized");
        second["base"]["iosMinInclusive"] = json!("16.7.0");
        second["base"]["iosMaxInclusive"] = json!("16.8");
        overlap["qualifications"]
            .as_array_mut()
            .unwrap()
            .push(second);

        assert!(parse_registry_bytes(&serde_json::to_vec(&overlap).unwrap()).is_err());
    }

    #[test]
    fn production_registry_is_empty_and_rejects_fixture_qualifications() {
        let production = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../sidecars/wda/interaction-capabilities.json");
        let registry = load_production_registry(&production).expect("production registry");
        assert!(registry.qualifications().is_empty());

        let mut fixture = registry_value();
        fixture["qualifications"][0]["environment"] = json!("PENDING_MAC_DEVICE");
        let directory = std::env::temp_dir().join(format!(
            "riviu-interaction-capabilities-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("registry.json");
        std::fs::write(&path, serde_json::to_vec(&fixture).unwrap()).unwrap();
        assert!(load_production_registry(&path).is_err());
        let _ = std::fs::remove_dir_all(directory);
    }
}
