use std::fs::{self, File};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use flate2::write::GzEncoder;
use flate2::Compression;
use plist::{Dictionary, Value};
use rtmmo_re::model::{BaselineDiff, BaselineLock};
use sha2::{Digest, Sha512};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rtmmo-re"))
}

fn run(args: &[&str]) -> Output {
    Command::new(binary()).args(args).output().unwrap()
}

fn minimal_macho() -> Vec<u8> {
    let values = [
        object::macho::MH_MAGIC_64,
        object::macho::CPU_TYPE_ARM64,
        object::macho::CPU_SUBTYPE_ARM64_ALL,
        object::macho::MH_EXECUTE,
        0,
        0,
        0,
        0,
    ];
    values.into_iter().flat_map(u32::to_le_bytes).collect()
}

fn write_fixture_ipa(path: &Path) {
    let mut dictionary = Dictionary::new();
    dictionary.insert(
        "CFBundleIdentifier".into(),
        Value::String("com.riviu.fixture".into()),
    );
    dictionary.insert("CFBundleExecutable".into(), Value::String("Fixture".into()));
    let mut plist = Vec::new();
    plist::to_writer_xml(&mut plist, &Value::Dictionary(dictionary)).unwrap();

    let mut writer = ZipWriter::new(File::create(path).unwrap());
    writer
        .start_file("Payload/Fixture.app/Fixture", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(&minimal_macho()).unwrap();
    writer
        .start_file(
            "Payload/Fixture.app/Info.plist",
            SimpleFileOptions::default(),
        )
        .unwrap();
    writer.write_all(&plist).unwrap();
    writer.finish().unwrap();
}

#[test]
fn cli_inventory_is_redacted_and_deterministic() {
    let temp = TempDir::new().unwrap();
    let ipa = temp.path().join("RTmmo-FIXTURE_TOKEN.ipa");
    let first = temp.path().join("first.json");
    let second = temp.path().join("second.json");
    write_fixture_ipa(&ipa);

    for output in [&first, &second] {
        let result = run(&[
            "inventory",
            "--ipa",
            ipa.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ]);
        assert!(
            result.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    let first_bytes = fs::read(&first).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&first_bytes).unwrap();
    assert_eq!(value["schemaVersion"], 1);
    assert!(value["redactionCount"].as_u64().unwrap() > 0);
    assert_eq!(first_bytes, fs::read(&second).unwrap());
    assert!(first_bytes.ends_with(b"\n"));
    assert!(!String::from_utf8_lossy(&first_bytes).contains("RTmmo-"));
}

#[test]
fn cli_inventory_returns_input_error_for_malformed_ipa() {
    let temp = TempDir::new().unwrap();
    let ipa = temp.path().join("Broken.ipa");
    let output = temp.path().join("inventory.json");
    fs::write(&ipa, b"not a zip").unwrap();

    let result = run(&[
        "inventory",
        "--ipa",
        ipa.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ]);

    assert_eq!(result.status.code(), Some(2));
    assert!(!output.exists());
}

#[test]
fn cli_verify_redaction_uses_dedicated_exit_code() {
    let temp = TempDir::new().unwrap();
    let clean = temp.path().join("clean.json");
    let leaked = temp.path().join("leaked.json");
    let disguised = temp.path().join("disguised.json");
    let duplicate = temp.path().join("duplicate.json");
    let windows_path = temp.path().join("windows-path.json");
    let malformed_raw = temp.path().join("malformed-raw.json");
    fs::write(&clean, b"{\"value\":\"<redacted-agent-token>\"}\n").unwrap();
    fs::write(&leaked, b"{\"value\":\"RTmmo-LEAKED_TOKEN\"}\n").unwrap();
    fs::write(
        &disguised,
        b"{\"gitHead\":\"0123456789abcdef0123456789abcdef01234567\"}\n",
    )
    .unwrap();
    fs::write(&duplicate, b"{\"value\":\"first\",\"value\":\"second\"}\n").unwrap();
    fs::write(
        &windows_path,
        br#"{"value":"C:\\Users\\builder\\private.m"}
"#,
    )
    .unwrap();
    fs::write(&malformed_raw, b"{\"value\":\"clean\"} RTmmo-RAW_TOKEN\n").unwrap();

    let clean_result = run(&["verify-redaction", "--input", clean.to_str().unwrap()]);
    let leaked_result = run(&["verify-redaction", "--input", leaked.to_str().unwrap()]);

    assert!(clean_result.status.success());
    assert_eq!(leaked_result.status.code(), Some(3));
    assert_eq!(
        run(&["verify-redaction", "--input", disguised.to_str().unwrap(),])
            .status
            .code(),
        Some(3)
    );
    let duplicate_result = run(&["verify-redaction", "--input", duplicate.to_str().unwrap()]);
    assert_eq!(duplicate_result.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&duplicate_result.stderr).contains("duplicate JSON key"));
    assert_eq!(
        run(&[
            "verify-redaction",
            "--input",
            windows_path.to_str().unwrap(),
        ])
        .status
        .code(),
        Some(3)
    );
    let malformed_result = run(&[
        "verify-redaction",
        "--input",
        malformed_raw.to_str().unwrap(),
    ]);
    assert_eq!(malformed_result.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&malformed_result.stderr).contains("raw report bytes"));
}

