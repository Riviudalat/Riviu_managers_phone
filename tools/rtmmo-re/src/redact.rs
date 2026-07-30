use std::sync::OnceLock;

use regex::Regex;

fn token_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"RTmmo-[A-Za-z0-9_-]+").expect("valid token regex"))
}

fn device_id_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"(?i)\b[0-9a-f]{40}\b").expect("valid device ID regex"))
}

fn unix_home_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"/Users/[^/\\]+").expect("valid Unix home regex"))
}

fn windows_home_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"(?i)C:\\Users\\[^\\]+").expect("valid Windows home regex"))
}

fn replace_all(input: &str, pattern: &Regex, replacement: &str) -> (String, usize) {
    let count = pattern.find_iter(input).count();
    if count == 0 {
        return (input.to_string(), 0);
    }
    (pattern.replace_all(input, replacement).into_owned(), count)
}

pub fn text(input: &str) -> (String, usize) {
    let (without_tokens, token_count) =
        replace_all(input, token_pattern(), "<redacted-agent-token>");
    let (redacted, device_count) =
        replace_all(&without_tokens, device_id_pattern(), "<redacted-device-id>");
    (redacted, token_count + device_count)
}

pub fn path(input: &str) -> (String, usize) {
    let (without_unix_home, unix_count) = replace_all(input, unix_home_pattern(), "<home>");
    let (redacted, windows_count) =
        replace_all(&without_unix_home, windows_home_pattern(), "<home>");
    (redacted, unix_count + windows_count)
}

pub fn all(input: &str) -> (String, usize) {
    let (without_secrets, secret_count) = text(input);
    let (redacted, path_count) = path(&without_secrets);
    (redacted, secret_count + path_count)
}
