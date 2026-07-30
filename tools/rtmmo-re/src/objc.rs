use std::sync::OnceLock;

use anyhow::{bail, Context, Result};
use object::{BinaryFormat, Object, ObjectSection};
use regex::Regex;

use crate::redact;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringTable {
    pub values: Vec<String>,
    pub redaction_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjcMetadata {
    pub classes: Vec<String>,
    pub methods: Vec<String>,
    pub route_candidates: Vec<String>,
    pub redaction_count: usize,
}

pub fn strings(data: &[u8]) -> StringTable {
    let mut values = Vec::new();
    let mut redaction_count = 0;
    for raw in data.split(|byte| *byte == 0).filter(|raw| !raw.is_empty()) {
        let Ok(value) = std::str::from_utf8(raw) else {
            continue;
        };
        let (value, count) = redact::all(value);
        redaction_count += count;
        values.push(value);
    }
    values.sort();
    values.dedup();
    StringTable {
        values,
        redaction_count,
    }
}

pub fn from_sections(class_names: &[u8], method_names: &[u8], cstrings: &[u8]) -> ObjcMetadata {
    let classes = identifier_strings(class_names, is_class_name);
    let methods = identifier_strings(method_names, is_method_name);
    let routes = routes(cstrings);
    ObjcMetadata {
        classes: classes.values,
        methods: methods.values,
        route_candidates: routes.values,
        redaction_count: classes.redaction_count + methods.redaction_count + routes.redaction_count,
    }
}

fn identifier_strings(data: &[u8], predicate: impl Fn(&str) -> bool) -> StringTable {
    let mut table = strings(data);
    table.values.retain(|value| predicate(value));
    table
}

fn is_class_name(value: &str) -> bool {
    class_pattern().is_match(value)
        && value
            .bytes()
            .any(|byte| byte.is_ascii_alphabetic() || byte == b'_')
}

fn is_method_name(value: &str) -> bool {
    method_pattern().is_match(value) && !is_property_type_encoding(value)
}

fn is_property_type_encoding(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 2
        && bytes[0] == b'T'
        && matches!(
            bytes[1],
            b'B' | b'C'
                | b'c'
                | b'S'
                | b's'
                | b'I'
                | b'i'
                | b'L'
                | b'l'
                | b'Q'
                | b'q'
                | b'f'
                | b'd'
                | b'D'
                | b'b'
                | b'v'
                | b'@'
                | b'#'
                | b':'
                | b'*'
                | b'?'
        )
}

fn class_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"^[A-Za-z_$][A-Za-z0-9_.$]*$").expect("valid Objective-C class regex")
    })
}

fn method_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"^\.?[A-Za-z_][A-Za-z0-9_]*(?::[A-Za-z_][A-Za-z0-9_]*)*:?$")
            .expect("valid Objective-C selector regex")
    })
}

pub fn inspect(bytes: &[u8]) -> Result<ObjcMetadata> {
    let file = object::File::parse(bytes).context("parse Mach-O Objective-C metadata")?;
    if file.format() != BinaryFormat::MachO {
        bail!("input is not a Mach-O image");
    }

    let mut class_names = Vec::new();
    let mut method_names = Vec::new();
    let mut cstrings = Vec::new();
    for section in file.sections() {
        let target = match section
            .name_bytes()
            .context("read Objective-C section name")?
        {
            b"__objc_classname" => &mut class_names,
            b"__objc_methname" => &mut method_names,
            b"__cstring" => &mut cstrings,
            _ => continue,
        };
        let data = section
            .uncompressed_data()
            .context("read Objective-C string section")?;
        target.extend_from_slice(&data);
        target.push(0);
    }
    Ok(from_sections(&class_names, &method_names, &cstrings))
}

fn routes(data: &[u8]) -> StringTable {
    let mut values = Vec::new();
    let mut redaction_count = 0;
    for raw in data.split(|byte| *byte == 0).filter(|raw| !raw.is_empty()) {
        let Ok(value) = std::str::from_utf8(raw) else {
            continue;
        };
        if !is_route_candidate(value) {
            continue;
        }
        let normalized = normalize_session_route(value);
        let (value, count) = redact::all(&normalized);
        redaction_count += count;
        values.push(value);
    }
    values.sort();
    values.dedup();
    StringTable {
        values,
        redaction_count,
    }
}

fn is_route_candidate(value: &str) -> bool {
    const FILESYSTEM_PREFIXES: [&str; 10] = [
        "/Applications/",
        "/Developer/",
        "/Library/",
        "/System/",
        "/Users/",
        "/Volumes/",
        "/dev/",
        "/private/",
        "/tmp/",
        "/usr/",
    ];
    route_pattern().is_match(value)
        && !value.starts_with("//")
        && !FILESYSTEM_PREFIXES
            .iter()
            .any(|prefix| value.starts_with(prefix))
}

fn route_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(r"^/[A-Za-z0-9_{}:/.\-]+$").expect("valid Objective-C route regex")
    })
}

fn normalize_session_route(route: &str) -> String {
    let Some(session_path) = route.strip_prefix("/session/") else {
        return route.to_owned();
    };
    if session_path.is_empty() {
        return route.to_owned();
    }
    match session_path.split_once('/') {
        Some((_, suffix)) => format!("/session/{{sessionId}}/{suffix}"),
        None => "/session/{sessionId}".to_owned(),
    }
}
