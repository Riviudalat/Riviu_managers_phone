use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Component, Path};
use std::sync::OnceLock;

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use flate2::read::GzDecoder;
use regex::Regex;
use sha2::{Digest, Sha256, Sha512};
use walkdir::WalkDir;

use crate::model::{BaselineDiff, BaselineLock, BaselineSource};
use crate::redact;

const MAX_SOURCE_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SOURCE_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_SOURCE_FILES: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedBaselineSource {
    pub source: BaselineSource,
    pub sha256: String,
}

pub fn read_lock(path: &Path) -> Result<BaselineLock> {
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read baseline lock {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse baseline lock {}", path.display()))
}

pub fn verify_integrity(bytes: &[u8], expected: &str) -> Result<()> {
    let encoded = expected
        .strip_prefix("sha512-")
        .ok_or_else(|| anyhow::anyhow!("baseline integrity must use sha512"))?;
    let expected_digest = STANDARD
        .decode(encoded)
        .context("invalid sha512 integrity base64")?;
    let actual_digest = Sha512::digest(bytes);

    if expected_digest.as_slice() != actual_digest.as_slice() {
        bail!("baseline integrity mismatch");
    }
    Ok(())
}

pub fn scan_source(root: &Path) -> Result<BaselineSource> {
    Ok(scan_documents(&source_documents(root)?))
}

pub fn verify_source_archive(root: &Path, archive_bytes: &[u8]) -> Result<VerifiedBaselineSource> {
    let extracted = source_documents(root)?;
    let packed = archive_documents(archive_bytes)?;
    if extracted != packed {
        bail!("extracted baseline source does not match verified npm archive");
    }
    Ok(VerifiedBaselineSource {
        source: scan_documents(&extracted),
        sha256: documents_sha256(&extracted),
    })
}

fn source_documents(root: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    if !root.is_dir() {
        bail!(
            "baseline source root is not a directory: {}",
            root.display()
        );
    }

    let mut documents = BTreeMap::new();
    let mut total_bytes = 0_u64;
    for entry in WalkDir::new(root).follow_links(false).sort_by_file_name() {
        let entry = entry
            .with_context(|| format!("failed to walk baseline source under {}", root.display()))?;
        if !entry.file_type().is_file() || !is_source_file(entry.path()) {
            continue;
        }

        let relative_path = relative_path(root, entry.path())?;
        let metadata = entry.metadata().with_context(|| {
            format!("failed to read source metadata {}", entry.path().display())
        })?;
        if metadata.len() > MAX_SOURCE_FILE_BYTES {
            bail!("baseline source file exceeds size limit: {relative_path}");
        }
        let bytes = fs::read(entry.path())
            .with_context(|| format!("failed to read source file {}", entry.path().display()))?;
        total_bytes = total_bytes
            .checked_add(bytes.len() as u64)
            .context("baseline source size overflow")?;
        if total_bytes > MAX_SOURCE_TOTAL_BYTES {
            bail!("baseline source exceeds total size limit");
        }
        if documents.insert(relative_path.clone(), bytes).is_some() {
            bail!("duplicate baseline source path: {relative_path}");
        }
        if documents.len() > MAX_SOURCE_FILES {
            bail!("baseline source has too many files");
        }
    }
    Ok(documents)
}

fn archive_documents(archive_bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>> {
    let decoder = GzDecoder::new(archive_bytes);
    let mut archive = tar::Archive::new(decoder);
    let mut documents = BTreeMap::new();
    let mut total_bytes = 0_u64;
    for entry in archive.entries().context("read npm tarball entries")? {
        let mut entry = entry.context("read npm tarball entry")?;
        let path = entry.path().context("read npm tarball entry path")?;
        validate_archive_path(&path)?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = path
            .to_str()
            .context("npm tarball source path is not UTF-8")?
            .replace('\\', "/");
        let Some(relative) = path.strip_prefix("package/") else {
            bail!("npm tarball entry is outside package/: {path}");
        };
        if !is_source_file(Path::new(relative)) {
            continue;
        }
        let declared_size = entry.header().size().context("read npm source size")?;
        if declared_size > MAX_SOURCE_FILE_BYTES {
            bail!("npm source file exceeds size limit: {relative}");
        }
        let mut bytes = Vec::new();
        (&mut entry)
            .take(MAX_SOURCE_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .with_context(|| format!("read npm source file: {relative}"))?;
        if bytes.len() as u64 != declared_size {
            bail!("npm source file size mismatch: {relative}");
        }
        total_bytes = total_bytes
            .checked_add(bytes.len() as u64)
            .context("npm source size overflow")?;
        if total_bytes > MAX_SOURCE_TOTAL_BYTES {
            bail!("npm source exceeds total size limit");
        }
        if documents.insert(relative.to_owned(), bytes).is_some() {
            bail!("duplicate npm source path: {relative}");
        }
        if documents.len() > MAX_SOURCE_FILES {
            bail!("npm source has too many files");
        }
    }
    Ok(documents)
}

fn validate_archive_path(path: &Path) -> Result<()> {
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::ParentDir
            )
        })
    {
        bail!("unsafe npm tarball entry path")
    }
    Ok(())
}

