use std::path::Path;

use rtmmo_re::{archive, dwarf};

#[test]
fn bundled_dsym_exposes_compile_units_functions_and_redacted_paths() {
    let ipa_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sidecars/wda/RiviuAgent.ipa");
    let archive = archive::read_ipa(&ipa_path).unwrap();
    let (path, bytes) = archive::macho_candidates(&archive)
        .into_iter()
        .find(|(path, _)| path.contains("/Contents/Resources/DWARF/"))
        .expect("bundled dSYM image");

    let info = dwarf::inspect(path, bytes).unwrap();

    assert!(info.compile_units > 0);
    assert!(info.subprograms > 0);
    assert!(info.line_sequences > 0);
    assert!(info.line_rows > 0);
    assert!(!info.functions.is_empty());
    assert!(info
        .functions
        .iter()
        .any(|function| !function.ranges.is_empty()));
    assert!(!info.source_paths.is_empty());
    assert!(info
        .source_paths
        .iter()
        .all(|path| !path.contains("/Users/") && !path.contains(r"C:\Users\")));
    assert!(info
        .function_names
        .iter()
        .all(|name| !name.contains("RTmmo-")));
}

#[test]
fn dwarf_rejects_non_macho_input() {
    let error = dwarf::inspect("fixture", b"not a Mach-O").unwrap_err();

    assert!(error.to_string().contains("Mach-O"));
}
