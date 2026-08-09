//! Target-driven TikTok comment threads.
//!
//! This module owns the pure contracts used by the desktop Interaction surface.
//! Device driving stays behind `DeviceControlPlane`; no WDA or stream code lives
//! here so parser/planner behavior can be tested without an iPhone.

use std::collections::HashSet;

use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

const MAX_MESSAGE_COUNT: u8 = 6;
const MIN_MESSAGE_COUNT: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TikTokPostKind {
    Video,
    Photo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedTikTokTarget {
    pub original_url: String,
    pub normalized_url: String,
    pub target_key: String,
    pub content_id: String,
    pub author: String,
    pub kind: TikTokPostKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TikTokLinkLine {
    pub line_no: usize,
    pub original: String,
    pub target: Option<ResolvedTikTokTarget>,
    pub error: Option<LinkErrorCode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkErrorCode {
    Empty,
    InvalidUrl,
    UnsupportedScheme,
    UnsupportedHost,
    UserInfoNotAllowed,
    CustomPortNotAllowed,
    UnsupportedTargetKind,
    UnresolvedShortLink,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LinkParseError {
    #[error("line {line}: {code:?}")]
    Line { line: usize, code: LinkErrorCode },
}

/// Parse direct TikTok post URLs. Short links are surfaced as a typed preview
/// error until the injectable redirect resolver resolves them server-side.
pub fn parse_tiktok_links(raw: &str) -> Vec<TikTokLinkLine> {
    raw.lines()
        .enumerate()
        .filter_map(|(index, value)| {
            let original = value.trim().to_string();
            if original.is_empty() {
                return None;
            }
            let line_no = index + 1;
            match parse_one(&original) {
                Ok(target) => Some(TikTokLinkLine {
                    line_no,
                    original,
                    target: Some(target),
                    error: None,
                }),
                Err(code) => Some(TikTokLinkLine {
                    line_no,
                    original,
                    target: None,
                    error: Some(code),
                }),
            }
        })
        .collect()
}

fn parse_one(original: &str) -> Result<ResolvedTikTokTarget, LinkErrorCode> {
    let url = Url::parse(original).map_err(|_| LinkErrorCode::InvalidUrl)?;
    if url.scheme() != "https" {
        return Err(LinkErrorCode::UnsupportedScheme);
    }
    if url.username() != "" || url.password().is_some() {
        return Err(LinkErrorCode::UserInfoNotAllowed);
    }
    if url.port().is_some() {
        return Err(LinkErrorCode::CustomPortNotAllowed);
    }
    let host = url.host_str().ok_or(LinkErrorCode::UnsupportedHost)?;
    let host_is_tiktok = matches!(
        host,
        "tiktok.com" | "www.tiktok.com" | "m.tiktok.com" | "vm.tiktok.com" | "vt.tiktok.com"
    );
    if !host_is_tiktok {
        return Err(LinkErrorCode::UnsupportedHost);
    }

    let segments: Vec<_> = url
        .path_segments()
        .map(|parts| {
            parts
                .filter(|part| !part.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if matches!(host, "vt.tiktok.com" | "vm.tiktok.com")
        || segments.first().map(String::as_str) == Some("t")
    {
        return Err(LinkErrorCode::UnresolvedShortLink);
    }
    if segments.len() != 3 || !segments[0].starts_with('@') || segments[0].len() < 2 {
        return Err(LinkErrorCode::UnsupportedTargetKind);
    }
    let kind = match segments[1].as_str() {
        "video" => TikTokPostKind::Video,
        "photo" => TikTokPostKind::Photo,
        _ => return Err(LinkErrorCode::UnsupportedTargetKind),
    };
    if segments[2].is_empty() || !segments[2].bytes().all(|b| b.is_ascii_digit()) {
        return Err(LinkErrorCode::InvalidUrl);
    }

    let mut normalized = url;
    normalized.set_query(None);
    normalized.set_fragment(None);
    let normalized_url = normalized.to_string();
    let content_id = segments[2].clone();
    Ok(ResolvedTikTokTarget {
        original_url: original.to_string(),
        normalized_url,
        target_key: format!("content:{content_id}"),
        content_id,
        author: segments[0].trim_start_matches('@').to_string(),
        kind,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadCampaignRequest {
    pub request_id: String,
    pub targets: Vec<ResolvedTikTokTarget>,
    pub actor_udids: Vec<String>,
    pub message_count: u8,
    pub instruction: String,
    pub max_words: u8,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ThreadValidationError {
    #[error("request id is empty")]
    EmptyRequestId,
    #[error("at least one target is required")]
    NoTargets,
    #[error("two to six messages are required")]
    InvalidMessageCount,
    #[error("two to six distinct actors are required")]
    InvalidActorCount,
    #[error("message count must cover every selected actor")]
    TooFewMessagesForActors,
    #[error("duplicate actor")]
    DuplicateActor,
    #[error("duplicate target")]
    DuplicateTarget,
    #[error("comment length must be between four and twenty words")]
    InvalidMaxWords,
}

impl ThreadCampaignRequest {
    pub fn validate(&self) -> Result<(), ThreadValidationError> {
        if self.request_id.trim().is_empty() {
            return Err(ThreadValidationError::EmptyRequestId);
        }
        if self.targets.is_empty() {
            return Err(ThreadValidationError::NoTargets);
        }
        if !(MIN_MESSAGE_COUNT..=MAX_MESSAGE_COUNT).contains(&self.message_count) {
            return Err(ThreadValidationError::InvalidMessageCount);
        }
        if !(2..=6).contains(&self.actor_udids.len()) {
            return Err(ThreadValidationError::InvalidActorCount);
        }
        let mut actors = HashSet::new();
        if self
            .actor_udids
            .iter()
            .any(|udid| udid.trim().is_empty() || !actors.insert(udid))
        {
            return Err(ThreadValidationError::DuplicateActor);
        }
        let mut targets = HashSet::new();
        if self
            .targets
            .iter()
            .any(|target| !targets.insert(target.target_key.as_str()))
        {
            return Err(ThreadValidationError::DuplicateTarget);
        }
        if (self.message_count as usize) < self.actor_udids.len() {
            return Err(ThreadValidationError::TooFewMessagesForActors);
        }
        if !(4..=20).contains(&self.max_words) {
            return Err(ThreadValidationError::InvalidMaxWords);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadMessagePlan {
    pub target_key: String,
    pub ordinal: u8,
    pub actor_udid: String,
    pub parent_ordinal: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadPlan {
    pub request_id: String,
    pub assignments: Vec<ThreadMessagePlan>,
}

pub fn plan_threads(request: &ThreadCampaignRequest) -> Result<ThreadPlan, ThreadValidationError> {
    request.validate()?;
    let mut assignments =
        Vec::with_capacity(request.targets.len() * request.message_count as usize);
    for (target_index, target) in request.targets.iter().enumerate() {
        for ordinal in 0..request.message_count {
            let actor_index = (target_index + ordinal as usize) % request.actor_udids.len();
            assignments.push(ThreadMessagePlan {
                target_key: target.target_key.clone(),
                ordinal,
                actor_udid: request.actor_udids[actor_index].clone(),
                parent_ordinal: if ordinal == 0 {
                    None
                } else {
                    Some(ordinal - 1)
                },
            });
        }
    }
    Ok(ThreadPlan {
        request_id: request.request_id.clone(),
        assignments,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedThreadMessage {
    pub ordinal: u8,
    pub actor_udid: String,
    pub text: String,
    pub text_sha256: String,
    pub parent_ordinal: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSendEvidence {
    pub text_sha256: String,
    pub armed_frame_sha256: String,
    pub cleared_frame_sha256: String,
}

/// SHA-256 of a frame's exact bytes.
///
/// The two frame fields on [`ThreadSendEvidence`] used to be filled with
/// `nurture::frame_digest` — a 64-bit FNV-1a over roughly 512 *sampled* bytes,
/// which is a cheap "did this change?" fingerprint and nothing like a SHA-256.
/// The values went into the campaign record and into
/// `interaction_artifacts.sha256` under names that claimed otherwise, so
/// evidence nobody could verify also could not be recognised as unverifiable.
pub fn frame_sha256(frame: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(frame);
    format!("{:x}", digest.finalize())
}

impl PreparedThreadMessage {
    pub fn new(plan: &ThreadMessagePlan, text: impl Into<String>) -> Self {
        let text = normalize_comment_text(&text.into());
        let mut digest = Sha256::new();
        digest.update(text.as_bytes());
        let text_sha256 = format!("{:x}", digest.finalize());
        Self {
            ordinal: plan.ordinal,
            actor_udid: plan.actor_udid.clone(),
            text,
            text_sha256,
            parent_ordinal: plan.parent_ordinal,
        }
    }
}

pub fn normalize_comment_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentLocatorIdentity {
    pub author_label: String,
    pub text: String,
    pub locator_version: String,
    pub frame_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentOcrObservation {
    pub text: String,
    pub confidence: f32,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentParentMatch {
    pub identity: CommentLocatorIdentity,
    pub reply_x: f64,
    pub reply_y: f64,
}

/// Normalize OCR labels without losing Vietnamese diacritics. Requiring exact
/// normalized text is intentional: a fuzzy match could bind a reply to a
/// neighboring comment in a dense drawer.
/// Accented Latin letters folded to their base, so the locator survives an OCR
/// engine that cannot render tone marks.
///
/// This is not cosmetic. Which engine reads the screen depends on the language
/// packs the operating system has installed; measured on a Windows machine
/// carrying only `en-US`, "Trả lời" comes back as "Trå löi" and "Đà Lạt" as
/// "Dä Lat". Folding both sides makes those compare equal. The module already
/// carried the accent-free "tra loi" spelling by hand next to the accented one,
/// so the case was known — this generalises it past the one word that was
/// hard-coded.
///
/// It is a partial remedy and the limit is worth stating: folding rescues a
/// letter that lost its mark, not one the engine replaced outright. The same
/// capture read "thư" as "thif" and "mới" as "mdi", which no folding can
/// reconcile. Matching a Vietnamese comment body still needs the Vietnamese
/// pack installed; what this buys everywhere is the control labels and the
/// ASCII author handles.
///
/// Folding is only safe because a duplicated match is now refused rather than
/// resolved: two comments differing only in tone marks collide here, and
/// `locate_parent_comment` fails on the ambiguity instead of guessing.
/// Both the correct Vietnamese letters *and* the Latin accented letters an
/// engine substitutes for them, because the comparison has to survive either.
const LATIN_FOLD: &[(char, &str)] = &[
    ('a', "àáạảãâầấậẩẫăằắặẳẵäåāăą"),
    ('e', "èéẹẻẽêềếệểễëēĕėęě"),
    ('i', "ìíịỉĩïĩīĭįı"),
    ('o', "òóọỏõôồốộổỗơờớợởỡöøōŏő"),
    ('u', "ùúụủũưừứựửữüūŭůűų"),
    ('y', "ỳýỵỷỹÿŷ"),
    ('d', "đďð"),
    ('c', "çćĉċč"),
    ('n', "ñńņň"),
    ('s', "śŝşš"),
    ('t', "ţťŧ"),
    ('z', "źżž"),
    ('g', "ĝğġģ"),
    ('l', "ĺļľłŀ"),
    ('r', "ŕŗř"),
];

fn fold_latin(c: char) -> char {
    LATIN_FOLD
        .iter()
        .find(|(_, variants)| variants.chars().any(|variant| variant == c))
        .map(|(base, _)| *base)
        .unwrap_or(c)
}

pub fn normalize_locator_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .chars()
        .map(fold_latin)
        .collect()
}

pub fn locate_parent_comment(
    observations: &[CommentOcrObservation],
    identity: &CommentLocatorIdentity,
) -> Option<CommentParentMatch> {
    let wanted_author = normalize_locator_text(&identity.author_label);
    let wanted_text = normalize_locator_text(&identity.text);
    // The text must appear exactly once. Two lines reading the same thing —
    // a repeated campaign message, someone quoting it back — give no way to
    // tell which one is ours, and picking whichever OCR happened to list first
    // would anchor the whole reply to a stranger's comment.
    let mut matches_text = observations.iter().filter(|observation| {
        normalize_locator_text(&observation.text) == wanted_text && observation.confidence >= 0.55
    });
    let text = matches_text.next()?;
    if matches_text.next().is_some() {
        return None;
    }
    let author = observations.iter().find(|observation| {
        let label = normalize_locator_text(&observation.text);
        !wanted_author.is_empty()
            && label == wanted_author
            && observation.y <= text.y + 0.02
            && observation.y + observation.height >= text.y - 0.08
    })?;
    // Every comment carries its own "Trả lời", and the band below this one is
    // wide enough to reach the next comment's. Take the closest, not the first
    // the OCR happened to emit — OCR order is not screen order, so `find` here
    // could tap the reply control belonging to somebody else's comment and post
    // the campaign's reply underneath it.
    let text_bottom = text.y + text.height;
    let reply = observations
        .iter()
        .filter(|observation| {
            let label = normalize_locator_text(&observation.text);
            matches!(label.as_str(), "reply" | "tra loi")
                && observation.y >= text.y - 0.02
                && observation.y <= text_bottom + 0.12
                && observation.x >= text.x
        })
        .min_by(|a, b| {
            let da = (a.y - text_bottom).abs();
            let db = (b.y - text_bottom).abs();
            da.total_cmp(&db)
        })?;
    Some(CommentParentMatch {
        identity: CommentLocatorIdentity {
            author_label: author.text.clone(),
            text: text.text.clone(),
            locator_version: identity.locator_version.clone(),
            frame_sha256: identity.frame_sha256.clone(),
        },
        reply_x: reply.x + reply.width / 2.0,
        reply_y: reply.y + reply.height / 2.0,
    })
}

/// `locator_version` records which reader produced `observations`. It used to
/// be hard-coded `"vision-v1"`, which is only true on macOS — a Windows run
/// reads through `Windows.Media.Ocr`, whose output differs enough to matter
/// (no per-word confidence, and tone marks lost without the Vietnamese pack).
/// Stamping the wrong reader onto stored evidence makes a later mismatch
/// impossible to explain.
pub fn discover_comment_identity(
    observations: &[CommentOcrObservation],
    exact_text: &str,
    frame_sha256: &str,
    locator_version: &str,
) -> Option<CommentLocatorIdentity> {
    let text = observations.iter().find(|observation| {
        normalize_locator_text(&observation.text) == normalize_locator_text(exact_text)
            && observation.confidence >= 0.55
    })?;
    let author = observations.iter().find(|observation| {
        let label = normalize_locator_text(&observation.text);
        !label.is_empty()
            && observation.y <= text.y + 0.02
            && observation.y + observation.height >= text.y - 0.08
            && observation.x <= text.x + 0.1
            && !matches!(label.as_str(), "reply" | "tra loi")
    })?;
    Some(CommentLocatorIdentity {
        author_label: author.text.clone(),
        text: text.text.clone(),
        locator_version: locator_version.into(),
        frame_sha256: frame_sha256.into(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentArtifact {
    pub id: String,
    pub target_key: String,
    pub ordinal: u8,
    pub actor_udid: String,
    pub parent_artifact_id: Option<String>,
    pub normalized_text: String,
    pub text_sha256: String,
    pub identity: CommentLocatorIdentity,
    pub sent_at: String,
    pub screenshot_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThreadMessageState {
    Queued,
    Preparing,
    Ready,
    Sending,
    Succeeded,
    Failed,
    Uncertain,
    SkippedParent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThreadCampaignState {
    Queued,
    Running,
    Succeeded,
    Partial,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionCampaignSummary {
    pub id: String,
    pub request_id: String,
    pub state: ThreadCampaignState,
    pub message_count: u8,
    pub target_count: u32,
    pub succeeded_messages: u32,
    /// Messages that were meant to be posted and were not, for any reason —
    /// `failed`, `uncertain`, **and `skipped_parent`**.
    ///
    /// The last one used to be counted nowhere. A thread whose parent could not
    /// be identified skips every remaining message, so a six-message campaign
    /// could report "1 succeeded, 0 failed" while five were silently dropped.
    /// The per-assignment chip already distinguished them; only the total lied.
    pub failed_messages: u32,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionAssignmentRecord {
    pub id: String,
    pub target_key: String,
    pub ordinal: u8,
    pub actor_udid: String,
    pub parent_assignment_id: Option<String>,
    pub state: ThreadMessageState,
    pub prepared_text: Option<String>,
    pub error_code: Option<String>,
}

/// A saved frame from a thread campaign, and where it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionArtifactRecord {
    pub id: String,
    pub assignment_id: Option<String>,
    pub kind: String,
    pub relative_path: Option<String>,
    pub sha256: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionCampaignDetail {
    pub summary: InteractionCampaignSummary,
    pub assignments: Vec<InteractionAssignmentRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadPreview {
    pub lines: Vec<TikTokLinkLine>,
    pub plan: Option<ThreadPlan>,
    pub valid_target_count: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(id: &str) -> ResolvedTikTokTarget {
        ResolvedTikTokTarget {
            original_url: format!("https://www.tiktok.com/@creator/video/{id}"),
            normalized_url: format!("https://www.tiktok.com/@creator/video/{id}"),
            target_key: format!("content:{id}"),
            content_id: id.to_string(),
            author: "creator".to_string(),
            kind: TikTokPostKind::Video,
        }
    }

    fn request(
        targets: Vec<ResolvedTikTokTarget>,
        actors: Vec<&str>,
        count: u8,
    ) -> ThreadCampaignRequest {
        ThreadCampaignRequest {
            request_id: "req-1".into(),
            targets,
            actor_udids: actors.into_iter().map(str::to_string).collect(),
            message_count: count,
            instruction: "ngắn, tự nhiên".into(),
            max_words: 12,
        }
    }

    #[test]
    fn direct_video_and_photo_links_are_normalized_and_tracking_removed() {
        let lines = parse_tiktok_links(
            "https://www.tiktok.com/@creator/video/123?utm_source=x#fragment\nhttps://m.tiktok.com/@creator/photo/456",
        );
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].target.as_ref().unwrap().content_id, "123");
        assert_eq!(
            lines[0].target.as_ref().unwrap().kind,
            TikTokPostKind::Video
        );
        assert!(!lines[0]
            .target
            .as_ref()
            .unwrap()
            .normalized_url
            .contains("utm_"));
        assert_eq!(
            lines[1].target.as_ref().unwrap().kind,
            TikTokPostKind::Photo
        );
    }

    #[test]
    fn unsupported_hosts_paths_and_short_links_are_typed_per_line() {
        let lines = parse_tiktok_links(
            "http://www.tiktok.com/@a/video/1\nhttps://example.com/@a/video/1\nhttps://vt.tiktok.com/ZABC/\nhttps://www.tiktok.com/@a/live/1",
        );
        assert_eq!(lines[0].error, Some(LinkErrorCode::UnsupportedScheme));
        assert_eq!(lines[1].error, Some(LinkErrorCode::UnsupportedHost));
        assert_eq!(lines[2].error, Some(LinkErrorCode::UnresolvedShortLink));
        assert_eq!(lines[3].error, Some(LinkErrorCode::UnsupportedTargetKind));
    }

    #[test]
    fn rotation_changes_root_actor_per_target_and_links_parent_chain() {
        let request = request(vec![target("1"), target("2")], vec!["A", "B"], 4);
        let plan = plan_threads(&request).unwrap();
        let first: Vec<_> = plan.assignments[..4]
            .iter()
            .map(|a| a.actor_udid.as_str())
            .collect();
        let second: Vec<_> = plan.assignments[4..]
            .iter()
            .map(|a| a.actor_udid.as_str())
            .collect();
        assert_eq!(first, ["A", "B", "A", "B"]);
        assert_eq!(second, ["B", "A", "B", "A"]);
        assert_eq!(plan.assignments[0].parent_ordinal, None);
        assert_eq!(plan.assignments[3].parent_ordinal, Some(2));
    }

    #[test]
    fn validation_rejects_too_few_messages_or_duplicate_actors() {
        let too_few = request(vec![target("1")], vec!["A", "B", "C"], 2);
        assert_eq!(
            too_few.validate(),
            Err(ThreadValidationError::TooFewMessagesForActors)
        );
        let duplicate = request(vec![target("1")], vec!["A", "A"], 2);
        assert_eq!(
            duplicate.validate(),
            Err(ThreadValidationError::DuplicateActor)
        );
    }

    #[test]
    fn prepared_text_is_normalized_and_hashed_before_ui() {
        let request = request(vec![target("1")], vec!["A", "B"], 2);
        let plan = plan_threads(&request).unwrap();
        let message = PreparedThreadMessage::new(&plan.assignments[0], "  Quán   này   xinh quá  ");
        assert_eq!(message.text, "Quán này xinh quá");
        assert_eq!(message.text_sha256.len(), 64);
    }

    #[test]
    fn parent_locator_requires_exact_author_text_and_reply_control() {
        let observations = vec![
            CommentOcrObservation {
                text: "creator_a".into(),
                confidence: 0.98,
                x: 0.10,
                y: 0.30,
                width: 0.2,
                height: 0.03,
            },
            CommentOcrObservation {
                text: "Quán này xinh quá".into(),
                confidence: 0.94,
                x: 0.10,
                y: 0.34,
                width: 0.4,
                height: 0.04,
            },
            CommentOcrObservation {
                text: "Trả lời".into(),
                confidence: 0.91,
                x: 0.55,
                y: 0.38,
                width: 0.1,
                height: 0.03,
            },
        ];
        let identity = CommentLocatorIdentity {
            author_label: "creator_a".into(),
            text: " quán   này xinh quá ".into(),
            locator_version: "vision-v1".into(),
            frame_sha256: "frame".into(),
        };
        let match_ = locate_parent_comment(&observations, &identity).unwrap();
        assert_eq!(match_.identity.author_label, "creator_a");
        assert!(match_.reply_x > 0.5);
        assert!(locate_parent_comment(
            &observations,
            &CommentLocatorIdentity {
                author_label: "other".into(),
                ..identity
            }
        )
        .is_none());
    }

    /// Every comment carries its own "Trả lời", and the search band below the
    /// parent reaches the next comment when the two are packed close together.
    /// OCR order is not screen order, so taking the first match could tap the
    /// reply control belonging to somebody else's comment — and post the
    /// campaign's reply underneath a stranger.
    #[test]
    fn the_reply_control_taken_is_the_parent_own_not_the_next_comment() {
        let observations = vec![
            CommentOcrObservation {
                text: "creator_a".into(),
                confidence: 0.98,
                x: 0.10,
                y: 0.30,
                width: 0.2,
                height: 0.03,
            },
            CommentOcrObservation {
                text: "Quán này xinh quá".into(),
                confidence: 0.94,
                x: 0.10,
                y: 0.34,
                width: 0.4,
                height: 0.03,
            },
            // The *next* comment's reply control, emitted first by the OCR and
            // still inside the band, but further from the parent.
            CommentOcrObservation {
                text: "Trả lời".into(),
                confidence: 0.90,
                x: 0.55,
                y: 0.46,
                width: 0.1,
                height: 0.03,
            },
            // The parent's own, right beneath it.
            CommentOcrObservation {
                text: "Trả lời".into(),
                confidence: 0.90,
                x: 0.55,
                y: 0.375,
                width: 0.1,
                height: 0.03,
            },
        ];
        let identity = CommentLocatorIdentity {
            author_label: "creator_a".into(),
            text: "Quán này xinh quá".into(),
            locator_version: "vision-v1".into(),
            frame_sha256: "frame".into(),
        };

        let match_ = locate_parent_comment(&observations, &identity).expect("parent located");
        assert!(
            (match_.reply_y - 0.39).abs() < 0.02,
            "tapped the reply at {:.3}, which belongs to the comment below",
            match_.reply_y
        );
    }

    /// The locator has to survive OCR that cannot render tone marks.
    ///
    /// Which engine reads the screen depends on the operating system's
    /// installed language packs: a Windows machine with only the English pack
    /// reads "Trả lời" as "Trå löi". Folding both sides to their base letters is
    /// what makes the comparison work either way — and the module already
    /// carried the accent-free "tra loi" spelling by hand, so the case was
    /// known; this generalises it.
    #[test]
    fn locator_text_matches_whether_or_not_the_ocr_kept_the_tone_marks() {
        assert_eq!(normalize_locator_text("Trả lời"), "tra loi");
        assert_eq!(normalize_locator_text("Trå löi"), "tra loi");
        assert_eq!(
            normalize_locator_text("  Quán   NÀY  xinh quá "),
            normalize_locator_text("quan nay xinh qua")
        );
        assert_eq!(normalize_locator_text("Đà Lạt"), "da lat");
        assert_eq!(normalize_locator_text("Dä Lat"), "da lat");
        assert_eq!(normalize_locator_text("Café 123"), "cafe 123");
        // The limit, stated as a test so nobody assumes more: folding restores a
        // letter that lost its mark, not one the engine replaced. The same real
        // capture read "mới" as "mdi" and "thư" as "thif", and no amount of
        // folding reconciles those — a Vietnamese comment body still needs the
        // Vietnamese OCR pack.
        assert_ne!(
            normalize_locator_text("mdi"),
            normalize_locator_text("mới"),
            "a substituted letter is not a folding problem and must not silently \
             appear to match"
        );
    }

    /// Two lines reading the same thing give no way to tell which one is ours.
    /// A repeated campaign message does exactly that, and anchoring to the wrong
    /// one puts the whole rest of the thread under a stranger's comment.
    #[test]
    fn a_duplicated_comment_text_is_refused_rather_than_guessed() {
        let line = |y: f64| CommentOcrObservation {
            text: "Quán này xinh quá".into(),
            confidence: 0.94,
            x: 0.10,
            y,
            width: 0.4,
            height: 0.03,
        };
        let observations = vec![
            CommentOcrObservation {
                text: "creator_a".into(),
                confidence: 0.98,
                x: 0.10,
                y: 0.30,
                width: 0.2,
                height: 0.03,
            },
            line(0.34),
            line(0.60),
            CommentOcrObservation {
                text: "Trả lời".into(),
                confidence: 0.91,
                x: 0.55,
                y: 0.375,
                width: 0.1,
                height: 0.03,
            },
        ];
        let identity = CommentLocatorIdentity {
            author_label: "creator_a".into(),
            text: "Quán này xinh quá".into(),
            locator_version: "vision-v1".into(),
            frame_sha256: "frame".into(),
        };

        assert!(
            locate_parent_comment(&observations, &identity).is_none(),
            "two identical lines are ambiguous and must not be resolved by picking one"
        );
    }

    #[test]
    fn discovered_identity_uses_nearby_author_not_first_ocr_line() {
        let observations = vec![
            CommentOcrObservation {
                text: "Comments".into(),
                confidence: 0.99,
                x: 0.4,
                y: 0.05,
                width: 0.2,
                height: 0.03,
            },
            CommentOcrObservation {
                text: "actor_1".into(),
                confidence: 0.9,
                x: 0.1,
                y: 0.3,
                width: 0.2,
                height: 0.03,
            },
            CommentOcrObservation {
                text: "Món này đáng thử".into(),
                confidence: 0.9,
                x: 0.1,
                y: 0.34,
                width: 0.4,
                height: 0.04,
            },
        ];
        let identity =
            discover_comment_identity(&observations, "Món này đáng thử", "abc", "test-ocr")
                .unwrap();
        assert_eq!(identity.author_label, "actor_1");
        // The reader is recorded, not assumed: it used to be hard-coded
        // "vision-v1" even when a Windows run had read the frame.
        assert_eq!(identity.locator_version, "test-ocr");
    }
}
