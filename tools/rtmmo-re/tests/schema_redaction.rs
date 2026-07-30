use rtmmo_re::{model, redact};

#[test]
fn redacts_vendor_tokens_and_udids_before_serialization() {
    let input = "header RTmmo-SAMPLE_TOKEN device 0123456789abcdef0123456789abcdef01234567";
    let (text, count) = redact::text(input);

    assert_eq!(
        text,
        "header <redacted-agent-token> device <redacted-device-id>"
    );
    assert_eq!(count, 2);
}

#[test]
fn does_not_redact_hex_embedded_inside_a_larger_identifier() {
    let input = "x0123456789abcdef0123456789abcdef01234567y";
    let (text, count) = redact::text(input);

    assert_eq!(text, input);
    assert_eq!(count, 0);
}

#[test]
fn normalizes_build_machine_home_paths() {
    assert_eq!(
        redact::path("/Users/builder/project/File.m"),
        ("<home>/project/File.m".to_string(), 1)
    );
    assert_eq!(
        redact::path(r"C:\Users\builder\project\File.m"),
        (r"<home>\project\File.m".to_string(), 1)
    );
}

#[test]
fn composed_redaction_covers_secrets_and_home_paths() {
    let input = "/Users/builder/RTmmo-SAMPLE_TOKEN/0123456789abcdef0123456789abcdef01234567";
    let (value, count) = redact::all(input);

    assert_eq!(value, "<home>/<redacted-agent-token>/<redacted-device-id>");
    assert_eq!(count, 3);
}

#[test]
fn file_digest_serializes_with_camel_case_fields() {
    let digest = model::FileDigest {
        path: "Payload/App".into(),
        size: 7,
        sha256: "ab".repeat(32),
    };
    let value = serde_json::to_value(digest).unwrap();

    assert_eq!(value["sha256"], "ab".repeat(32));
    assert_eq!(value["size"], 7);
}
