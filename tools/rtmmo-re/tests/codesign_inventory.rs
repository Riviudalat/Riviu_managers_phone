use std::path::Path;

use plist::{Dictionary, Value};
use rtmmo_re::{archive, codesign, macho};

fn push_be_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn entitlement_dictionary() -> Dictionary {
    let mut dictionary = Dictionary::new();
    dictionary.insert("get-task-allow".into(), Value::Boolean(false));
    dictionary.insert(
        "application-identifier".into(),
        Value::String("TEAM.com.riviu.fixture".into()),
    );
    dictionary
}

fn xml_plist(value: &Value) -> Vec<u8> {
    let mut bytes = Vec::new();
    plist::to_writer_xml(&mut bytes, value).unwrap();
    bytes
}

fn superblob(entitlements: &[u8]) -> Vec<u8> {
    let blob_length = 8 + entitlements.len();
    let total_length = 20 + blob_length;
    let mut bytes = Vec::with_capacity(total_length);
    push_be_u32(&mut bytes, 0xfade0cc0);
    push_be_u32(&mut bytes, total_length as u32);
    push_be_u32(&mut bytes, 1);
    push_be_u32(&mut bytes, 5);
    push_be_u32(&mut bytes, 20);
    push_be_u32(&mut bytes, 0xfade7171);
    push_be_u32(&mut bytes, blob_length as u32);
    bytes.extend_from_slice(entitlements);
    bytes
}

fn profile_value() -> Value {
    let mut entitlements = entitlement_dictionary();
    entitlements.insert(
        "ProvisionedDevices".into(),
        Value::Array(vec![Value::String("device-secret".into())]),
    );
    entitlements.insert("DeveloperCertificates".into(), Value::Data(vec![1, 2, 3]));
    entitlements.insert("databasePassword".into(), Value::String("secret".into()));

    let mut profile = Dictionary::new();
    profile.insert("Entitlements".into(), Value::Dictionary(entitlements));
    profile.insert(
        "ProvisionedDevices".into(),
        Value::Array(vec![Value::String("outer-device-secret".into())]),
    );
    profile.insert(
        "DeveloperCertificates".into(),
        Value::Array(vec![Value::Data(vec![4, 5, 6])]),
    );
    Value::Dictionary(profile)
}

#[test]
fn codesign_reads_xml_entitlements_from_superblob() {
    let xml = xml_plist(&Value::Dictionary(entitlement_dictionary()));
    let entitlements = codesign::entitlements_from_superblob(&superblob(&xml)).unwrap();

    assert_eq!(entitlements["get-task-allow"], false);
    assert_eq!(
        entitlements["application-identifier"],
        "TEAM.com.riviu.fixture"
    );
}

#[test]
fn codesign_rejects_out_of_bounds_superblob_offsets() {
    let mut bytes = Vec::new();
    push_be_u32(&mut bytes, 0xfade0cc0);
    push_be_u32(&mut bytes, 20);
    push_be_u32(&mut bytes, 1);
    push_be_u32(&mut bytes, 5);
    push_be_u32(&mut bytes, u32::MAX);

    let error = codesign::entitlements_from_superblob(&bytes).unwrap_err();

    assert!(error.to_string().contains("offset"));
}

#[test]
fn profile_returns_only_redacted_non_sensitive_entitlements() {
    let xml = xml_plist(&profile_value());
    let mut cms_like = b"CMS-prefix".to_vec();
    cms_like.extend_from_slice(&xml);
    cms_like.extend_from_slice(b"CMS-suffix");

    let entitlements = codesign::profile_entitlements(&cms_like).unwrap();
    let serialized = serde_json::to_string(&entitlements).unwrap();

    assert_eq!(entitlements["get-task-allow"], false);
    assert!(!serialized.contains("ProvisionedDevices"));
    assert!(!serialized.contains("DeveloperCertificates"));
    assert!(!serialized.to_ascii_lowercase().contains("password"));
    assert!(!serialized.contains("device-secret"));
}

#[test]
fn profile_finds_embedded_binary_plist() {
    let mut plist = Vec::new();
    plist::to_writer_binary(&mut plist, &profile_value()).unwrap();
    let mut cms_like = b"CMS-prefix".to_vec();
    cms_like.extend_from_slice(&plist);
    cms_like.extend_from_slice(b"suffix");

    let entitlements = codesign::profile_entitlements(&cms_like).unwrap();

    assert_eq!(entitlements["get-task-allow"], false);
}

#[test]
fn bundled_artifact_has_structured_signing_metadata() {
    let ipa_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sidecars/wda/RiviuAgent.ipa");
    let archive = archive::read_ipa(&ipa_path).unwrap();
    let profiles = archive::mobileprovision_candidates(&archive);
    assert_eq!(profiles.len(), 1);
    let profile = codesign::profile_entitlements(profiles[0].1).unwrap();
    let profile_json = serde_json::to_string(&profile).unwrap();
    assert!(!profile.is_empty());
    assert!(!profile_json.contains("ProvisionedDevices"));
    assert!(!profile_json.contains("DeveloperCertificates"));

    let images = archive::macho_candidates(&archive)
        .into_iter()
        .map(|(path, bytes)| macho::inspect(path, bytes).unwrap())
        .collect::<Vec<_>>();
    assert!(images
        .iter()
        .filter(|image| !image.path.contains("/Contents/Resources/DWARF/"))
        .any(|image| !image.entitlements.is_empty()));
}
