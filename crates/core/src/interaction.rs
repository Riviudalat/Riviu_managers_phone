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

/// Ceiling on messages per link and on actors in one run.
///
/// Was six for both, which is what made a twenty-phone run impossible — the request
/// was refused before anything else could go wrong. Sixty-four is a guard rail rather
/// than a target: the real limit is how many phones are plugged in, and a number this
/// far above the fleet exists only so a typo cannot ask for eight thousand.
const MAX_MESSAGE_COUNT: u8 = 64;
const MIN_MESSAGE_COUNT: u8 = 2;
pub(crate) const MAX_ACTOR_COUNT: usize = 64;
/// Two, because one account replying to itself is not a conversation.
const MIN_ACTOR_COUNT: usize = 2;
/// A cohort has to be able to hold a conversation on its own, so it needs two.
const MIN_COHORT_SIZE: u8 = 2;

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

/// Whether the messages on a post answer each other or stand alone.
///
/// `Threaded` is the richer result and the expensive one: every reply after the
/// first has to find its parent comment on screen, which means reading that
/// comment's own text back with OCR. `Standalone` drops the chain — each
/// account leaves its own top-level comment — and with it the entire locator,
/// so it needs no OCR at all and runs anywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThreadMode {
    /// Reply chain: message N answers message N-1.
    #[default]
    Threaded,
    /// Independent top-level comments from each account.
    Standalone,
}

