use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Cursor;

use flate2::write::GzEncoder;
use flate2::Compression;

use rtmmo_re::baseline;
use rtmmo_re::model::{BaselineLock, BaselineSource};

fn source(classes: &[&str], methods: &[&str], routes: &[&str]) -> BaselineSource {
    BaselineSource {
        objc_classes: classes.iter().map(|value| (*value).to_owned()).collect(),
        objc_methods: methods.iter().map(|value| (*value).to_owned()).collect(),
        route_candidates: routes.iter().map(|value| (*value).to_owned()).collect(),
        class_provenance: BTreeMap::new(),
        method_provenance: BTreeMap::new(),
        route_provenance: BTreeMap::new(),
    }
}

fn fixture_lock() -> BaselineLock {
    BaselineLock {
        package: "appium-webdriveragent".to_owned(),
        version: "15.1.4".to_owned(),
        git_head: "20b705f8f96dee2939c022de6352720a311adb71".to_owned(),
        tarball: "https://registry.npmjs.org/fixture.tgz".to_owned(),
        integrity: "sha512-fixture".to_owned(),
    }
}

fn npm_archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut builder = tar::Builder::new(encoder);
    for (path, bytes) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, format!("package/{path}"), Cursor::new(*bytes))
            .unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap()
}

#[test]
fn baseline_checked_in_lock_pins_the_exact_upstream_package() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("baselines/wda-15.1.4.json");
    let lock = baseline::read_lock(&path).unwrap();

    assert_eq!(lock.package, "appium-webdriveragent");
    assert_eq!(lock.version, "15.1.4");
    assert_eq!(lock.git_head, "20b705f8f96dee2939c022de6352720a311adb71");
    assert_eq!(
        lock.integrity,
        "sha512-1tPVzIVPsBKynbTFqJyk3Hrf/FZ6kDmeP81P24hJ6q3gYHd2ljsI6OYEhINSbzxDdDmgTuWyYoUa1YtFvZC8oA=="
    );
}

#[test]
fn baseline_integrity_accepts_exact_bytes_and_rejects_mutation() {
    let bytes = b"baseline-fixture-v1";
    let integrity = "sha512-y5zLnaR1l7aTRDP0hgY9qn49T2kgGy2LM/l039gYTrQ2BEj5NvoidGE6dKcuF/JvQGX5r9sLFYNPkLareYIeLg==";

    baseline::verify_integrity(bytes, integrity).unwrap();

    let error = baseline::verify_integrity(b"baseline-fixture-v2", integrity).unwrap_err();
    assert!(error.to_string().contains("integrity mismatch"));
}

#[test]
fn baseline_integrity_rejects_unknown_or_malformed_algorithms() {
    let error = baseline::verify_integrity(b"fixture", "sha256-Zml4dHVyZQ==").unwrap_err();
    assert!(error.to_string().contains("sha512"));

    let error = baseline::verify_integrity(b"fixture", "sha512-***").unwrap_err();
    assert!(error.to_string().contains("base64"));
}

#[test]
fn baseline_scan_collects_classes_selectors_routes_and_provenance() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("FBSession.h"),
        r#"
        @interface FBSession : NSObject
        - (void)declaredMethod;
        @end
        "#,
    )
    .unwrap();
    fs::write(
        root.path().join("FBSession.m"),
        r#"
        @implementation FBSession
        - (void)declaredMethod { }
        [element typeText:text];
        [[FBRoute POST:@"/wda/keys"] respondWithTarget:self action:@selector(handleKeys:)];
        [[FBRoute GET:@"/status"].withoutSession respondWithTarget:self action:@selector(handleStatus:)];
        SEL Handler = @selector(handleKeys:);
        @end
        "#,
    )
    .unwrap();
    fs::create_dir(root.path().join("Nested")).unwrap();
    fs::write(
        root.path().join("Nested/Fixture.swift"),
        r#"
        final class SwiftFixture {}
        let healthRoute = "/status"
        let systemPath = "/System/Library/CoreServices/SystemVersion.plist"
        let xpath = "//XCUIElementTypeButton"
        let buildPath = "/Users/builder/private/File.m"
        "#,
    )
    .unwrap();
    fs::write(
        root.path().join("Ignored.txt"),
        "@interface IgnoredClass\n@\"/must/not/appear\"",
    )
    .unwrap();

    let scanned = baseline::scan_source(root.path()).unwrap();

    assert!(scanned.objc_classes.contains("FBSession"));
    assert!(scanned.objc_classes.contains("SwiftFixture"));
    assert!(!scanned.objc_classes.contains("IgnoredClass"));
    assert!(scanned.objc_methods.contains("declaredMethod"));
    assert!(scanned.objc_methods.contains("typeText:"));
    assert!(scanned.objc_methods.contains("handleKeys:"));
    assert!(scanned
        .route_candidates
        .contains("/session/{sessionId}/wda/keys"));
    assert!(!scanned.route_candidates.contains("/wda/keys"));
    assert!(scanned.route_candidates.contains("/status"));
    assert!(!scanned
        .route_candidates
        .contains("/session/{sessionId}/status"));
    assert!(!scanned.route_candidates.contains("/must/not/appear"));
    assert!(!scanned
        .route_candidates
        .iter()
        .any(|route| route.contains("/Users/") || route.contains("<home>")));
    assert_eq!(
        scanned.class_provenance.get("FBSession").unwrap(),
        &vec!["FBSession.h".to_owned(), "FBSession.m".to_owned()]
    );
    assert_eq!(
        scanned.route_provenance.get("/status").unwrap(),
        &vec!["FBSession.m".to_owned()]
    );
    assert!(!scanned
        .route_candidates
        .contains("/System/Library/CoreServices/SystemVersion.plist"));
    assert!(!scanned.route_candidates.contains("//XCUIElementTypeButton"));
    assert!(baseline::provenance_complete(&scanned));
}