fn scan_documents(documents: &BTreeMap<String, Vec<u8>>) -> BaselineSource {
    let mut result = BaselineSource::default();
    for (path, bytes) in documents {
        scan_text(&String::from_utf8_lossy(bytes), path, &mut result);
    }
    result
}

fn documents_sha256(documents: &BTreeMap<String, Vec<u8>>) -> String {
    let mut hasher = Sha256::new();
    for (path, bytes) in documents {
        hasher.update((path.len() as u64).to_le_bytes());
        hasher.update(path.as_bytes());
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(Sha256::digest(bytes));
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn compare_sources(
    lock: &BaselineLock,
    baseline: &BaselineSource,
    oracle: &BaselineSource,
    archive_sha256: &str,
    inventory_sha256: &str,
    source_sha256: &str,
) -> BaselineDiff {
    BaselineDiff {
        schema_version: 1,
        package: lock.package.clone(),
        package_version: lock.version.clone(),
        git_head: lock.git_head.clone(),
        integrity: lock.integrity.clone(),
        archive_sha256: archive_sha256.to_owned(),
        inventory_sha256: inventory_sha256.to_owned(),
        source_sha256: source_sha256.to_owned(),
        baseline_source: baseline.clone(),
        oracle_source: oracle.clone(),
        class_overlap: intersection(&baseline.objc_classes, &oracle.objc_classes),
        class_baseline_only: difference(&baseline.objc_classes, &oracle.objc_classes),
        class_oracle_only: difference(&oracle.objc_classes, &baseline.objc_classes),
        method_overlap: intersection(&baseline.objc_methods, &oracle.objc_methods),
        method_baseline_only: difference(&baseline.objc_methods, &oracle.objc_methods),
        method_oracle_only: difference(&oracle.objc_methods, &baseline.objc_methods),
        route_overlap: intersection(&baseline.route_candidates, &oracle.route_candidates),
        route_baseline_only: difference(&baseline.route_candidates, &oracle.route_candidates),
        route_oracle_only: difference(&oracle.route_candidates, &baseline.route_candidates),
    }
}

pub fn provenance_complete(source: &BaselineSource) -> bool {
    provenance_map_complete(&source.objc_classes, &source.class_provenance)
        && provenance_map_complete(&source.objc_methods, &source.method_provenance)
        && provenance_map_complete(&source.route_candidates, &source.route_provenance)
}

fn provenance_map_complete(
    values: &BTreeSet<String>,
    provenance: &BTreeMap<String, Vec<String>>,
) -> bool {
    values.iter().all(|value| {
        provenance
            .get(value)
            .is_some_and(|paths| !paths.is_empty() && paths.iter().all(|path| !path.is_empty()))
    }) && provenance.keys().all(|value| values.contains(value))
}

fn is_source_file(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "h" | "m" | "mm" | "swift"
            )
        })
        .unwrap_or(false)
}

