use rtmmo_re::objc;

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_name(bytes: &mut Vec<u8>, value: &[u8]) {
    assert!(value.len() <= 16);
    bytes.extend_from_slice(value);
    bytes.resize(bytes.len() + (16 - value.len()), 0);
}

fn objc_macho_fixture() -> Vec<u8> {
    let sections: [(&[u8], &[u8]); 3] = [
        (b"__objc_classname", b"FBSession\0"),
        (b"__objc_methname", b"typeText:\0"),
        (b"__cstring", b"/session/live-session/wda/keys\0"),
    ];
    let command_size = 72 + sections.len() * 80;
    let data_offset = 32 + command_size;
    let data_size: usize = sections.iter().map(|(_, data)| data.len()).sum();
    let mut bytes = Vec::with_capacity(data_offset + data_size);

    push_u32(&mut bytes, object::macho::MH_MAGIC_64);
    push_u32(&mut bytes, object::macho::CPU_TYPE_ARM64);
    push_u32(&mut bytes, object::macho::CPU_SUBTYPE_ARM64_ALL);
    push_u32(&mut bytes, object::macho::MH_OBJECT);
    push_u32(&mut bytes, 1);
    push_u32(&mut bytes, command_size as u32);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);

    push_u32(&mut bytes, object::macho::LC_SEGMENT_64);
    push_u32(&mut bytes, command_size as u32);
    push_name(&mut bytes, b"__TEXT");
    push_u64(&mut bytes, 0);
    push_u64(&mut bytes, data_size as u64);
    push_u64(&mut bytes, data_offset as u64);
    push_u64(&mut bytes, data_size as u64);
    push_u32(&mut bytes, 7);
    push_u32(&mut bytes, 5);
    push_u32(&mut bytes, sections.len() as u32);
    push_u32(&mut bytes, 0);

    let mut section_offset = data_offset;
    for (name, data) in &sections {
        push_name(&mut bytes, name);
        push_name(&mut bytes, b"__TEXT");
        push_u64(&mut bytes, (section_offset - data_offset) as u64);
        push_u64(&mut bytes, data.len() as u64);
        push_u32(&mut bytes, section_offset as u32);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, object::macho::S_CSTRING_LITERALS);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 0);
        section_offset += data.len();
    }

    for (_, data) in sections {
        bytes.extend_from_slice(data);
    }
    bytes
}

#[test]
fn objc_strings_skip_invalid_utf8_then_sort_and_deduplicate() {
    let parsed = objc::strings(b"typeText:\0\xff\xfe\0FBSession\0FBSession\0");

    assert_eq!(parsed.values, vec!["FBSession", "typeText:"]);
    assert_eq!(parsed.redaction_count, 0);
}

#[test]
fn objc_metadata_redacts_values_and_normalizes_session_routes() {
    let metadata = objc::from_sections(
        b"ZClass\0RTmmo-SAMPLE_TOKEN\0ZClass\0$\0$Swift.Valid\0\x01\0v24@0:8\0",
        b"typeText:\0launch:\0typeText:\0TB\0TQ\0Td\0\x08\0v24@0:8\0",
        b"/status\0/session/live-123/wda/keys\0/session/{sessionId}/wda/swipe\0/session/live-terminal\0/session/:sessionID\0/not allowed?\0relative/path\0//XCUIElementTypeButton\0/System/Library/CoreServices/SystemVersion.plist\0/private/var/mobile/file\0",
    );

    assert_eq!(metadata.classes, vec!["$Swift.Valid", "ZClass"]);
    assert_eq!(metadata.methods, vec!["launch:", "typeText:"]);
    assert_eq!(
        metadata.route_candidates,
        vec![
            "/session/{sessionId}",
            "/session/{sessionId}/wda/keys",
            "/session/{sessionId}/wda/swipe",
            "/status",
        ]
    );
    assert_eq!(metadata.redaction_count, 1);
    assert!(metadata
        .classes
        .iter()
        .chain(&metadata.methods)
        .chain(&metadata.route_candidates)
        .all(|value| !value.contains("RTmmo-")));
}

#[test]
fn objc_inspect_reads_named_macho_sections() {
    let metadata = objc::inspect(&objc_macho_fixture()).unwrap();

    assert_eq!(metadata.classes, vec!["FBSession"]);
    assert_eq!(metadata.methods, vec!["typeText:"]);
    assert_eq!(
        metadata.route_candidates,
        vec!["/session/{sessionId}/wda/keys"]
    );
}