#[test]
fn verified_source_archive_binds_extracted_source_to_tarball_bytes() {
    let root = tempfile::tempdir().unwrap();
    let source = b"@interface FBSession : NSObject\n@end\n";
    fs::write(root.path().join("FBSession.h"), source).unwrap();
    let archive = npm_archive(&[("FBSession.h", source)]);

    let verified = baseline::verify_source_archive(root.path(), &archive).unwrap();

    assert!(verified.source.objc_classes.contains("FBSession"));
    assert_eq!(verified.sha256.len(), 64);

    fs::write(
        root.path().join("FBSession.h"),
        b"@interface Replaced : NSObject\n@end\n",
    )
    .unwrap();
    let error = baseline::verify_source_archive(root.path(), &archive).unwrap_err();
    assert!(error
        .to_string()
        .contains("does not match verified npm archive"));
}

#[test]
fn baseline_delta_is_sorted_and_labels_only_observed_oracle_sets() {
    let baseline_source = source(
        &["FBSession", "SharedClass"],
        &["shared:", "typeText:"],
        &["/session/{sessionId}/wda/keys", "/status"],
    );
    assert!(!baseline::provenance_complete(&baseline_source));
    let oracle_source = source(
        &["ZOracle", "SharedClass", "AOracle"],
        &["zCustom:", "shared:", "aCustom:"],
        &["/z", "/status", "/a"],
    );

    let diff = baseline::compare_sources(
        &fixture_lock(),
        &baseline_source,
        &oracle_source,
        "archive-sha256",
        "inventory-sha256",
        "source-sha256",
    );

    assert_eq!(diff.schema_version, 1);
    assert_eq!(diff.package, "appium-webdriveragent");
    assert_eq!(diff.package_version, "15.1.4");
    assert_eq!(diff.git_head, "20b705f8f96dee2939c022de6352720a311adb71");
    assert_eq!(diff.integrity, "sha512-fixture");
    assert_eq!(diff.archive_sha256, "archive-sha256");
    assert_eq!(diff.inventory_sha256, "inventory-sha256");
    assert_eq!(diff.source_sha256, "source-sha256");
    assert_eq!(diff.class_overlap, vec!["SharedClass"]);
    assert_eq!(diff.class_baseline_only, vec!["FBSession"]);
    assert_eq!(diff.class_oracle_only, vec!["AOracle", "ZOracle"]);
    assert_eq!(diff.method_overlap, vec!["shared:"]);
    assert_eq!(diff.method_baseline_only, vec!["typeText:"]);
    assert_eq!(diff.method_oracle_only, vec!["aCustom:", "zCustom:"]);
    assert_eq!(diff.route_overlap, vec!["/status"]);
    assert_eq!(
        diff.route_baseline_only,
        vec!["/session/{sessionId}/wda/keys"]
    );
    assert_eq!(diff.route_oracle_only, vec!["/a", "/z"]);
    assert!(diff
        .baseline_source
        .route_candidates
        .contains("/session/{sessionId}/wda/keys"));
    assert_eq!(
        diff.oracle_source.route_candidates,
        oracle_source.route_candidates
    );

    let serialized = serde_json::to_value(diff).unwrap();
    assert!(serialized.get("customClasses").is_none());
}

#[test]
fn baseline_source_sets_are_deterministic() {
    let input: BTreeSet<String> = ["z", "a", "m"].into_iter().map(str::to_owned).collect();
    assert_eq!(input.into_iter().collect::<Vec<_>>(), vec!["a", "m", "z"]);
}
