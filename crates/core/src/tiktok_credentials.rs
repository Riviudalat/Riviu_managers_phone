//! What Riviu may know about a TikTok login **outside** the OS credential store.
//!
//! The secret pair (identifier and password) lives only in the OS credential store, owned by
//! `riviu-signing`. This module is the non-secret half: how a handle is written, which kind of
//! identifier the operator saved, the masked form shown in the UI, and the IPC shapes that
//! carry them. Nothing here holds or returns a password, and nothing returns a full
//! identifier: an email or a phone number is personal data even when it is not a secret.
//!
//! There is no login automation here. The rows let the reinstall + login command
//! (`tiktok_login`) find whether a phone's account has a stored credential, and record the
//! outcome of its one login submit.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// TikTok's own username rule: letters, digits, `_` and `.`, at most 24 characters.
pub const TIKTOK_HANDLE_MAX_CHARS: usize = 24;
const TIKTOK_HANDLE_MIN_CHARS: usize = 2;
/// RFC 5321's path limit. Longer is a paste mistake, not an address.
const IDENTIFIER_MAX_CHARS: usize = 254;

/// The key a credential is stored under: lowercase, no leading `@`.
///
/// Accepts what an operator types (` @Riviu.Demo `) and returns the one spelling the store
/// uses (`riviu.demo`), so the same account cannot end up under two keys. The error names the
/// value, which is fine: a handle is public.
pub fn canonical_tiktok_handle(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let handle = trimmed
        .strip_prefix('@')
        .unwrap_or(trimmed)
        .to_ascii_lowercase();
    if handle.is_empty() {
        return Err("Thiếu nick TikTok".into());
    }
    let length = handle.chars().count();
    let valid = (TIKTOK_HANDLE_MIN_CHARS..=TIKTOK_HANDLE_MAX_CHARS).contains(&length)
        && !handle.ends_with('.')
        && handle
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '.' || ch == '_');
    if !valid {
        return Err(format!(
            "Nick “{handle}” không hợp lệ: chỉ gồm a-z, 0-9, dấu chấm hoặc gạch dưới, \
             2–24 ký tự, không kết thúc bằng dấu chấm"
        ));
    }
    Ok(handle)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TikTokIdentifierKind {
    Email,
    Username,
    Phone,
}

impl TikTokIdentifierKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::Username => "username",
            Self::Phone => "phone",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "email" => Some(Self::Email),
            "username" => Some(Self::Username),
            "phone" => Some(Self::Phone),
            _ => None,
        }
    }
}

/// Classify what the operator typed into the login field, and return it trimmed.
///
/// The identifier is trimmed because surrounding blanks are never part of an email, phone
/// number or username. A password is never passed through here and never trimmed.
///
/// Errors never repeat the value: an identifier is personal data, and an error message ends
/// up in toasts, logs and screenshots.
pub fn classify_tiktok_identifier(raw: &str) -> Result<(String, TikTokIdentifierKind), String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Thiếu tên đăng nhập (email, số điện thoại hoặc username)".into());
    }
    if trimmed.chars().count() > IDENTIFIER_MAX_CHARS || trimmed.chars().any(char::is_control) {
        return Err("Tên đăng nhập không hợp lệ".into());
    }
    if let Some(username) = trimmed.strip_prefix('@').filter(|rest| !rest.contains('@')) {
        return username_identifier(username);
    }
    if trimmed.contains('@') {
        return email_identifier(trimmed);
    }
    if phone_digits(trimmed).is_some() {
        return Ok((trimmed.to_owned(), TikTokIdentifierKind::Phone));
    }
    username_identifier(trimmed)
}

fn email_identifier(value: &str) -> Result<(String, TikTokIdentifierKind), String> {
    let invalid = || "Email đăng nhập không hợp lệ".to_string();
    let (local, domain) = value.split_once('@').ok_or_else(invalid)?;
    let valid = !local.is_empty()
        && !domain.contains('@')
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !value.chars().any(char::is_whitespace);
    if !valid {
        return Err(invalid());
    }
    Ok((value.to_owned(), TikTokIdentifierKind::Email))
}

fn username_identifier(value: &str) -> Result<(String, TikTokIdentifierKind), String> {
    // The same rule as a handle; the login form accepts any case and TikTok stores lowercase.
    canonical_tiktok_handle(value)
        .map(|username| (username, TikTokIdentifierKind::Username))
        .map_err(|_| {
            "Tên đăng nhập không phải email, số điện thoại hay username TikTok hợp lệ".to_string()
        })
}

/// The digits of a phone number as an operator writes it (`+84 901-234-567`), if it is one.
fn phone_digits(value: &str) -> Option<String> {
    let compact: String = value
        .chars()
        .filter(|ch| !matches!(ch, ' ' | '-'))
        .collect();
    let digits = compact.strip_prefix('+').unwrap_or(&compact);
    let valid = (8..=15).contains(&digits.len()) && digits.chars().all(|ch| ch.is_ascii_digit());
    valid.then_some(compact)
}