fn relative_path(root: &Path, path: &Path) -> Result<String> {
    let relative = path.strip_prefix(root).with_context(|| {
        format!(
            "source path {} is outside baseline root {}",
            path.display(),
            root.display()
        )
    })?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

fn scan_text(text: &str, path: &str, result: &mut BaselineSource) {
    for captures in objc_class_regex().captures_iter(text) {
        insert_evidence(
            &mut result.objc_classes,
            &mut result.class_provenance,
            captures[1].to_owned(),
            path,
        );
    }
    for captures in swift_class_regex().captures_iter(text) {
        insert_evidence(
            &mut result.objc_classes,
            &mut result.class_provenance,
            captures[1].to_owned(),
            path,
        );
    }

    for captures in objc_method_regex().captures_iter(text) {
        if let Some(selector) = selector_from_signature(&captures[1]) {
            insert_evidence(
                &mut result.objc_methods,
                &mut result.method_provenance,
                selector,
                path,
            );
        }
    }
    for captures in selector_literal_regex().captures_iter(text) {
        insert_evidence(
            &mut result.objc_methods,
            &mut result.method_provenance,
            captures[1].to_owned(),
            path,
        );
    }
    for selector in objc_message_selectors(text) {
        insert_evidence(
            &mut result.objc_methods,
            &mut result.method_provenance,
            selector,
            path,
        );
    }

    for captures in route_registration_regex().captures_iter(text) {
        let method = &captures[1];
        let route_match = captures.get(2).expect("route registration capture");
        let without_session = captures.get(3).is_some();
        let Some(route) = route_candidate(&effective_route(
            method,
            route_match.as_str(),
            without_session,
        )) else {
            continue;
        };
        insert_evidence(
            &mut result.route_candidates,
            &mut result.route_provenance,
            route,
            path,
        );
    }
}

fn selector_from_signature(signature: &str) -> Option<String> {
    let parts: Vec<&str> = selector_part_regex()
        .captures_iter(signature)
        .map(|captures| captures.get(1).expect("selector capture").as_str())
        .collect();
    if !parts.is_empty() {
        return Some(format!("{}:", parts.join(":")));
    }

    first_identifier_regex()
        .captures(signature)
        .map(|captures| captures[1].to_owned())
}

fn objc_message_selectors(text: &str) -> Vec<String> {
    #[derive(Clone, Copy)]
    enum LexState {
        Normal,
        String,
        Character,
        LineComment,
        BlockComment,
    }

    let bytes = text.as_bytes();
    let mut index = 0;
    let mut state = LexState::Normal;
    let mut stack: Vec<(String, bool)> = Vec::new();
    let mut selectors = Vec::new();

    while index < bytes.len() {
        let current = bytes[index];
        let next = bytes.get(index + 1).copied();
        match state {
            LexState::Normal => match (current, next) {
                (b'/', Some(b'/')) => {
                    state = LexState::LineComment;
                    index += 1;
                }
                (b'/', Some(b'*')) => {
                    state = LexState::BlockComment;
                    index += 1;
                }
                (b'"', _) => {
                    append_to_message(&mut stack, ' ');
                    state = LexState::String;
                }
                (b'\'', _) => {
                    append_to_message(&mut stack, ' ');
                    state = LexState::Character;
                }
                (b'[', _) => {
                    let is_collection_literal = index > 0 && bytes[index - 1] == b'@';
                    stack.push((String::new(), is_collection_literal));
                }
                (b']', _) => {
                    if let Some((message, is_collection_literal)) = stack.pop() {
                        if !is_collection_literal {
                            if let Some(selector) = selector_from_message(&message) {
                                selectors.push(selector);
                            }
                        }
                    }
                }
                _ => append_to_message(&mut stack, current as char),
            },
            LexState::String => match (current, next) {
                (b'\\', Some(_)) => index += 1,
                (b'"', _) => state = LexState::Normal,
                _ => {}
            },
            LexState::Character => match (current, next) {
                (b'\\', Some(_)) => index += 1,
                (b'\'', _) => state = LexState::Normal,
                _ => {}
            },
            LexState::LineComment => {
                if current == b'\n' {
                    append_to_message(&mut stack, ' ');
                    state = LexState::Normal;
                }
            }
            LexState::BlockComment => {
                if current == b'*' && next == Some(b'/') {
                    append_to_message(&mut stack, ' ');
                    state = LexState::Normal;
                    index += 1;
                }
            }
        }
        index += 1;
    }

    selectors.sort();
    selectors.dedup();
    selectors
}

fn append_to_message(stack: &mut [(String, bool)], value: char) {
    if let Some((message, _)) = stack.last_mut() {
        message.push(value);
    }
}

fn selector_from_message(message: &str) -> Option<String> {
    let without_selector_literals = selector_expression_regex().replace_all(message, " ");
    let parts: Vec<&str> = selector_part_regex()
        .captures_iter(&without_selector_literals)
        .map(|captures| captures.get(1).expect("message selector capture").as_str())
        .collect();
    (!parts.is_empty()).then(|| format!("{}:", parts.join(":")))
}

fn normalize_route(route: &str) -> String {
    session_parameter_regex()
        .replace_all(route, "{sessionId}")
        .into_owned()
}

fn effective_route(method: &str, route: &str, without_session: bool) -> String {
    let route = normalize_route(route);
    let session_prefix = "/session/{sessionId}";
    let requires_session = method != "OPTIONS" && !without_session;

    if requires_session {
        if route.starts_with(session_prefix) {
            route
        } else {
            format!("{session_prefix}{route}")
        }
    } else if let Some(without_prefix) = route.strip_prefix(session_prefix) {
        if without_prefix.is_empty() {
            "/".to_owned()
        } else {
            without_prefix.to_owned()
        }
    } else {
        route
    }
}

fn route_candidate(route: &str) -> Option<String> {
    let (route, replacements) = redact::all(route);
    (replacements == 0).then_some(route)
}

fn insert_evidence(
    values: &mut BTreeSet<String>,
    provenance: &mut BTreeMap<String, Vec<String>>,
    value: String,
    path: &str,
) {
    let value = redact::all(&value).0;
    let path = redact::all(path).0;
    values.insert(value.clone());
    let paths = provenance.entry(value).or_default();
    if !paths.iter().any(|candidate| candidate == &path) {
        paths.push(path);
        paths.sort();
    }
}

fn intersection(left: &BTreeSet<String>, right: &BTreeSet<String>) -> Vec<String> {
    left.intersection(right).cloned().collect()
}

fn difference(left: &BTreeSet<String>, right: &BTreeSet<String>) -> Vec<String> {
    left.difference(right).cloned().collect()
}

fn objc_class_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?m)@(?:interface|implementation|protocol)\s+([A-Za-z_][A-Za-z0-9_]*)")
            .expect("valid Objective-C class regex")
    })
}