#[test]
fn cli_without_subcommand_preserves_version_output() {
    let output = run(&[]);

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "rtmmo-re 0.1.0\n"
    );
}

fn write_lock(path: &Path, archive: &[u8]) {
    let lock = BaselineLock {
        package: "appium-webdriveragent".into(),
        version: "15.1.4".into(),
        git_head: "20b705f8f96dee2939c022de6352720a311adb71".into(),
        tarball: "https://registry.npmjs.org/fixture.tgz".into(),
        integrity: format!("sha512-{}", STANDARD.encode(Sha512::digest(archive))),
    };
    fs::write(path, serde_json::to_vec(&lock).unwrap()).unwrap();
}

fn write_npm_archive(path: &Path, entries: &[(&str, &[u8])]) {
    let encoder = GzEncoder::new(File::create(path).unwrap(), Compression::default());
    let mut builder = tar::Builder::new(encoder);
    for (entry_path, bytes) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(
                &mut header,
                format!("package/{entry_path}"),
                Cursor::new(*bytes),
            )
            .unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap();
}

#[test]
fn cli_verifies_baseline_and_generates_a_delta() {
    let temp = TempDir::new().unwrap();
    let archive = temp.path().join("baseline.tgz");
    let lock = temp.path().join("lock.json");
    let ipa = temp.path().join("Fixture.ipa");
    let inventory = temp.path().join("inventory.json");
    let source = temp.path().join("source");
    let delta = temp.path().join("delta.json");
    write_fixture_ipa(&ipa);
    fs::create_dir(&source).unwrap();
    let source_bytes = b"@interface Fixture : NSObject\n- (void)run;\n@end\n";
    fs::write(source.join("Fixture.m"), source_bytes).unwrap();
    write_npm_archive(&archive, &[("Fixture.m", source_bytes)]);
    write_lock(&lock, &fs::read(&archive).unwrap());

    let verify = run(&[
        "baseline-verify",
        "--lock",
        lock.to_str().unwrap(),
        "--archive",
        archive.to_str().unwrap(),
    ]);
    assert!(verify.status.success());
    assert!(run(&[
        "inventory",
        "--ipa",
        ipa.to_str().unwrap(),
        "--output",
        inventory.to_str().unwrap(),
    ])
    .status
    .success());
    let diff = run(&[
        "baseline-diff",
        "--inventory",
        inventory.to_str().unwrap(),
        "--source",
        source.to_str().unwrap(),
        "--archive",
        archive.to_str().unwrap(),
        "--lock",
        lock.to_str().unwrap(),
        "--output",
        delta.to_str().unwrap(),
    ]);
    assert!(
        diff.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&diff.stderr)
    );
    let value: BaselineDiff = serde_json::from_slice(&fs::read(delta).unwrap()).unwrap();
    assert_eq!(value.package_version, "15.1.4");
    let expected_archive_sha256 = sha2::Sha256::digest(fs::read(&archive).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(value.archive_sha256, expected_archive_sha256);
}