/// Whether the replies answer each other in a line, or all answer the first one.
///
/// A second axis, deliberately not folded into [`ThreadMode`]: that one says *whether*
/// the messages form a chain at all, this one says what shape the chain has. Collapsing
/// them would make `Standalone` and `Star` look like alternatives when they are not —
/// `Standalone` has no parents, `Star` has one parent shared by everybody.
///
/// **`Star` is what makes a fleet run parallel.** In `Chain`, message N cannot start
/// until N-1 has been posted *and read back*, so twenty accounts are twenty sequential
/// waits by construction. In `Star` every reply depends only on ordinal 0, so once the
/// root is up the rest are independent of each other — and a reply that fails must not
/// take its siblings with it (see `chain_broken_at` in the runner).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThreadShape {
    /// Message N answers message N-1. The behaviour every campaign had before this
    /// existed, and what `#[serde(default)]` gives a row persisted back then.
    #[default]
    Chain,
    /// Every message after the first answers message 0.
    Star,
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
    /// Defaults to `Threaded` so campaigns persisted before this existed still
    /// deserialise into the behaviour they were created with.
    #[serde(default)]
    pub mode: ThreadMode,
    /// Chain or star. Only read in [`ThreadMode::Threaded`]; `Standalone` has no parents
    /// to arrange. Defaults to `Chain`, which is what every stored campaign was.
    #[serde(default)]
    pub shape: ThreadShape,
    /// Split the actors into cohorts of this size, each cohort taking its own links.
    ///
    /// `None` means one cohort holding every actor — the behaviour this had before, where
    /// the whole fleet works the same link one phone at a time. Setting it is what lets
    /// twenty phones be six conversations happening at once instead of one long queue.
    ///
    /// The size is a *target*, not a promise: the remainder is spread across cohorts
    /// rather than left idle, so twenty actors at size three become 4,4,3,3,3,3 and every
    /// phone has work. See [`partition_actors`].
    #[serde(default)]
    pub cohort_size: Option<u8>,
    /// Comments written by the operator, used **instead of** the AI when non-empty.
    ///
    /// A pool rather than a fixed list per message: it is dealt out across
    /// (target, ordinal) by [`Self::manual_comment_for`], so ten links do not all receive
    /// the same first comment. `instruction` and `max_words` stay on the request and are
    /// simply unused in this mode — they are what a campaign switches back to.
    ///
    /// `#[serde(default)]` so every campaign persisted before this existed reads as an
    /// empty pool, which is the AI mode it was created with.
    #[serde(default)]
    pub manual_comments: Vec<String>,
    /// Also like each target, once per actor that comments on it.
    ///
    /// Off by default, and off is also what a stored campaign from before this reads as.
    #[serde(default)]
    pub like_target: bool,
    /// @-handles to tag at the front of every comment, without the leading `@`.
    ///
    /// Inserted as **plain text** (`@name`), which TikTok does not turn into a linked
    /// mention or a notification — matching what a person typing the same characters gets.
    /// A handle that belongs to a fleet phone is *also* added to `actor_udids` by the caller,
    /// so tagging an owned account brings that phone into the same post to reply; a handle
    /// that matches no phone is tagged in text only. Empty (the default, and what a campaign
    /// stored before this reads as) prepends nothing.
    #[serde(default)]
    pub mentions: Vec<String>,
    /// Whether each reply tags the account it is replying to.
    ///
    /// The fleet talking to itself: with this on, a reply opens with the `@handle` of the
    /// phone whose comment it answers, which is what a real thread between people looks like
    /// and what makes the exchange visible to anyone reading the post. `Chain` therefore tags
    /// down the line — máy 2 tags máy 1, máy 4 tags máy 2 — and `Star`, where every reply
    /// answers ordinal 0, has them all tag the account that opened.
    ///
    /// The handle comes from each phone's own `device_meta.handle`, so a phone whose handle
    /// the operator never filled in is simply not tagged rather than tagged with a guess.
    /// `Standalone` has no parent to tag and is unaffected.
    ///
    /// Off by default, which is what every campaign stored before this reads as.
    #[serde(default)]
    pub mention_parent: bool,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ThreadValidationError {
    #[error("request id is empty")]
    EmptyRequestId,
    #[error("at least one target is required")]
    NoTargets,
    #[error("message count must be between two and sixty-four")]
    InvalidMessageCount,
    #[error("actor count must be between two and sixty-four, and every actor distinct")]
    InvalidActorCount,
    #[error("a cohort needs at least two actors")]
    InvalidCohortSize,
    #[error("message count must cover every selected actor")]
    TooFewMessagesForActors,
    #[error("duplicate actor")]
    DuplicateActor,
    #[error("duplicate target")]
    DuplicateTarget,
    #[error("comment length must be between four and twenty words")]
    InvalidMaxWords,
    #[error("a manual comment is empty")]
    EmptyManualComment,
    #[error("manual mode needs at least as many comments as there are messages")]
    TooFewManualComments,
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
        if !(MIN_ACTOR_COUNT..=MAX_ACTOR_COUNT).contains(&self.actor_udids.len()) {
            return Err(ThreadValidationError::InvalidActorCount);
        }
        if self.cohort_size.is_some_and(|size| size < MIN_COHORT_SIZE) {
            return Err(ThreadValidationError::InvalidCohortSize);
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
        // **Per cohort, not per fleet.** The rule is that every actor gets a turn, and a
        // cohort is where turns are taken: twenty phones in cohorts of three need three
        // messages a link, not twenty. Measured against the *largest* cohort, because the
        // remainder makes them uneven by one and the biggest is the one that must fit.
        let largest_cohort = partition_actors(&self.actor_udids, self.cohort_size)
            .iter()
            .map(Vec::len)
            .max()
            .unwrap_or(0);
        if (self.message_count as usize) < largest_cohort {
            return Err(ThreadValidationError::TooFewMessagesForActors);
        }
        if !(4..=20).contains(&self.max_words) {
            return Err(ThreadValidationError::InvalidMaxWords);
        }
        if self.is_manual() {
            if self
                .manual_comments
                .iter()
                .any(|text| text.trim().is_empty())
            {
                return Err(ThreadValidationError::EmptyManualComment);
            }
            // A chain shorter than the pool is fine; a pool shorter than the chain is not.
            // Threaded means message N answers N-1, so a pool of two on a chain of three
            // would have an account reply to a comment word-for-word identical to its own.
            if self.manual_comments.len() < self.message_count as usize {
                return Err(ThreadValidationError::TooFewManualComments);
            }
        }
        Ok(())
    }

    /// Whether the operator's own comments are in use rather than the AI.
    ///
    /// One question, one place, because it decides both validation and which text the runner
    /// asks for — and those two disagreeing is how a campaign gets validated as manual and
    /// then run as AI.
    pub fn is_manual(&self) -> bool {
        !self.manual_comments.is_empty()
    }

    /// Which of the operator's comments this message uses.
    ///
    /// Walks the pool across targets as well as ordinals, so the first comment on link two
    /// is not the first comment on link one. Deterministic on purpose: the same campaign
    /// replayed sends the same text, which is what makes the stored evidence checkable.
    ///
    /// Returns `None` in AI mode so a caller cannot silently get an empty string.
    pub fn manual_comment_for(&self, target_index: usize, ordinal: u8) -> Option<&str> {
        if self.manual_comments.is_empty() {
            return None;
        }
        let stride = self.message_count.max(1) as usize;
        let index =
            (target_index.wrapping_mul(stride) + ordinal as usize) % self.manual_comments.len();
        self.manual_comments.get(index).map(String::as_str)
    }

    /// Whether the run has to open each target once up front just to photograph it.
    ///
    /// Only the AI needs that: it writes from frames of the post. A manual pool covers
    /// **every** `(target, ordinal)` — [`Self::validate`] refuses a pool shorter than the
    /// chain — so in manual mode those frames are never read.
    ///
    /// This is not a micro-optimisation, it is a correctness fix, and the mechanism is
    /// worth stating because it is not obvious. The evidence pass opens the target on the
    /// *same phone* that ordinal 0 will use, and nothing navigates that phone away in
    /// between: tearing down the context stops the stream and invalidates the session, but
    /// the next one merely resumes the app. So by the time ordinal 0 runs its own arrival
    /// check, the post already on screen **is** the target — and the check's whole
    /// signal is that the post *changed*. It therefore refuses, deterministically, every
    /// ordinal 0 of every target, and blames the link. Measured on 13/08/2026: the Redmi
    /// was refused `target_open_screen_unchanged` on the exact link the Note 8 commented
    /// on successfully minutes later.
    pub fn needs_ai_evidence_frames(&self) -> bool {
        !self.is_manual()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadMessagePlan {
    pub target_key: String,
    pub ordinal: u8,
    pub actor_udid: String,
    pub parent_ordinal: Option<u8>,
    /// Which cohort owns this message, and therefore which task will run it.
    ///
    /// On the plan item rather than in a separate map because the runner's only question
    /// is "what does cohort N have to do", and a flat list it has to re-group is a list it
    /// can re-group wrongly. Zero for every message when no cohort size was asked for.
    #[serde(default)]
    pub cohort: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadPlan {
    pub request_id: String,
    pub assignments: Vec<ThreadMessagePlan>,
}

/// Split the actors into cohorts, spreading the remainder rather than stranding it.
///
/// `None` returns one cohort holding everybody, which is the arrangement this feature had
/// before cohorts existed: the whole selection works the same link, one phone at a time.
///
/// **The remainder is spread, not dropped and not left as a runt.** Twenty actors at size
/// three is six cohorts of 4,4,3,3,3,3 — not six of three with two phones idle, and not
/// five of three plus a cohort of five. A phone that was selected is a phone the operator
/// expects to work, and a cohort far larger than the others finishes long after the rest
/// and turns a parallel run back into a queue.
///
/// Pure, and separate from planning, because the partition is the thing worth asserting on
/// its own: every actor appears exactly once, no cohort is empty, and the sizes differ by
/// at most one.
pub fn partition_actors(actors: &[String], cohort_size: Option<u8>) -> Vec<Vec<String>> {
    let Some(size) = cohort_size.map(usize::from).filter(|size| *size > 0) else {
        return vec![actors.to_vec()];
    };
    // A cohort larger than the selection is one cohort, not an error: asking for teams of
    // five out of three phones plainly means "all of them together".
    let cohorts = (actors.len() / size).max(1);
    let base = actors.len() / cohorts;
    let mut remainder = actors.len() % cohorts;
    let mut out = Vec::with_capacity(cohorts);
    let mut rest = actors;
    for _ in 0..cohorts {
        let mut take = base;
        if remainder > 0 {
            take += 1;
            remainder -= 1;
        }
        let (head, tail) = rest.split_at(take.min(rest.len()));
        out.push(head.to_vec());
        rest = tail;
    }
    out
}

/// Expand a request into the work list, cohort by cohort.
///
/// The links are dealt round-robin across cohorts, so cohort 0 takes link 0, link C,
/// link 2C… That matters more than it looks: dealing them in blocks instead would give the
/// first cohort every early link, and a run cancelled part way would have covered a few
/// links thoroughly and the rest not at all.
///
/// Inside a cohort nothing changed. The actor still rotates by
/// `(target_index + ordinal) % cohort_len` — with `target_index` counted *within the
/// cohort*, so the rotation keeps doing what it was for: giving link two a different root
/// actor than link one.
pub fn plan_threads(request: &ThreadCampaignRequest) -> Result<ThreadPlan, ThreadValidationError> {
    request.validate()?;
    let cohorts = partition_actors(&request.actor_udids, request.cohort_size);
    let mut assignments =
        Vec::with_capacity(request.targets.len() * request.message_count as usize);
    for (cohort_index, cohort) in cohorts.iter().enumerate() {
        if cohort.is_empty() {
            continue;
        }
        let mine = request
            .targets
            .iter()
            .enumerate()
            .filter(|(index, _)| index % cohorts.len() == cohort_index);
        for (local_index, (_, target)) in mine.enumerate() {
            for ordinal in 0..request.message_count {
                let actor_index = (local_index + ordinal as usize) % cohort.len();
                assignments.push(ThreadMessagePlan {
                    target_key: target.target_key.clone(),
                    ordinal,
                    actor_udid: cohort[actor_index].clone(),
                    // Standalone leaves every message parentless, which is the
                    // whole of the difference: no parent means no locator, no OCR,
                    // and no chain to break. Star gives them all the same parent,
                    // which is what makes them independent of each other.
                    parent_ordinal: match (request.mode, request.shape, ordinal) {
                        (ThreadMode::Standalone, _, _) => None,
                        (ThreadMode::Threaded, _, 0) => None,
                        (ThreadMode::Threaded, ThreadShape::Chain, ordinal) => Some(ordinal - 1),
                        (ThreadMode::Threaded, ThreadShape::Star, _) => Some(0),
                    },
                    cohort: cohort_index as u16,
                });
            }
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
    /// Handles to tag on this message, without the `@`.
    ///
    /// Carried beside the text rather than baked into it, because the two backends can do
    /// genuinely different things with them: the hierarchy driver types the body and then
    /// picks each handle out of TikTok's suggestion list, which produces a **real** mention;
    /// the pixel driver has no way to reach that list and prepends them as literal text, the
    /// way both used to. Baking the prefix into `text` would have forced the second behaviour
    /// on both, and it also puts the handles into `text_sha256`, which is the digest the
    /// evidence checks a delivered comment against.
    ///
    /// `#[serde(default)]` so a message prepared before this reads as "no tags".
    #[serde(default)]
    pub mentions: Vec<String>,
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
        Self::with_mentions(plan, text, Vec::new())
    }

    pub fn with_mentions(
        plan: &ThreadMessagePlan,
        text: impl Into<String>,
        mentions: Vec<String>,
    ) -> Self {
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
            mentions,
        }
    }

    /// The handles as literal text, for a backend that cannot reach the suggestion list.
    pub fn literal_mention_prefix(&self) -> String {
        if self.mentions.is_empty() {
            return String::new();
        }
        format!(
            "{} ",
            self.mentions
                .iter()
                .map(|handle| format!("@{}", handle.trim().trim_start_matches('@')))
                .collect::<Vec<_>>()
                .join(" ")
        )
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
    // Same uniqueness rule `locate_parent_comment` applies. It was missing
    // here, which quietly undid the safety argument for accent folding: two
    // lines that fold to the same string were refused when locating the parent
    // and silently resolved to whichever OCR listed first when discovering it —
    // and this is the end that decides what the *rest of the thread* replies to.
    let mut matches_text = observations.iter().filter(|observation| {
        normalize_locator_text(&observation.text) == normalize_locator_text(exact_text)
            && observation.confidence >= 0.55
    });
    let text = matches_text.next()?;
    if matches_text.next().is_some() {
        return None;
    }
    let text_index = observations
        .iter()
        .position(|observation| std::ptr::eq(observation, text))?;
    let author = observations
        .iter()
        .enumerate()
        .find(|(index, observation)| {
            let label = normalize_locator_text(&observation.text);
            // The comment line satisfies every predicate below against itself:
            // its own `y` is within 0.02 of its own `y`, its own `x` within 0.1
            // of its own `x`, and it is neither empty nor a reply label. So if
            // OCR emitted the body before the author line, the author label
            // became the comment text — and that wrong identity is what the
            // next message in the thread then hunts for.
            *index != text_index
                && !label.is_empty()
                && observation.y <= text.y + 0.02
                && observation.y + observation.height >= text.y - 0.08
                && observation.x <= text.x + 0.1
                && !matches!(label.as_str(), "reply" | "tra loi")
        })
        .map(|(_, observation)| observation)?;
    Some(CommentLocatorIdentity {
        author_label: author.text.clone(),
        text: text.text.clone(),
        locator_version: locator_version.into(),
        frame_sha256: frame_sha256.into(),
    })
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
    /// Why the campaign ended, when something ended it.
    ///
    /// Selected and rendered rather than only written. It was stored from the start and
    /// read by nobody: a live AI failure on 13/08/2026 put the whole reason in this
    /// column and the operator's only signal was the word "Lỗi".
    pub error_code: Option<String>,
    pub updated_at: String,
    /// What this campaign actually was, for a human reading a list of them.
    ///
    /// The Monitor tab had nothing to name a row with but `requestId.slice(0, 14)` — a UUID
    /// fragment — so seven campaigns against three different posts were seven
    /// indistinguishable rows, and finding the one you just launched meant guessing.
    /// Everything here already existed in `request_json`; none of it was ever read back.
    ///
    /// Derived at read time rather than denormalised into columns: the request blob is the
    /// record, and a copy of it in columns is a second version that can drift from the one
    /// the campaign actually ran with. `None` when the blob will not parse, which must stay
    /// survivable — a corrupt request is a reason to show a row plainly, not to hide it.
    #[serde(default)]
    pub brief: Option<InteractionCampaignBrief>,
}

/// The human-readable shape of a campaign, read back out of its stored request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionCampaignBrief {
    /// Author of the first link, e.g. `.lt.gi.mang.v`.
    pub first_author: Option<String>,
    /// Content id of the first link, so two campaigns against one author are still distinct.
    pub first_content_id: Option<String>,
    pub mode: ThreadMode,
    pub shape: ThreadShape,
    pub cohort_size: Option<u8>,
    pub actor_count: u32,
    /// Whether the operator wrote the comments, rather than the AI.
    pub manual: bool,
    pub like_target: bool,
}

impl InteractionCampaignBrief {
    /// Read a brief out of the request a campaign was created with.
    pub fn from_request(request: &ThreadCampaignRequest) -> Self {
        let first = request.targets.first();
        Self {
            first_author: first.map(|target| target.author.clone()),
            first_content_id: first.map(|target| target.content_id.clone()),
            mode: request.mode,
            shape: request.shape,
            cohort_size: request.cohort_size,
            actor_count: request.actor_udids.len() as u32,
            manual: request.is_manual(),
            like_target: request.like_target,
        }
    }
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
    /// The evidence blob this message was stored with, or `None` if it never sent.
    ///
    /// Read back for one reason: it is where a succeeded root's
    /// [`CommentLocatorIdentity`] lives, under `postedIdentity`. Without it a retry has no
    /// way to learn what the parent comment was — the identity is only ever produced by
    /// *sending*, and a root that already succeeded is deliberately not sent again. The
    /// retry then skipped every reply under it with `parent_identity_not_confirmed`, which
    /// is precisely the case Retry exists for.
    ///
    /// `#[serde(default)]` so a payload from before this reads as absent rather than
    /// failing to parse.
    ///
    /// **Not sent to the desktop.** It carries frame hashes and locator internals the UI has
    /// no use for; what the UI needs is [`Self::like_note`], which is a field of its own.
    #[serde(skip)]
    pub evidence_json: Option<String>,
    /// What happened to the like, for the operator to read. Derived from the evidence when
    /// the record is loaded — see [`Self::like_note`].
    #[serde(default)]
    pub like: Option<String>,
    /// What happened to the `@` tags — see [`Self::mention_note`]. Same shape, same reason.
    #[serde(default)]
    pub mention: Option<String>,
}

impl InteractionAssignmentRecord {
    /// The comment this message posted, if it posted one and said so.
    ///
    /// Lives here rather than in the runner because both the runner and any future reader
    /// of a stored campaign want the same answer, and two parsers of the same blob are two
    /// chances to disagree about what `postedIdentity` means.
    /// What happened to the like on this message, if one was asked for.
    ///
    /// A like that fails must not cost the comment, so it is not an error code — but it was
    /// previously not *anything*: the outcome went to the log and the operator watching the
    /// Monitor tab saw a message succeed with no hint that the like had been refused.
    pub fn like_note(&self) -> Option<String> {
        serde_json::from_str::<serde_json::Value>(self.evidence_json.as_deref()?)
            .ok()?
            .get("like")?
            .as_str()
            .map(str::to_string)
    }

    /// What happened to the `@` tags on this message, if it carried any.
    ///
    /// Read out of the evidence the same way the like note is: a tag TikTok never offered was
    /// typed as plain text, which posts fine and notifies nobody, and the difference is
    /// invisible in the comment itself.
    pub fn mention_note(&self) -> Option<String> {
        serde_json::from_str::<serde_json::Value>(self.evidence_json.as_deref()?)
            .ok()?
            .get("mention")?
            .as_str()
            .map(str::to_string)
    }

    pub fn posted_identity(&self) -> Option<CommentLocatorIdentity> {
        serde_json::from_str::<serde_json::Value>(self.evidence_json.as_deref()?)
            .ok()?
            .get("postedIdentity")
            .and_then(|value| serde_json::from_value(value.clone()).ok())
    }
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

/// What the desktop learned about one target from outside the phones, ready to be read.
///
/// **This type exists because of §9.103 §4.** `nurture_list_comment_attempts` was registered
/// and allowlisted for weeks while `api.ts` never called it, so the only way to audit a
/// comment was a log dump — and a number nobody reads cannot be checked. The web lookup writes
/// its findings to `interaction_targets.context_json`; without something like this that column
/// is the same dead end.
///
/// It is a **projection**, not the stored row: the column holds the caption in full and this
/// carries its length and a preview. The length is the measurement (the accessibility tree
/// truncates, so "105 fetched" against "76 on screen" is the whole point), and the caption
/// itself is already on TikTok.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionTargetNote {
    pub target_key: String,
    pub line_no: u32,
    pub normalized_url: String,
    pub kind: TikTokPostKind,
    /// Characters of caption the lookup returned. `None` means no caption came back.
    pub caption_chars: Option<u32>,
    /// The opening of it, bounded — enough to recognise which post this is.
    pub caption_preview: Option<String>,
    pub duration_secs: Option<u64>,
    /// Slides the post has, per the web. `None` for a video, and for a lookup that failed.
    pub slide_count: Option<u32>,
    /// Whether the post carries speech. `Some(false)` is why no transcript was even asked for.
    pub has_original_audio: Option<bool>,
    pub subtitle_langs: Vec<String>,
    /// The track a transcript would come from, as `lang/source` — `vie-VN/ASR` is the original
    /// speech, `eng-US/MT` a machine translation of it.
    pub transcript_track: Option<String>,
    /// The lookup's failure code, when it produced nothing: `ip_blocked`, `post_unavailable`,
    /// `transient`, `no_ytdlp`. **`None` here with everything else empty means nobody looked**,
    /// which is a different thing and the reason this field exists.
    pub error_code: Option<String>,
    pub error_detail: Option<String>,
}

/// How much caption travels to the panel.
const NOTE_CAPTION_PREVIEW_CHARS: usize = 64;

impl InteractionTargetNote {
    /// The JSON that `interaction_targets.context_json` stores for one lookup outcome.
    ///
    /// **Deliberately the sibling of [`Self::from_row`].** These are the write and the read half
    /// of one column, and halves that live in different modules drift — nothing type-checks a
    /// JSON key against the code that reads it. Keeping them adjacent is what makes the
    /// round-trip test below possible, and that test is the only thing standing between a
    /// renamed key and a panel that silently shows nothing.
    ///
    /// **A refused lookup is filed too.** "No caption because TikTok blocks this address" and
    /// "no caption because nobody looked" are the same empty column otherwise, and only one of
    /// them is worth an operator's attention.
    pub fn context_json(
        outcome: Result<&crate::tiktok_web::PostWebContext, &crate::tiktok_web::WebLookupError>,
    ) -> Option<String> {
        let note = match outcome {
            Ok(context) => serde_json::json!({
                "web": {
                    "captionChars": context.caption.as_ref().map(|caption| caption.chars().count()),
                    "caption": context.caption,
                    "durationSecs": context.duration_secs,
                    "slideCount": context.slide_urls.len(),
                    "hasOriginalAudio": context.has_original_audio,
                    "subtitleLangs": context.subtitle_langs(),
                    "transcriptTrack": context.transcript_track().map(|track| {
                        serde_json::json!({ "lang": track.lang, "source": track.source })
                    }),
                }
            }),
            Err(error) => serde_json::json!({
                "web": { "error": error.code(), "detail": error.to_string() }
            }),
        };
        serde_json::to_string(&note).ok()
    }

    /// Build a note from one `interaction_targets` row.
    ///
    /// Pure, and separate from the query, so the shape of `context_json` can be tested against
    /// captured JSON instead of against a database. That column is written by
    /// `interaction_campaign::file_target_context` and read only here, so the two have to be
    /// checked against each other somewhere.
    pub fn from_row(
        target_key: String,
        line_no: u32,
        normalized_url: String,
        kind: TikTokPostKind,
        context_json: Option<&str>,
    ) -> Self {
        let mut note = Self {
            target_key,
            line_no,
            normalized_url,
            kind,
            caption_chars: None,
            caption_preview: None,
            duration_secs: None,
            slide_count: None,
            has_original_audio: None,
            subtitle_langs: Vec::new(),
            transcript_track: None,
            error_code: None,
            error_detail: None,
        };
        let Some(web) = context_json
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .and_then(|value| value.get("web").cloned())
        else {
            return note;
        };
        note.error_code = web
            .get("error")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        note.error_detail = web
            .get("detail")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        note.caption_chars = web
            .get("captionChars")
            .and_then(|value| value.as_u64())
            .map(|chars| chars as u32);
        note.caption_preview = web
            .get("caption")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|caption| !caption.is_empty())
            .map(|caption| caption.chars().take(NOTE_CAPTION_PREVIEW_CHARS).collect());
        note.duration_secs = web.get("durationSecs").and_then(|value| value.as_u64());
        // Zero slides is a video, not "a carousel with none" — reporting `0` would put a
        // meaningless number in the panel next to every video.
        note.slide_count = web
            .get("slideCount")
            .and_then(|value| value.as_u64())
            .filter(|count| *count > 0)
            .map(|count| count as u32);
        note.has_original_audio = web
            .get("hasOriginalAudio")
            .and_then(|value| value.as_bool());
        note.subtitle_langs = web
            .get("subtitleLangs")
            .and_then(|value| value.as_array())
            .map(|langs| {
                langs
                    .iter()
                    .filter_map(|lang| lang.as_str())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        note.transcript_track = web.get("transcriptTrack").and_then(|track| {
            let lang = track.get("lang").and_then(|value| value.as_str())?;
            let source = track.get("source").and_then(|value| value.as_str())?;
            Some(format!("{lang}/{source}"))
        });
        note
    }

    /// Whether the lookup told us anything at all.
    ///
    /// Read by the panel to distinguish the three states an operator cares about: a target that
    /// was enriched, one whose lookup was refused (`error_code`), and one nobody looked up.
    pub fn is_blank(&self) -> bool {
        self.caption_chars.is_none()
            && self.slide_count.is_none()
            && self.error_code.is_none()
            && self.subtitle_langs.is_empty()
    }
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
    /// How many cohorts this plan would run at once — `partition_actors(..).len()`.
    ///
    /// The desktop used to compute this itself, in TypeScript, by reimplementing
    /// [`partition_actors`] including its remainder-spreading. Two implementations of one
    /// split is two chances to disagree about what the operator is about to launch.
    #[serde(default)]
    pub cohort_count: u32,
    /// How many device streams the whole app can hold open at once.
    ///
    /// Paired with `cohort_count` because exceeding it is a **refusal, not a queue**:
    /// `preview_foreground_victim` answers `CapacityExhausted` and the assignment is marked
    /// Failed. Eight cohorts against a budget of four does not run slowly, it fails half the
    /// campaign — and the only moment that is cheap to learn is before pressing start.
    #[serde(default)]
    pub stream_capacity: u32,
}

#[cfg(test)]
mod tests {
    /// The two payloads the desktop reads field-by-field, pinned by name.
    ///
    /// TypeScript cannot check a Rust struct, so a renamed or dropped field here shows up as
    /// `undefined` in the UI and nowhere else. `ThreadPlanAssignment` on the desktop side had
    /// already drifted this way: the planner emitted `cohort` and the TS type never declared
    /// it, so a cohort could not be displayed even though it was on the wire.
    mod wire_shape {
        use super::super::*;

        #[test]
        fn the_preview_wire_shape_is_what_the_frontend_types_say() {
            let preview = ThreadPreview {
                lines: Vec::new(),
                plan: Some(ThreadPlan {
                    request_id: "r".into(),
                    assignments: vec![ThreadMessagePlan {
                        target_key: "content:1".into(),
                        ordinal: 1,
                        actor_udid: "udid".into(),
                        parent_ordinal: Some(0),
                        cohort: 2,
                    }],
                }),
                valid_target_count: 1,
                cohort_count: 3,
                stream_capacity: 4,
            };
            let json = serde_json::to_value(&preview).expect("serialise preview");
            for key in [
                "lines",
                "plan",
                "validTargetCount",
                "cohortCount",
                "streamCapacity",
            ] {
                assert!(json.get(key).is_some(), "preview lost `{key}`");
            }
            let assignment = &json["plan"]["assignments"][0];
            for key in [
                "targetKey",
                "ordinal",
                "actorUdid",
                "parentOrdinal",
                "cohort",
            ] {
                assert!(
                    assignment.get(key).is_some(),
                    "plan assignment lost `{key}`"
                );
            }
        }

        #[test]
        fn a_brief_names_the_campaign_by_its_first_link() {
            let mut request = ThreadCampaignRequest {
                request_id: "r".into(),
                targets: vec![super::target("7668947001618320660")],
                actor_udids: vec!["one".into(), "two".into(), "three".into()],
                message_count: 3,
                instruction: "tự nhiên".into(),
                max_words: 12,
                mode: ThreadMode::Threaded,
                shape: ThreadShape::Star,
                cohort_size: Some(3),
                manual_comments: Vec::new(),
                like_target: true,
                mentions: Vec::new(),
                mention_parent: false,
            };
            let brief = InteractionCampaignBrief::from_request(&request);
            assert_eq!(
                brief.first_content_id.as_deref(),
                Some("7668947001618320660")
            );
            assert_eq!(brief.actor_count, 3);
            assert_eq!(brief.shape, ThreadShape::Star);
            assert!(brief.like_target);
            assert!(!brief.manual, "an empty pool means the AI writes");

            request.manual_comments = vec!["đẹp quá".into(), "ở đâu vậy ạ".into(), "lưu".into()];
            assert!(InteractionCampaignBrief::from_request(&request).manual);

            let json = serde_json::to_value(InteractionCampaignBrief::from_request(&request))
                .expect("serialise brief");
            for key in [
                "firstAuthor",
                "firstContentId",
                "mode",
                "shape",
                "cohortSize",
                "actorCount",
                "manual",
                "likeTarget",
            ] {
                assert!(json.get(key).is_some(), "brief lost `{key}`");
            }
        }
    }

    // Manual comments and the like flag. Both are additive to a persisted request, so the
    // round-trip of an old payload is asserted too — a stored campaign must keep behaving the
    // way it was created.
    mod operator_written_comments {
        use super::super::*;

        fn request(pool: Vec<&str>) -> ThreadCampaignRequest {
            ThreadCampaignRequest {
                request_id: "r".into(),
                targets: vec![super::target("1"), super::target("2")],
                actor_udids: vec!["one".into(), "two".into()],
                message_count: 2,
                instruction: "tự nhiên".into(),
                max_words: 12,
                mode: ThreadMode::Threaded,
                shape: ThreadShape::Chain,
                cohort_size: None,
                manual_comments: pool.into_iter().map(str::to_string).collect(),
                like_target: false,
                mentions: Vec::new(),
                mention_parent: false,
            }
        }

        #[test]
        fn an_empty_pool_means_the_ai_writes() {
            let ai = request(vec![]);
            assert!(!ai.is_manual());
            assert_eq!(ai.manual_comment_for(0, 0), None);
            assert!(ai.validate().is_ok());
        }

        #[test]
        fn the_pool_is_dealt_across_links_not_repeated_from_the_top() {
            // The property that matters for a ten-link campaign: link two must not open with
            // the same sentence as link one, or every target gets an identical thread.
            let manual = request(vec!["một", "hai", "ba", "bốn"]);
            assert!(manual.is_manual());
            assert_eq!(manual.manual_comment_for(0, 0), Some("một"));
            assert_eq!(manual.manual_comment_for(0, 1), Some("hai"));
            assert_eq!(manual.manual_comment_for(1, 0), Some("ba"));
            assert_eq!(manual.manual_comment_for(1, 1), Some("bốn"));
            // And it wraps rather than running out, deterministically — a replay of the same
            // campaign sends the same text, which is what makes the stored evidence checkable.
            assert_eq!(manual.manual_comment_for(2, 0), Some("một"));
            assert_eq!(
                manual.manual_comment_for(2, 0),
                manual.manual_comment_for(2, 0)
            );
        }

        #[test]
        fn a_manual_pool_covers_every_message_so_no_ai_evidence_frame_is_needed() {
            // The two halves are one claim: because the pool covers every (target,
            // ordinal), the frames the AI would need are never read — so opening each
            // target to photograph it is pure cost. And that cost is not neutral: the
            // evidence open lands on ordinal 0's own phone and makes its arrival check
            // refuse. See `needs_ai_evidence_frames`.
            let manual = request(vec!["một", "hai", "ba", "bốn", "năm", "sáu"]);
            assert!(!manual.needs_ai_evidence_frames());
            for target_index in 0..3 {
                for ordinal in 0..manual.message_count {
                    assert!(
                        manual.manual_comment_for(target_index, ordinal).is_some(),
                        "manual mode must cover ({target_index}, {ordinal})"
                    );
                }
            }
        }

        #[test]
        fn an_ai_campaign_still_declares_that_it_needs_evidence_frames() {
            // Guards against "simplifying" this into always skipping: the AI writes from
            // frames of the post, so for it the open is the whole point.
            let ai = request(vec![]);
            assert!(ai.needs_ai_evidence_frames());
            assert_eq!(ai.manual_comment_for(0, 0), None);
        }

        #[test]
        fn a_pool_smaller_than_the_chain_is_refused() {
            // Threaded means message N answers N-1. With two messages and one comment, an
            // account would reply to a comment word-for-word identical to its own.
            let mut short = request(vec!["chỉ một câu"]);
            assert_eq!(
                short.validate(),
                Err(ThreadValidationError::TooFewManualComments)
            );
            short.manual_comments.push("câu thứ hai".into());
            assert!(short.validate().is_ok());
        }

        #[test]
        fn a_blank_comment_is_refused_rather_than_sent() {
            let blank = request(vec!["ổn", "   "]);
            assert_eq!(
                blank.validate(),
                Err(ThreadValidationError::EmptyManualComment)
            );
        }

        #[test]
        fn a_campaign_stored_before_these_existed_still_reads_as_ai_and_no_like() {
            // Both fields are `#[serde(default)]`, and this is the assertion that keeps them
            // that way: a row written by an older build must not start liking posts.
            let stored = serde_json::json!({
                "requestId": "old",
                "targets": [],
                "actorUdids": ["one", "two"],
                "messageCount": 2,
                "instruction": "tự nhiên",
                "maxWords": 12,
                "mode": "threaded"
            });
            let decoded: ThreadCampaignRequest =
                serde_json::from_value(stored).expect("an older payload must still decode");
            assert!(decoded.manual_comments.is_empty());
            assert!(!decoded.is_manual());
            assert!(!decoded.like_target);
        }
    }

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
            // Both default on the wire; a fixture spells them out so the shape stays
            // visible and a new field cannot be forgotten silently.
            manual_comments: Vec::new(),
            like_target: false,
            mode: ThreadMode::Threaded,
            shape: ThreadShape::Chain,
            cohort_size: None,
            mentions: Vec::new(),
            mention_parent: false,
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

    /// Standalone drops the chain entirely, which is the whole point of it:
    /// no parent means no locator, no OCR, and nothing that can break halfway
    /// and take the rest of the messages with it.
    #[test]
    fn standalone_leaves_every_message_parentless() {
        let mut request = request(vec![target("1"), target("2")], vec!["A", "B"], 4);
        request.mode = ThreadMode::Standalone;
        let plan = plan_threads(&request).expect("plan");

        assert_eq!(plan.assignments.len(), 8);
        assert!(
            plan.assignments
                .iter()
                .all(|assignment| assignment.parent_ordinal.is_none()),
            "a standalone campaign has no replies to locate"
        );
        // Actor rotation is untouched: the point is who posts, not what they
        // answer.
        let first: Vec<&str> = plan
            .assignments
            .iter()
            .filter(|assignment| assignment.target_key == "content:1")
            .map(|assignment| assignment.actor_udid.as_str())
            .collect();
        assert_eq!(first, ["A", "B", "A", "B"]);
    }

    /// And threaded still chains, so the mode is a real choice rather than a
    /// flag that quietly does nothing.
    #[test]
    fn threaded_still_answers_the_previous_message() {
        let mut request = request(vec![target("1")], vec!["A", "B"], 4);
        request.mode = ThreadMode::Threaded;
        let plan = plan_threads(&request).expect("plan");

        let parents: Vec<Option<u8>> = plan
            .assignments
            .iter()
            .map(|assignment| assignment.parent_ordinal)
            .collect();
        assert_eq!(parents, [None, Some(0), Some(1), Some(2)]);
    }

    /// The comment line is not its own author.
    ///
    /// Every predicate the author scan applies is satisfied by the text
    /// observation against itself, so the only thing that kept this from firing
    /// was OCR emitting the author line first — which is not a guarantee, it is
    /// the order one fixture happened to have. When it does fire, the stored
    /// identity carries the comment body as the author label, and the next
    /// message in the thread hunts for a comment written by that string.
    #[test]
    fn the_comment_body_is_never_taken_as_its_own_author() {
        let body = CommentOcrObservation {
            text: "Quán này xinh quá".into(),
            confidence: 0.94,
            x: 0.10,
            y: 0.34,
            width: 0.4,
            height: 0.03,
        };
        let author = CommentOcrObservation {
            text: "actor_1".into(),
            confidence: 0.97,
            x: 0.10,
            y: 0.30,
            width: 0.2,
            height: 0.03,
        };

        // Body first — the order the old code could not survive.
        let identity = discover_comment_identity(
            &[body.clone(), author.clone()],
            "Quán này xinh quá",
            "sha",
            "test-ocr",
        )
        .expect("identity");
        assert_eq!(identity.author_label, "actor_1");

        // With no author line at all it must refuse, not fall back to itself.
        assert!(
            discover_comment_identity(&[body], "Quán này xinh quá", "sha", "test-ocr").is_none(),
            "a comment with no readable author has no identity"
        );
    }

    /// The same refuse-on-ambiguity rule the parent locator has. Without it the
    /// two ends of the chain disagreed about what counts as identifiable.
    #[test]
    fn a_duplicated_body_has_no_discoverable_identity() {
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
                text: "actor_1".into(),
                confidence: 0.97,
                x: 0.10,
                y: 0.30,
                width: 0.2,
                height: 0.03,
            },
            line(0.34),
            line(0.62),
        ];

        assert!(
            discover_comment_identity(&observations, "Quán này xinh quá", "sha", "test-ocr")
                .is_none()
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

    #[test]
    fn a_star_hangs_every_reply_off_the_root_rather_than_off_its_neighbour() {
        // The shape the operator asked for: one account comments, the rest answer *that*
        // comment. It is also the only shape that can run in parallel — in a chain, message
        // N cannot start until N-1 has been posted and read back, so twenty accounts are
        // twenty sequential waits by construction.
        let mut req = request(vec![target("1")], vec!["a", "b", "c", "d"], 4);
        req.shape = ThreadShape::Star;
        let plan = plan_threads(&req).expect("star plan");

        assert_eq!(
            plan.assignments[0].parent_ordinal, None,
            "ordinal 0 is the root"
        );
        for message in plan.assignments.iter().skip(1) {
            assert_eq!(
                message.parent_ordinal,
                Some(0),
                "ordinal {} answered its neighbour instead of the root",
                message.ordinal
            );
        }
    }

    #[test]
    fn a_chain_is_untouched_by_the_new_axis() {
        // `Chain` is what every stored campaign deserialises to, so this is the regression
        // that says the shape field did not quietly rewrite them.
        let plan =
            plan_threads(&request(vec![target("1")], vec!["a", "b", "c"], 3)).expect("chain plan");
        let parents: Vec<_> = plan
            .assignments
            .iter()
            .map(|message| message.parent_ordinal)
            .collect();
        assert_eq!(parents, vec![None, Some(0), Some(1)]);
    }

    #[test]
    fn cohorts_spread_the_remainder_instead_of_stranding_it() {
        // Twenty phones in teams of three. Six cohorts, and the two left over join
        // existing teams rather than idling or forming a runt of two that finishes last.
        let actors: Vec<String> = (0..20).map(|n| format!("dev-{n:02}")).collect();
        let cohorts = partition_actors(&actors, Some(3));

        assert_eq!(cohorts.len(), 6);
        let mut sizes: Vec<usize> = cohorts.iter().map(Vec::len).collect();
        sizes.sort_unstable();
        assert_eq!(sizes, vec![3, 3, 3, 3, 4, 4]);

        // Every phone once, and no phone twice: a device in two cohorts would be two tasks
        // reaching for the same exclusive lease, which is the deadlock this partition
        // exists to make impossible.
        let mut seen: Vec<&String> = cohorts.iter().flatten().collect();
        seen.sort();
        seen.dedup();
        assert_eq!(
            seen.len(),
            actors.len(),
            "a phone was dropped or duplicated"
        );
    }

    #[test]
    fn a_cohort_larger_than_the_selection_is_simply_one_cohort() {
        // Asking for teams of five out of three phones plainly means "all of them
        // together", and refusing it would be pedantry rather than safety.
        let actors: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
        assert_eq!(partition_actors(&actors, Some(5)), vec![actors.clone()]);
        assert_eq!(partition_actors(&actors, None), vec![actors]);
    }

    #[test]
    fn links_are_dealt_round_robin_so_a_cancelled_run_covered_a_bit_of_everything() {
        // Blocks would give cohort 0 every early link, and a run stopped part way would
        // have finished the first few links and never touched the rest.
        let targets = (1..=6).map(|n| target(&n.to_string())).collect();
        let mut req = request(targets, vec!["a", "b", "c", "d"], 2);
        req.cohort_size = Some(2);
        let plan = plan_threads(&req).expect("cohort plan");

        let mut by_cohort: std::collections::BTreeMap<u16, Vec<&str>> = Default::default();
        for message in &plan.assignments {
            let keys = by_cohort.entry(message.cohort).or_default();
            if !keys.contains(&message.target_key.as_str()) {
                keys.push(&message.target_key);
            }
        }
        assert_eq!(by_cohort[&0], vec!["content:1", "content:3", "content:5"]);
        assert_eq!(by_cohort[&1], vec!["content:2", "content:4", "content:6"]);

        // And a cohort only ever drives its own phones — the property that lets the two
        // run at the same time without contending for a lease.
        for message in &plan.assignments {
            let mine = if message.cohort == 0 {
                ["a", "b"]
            } else {
                ["c", "d"]
            };
            assert!(
                mine.contains(&message.actor_udid.as_str()),
                "cohort {} reached for {}",
                message.cohort,
                message.actor_udid
            );
        }
    }

    #[test]
    fn twenty_actors_are_allowed_and_one_still_is_not() {
        // The cap was six, which is what made a twenty-phone run impossible before
        // anything else could go wrong.
        let actors: Vec<String> = (0..20).map(|n| format!("dev-{n:02}")).collect();
        let borrowed: Vec<&str> = actors.iter().map(String::as_str).collect();
        let mut req = request(vec![target("1")], borrowed, 20);
        assert!(req.validate().is_ok());

        req.actor_udids.truncate(1);
        assert_eq!(
            req.validate(),
            Err(ThreadValidationError::InvalidActorCount),
            "one account replying to itself is not a conversation"
        );
    }

    #[test]
    fn the_message_count_has_to_cover_the_largest_cohort_not_the_whole_fleet() {
        // Twenty phones in teams of three need three messages a link, not twenty. Measured
        // against the largest cohort because the remainder makes them uneven by one.
        let actors: Vec<String> = (0..20).map(|n| format!("dev-{n:02}")).collect();
        let borrowed: Vec<&str> = actors.iter().map(String::as_str).collect();
        let mut req = request(vec![target("1")], borrowed, 4);
        req.cohort_size = Some(3);
        assert!(
            req.validate().is_ok(),
            "four covers the biggest team of four"
        );

        req.message_count = 3;
        assert_eq!(
            req.validate(),
            Err(ThreadValidationError::TooFewMessagesForActors),
            "the two cohorts of four would leave a phone with no turn"
        );
    }

    #[test]
    fn a_cohort_of_one_is_refused() {
        let mut req = request(vec![target("1")], vec!["a", "b", "c", "d"], 4);
        req.cohort_size = Some(1);
        assert_eq!(
            req.validate(),
            Err(ThreadValidationError::InvalidCohortSize)
        );
    }

    /// The evidence blob a succeeded message is stored with, in the shape the runner writes.
    fn evidence_with_identity() -> String {
        serde_json::json!({
            "send": { "verdict": "sent" },
            "postedIdentity": {
                "authorLabel": "hanh.trang.dalat",
                "text": "quán này nhìn ngon quá",
                "locatorVersion": "hierarchy-1",
                "frameSha256": "b".repeat(64),
            },
            "reader": "hierarchy",
            "arrival": "identified",
        })
        .to_string()
    }

    fn stored(evidence: Option<&str>) -> InteractionAssignmentRecord {
        InteractionAssignmentRecord {
            id: "assignment-1".into(),
            target_key: "content:123".into(),
            ordinal: 0,
            actor_udid: "ce06".into(),
            parent_assignment_id: None,
            state: ThreadMessageState::Succeeded,
            prepared_text: Some("quán này nhìn ngon quá".into()),
            error_code: None,
            evidence_json: evidence.map(str::to_string),
            like: None,
            mention: None,
        }
    }

    #[test]
    fn a_succeeded_root_can_say_what_it_posted_after_a_restart() {
        // The identity is only ever produced by *sending*, and a message that already
        // succeeded is deliberately never sent again. So on a retry the runner's in-memory
        // map starts empty, and every reply under a succeeded root was skipped with
        // `parent_identity_not_confirmed` — the one case Retry exists for, and it could not
        // work. The identity was on disk the whole time; nothing read it back.
        let identity = stored(Some(&evidence_with_identity()))
            .posted_identity()
            .expect("a succeeded root knows its own comment");

        assert_eq!(identity.author_label, "hanh.trang.dalat");
        assert_eq!(identity.text, "quán này nhìn ngon quá");
        assert_eq!(identity.locator_version, "hierarchy-1");
    }

    #[test]
    fn a_message_that_never_posted_offers_no_identity() {
        // Both shapes of "nothing to reply to", and neither may be guessed at: a failure
        // with evidence of the failure, and a message that never got that far. Returning
        // something here would make a reply hunt for a comment that is not on the screen.
        assert_eq!(stored(None).posted_identity(), None);
        let failed = serde_json::json!({ "send": { "verdict": "notArmed" } }).to_string();
        assert_eq!(stored(Some(&failed)).posted_identity(), None);
        assert_eq!(stored(Some("không phải json")).posted_identity(), None);
    }

    #[test]
    fn a_refused_like_is_something_the_operator_can_read() {
        // It used to go to `log::warn!` and nowhere else, so a message showed as succeeded
        // and the operator had no way to learn the like had been refused. A like that fails
        // must not cost the comment — but "must not fail the message" is not the same as
        // "must not be mentioned".
        let refused = serde_json::json!({
            "send": { "verdict": "sent" },
            "like": "không tim được: capability likeTarget is not supported by this driver",
        })
        .to_string();
        assert_eq!(
            stored(Some(&refused)).like_note().as_deref(),
            Some("không tim được: capability likeTarget is not supported by this driver")
        );

        // And a campaign that never asked for a like says nothing rather than "no".
        assert_eq!(
            stored(Some(&evidence_with_identity())).like_note(),
            None,
            "a run with likeTarget off must not grow a note about likes"
        );
    }
}

/// The write and read halves of `interaction_targets.context_json`, checked against each other.
///
/// Nothing else can check them: a JSON key is a string on both sides, so a rename compiles, the
/// campaign keeps filing notes, and the panel quietly shows an empty row for every target.
#[cfg(test)]
mod target_note_tests {
    use super::*;
    use crate::tiktok_web::{PostWebContext, SubtitleTrack, WebLookupError};

    fn note_of(context: &PostWebContext) -> InteractionTargetNote {
        let json = InteractionTargetNote::context_json(Ok(context)).expect("json");
        InteractionTargetNote::from_row(
            "video:1".into(),
            1,
            "https://www.tiktok.com/@a/video/1".into(),
            TikTokPostKind::Video,
            Some(&json),
        )
    }

    /// **Every field a lookup can produce survives the column.**
    ///
    /// The numbers are the measured ones from 26/08/2026 so the test reads as the case it is
    /// really about: a 52-second vlog with a 105-character caption and an ASR track.
    #[test]
    fn a_full_lookup_round_trips_through_the_column() {
        let context = PostWebContext {
            caption: Some("Cùng tớ khám phá lịch trình 1 ngày trải nghiệm Đà Lạt nha".into()),
            duration_secs: Some(52),
            slide_urls: Vec::new(),
            has_original_audio: Some(true),
            subtitles: vec![
                SubtitleTrack {
                    lang: "eng-US".into(),
                    source: "MT".into(),
                    url: "https://cdn/en.vtt".into(),
                },
                SubtitleTrack {
                    lang: "vie-VN".into(),
                    source: "ASR".into(),
                    url: "https://cdn/vi.vtt".into(),
                },
            ],
            cover_url: None,
        };
        let note = note_of(&context);
        assert_eq!(note.caption_chars, Some(57));
        assert!(note
            .caption_preview
            .as_deref()
            .is_some_and(|preview| preview.starts_with("Cùng tớ khám phá")));
        assert_eq!(note.duration_secs, Some(52));
        assert_eq!(note.has_original_audio, Some(true));
        assert_eq!(note.subtitle_langs, vec!["eng-US", "vie-VN"]);
        // The ASR track, not the first one listed — the same rule `transcript_track` applies.
        assert_eq!(note.transcript_track.as_deref(), Some("vie-VN/ASR"));
        assert_eq!(note.error_code, None);
        assert!(!note.is_blank());
    }

    /// A carousel reports its slide count; a video reports none rather than zero.
    ///
    /// `slideCount: 0` is what the writer stores for every video, and rendering "0 ảnh" next to
    /// each of them is a number that means nothing.
    #[test]
    fn a_video_reports_no_slide_count_while_a_carousel_reports_its_own() {
        let mut context = PostWebContext::default();
        assert_eq!(note_of(&context).slide_count, None);
        context.slide_urls = (0..8)
            .map(|index| format!("https://cdn/{index}.jpg"))
            .collect();
        assert_eq!(note_of(&context).slide_count, Some(8));
    }

    /// **A refused lookup is filed, and reads back as refused rather than as empty.**
    ///
    /// This is the distinction the whole column exists for: two of seven real targets answer
    /// `Your IP address is blocked from accessing this post`, and an operator seeing a blank row
    /// has no way to tell that from a target nobody looked up.
    #[test]
    fn a_refusal_reads_back_with_its_reason() {
        let json =
            InteractionTargetNote::context_json(Err(&WebLookupError::Blocked)).expect("json");
        let note = InteractionTargetNote::from_row(
            "photo:2".into(),
            2,
            "https://www.tiktok.com/@a/photo/2".into(),
            TikTokPostKind::Photo,
            Some(&json),
        );
        assert_eq!(note.error_code.as_deref(), Some("ip_blocked"));
        assert!(note
            .error_detail
            .as_deref()
            .is_some_and(|detail| detail.contains("chặn IP")));
        assert!(
            !note.is_blank(),
            "a refusal is something to show, not nothing"
        );
    }

    /// **No column at all is blank, and blank is its own state.**
    ///
    /// A campaign that ran before this existed, or one whose targets were all manual, has
    /// nothing filed. Reporting that as a failure would invent a problem.
    #[test]
    fn a_target_nobody_looked_up_is_blank_and_not_an_error() {
        let note = InteractionTargetNote::from_row(
            "video:3".into(),
            3,
            "https://www.tiktok.com/@a/video/3".into(),
            TikTokPostKind::Video,
            None,
        );
        assert!(note.is_blank());
        assert_eq!(note.error_code, None);
    }

    /// Junk in the column degrades to blank rather than to a panic.
    #[test]
    fn unreadable_json_reads_back_as_blank() {
        for stored in ["", "not json", "{}", r#"{"web":null}"#] {
            let note = InteractionTargetNote::from_row(
                "video:4".into(),
                4,
                "https://www.tiktok.com/@a/video/4".into(),
                TikTokPostKind::Video,
                Some(stored),
            );
            assert!(note.is_blank(), "stored {stored:?}");
        }
    }

    /// The preview is bounded, because a 399-character caption in a table cell is a wall.
    #[test]
    fn a_long_caption_is_previewed_not_pasted() {
        let context = PostWebContext {
            caption: Some("x".repeat(400)),
            ..PostWebContext::default()
        };
        let note = note_of(&context);
        assert_eq!(
            note.caption_chars,
            Some(400),
            "the full length is still reported"
        );
        assert_eq!(
            note.caption_preview.as_deref().map(str::len),
            Some(NOTE_CAPTION_PREVIEW_CHARS)
        );
    }

    /// **The panel's own field names, pinned against the frontend interface.**
    ///
    /// The repo's wire-parity test only scans `types.rs`, so an interaction type mirrored in
    /// `types.ts` has no gate at all — which is how a field could be added on one side and
    /// rendered as `undefined` on the other. This checks the one type this change adds.
    #[test]
    fn the_frontend_mirrors_this_note_field_for_field() {
        let ts = include_str!("../../../apps/desktop/src/types.ts").replace("\r\n", "\n");
        let block = ts
            .split("export interface InteractionTargetNote {")
            .nth(1)
            .expect("the frontend must declare InteractionTargetNote")
            .split("\n}")
            .next()
            .expect("a closing brace");
        let declared: std::collections::BTreeSet<String> = block
            .lines()
            .map(str::trim)
            .filter(|line| {
                !line.starts_with("//") && !line.starts_with('*') && !line.starts_with("/*")
            })
            .filter_map(|line| line.split_once(':'))
            .map(|(field, _)| field.trim().trim_end_matches('?').to_string())
            .filter(|field| !field.is_empty() && field.chars().all(|c| c.is_alphanumeric()))
            .collect();
        // The camelCase names `serde(rename_all)` actually sends.
        let expected: std::collections::BTreeSet<String> = [
            "targetKey",
            "lineNo",
            "normalizedUrl",
            "kind",
            "captionChars",
            "captionPreview",
            "durationSecs",
            "slideCount",
            "hasOriginalAudio",
            "subtitleLangs",
            "transcriptTrack",
            "errorCode",
            "errorDetail",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        assert_eq!(
            declared, expected,
            "the frontend interface and this struct disagree"
        );

        // And the struct really does send those names, rather than the list drifting on its own.
        let sent = serde_json::to_value(InteractionTargetNote::from_row(
            "video:1".into(),
            1,
            "u".into(),
            TikTokPostKind::Video,
            None,
        ))
        .expect("serialise");
        let sent: std::collections::BTreeSet<String> = sent
            .as_object()
            .expect("an object")
            .keys()
            .cloned()
            .collect();
        assert_eq!(
            sent, expected,
            "serde sends different names than the list above"
        );
    }
}
