use std::path::Path;

use rtmmo_re::{archive, macho};

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn arm64_macho_fixture() -> Vec<u8> {
    let mut bytes = Vec::new();

    push_u32(&mut bytes, object::macho::MH_MAGIC_64);
    push_u32(&mut bytes, object::macho::CPU_TYPE_ARM64);
    push_u32(&mut bytes, object::macho::CPU_SUBTYPE_ARM64_ALL);
    push_u32(&mut bytes, object::macho::MH_EXECUTE);
    push_u32(&mut bytes, 2);
    push_u32(&mut bytes, 48);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);

    push_u32(&mut bytes, object::macho::LC_UUID);
    push_u32(&mut bytes, 24);
    bytes.extend(0_u8..16);

    push_u32(&mut bytes, object::macho::LC_ENCRYPTION_INFO_64);
    push_u32(&mut bytes, 24);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);

    bytes
}

fn macho_symbol_scope_fixture() -> Vec<u8> {
    const HEADER_SIZE: u32 = 32;
    const SYMTAB_COMMAND_SIZE: u32 = 24;
    const SYMBOL_SIZE: u32 = 16;

    let string_table = b"\0_public\0_private\0";
    let symbol_offset = HEADER_SIZE + SYMTAB_COMMAND_SIZE;
    let string_offset = symbol_offset + 2 * SYMBOL_SIZE;
    let mut bytes = Vec::new();

    push_u32(&mut bytes, object::macho::MH_MAGIC_64);
    push_u32(&mut bytes, object::macho::CPU_TYPE_ARM64);
    push_u32(&mut bytes, object::macho::CPU_SUBTYPE_ARM64_ALL);
    push_u32(&mut bytes, object::macho::MH_OBJECT);
    push_u32(&mut bytes, 1);
    push_u32(&mut bytes, SYMTAB_COMMAND_SIZE);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);

    push_u32(&mut bytes, object::macho::LC_SYMTAB);
    push_u32(&mut bytes, SYMTAB_COMMAND_SIZE);
    push_u32(&mut bytes, symbol_offset);
    push_u32(&mut bytes, 2);
    push_u32(&mut bytes, string_offset);
    push_u32(&mut bytes, string_table.len() as u32);

    push_u32(&mut bytes, 1);
    bytes.push(object::macho::N_ABS | object::macho::N_EXT);
    bytes.push(0);
    push_u16(&mut bytes, 0);
    bytes.extend_from_slice(&1_u64.to_le_bytes());

    push_u32(&mut bytes, 9);
    bytes.push(object::macho::N_ABS | object::macho::N_EXT | object::macho::N_PEXT);
    bytes.push(0);
    push_u16(&mut bytes, 0);
    bytes.extend_from_slice(&2_u64.to_le_bytes());

    bytes.extend_from_slice(string_table);
    bytes
}

#[test]
fn macho_reads_arm64_uuid_and_encryption_command() {
    let info = macho::inspect("Fixture", &arm64_macho_fixture()).unwrap();

    assert_eq!(info.architecture, "aarch64");
    assert!(info.is_64);
    assert!(info.little_endian);
    assert_eq!(
        info.uuid.as_deref(),
        Some("00010203-0405-0607-0809-0a0b0c0d0e0f")
    );
    assert_eq!(info.crypt_id, Some(0));
}

#[test]
fn macho_rejects_non_macho_input() {
    let error = macho::inspect("Fixture", b"not a Mach-O").unwrap_err();

    assert!(error.to_string().contains("not a Mach-O"));
}

#[test]
fn macho_exports_only_dynamic_symbols_not_private_externals() {
    let info = macho::inspect("Fixture", &macho_symbol_scope_fixture()).unwrap();

    assert_eq!(info.symbol_count, 2);
    assert_eq!(info.exported_symbols, vec!["_public"]);
}

#[test]
fn bundled_artifact_has_four_arm64_macho_images() {
    let ipa_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sidecars/wda/RiviuAgent.ipa");
    let archive = archive::read_ipa(&ipa_path).unwrap();
    let candidates = archive::macho_candidates(&archive);
    let mut inspected = candidates
        .iter()
        .map(|(path, bytes)| macho::inspect(path, bytes).unwrap())
        .collect::<Vec<_>>();
    inspected.sort_by(|left, right| left.path.cmp(&right.path));

    assert_eq!(inspected.len(), 4, "candidates: {inspected:#?}");
    assert!(inspected
        .iter()
        .all(|image| image.architecture == "aarch64"));
    for image in &inspected {
        if image.path.contains("/Contents/Resources/DWARF/") {
            assert_eq!(image.crypt_id, None, "dSYM has no encryption command");
        } else {
            assert_eq!(image.crypt_id, Some(0), "runtime image: {}", image.path);
        }
    }
    assert!(inspected.iter().any(|image| {
        image.path.ends_with("WebDriverAgentLib")
            && image
                .linked_dylibs
                .iter()
                .any(|name| name.contains("XCTest.framework"))
    }));
    assert!(inspected
        .iter()
        .any(|image| !image.objc_classes.is_empty() && !image.objc_methods.is_empty()));
    assert!(inspected
        .iter()
        .any(|image| !image.exported_symbols.is_empty()));
    assert!(inspected.iter().all(|image| image
        .exported_symbols
        .windows(2)
        .all(|pair| pair[0] <= pair[1])));
}