/// The form an operator sees: enough to tell two accounts apart, not enough to reuse.
///
/// `thuc.nhan@gmail.com` → `th***@gmail.com`, `+84901234567` → `+84***567`,
/// `riviu.demo` → `ri***o`.
pub fn mask_tiktok_identifier(identifier: &str, kind: TikTokIdentifierKind) -> String {
    fn head(value: &str, count: usize) -> String {
        value.chars().take(count).collect()
    }
    fn tail(value: &str, count: usize) -> String {
        let length = value.chars().count();
        value.chars().skip(length.saturating_sub(count)).collect()
    }
    match kind {
        TikTokIdentifierKind::Email => {
            let (local, domain) = identifier.split_once('@').unwrap_or((identifier, ""));
            let keep = if local.chars().count() >= 3 { 2 } else { 1 };
            format!("{}***@{domain}", head(local, keep))
        }
        TikTokIdentifierKind::Phone => {
            let compact = phone_digits(identifier).unwrap_or_default();
            let keep = if compact.starts_with('+') { 3 } else { 2 };
            format!("{}***{}", head(&compact, keep), tail(&compact, 3))
        }
        TikTokIdentifierKind::Username => {
            if identifier.chars().count() <= 3 {
                format!("{}***", head(identifier, 1))
            } else {
                format!("{}***{}", head(identifier, 2), tail(identifier, 1))
            }
        }
    }
}

/// Whether the OS credential store actually holds the secret an index row describes.
///
/// `unreadable` is not `missing`: a credential store that failed to answer says nothing
/// about whether the entry is there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TikTokCredentialPresence {
    Stored,
    Missing,
    Unreadable,
}

/// One row of the non-secret index in SQLite (migration 50).
///
/// `last_status` is written by the TikTok reinstall + login command: `loginSubmitted` before
/// its one submit, then the outcome. `cooldown_until` is reserved; nothing writes it yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TikTokCredentialIndexEntry {
    pub handle: String,
    pub identifier_kind: TikTokIdentifierKind,
    pub masked_identifier: String,
    pub updated_at: String,
    pub cooldown_until: Option<String>,
    pub last_status: Option<String>,
}

/// What `tiktok_credentials_list` returns: the index row plus a fresh presence check.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TikTokCredentialSummary {
    pub handle: String,
    pub identifier_kind: TikTokIdentifierKind,
    pub masked_identifier: String,
    pub updated_at: String,
    pub cooldown_until: Option<String>,
    pub last_status: Option<String>,
    pub presence: TikTokCredentialPresence,
}

impl TikTokCredentialSummary {
    pub fn new(entry: TikTokCredentialIndexEntry, presence: TikTokCredentialPresence) -> Self {
        Self {
            handle: entry.handle,
            identifier_kind: entry.identifier_kind,
            masked_identifier: entry.masked_identifier,
            updated_at: entry.updated_at,
            cooldown_until: entry.cooldown_until,
            last_status: entry.last_status,
            presence,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_have_one_spelling() {
        assert_eq!(
            canonical_tiktok_handle(" @Riviu.Demo ").unwrap(),
            "riviu.demo"
        );
        assert_eq!(canonical_tiktok_handle("a_1").unwrap(), "a_1");
        for invalid in [
            "",
            "@",
            "a",
            "abcdefghijklmnopqrstuvwxy",
            "user.",
            "has space",
            "https://www.tiktok.com/@user",
            "@@double",
            "ünicode",
        ] {
            assert!(canonical_tiktok_handle(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn identifiers_are_classified_masked_and_never_echoed_in_errors() {
        let cases = [
            (
                " thuc.nhan@gmail.com ",
                TikTokIdentifierKind::Email,
                "th***@gmail.com",
            ),
            ("ab@x.vn", TikTokIdentifierKind::Email, "a***@x.vn"),
            ("+84 901-234-567", TikTokIdentifierKind::Phone, "+84***567"),
            ("0901234567", TikTokIdentifierKind::Phone, "09***567"),
            ("@Riviu.Demo", TikTokIdentifierKind::Username, "ri***o"),
            ("abc", TikTokIdentifierKind::Username, "a***"),
        ];
        for (raw, kind, masked) in cases {
            let (identifier, classified) = classify_tiktok_identifier(raw).expect(raw);
            assert_eq!(classified, kind, "{raw}");
            assert_eq!(mask_tiktok_identifier(&identifier, kind), masked, "{raw}");
        }
        assert_eq!(
            classify_tiktok_identifier("@Riviu.Demo").unwrap().0,
            "riviu.demo"
        );

        for invalid in ["", "   ", "no-at-domain@", "a@b", "a b@c.vn", "x", "user."] {
            let error = classify_tiktok_identifier(invalid).expect_err(invalid);
            if invalid.trim().len() >= 3 {
                assert!(!error.contains(invalid.trim()), "{error}");
            }
        }
    }
}