#[test]
fn cli_gate_blocks_forged_inventory_even_when_delta_is_regenerated() {
    let temp = TempDir::new().unwrap();
    let ipa = temp.path().join("Fixture.ipa");
    let inventory_path = temp.path().join("inventory.json");
    let baseline_path = temp.path().join("baseline.json");
    let manifest_path = temp.path().join("manifest.json");
    let gate_path = temp.path().join("gate-a.md");
    let source = temp.path().join("source");
    let archive = temp.path().join("baseline.tgz");
    let lock = temp.path().join("lock.json");
    write_fixture_ipa(&ipa);
    assert!(run(&[
        "inventory",
        "--ipa",
        ipa.to_str().unwrap(),
        "--output",
        inventory_path.to_str().unwrap(),
    ])
    .status
    .success());
    let inventory: serde_json::Value =
        serde_json::from_slice(&fs::read(&inventory_path).unwrap()).unwrap();
    fs::write(
        &manifest_path,
        serde_json::to_vec(&serde_json::json!({
            "sha256": inventory["artifact"]["sha256"]
        }))
        .unwrap(),
    )
    .unwrap();
    let mut forged_inventory = inventory;
    forged_inventory["entries"][0]["size"] = serde_json::json!(999_999_u64);
    fs::write(
        &inventory_path,
        serde_json::to_vec_pretty(&forged_inventory).unwrap(),
    )
    .unwrap();
    fs::create_dir(&source).unwrap();
    let source_bytes = b"@interface Fixture : NSObject\n@end\n";
    fs::write(source.join("Fixture.m"), source_bytes).unwrap();
    write_npm_archive(&archive, &[("Fixture.m", source_bytes)]);
    write_lock(&lock, &fs::read(&archive).unwrap());
    assert!(run(&[
        "baseline-diff",
        "--inventory",
        inventory_path.to_str().unwrap(),
        "--source",
        source.to_str().unwrap(),
        "--archive",
        archive.to_str().unwrap(),
        "--lock",
        lock.to_str().unwrap(),
        "--output",
        baseline_path.to_str().unwrap(),
    ])
    .status
    .success());
    let routes = Path::new(env!("CARGO_MANIFEST_DIR")).join("contracts/oracle-routes.json");

    let result = run(&[
        "gate-a",
        "--ipa",
        ipa.to_str().unwrap(),
        "--inventory",
        inventory_path.to_str().unwrap(),
        "--baseline",
        baseline_path.to_str().unwrap(),
        "--routes",
        routes.to_str().unwrap(),
        "--baseline-source",
        source.to_str().unwrap(),
        "--baseline-archive",
        archive.to_str().unwrap(),
        "--baseline-lock",
        lock.to_str().unwrap(),
        "--manifest",
        manifest_path.to_str().unwrap(),
        "--output",
        gate_path.to_str().unwrap(),
    ]);

    assert_eq!(result.status.code(), Some(4));
    let report = fs::read_to_string(gate_path).unwrap();
    assert!(report.contains("Decision: BLOCKED"));
    assert!(report.contains("archive SHA-256"));
    assert!(report.contains("- [ ] **IPA inventory evidence chain**"));
}

#[test]
fn cli_gate_blocks_a_forged_delta_even_with_pinned_version_and_git_head() {
    let temp = TempDir::new().unwrap();
    let ipa = temp.path().join("Fixture.ipa");
    let inventory = temp.path().join("inventory.json");
    let baseline = temp.path().join("baseline.json");
    let manifest = temp.path().join("manifest.json");
    let gate = temp.path().join("gate-a.md");
    let source = temp.path().join("source");
    let archive = temp.path().join("baseline.tgz");
    let lock = temp.path().join("lock.json");
    write_fixture_ipa(&ipa);
    assert!(run(&[
        "inventory",
        "--ipa",
        ipa.to_str().unwrap(),
        "--output",
        inventory.to_str().unwrap(),
    ])
    .status
    .success());
    let inventory_json: serde_json::Value =
        serde_json::from_slice(&fs::read(&inventory).unwrap()).unwrap();
    fs::write(
        &manifest,
        serde_json::to_vec(&serde_json::json!({
            "sha256": inventory_json["artifact"]["sha256"]
        }))
        .unwrap(),
    )
    .unwrap();
    fs::create_dir(&source).unwrap();
    let source_bytes = b"@interface Fixture : NSObject\n@end\n";
    fs::write(source.join("Fixture.m"), source_bytes).unwrap();
    write_npm_archive(&archive, &[("Fixture.m", source_bytes)]);
    write_lock(&lock, &fs::read(&archive).unwrap());
    assert!(run(&[
        "baseline-diff",
        "--inventory",
        inventory.to_str().unwrap(),
        "--source",
        source.to_str().unwrap(),
        "--archive",
        archive.to_str().unwrap(),
        "--lock",
        lock.to_str().unwrap(),
        "--output",
        baseline.to_str().unwrap(),
    ])
    .status
    .success());
    let mut forged: BaselineDiff = serde_json::from_slice(&fs::read(&baseline).unwrap()).unwrap();
    forged.class_overlap.clear();
    forged.class_oracle_only.clear();
    fs::write(&baseline, serde_json::to_vec(&forged).unwrap()).unwrap();
    let routes = Path::new(env!("CARGO_MANIFEST_DIR")).join("contracts/oracle-routes.json");

    let result = run(&[
        "gate-a",
        "--ipa",
        ipa.to_str().unwrap(),
        "--inventory",
        inventory.to_str().unwrap(),
        "--baseline",
        baseline.to_str().unwrap(),
        "--routes",
        routes.to_str().unwrap(),
        "--baseline-source",
        source.to_str().unwrap(),
        "--baseline-archive",
        archive.to_str().unwrap(),
        "--baseline-lock",
        lock.to_str().unwrap(),
        "--manifest",
        manifest.to_str().unwrap(),
        "--output",
        gate.to_str().unwrap(),
    ]);

    assert_eq!(result.status.code(), Some(4));
    let report = fs::read_to_string(gate).unwrap();
    assert!(report.contains("Baseline evidence chain"));
    assert!(report.contains("- [x] **IPA inventory evidence chain**"));
}
