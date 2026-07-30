use std::fs::File;
use std::io::Write;
use std::path::Path;

use plist::{Dictionary, Value};
use rtmmo_re::archive;
use tempfile::TempDir;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

fn fixture_dictionary() -> Dictionary {
    let mut dictionary = Dictionary::new();
    dictionary.insert(
        "CFBundleIdentifier".into(),
        Value::String("com.riviu.fixture".into()),
    );
    dictionary.insert("CFBundleExecutable".into(), Value::String("Fixture".into()));
    dictionary.insert(
        "CFBundleShortVersionString".into(),
        Value::String("1.2.3".into()),
    );

    dictionary
}

fn fixture_plist() -> Vec<u8> {
    let mut output = Vec::new();
    plist::to_writer_xml(&mut output, &Value::Dictionary(fixture_dictionary())).unwrap();
    output
}

fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
    let file = File::create(path).unwrap();
    let mut writer = ZipWriter::new(file);
    for (name, bytes) in entries {
        writer
            .start_file(*name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
}

#[test]
fn archive_reads_plists_and_sorts_entries_without_extracting() {
    let temp = TempDir::new().unwrap();
    let ipa_path = temp.path().join("Fixture.ipa");
    let plist = fixture_plist();
    write_zip(
        &ipa_path,
        &[
            ("Payload/Fixture.app/Fixture", b"fixture executable"),
            ("Payload/Fixture.app/Info.plist", &plist),
        ],
    );

    let inventory = archive::read_ipa(&ipa_path).unwrap();

    assert_eq!(inventory.entries.len(), 2);
    assert_eq!(inventory.artifact.path, "Fixture.ipa");
    assert_eq!(inventory.artifact.size, ipa_path.metadata().unwrap().len());
    assert_eq!(
        inventory.bundles[0].bundle_id.as_deref(),
        Some("com.riviu.fixture")
    );
    assert_eq!(
        inventory.bundles[0].executable_path.as_deref(),
        Some("Payload/Fixture.app/Fixture")
    );
    assert!(inventory
        .entries
        .windows(2)
        .all(|pair| pair[0].path <= pair[1].path));
    assert_eq!(temp.path().read_dir().unwrap().count(), 1);
}

#[test]
fn archive_rejects_unsafe_entry_paths_before_parsing_content() {
    for name in [
        "/absolute",
        "../escape",
        "Payload/../../escape",
        "C:/escape",
        r"C:\escape",
        r"\\server\share",
        r"Payload\..\escape",
        "Payload//escape",
        "Payload/./escape",
        "Payload/\0escape",
    ] {
        let temp = TempDir::new().unwrap();
        let ipa_path = temp.path().join("Unsafe.ipa");
        write_zip(
            &ipa_path,
            &[
                ("Payload/Broken.app/Info.plist", b"malformed plist"),
                (name, b"entry"),
            ],
        );

        let error = archive::read_ipa(&ipa_path).unwrap_err();

        assert!(
            error.to_string().contains("unsafe ZIP entry path"),
            "unexpected error for {name}: {error:#}"
        );
    }
}

#[test]
fn archive_rejects_duplicate_paths_after_separator_normalization() {
    let temp = TempDir::new().unwrap();
    let ipa_path = temp.path().join("Duplicate.ipa");
    write_zip(
        &ipa_path,
        &[("Payload/file", b"one"), (r"Payload\file", b"two")],
    );

    let error = archive::read_ipa(&ipa_path).unwrap_err();

    assert!(error
        .to_string()
        .contains("duplicate normalized ZIP entry path"));
}

#[test]
fn archive_reads_binary_plists() {
    let temp = TempDir::new().unwrap();
    let ipa_path = temp.path().join("Binary.ipa");
    let mut plist = Vec::new();
    plist::to_writer_binary(&mut plist, &Value::Dictionary(fixture_dictionary())).unwrap();
    write_zip(
        &ipa_path,
        &[
            ("Payload/Fixture.app/Info.plist", &plist),
            ("Payload/Fixture.app/Fixture", b"fixture"),
        ],
    );

    let inventory = archive::read_ipa(&ipa_path).unwrap();

    assert_eq!(
        inventory.bundles[0].bundle_id.as_deref(),
        Some("com.riviu.fixture")
    );
}

#[test]
fn archive_rejects_wrong_plist_field_types_and_missing_executables() {
    for (mut dictionary, expected) in [
        (
            {
                let mut value = fixture_dictionary();
                value.insert("CFBundleIdentifier".into(), Value::Integer(1.into()));
                value
            },
            "CFBundleIdentifier is not a string",
        ),
        (
            {
                let mut value = fixture_dictionary();
                value.insert("CFBundleExecutable".into(), Value::String("Missing".into()));
                value
            },
            "does not resolve to a regular ZIP entry",
        ),
        (
            {
                let mut value = fixture_dictionary();
                value.insert(
                    "CFBundleExecutable".into(),
                    Value::String("../Fixture".into()),
                );
                value
            },
            "safe relative filename",
        ),
    ] {
        let temp = TempDir::new().unwrap();
        let ipa_path = temp.path().join("Malformed.ipa");
        let mut plist = Vec::new();
        plist::to_writer_xml(
            &mut plist,
            &Value::Dictionary(std::mem::take(&mut dictionary)),
        )
        .unwrap();
        write_zip(
            &ipa_path,
            &[
                ("Payload/Fixture.app/Info.plist", &plist),
                ("Payload/Fixture.app/Fixture", b"fixture"),
            ],
        );

        let error = archive::read_ipa(&ipa_path).unwrap_err();
        assert!(
            error.to_string().contains(expected),
            "expected {expected:?}, got {error:#}"
        );
    }
}

#[test]
fn archive_redacts_every_public_path() {
    let temp = TempDir::new().unwrap();
    let ipa_path = temp.path().join("RTmmo-LEAK_TOKEN.ipa");
    let udid = "0123456789abcdef0123456789abcdef01234567";
    write_zip(
        &ipa_path,
        &[
            ("Payload/RTmmo-ENTRY_TOKEN", b"token"),
            (&format!("Payload/{udid}"), b"device"),
            ("Payload/Users/builder/source", b"home"),
        ],
    );

    let inventory = archive::read_ipa(&ipa_path).unwrap();
    let public = std::iter::once(inventory.artifact.path.as_str())
        .chain(inventory.entries.iter().map(|entry| entry.path.as_str()))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(!public.contains("RTmmo-"));
    assert!(!public.contains(udid));
    assert!(!public.contains("/Users/builder"));
    assert!(inventory.redaction_count >= 4);
}

#[test]
fn archive_debug_output_never_contains_raw_entry_bytes() {
    let temp = TempDir::new().unwrap();
    let ipa_path = temp.path().join("Debug.ipa");
    write_zip(
        &ipa_path,
        &[("Payload/private.bin", b"PRIVATE_CERTIFICATE_FIXTURE")],
    );

    let inventory = archive::read_ipa(&ipa_path).unwrap();
    let debug = format!("{inventory:?}");

    assert!(!debug.contains("entry_bytes"));
    assert!(!debug.contains("PRIVATE_CERTIFICATE_FIXTURE"));
}

#[test]
fn bundled_artifact_exposes_expected_bundle_metadata() {
    let ipa_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sidecars/wda/RiviuAgent.ipa");
    let inventory = archive::read_ipa(&ipa_path).unwrap();
    let outer = inventory
        .bundles
        .iter()
        .find(|bundle| bundle.bundle_id.as_deref() == Some("com.mrph.svc"))
        .expect("outer RT-MMO bundle");
    let framework = inventory
        .bundles
        .iter()
        .find(|bundle| bundle.bundle_id.as_deref() == Some("com.facebook.WebDriverAgentLib"))
        .expect("WebDriverAgentLib bundle");

    assert_eq!(framework.short_version.as_deref(), Some("15.1.4"));
    assert_eq!(outer.dt_xcode.as_deref(), Some("2630"));
    assert!(inventory
        .entries
        .iter()
        .any(|entry| entry.path.ends_with("DWARF/WebDriverAgentRunner")));
}