fn swift_class_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?m)\b(?:class|protocol)\s+([A-Za-z_][A-Za-z0-9_]*)")
            .expect("valid Swift class regex")
    })
}

fn objc_method_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?ms)^[\t ]*[-+]\s*\([^\r\n)]*\)\s*([^;{]+)[;{]")
            .expect("valid Objective-C method regex")
    })
}

fn selector_literal_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"@selector\s*\(\s*([A-Za-z_][A-Za-z0-9_:]*)\s*\)")
            .expect("valid selector literal regex")
    })
}

fn selector_expression_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"@selector\s*\([^)]*\)").expect("valid selector expression regex")
    })
}

fn selector_part_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"([A-Za-z_][A-Za-z0-9_]*)\s*:").expect("valid selector part regex")
    })
}

fn first_identifier_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"^\s*([A-Za-z_][A-Za-z0-9_]*)").expect("valid identifier regex")
    })
}

fn route_registration_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(
            r#"(?s)FBRoute\s+(GET|POST|PUT|DELETE|OPTIONS)\s*:\s*@?\"(/[A-Za-z0-9_{}:/.\-]+)\"\s*\]\s*(\.\s*withoutSession)?"#,
        )
        .expect("valid route registration regex")
    })
}

fn session_parameter_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i):sessionid\b").expect("valid session parameter regex"))
}
