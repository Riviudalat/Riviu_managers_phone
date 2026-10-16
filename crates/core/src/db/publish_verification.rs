//! Restart-safe verification reads and compare-and-swap publication settlement.
use super::publish_sheet::{
    evidence_has_post_link, queue_sheet_row, reconciled_sheet_delivery_status,
};
use super::*;
use crate::publish_submission::normalize_publish_account;

#[cfg(test)]
#[path = "publish_verification_resume_tests.rs"]
mod resume_tests;

#[derive(Debug, Clone)]
pub struct PendingPublishVerification {
    pub assignment_id: String,
    pub campaign_id: String,
    pub bundle_id: String,
    pub udid: String,
    pub scheduled: bool,
    pub revision: i64,
    pub effect_intent: Option<String>,
    pub evidence_json: Option<String>,
    /// Exact durable stop marker observed at selection, including legacy markers.
    pub stop_marker: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishRecoveryCapability {
    pub allowed: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishRecoveryCapabilities {
    pub assignment_id: String,
    pub revision: i64,
    pub check_link: PublishRecoveryCapability,
    pub resume_verification: PublishRecoveryCapability,
    pub retry_before_post: PublishRecoveryCapability,
    pub verification_resumed: bool,
    pub recovery: Option<crate::publish_recovery::PublishRecoveryState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclusion: Option<PublishExcludeReceipt>,
    pub sheet_required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PublishResumeVerificationState {
    Accepted,
    AlreadyPending,
    AlreadyVerified,
    Stale,
    Ineligible,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishResumeVerificationResult {
    pub assignment_id: String,
    pub state: PublishResumeVerificationState,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishDeviceHold {
    pub assignment_id: String,
    pub campaign_id: String,
    pub updated_at: String,
    pub reason: String,
}

#[derive(Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishDeviceGuard {
    pub blocking: Vec<PublishDeviceHold>,
    pub link_review: Vec<PublishDeviceHold>,
}

/// A terminal legacy Posted observation can still owe a link indefinitely. It is
/// not an active upload once review stopped and the longest existing upload/verify
/// budget (four hours) has elapsed since that row's LAST write. Created-at alone
/// is unsafe: an old scheduled campaign could have dispatched just now.
/// Submitted/ambiguous rows, malformed dates, and live pipelines stay protected.
fn completed_upload_with_link_debt(
    state: &str,
    evidence: Option<&str>,
    updated_at: &str,
    active_pipeline: bool,
    now: DateTime<Utc>,
) -> bool {
    if active_pipeline || !matches!(state, "uncertain" | "verifying" | "succeeded") {
        return false;
    }
    let Some(evidence) =
        evidence.and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
    else {
        return false;
    };
    let post = evidence.get("post").unwrap_or(&evidence);
    evidence["verificationStatus"]["state"] == "needsReview"
        && post["state"] == "posted"
        && post["verdict"] == "Posted"
        && DateTime::parse_from_rfc3339(updated_at).is_ok_and(|updated| {
            now.signed_duration_since(updated)
                >= chrono::Duration::minutes(UPLOAD_IDLE_AFTER_MINUTES)
        })
}

impl PendingPublishVerification {
    /// Routing proof only. Network capture must still validate the frozen bundle,
    /// canonical identity and current authorization before and after its read.
    /// A present but invalid candidate is an error, never permission to use a phone.
    pub fn retained_metadata_candidate(&self) -> anyhow::Result<Option<crate::publish_submission::PendingMetadataCandidate>> {
        use crate::publish_submission::{MetadataAttemptState, PendingMetadataCandidate, PublishSubmissionProof};
        use sha2::Digest;
        let evidence: serde_json::Value = self.evidence_json.as_deref()
            .map(serde_json::from_str).transpose()?.unwrap_or_default();
        let value = evidence.get("verificationDiagnostic").and_then(|d| d.get("pendingMetadataCandidate"))
            .or_else(|| evidence.get("post").and_then(|p| p.get("verificationDiagnostic"))
                .and_then(|d| d.get("pendingMetadataCandidate")));
        let Some(value) = value else { return Ok(None); };
        let pending: PendingMetadataCandidate = serde_json::from_value(value.clone())?;
        let raw = self.effect_intent.as_deref().context("metadata candidate submission missing")?;
        let proof: PublishSubmissionProof = serde_json::from_str(raw)?;
        proof.validate()?;
        anyhow::ensure!(pending.schema_version == 1
            && matches!(pending.captured.metadata.state, MetadataAttemptState::Unavailable | MetadataAttemptState::Rejected)
            && pending.captured.metadata.attempts > 0
            && pending.campaign_id == self.campaign_id
            && pending.assignment_id == self.assignment_id
            && pending.bundle_id == self.bundle_id && proof.bundle_id == self.bundle_id
            && pending.intent_sha256 == format!("{:x}", sha2::Sha256::digest(raw.as_bytes()))
            && pending.caption_sha256 == proof.caption_sha256
            && pending.captured.submitted_at == proof.submitted_at
            && normalize_publish_account(&pending.captured.expected_account)?
                == normalize_publish_account(&proof.expected_account)?,
            "retained metadata candidate invalid; phone fallback forbidden");
        Ok(Some(pending))
    }

    /// Automatic scheduling policy does not invalidate a manual one-shot read.
    pub fn automatic_metadata_observation_allowed(&self, now: DateTime<Utc>) -> bool {
        let evidence: serde_json::Value = self.evidence_json.as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok()).unwrap_or_default();
        // A reopened re-check keeps its exhausted counters; its own window bounds it.
        !needs_review(self.evidence_json.as_deref())
            && self.is_due(now)
            && (evidence["verificationStatus"]["cause"] == AUTOMATIC_RECHECK_CAUSE
                || (evidence["verificationBudget"]["metadataObservations"]
                    .as_u64()
                    .unwrap_or(0)
                    < METADATA_RETRY_LIMIT
                    && evidence["verificationBudget"]["noProgressObservations"]
                        .as_u64()
                        .unwrap_or(0)
                        < VERIFICATION_NO_PROGRESS_LIMIT))
    }

    /// Old receipts without scheduling metadata remain immediately eligible.
    pub fn is_due(&self, now: DateTime<Utc>) -> bool {
        self.evidence_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .and_then(|v| {
                v["verificationStatus"]["nextCheckAt"]
                    .as_str()
                    .map(str::to_owned)
            })
            .and_then(|at| DateTime::parse_from_rfc3339(&at).ok())
            .map(|at| at.with_timezone(&Utc))
            .or_else(|| {
                self.scheduled
                    .then(|| first_scheduled_check(self.effect_intent.as_deref()))
                    .flatten()
            })
            .is_none_or(|at| now >= at)
    }
}

pub fn publish_campaign_input_digest(
    request: &crate::PublishCampaignRequest,
    detail: &crate::PublishCampaignDetail,
) -> anyhow::Result<String> {
    use sha2::Digest;
    let stable = serde_json::json!({
        "schemaVersion":1,"request":request,"bundles":detail.bundles,
        "targets":detail.assignments.iter().map(|assignment|serde_json::json!({
            "ordinal":assignment.ordinal,"bundleId":assignment.bundle_id,"udid":assignment.udid,
        })).collect::<Vec<_>>()
    });
    Ok(format!(
        "{:x}",
        sha2::Sha256::digest(serde_json::to_vec(&stable)?)
    ))
}

fn has_post_intent(intent: Option<&str>) -> bool {
    intent
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .is_some_and(|value| {
            matches!(
                value.get("effectIntent").and_then(|v| v.as_str()),
                Some("post" | "post_carousel")
            )
        })
}

fn may_verify(state: &str, intent: Option<&str>, evidence: Option<&str>) -> bool {
    may_verify_in(state, intent, &RowEvidence::new(evidence))
}

fn may_verify_in(state: &str, intent: Option<&str>, evidence: &RowEvidence) -> bool {
    match state {
        "verifying" | "uncertain" => {
            has_post_intent(intent)
                || post_intent_in(evidence.get())
                || review_parked(evidence.get())
        }
        // Historical success was only a feed-return observation. Recover its missing link
        // without rewriting the original evidence or ever entering the Post path again.
        "succeeded" => !post_link_in(evidence.get()),
        _ => false,
    }
}

// Upload-idle proof is independent of the bounded missing-link observation budget.
const UPLOAD_IDLE_AFTER_MINUTES: i64 = 240;
const SCHEDULED_FIRST_CHECK_SECONDS: i64 = 120;
// Owner 13/10/2026: re-check a posted link every 2 minutes instead of 5. The no-progress
// limit grows from 3 to 8 so the total wait stays about the same (15 -> 16 minutes).
pub(super) const VERIFICATION_CHECK_SECONDS: i64 = 120;
pub(super) const VERIFICATION_NO_PROGRESS_LIMIT: u64 = 8;
const VERIFICATION_NO_PROGRESS_CAUSE: &str = "verificationNoProgress";
const METADATA_RETRY_LIMIT: u64 = 12;
const METADATA_RETRY_CAUSE: &str = "metadataRetryBudgetExhausted";
// 14/10/2026 13:45-14:10, one host and 20 phones: oEmbed hung 15-20 s or answered 429/503
// while TikTok's other pages answered at once, and r112-r115 took up to 46 minutes from Post
// to Sheet. Unanswered public checks retry on the 2-minute cadence for this long from the
// first one instead of spending METADATA_RETRY_LIMIT. A cap, not a target.
pub(super) const METADATA_UNREACHABLE_MINUTES: i64 = 120;
const METADATA_UNREACHABLE_CAUSE: &str = "publicMetadataUnreachable";
// Phone 23 (ce031713cd92d90701, SM-G955F, TikTok 45.7.3 en), campaign fff151ba, 13/10/2026:
// Copy link still showed "Post is being processed" 36+ minutes after Post. A cap, not a target.
pub(super) const PROCESSING_WAIT_MINUTES: i64 = 60;
const PROCESSING_WAIT_CAUSE: &str = "processingWaitExhausted";
// The helper clipboard did not take or give back the sentinel: the phone's tooling failed, not
// the search for the post (r129 phone 48, r127 phone 32: caption and time proved on the
// Profile, then every Copy read failed). Such passes spend this cap, not the no-progress
// limit. Not measured: 15 passes at the 2-minute cadence plus a 1-3 minute pass span roughly
// 45-75 minutes of helper self-repair before the slow re-check below takes over, and a pass
// whose sentinel write fails taps nothing public. A cap, not a target.
const INFRASTRUCTURE_REASONS: [&str; 2] = ["clipboardUnwritable", "clipboardUnreadable"];
const INFRASTRUCTURE_RETRY_LIMIT: u64 = 15;
const INFRASTRUCTURE_RETRY_CAUSE: &str = "infrastructureRetryExhausted";
// A row parked only because TikTok's public metadata did not answer, TikTok still held the
// upload, or a slow phone or a read or clipboard primitive cut the pass short is not a negative
// about the post. The worker reopens it for one more read-only observation every 30 minutes, for
// 24 hours from its first park. Not measured: oEmbed stayed unanswered for hours on 14-16/10/2026
// and its own backoff caps at 10 minutes; 30 minutes keeps a parked row to two phone passes an
// hour, and 24 hours spans an overnight outage. At most 48 extra observations, none of which can
// reach the composer or Post.
const AUTOMATIC_RECHECK_MINUTES: i64 = 30;
const AUTOMATIC_RECHECK_WINDOW_HOURS: i64 = 24;
const AUTOMATIC_RECHECK_CAUSE: &str = "automaticRecheck";
/// Exhausted automatic budgets the re-check may reopen. A processing park always carries
/// `tiktokProcessing` or `clipboardUnchanged` (`processing_pending`).
const AUTOMATIC_RECHECK_CAUSES: [&str; 5] = [
    VERIFICATION_NO_PROGRESS_CAUSE,
    METADATA_RETRY_CAUSE,
    METADATA_UNREACHABLE_CAUSE,
    PROCESSING_WAIT_CAUSE,
    INFRASTRUCTURE_RETRY_CAUSE,
];
/// Last observations of a parked row that say nothing against the post itself: TikTok's public
/// metadata did not answer; a read, snapshot or helper-clipboard primitive failed; the capture
/// window or candidate budget ran out (`searchBudgetExhausted`), also after a proven tile whose
/// viewport scan it cut (`ambiguityScanIncomplete`); the Profile took longer than the recovery
/// window; Copy left the clipboard unchanged or gave back no post link; the copied short link did
/// not resolve; TikTok still showed the post as processing. Drafts, the composer, unmeasured
/// formats, the account, another post, a second match and a spent Copy budget stay manual.
const AUTOMATIC_RECHECK_REASONS: [&str; 12] = [
    "metadataUnavailable",
    "clipboardUnwritable",
    "clipboardUnreadable",
    "readFailed",
    "staleSnapshot",
    "searchBudgetExhausted",
    "ambiguityScanIncomplete",
    "profileUnavailable",
    "clipboardUnchanged",
    "clipboardNotPostLink",
    "redirectFailed",
    "tiktokProcessing",
];
// A pass that reaches the post taps Copy link, which TikTok very likely counts as a public share
// (`tiktok_share.rs`, "Copy link is a share, not a read"). Once this many re-check passes have
// tapped Copy, the row is no longer reopened and waits for the operator, as every park did before
// the re-check existed. Not measured: passes tapped Copy 1-4 times in the DB (each pass is itself
// capped at 24 taps), so 8 passes keep the extra shares of one post near 8-32 instead of up to
// 48 x 4 a day. Passes that never reach the Copy row (search, Profile or metadata only) spend
// nothing here. Ending the automatic re-check is the safe side: nothing posts, the link debt
// stays, and the operator's resume still opens a fresh budget. A cap, not a target.
const AUTOMATIC_RECHECK_COPY_PASSES: u64 = 8;

fn infrastructure_failure(reason_code: &str) -> bool {
    INFRASTRUCTURE_REASONS.contains(&reason_code)
}

/// A review the background worker may reopen on its own: an exhausted automatic budget whose
/// last observation said nothing against the post (`AUTOMATIC_RECHECK_REASONS`), while the
/// re-check's Copy cap is not spent. Another post, an account mismatch or a rejected candidate
/// never qualifies.
fn automatic_recheck_review(evidence: &serde_json::Value) -> bool {
    let status = &evidence["verificationStatus"];
    let diagnostic = &evidence["verificationDiagnostic"];
    status["state"] == "needsReview"
        && status["cause"]
            .as_str()
            .is_some_and(|cause| AUTOMATIC_RECHECK_CAUSES.contains(&cause))
        && status["reasonCode"]
            .as_str()
            .is_some_and(|code| AUTOMATIC_RECHECK_REASONS.contains(&code))
        && !matches!(
            diagnostic["phoneRoute"]["reasonCode"].as_str(),
            Some("accountMismatch" | "otherPublication" | "multipleMatchingPosts")
        )
        && diagnostic["pendingMetadataCandidate"]["captured"]["metadata"]["state"] != "rejected"
        && evidence["verificationBudget"]["recheckCopyPasses"]
            .as_u64()
            .unwrap_or(0)
            < AUTOMATIC_RECHECK_COPY_PASSES
}

fn rfc3339_utc(value: &serde_json::Value) -> Option<DateTime<Utc>> {
    value
        .as_str()
        .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.with_timezone(&Utc))
}

/// End of the re-check window: 24 hours from the first park of this budget.
fn automatic_recheck_until(evidence: &serde_json::Value) -> Option<DateTime<Utc>> {
    rfc3339_utc(&evidence["verificationBudget"]["recheckSince"])
        .or_else(|| rfc3339_utc(&evidence["verificationStatus"]["checkedAt"]))
        .map(|since| since + chrono::Duration::hours(AUTOMATIC_RECHECK_WINDOW_HOURS))
}

fn automatic_recheck_due(evidence: &serde_json::Value, now: DateTime<Utc>) -> bool {
    automatic_recheck_review(evidence)
        && rfc3339_utc(&evidence["verificationStatus"]["checkedAt"]).is_some_and(|parked| {
            now >= parked + chrono::Duration::minutes(AUTOMATIC_RECHECK_MINUTES)
        })
        && automatic_recheck_until(evidence).is_some_and(|until| now < until)
}

/// The re-check still owns this park: it comes due at least once more inside its window.
fn automatic_recheck_open(evidence: &serde_json::Value, now: DateTime<Utc>) -> bool {
    rfc3339_utc(&evidence["verificationStatus"]["checkedAt"]).is_some_and(|parked| {
        let next = now.max(parked + chrono::Duration::minutes(AUTOMATIC_RECHECK_MINUTES));
        automatic_recheck_due(evidence, next)
    })
}

/// Copy returned no link while TikTok still holds the upload: the processing notice, or an
/// unchanged clipboard on a pass whose restart guard found the upload not proven settled.
/// Waiting on TikTok is not a lack of progress until the processing wait runs out.
fn processing_pending(reason_code: &str, diagnostic: Option<&serde_json::Value>) -> bool {
    reason_code == "tiktokProcessing"
        || (reason_code == "clipboardUnchanged"
            && diagnostic.is_some_and(|d| d["appRestart"]["reason"] == "uploadNotProvenSettled"))
}

/// Measured from Post, or from the operator's resume, which opens a fresh wait.
fn processing_wait_until(
    budget: &serde_json::Value,
    intent: Option<&str>,
) -> Option<DateTime<Utc>> {
    let started = budget["processingWaitFrom"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            intent
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                .and_then(|value| value["submittedAt"].as_str().map(str::to_owned))
        })?;
    let started = DateTime::parse_from_rfc3339(&started)
        .ok()?
        .with_timezone(&Utc);
    Some(started + chrono::Duration::minutes(PROCESSING_WAIT_MINUTES))
}

fn new_verification_budget(intent: Option<&str>) -> serde_json::Value {
    use sha2::Digest;
    serde_json::json!({
        "version":1,"intentSha256":intent.map(|raw|format!("{:x}",sha2::Sha256::digest(raw.as_bytes()))),
        "publicationStage":0,"noProgressObservations":0,"observations":0,
        "metadataObservations":0,"metadataLimit":METADATA_RETRY_LIMIT,
        "limit":VERIFICATION_NO_PROGRESS_LIMIT,
        "infrastructureObservations":0,"infrastructureLimit":INFRASTRUCTURE_RETRY_LIMIT,
    })
}

const OWN_VIEWER_STAGES: [&str; 5] = [
    "postProof",
    "copy",
    "videoPublicProof",
    "expandedPhotoPublicProof",
    "restoreProfile",
];

/// Two observations are not operator-facing no-progress: a pass that proved the target
/// but ran out of time ruling out a second match, and a pass that met only the post
/// viewer its own previous pass left open after running out of time. At most every
/// other observation is exempt, so the no-progress limit still ends the loop.
fn exempt_from_no_progress(
    evidence: &serde_json::Value,
    reason_code: &str,
    diagnostic: Option<&serde_json::Value>,
) -> bool {
    if evidence["verificationBudget"]["lastObservationExempt"] == true {
        return false;
    }
    if reason_code == "ambiguityScanIncomplete" {
        return true;
    }
    let previous = &evidence["verificationDiagnostic"];
    let Some(current) = diagnostic else {
        return false;
    };
    let shape = &current["unknownScreenShape"];
    let viewer = matches!(
        current["screenState"].as_str(),
        Some("Post" | "Share" | "Comments")
    ) || (shape["hasBackControl"] == true
        && shape["hasShareControl"] == true
        && shape["hasCommentsControl"] == true
        && shape["hasProfileTab"] == false
        && shape["hasFeedTab"] == false
        && shape["hasPostTile"] == false);
    matches!(
        reason_code,
        "profileUnavailable" | "navigationBudgetExhausted"
    ) && current["stage"] == "profile"
        && current["candidatesVisited"].as_u64() == Some(0)
        && viewer
        && matches!(
            previous["reasonCode"].as_str(),
            Some("searchBudgetExhausted" | "ambiguityScanIncomplete")
        )
        && previous["stage"]
            .as_str()
            .is_some_and(|stage| OWN_VIEWER_STAGES.contains(&stage))
}

/// Only first-time target-caption/time proof advances this finite frontier. Rejected
/// captions, changing timestamps, snapshot generations and Copy counts are not progress.
fn advance_verification_budget(
    evidence: &serde_json::Value,
    intent: Option<&str>,
    diagnostic: Option<&serde_json::Value>,
    exempt: bool,
    processing_wait: bool,
    infrastructure: bool,
) -> serde_json::Value {
    use sha2::Digest;
    let initial = new_verification_budget(intent);
    let prior = &evidence["verificationBudget"];
    let prior = if prior["version"] == 1 && prior["intentSha256"] == initial["intentSha256"] {
        prior
    } else {
        &initial
    };
    let previous_stage = prior["publicationStage"].as_u64().unwrap_or(0).min(2);
    let proof = diagnostic.map(|value| &value["publicationEvidence"]);
    let observed_stage = match proof {
        Some(proof)
            if proof["captionMatched"] == true && proof["submissionTimeMatched"] == true =>
        {
            2
        }
        Some(proof) if proof["captionMatched"] == true => 1,
        _ => 0,
    };
    let stage = previous_stage.max(observed_stage);
    let prior_no_progress = prior["noProgressObservations"].as_u64().unwrap_or(0);
    let no_progress = if stage > previous_stage {
        0
    } else if exempt || processing_wait || infrastructure {
        prior_no_progress
    } else {
        prior_no_progress.saturating_add(1)
    };
    // The proven candidate is a lead for the next pass, never a verified link.
    let proven_candidate_url = proof
        .and_then(|proof| proof["provenCandidateUrl"].as_str())
        .map(serde_json::Value::from)
        .unwrap_or_else(|| prior["provenCandidateUrl"].clone());
    // The fingerprint describes the durable proof frontier, not the latest screen.
    let frontier =
        serde_json::json!({"intentSha256":initial["intentSha256"],"publicationStage":stage});
    let fingerprint = format!(
        "{:x}",
        sha2::Sha256::digest(frontier.to_string().as_bytes())
    );
    let mut budget = serde_json::json!({
        "version":1,"intentSha256":initial["intentSha256"],"publicationStage":stage,
        "fingerprint":fingerprint,"noProgressObservations":no_progress,
        "observations":prior["observations"].as_u64().unwrap_or(0).saturating_add(1),
        "metadataObservations":prior["metadataObservations"].as_u64().unwrap_or(0),
        "metadataLimit":METADATA_RETRY_LIMIT,
        "limit":VERIFICATION_NO_PROGRESS_LIMIT,
        "infrastructureObservations":prior["infrastructureObservations"].as_u64().unwrap_or(0)
            .saturating_add(u64::from(infrastructure)),
        "infrastructureLimit":INFRASTRUCTURE_RETRY_LIMIT,
        // A processing or infrastructure wait leaves the every-other exemption where it was.
        "lastObservationExempt":(exempt
            || ((processing_wait || infrastructure) && prior["lastObservationExempt"] == true))
            && stage == previous_stage,
    });
    for field in ["recheckSince", "recheckCount", "recheckCopyPasses"] {
        if !prior[field].is_null() {
            budget[field] = prior[field].clone();
        }
    }
    if !proven_candidate_url.is_null() {
        budget["provenCandidateUrl"] = proven_candidate_url;
    }
    if prior["processingWaitFrom"].is_string() {
        budget["processingWaitFrom"] = prior["processingWaitFrom"].clone();
    }
    if prior["metadataUnreachableSince"].is_string() {
        budget["metadataUnreachableSince"] = prior["metadataUnreachableSince"].clone();
        budget["metadataUnreachableObservations"] = prior["metadataUnreachableObservations"].clone();
    }
    budget
}

/// Destructive verifier restart authority only. An earlier observation of this exact
/// intent read the target's full caption and submission time on one own-post snapshot,
/// so TikTok already holds the post and its upload is no longer in flight. A submitted
/// receipt alone is not this proof. Never publication, link, Sheet or Post authority.
pub fn publish_upload_settled(intent: Option<&str>, evidence: Option<&str>) -> bool {
    let evidence: serde_json::Value = evidence
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default();
    let budget = &evidence["verificationBudget"];
    intent.is_some()
        && budget["version"] == 1
        && budget["intentSha256"] == new_verification_budget(intent)["intentSha256"]
        && budget["publicationStage"]
            .as_u64()
            .is_some_and(|stage| stage >= 2)
}

/// TikTok's public check did not answer: no response (timeout, connect failure or the
/// host's own oEmbed pacing), 429 or 5xx, or a cut-off, oversized or unreadable body.
/// A mismatch is Rejected and any other 4xx is an answer; both keep their budgets.
fn public_check_unanswered(attempt: &crate::publish_submission::MetadataAttempt) -> bool {
    use crate::publish_submission::{MetadataAttemptState, MetadataDiagnostic};
    matches!(attempt.state, MetadataAttemptState::Unavailable)
        && match attempt.stage {
            MetadataDiagnostic::Request
            | MetadataDiagnostic::BodyRead
            | MetadataDiagnostic::BodyLimit
            | MetadataDiagnostic::ResponseParse => true,
            MetadataDiagnostic::HttpStatus => attempt
                .http_status
                .is_some_and(|status| status == 429 || (500..600).contains(&status)),
            _ => false,
        }
}

/// A copied candidate permits bounded network reads, never publication or Sheet authority.
fn retained_metadata_observation(
    conn: &Connection,
    candidate: &PendingPublishVerification,
    diagnostic: Option<&serde_json::Value>,
) -> anyhow::Result<bool> {
    use crate::publish_submission::*;
    use sha2::Digest;
    let Some(value) = diagnostic.and_then(|d| d.get("pendingMetadataCandidate")) else {
        return Ok(false);
    };
    let Ok(pending) = serde_json::from_value::<PendingMetadataCandidate>(value.clone()) else {
        return Ok(false);
    };
    let Some(raw) = candidate.effect_intent.as_deref() else { return Ok(false); };
    let Ok(proof) = serde_json::from_str::<PublishSubmissionProof>(raw) else { return Ok(false); };
    if pending.schema_version != 1 || proof.validate().is_err()
        || !matches!(pending.captured.metadata.state, MetadataAttemptState::Unavailable)
        || pending.captured.metadata.attempts == 0
        || pending.campaign_id != candidate.campaign_id
        || pending.assignment_id != candidate.assignment_id
        || pending.bundle_id != candidate.bundle_id || proof.bundle_id != candidate.bundle_id
        || pending.intent_sha256 != format!("{:x}",sha2::Sha256::digest(raw.as_bytes()))
        || pending.caption_sha256 != proof.caption_sha256
        || pending.captured.submitted_at != proof.submitted_at
        || normalize_publish_account(&pending.captured.expected_account).ok()
            != normalize_publish_account(&proof.expected_account).ok()
    { return Ok(false); }
    let manifest: String = conn.query_row("SELECT manifest_json FROM publish_bundles WHERE id=?1 AND campaign_id=?2",
        params![candidate.bundle_id,candidate.campaign_id],|r|r.get(0))?;
    let Ok(bundle) = serde_json::from_str::<crate::PublishBundle>(&manifest) else { return Ok(false); };
    if bundle.id != pending.bundle_id || bundle.media_kind != proof.media_kind
        || bundle.caption_sha256 != proof.caption_sha256
        || format!("{:x}",sha2::Sha256::digest(bundle.caption.as_bytes())) != proof.caption_sha256
        || normalized_caption_sha256(&bundle.caption) != pending.captured.normalized_caption_sha256
    { return Ok(false); }
    let Ok(url) = url::Url::parse(&pending.captured.canonical_url) else { return Ok(false); };
    let targets = crate::interaction::parse_tiktok_links(&pending.captured.canonical_url);
    let Some(target) = targets.first().and_then(|t|t.target.as_ref()) else { return Ok(false); };
    if url.scheme() != "https" || !matches!(url.host_str(),Some("www.tiktok.com" | "tiktok.com" | "m.tiktok.com"))
        || targets.len() != 1 || target.normalized_url != pending.captured.canonical_url
        || target.content_id != pending.captured.post_id
        || normalize_publish_account(&target.author).ok() != normalize_publish_account(&proof.expected_account).ok()
    { return Ok(false); }
    let claimed_elsewhere: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM publish_post_identities WHERE post_url=?1 AND assignment_id<>?2)",
        params![pending.captured.canonical_url,candidate.assignment_id],|r|r.get(0))?;
    if claimed_elsewhere { return Ok(false); }
    let prepared_at: Option<String> = conn.query_row("SELECT recorded_at FROM operation_device_events
        WHERE source_kind='publish' AND source_id=?1 AND udid=?2 AND action='publishStep' AND state='opening_app'
        AND julianday(recorded_at)<=julianday(?3) AND julianday(recorded_at)>=julianday(?3)-30.0/1440.0
        ORDER BY recorded_at DESC LIMIT 1",params![candidate.campaign_id,candidate.udid,proof.submitted_at],|r|r.get(0)).optional()?;
    if prepared_at != pending.captured.prepared_at { return Ok(false); }
    let Ok(submitted) = DateTime::parse_from_rfc3339(&proof.submitted_at) else { return Ok(false); };
    let Ok(captured) = DateTime::parse_from_rfc3339(&pending.captured.captured_at) else { return Ok(false); };
    let Ok(checked) = DateTime::parse_from_rfc3339(&pending.captured.metadata.checked_at) else { return Ok(false); };
    let Ok(prepared) = DateTime::parse_from_rfc3339(prepared_at.as_deref().unwrap_or(&proof.submitted_at)) else { return Ok(false); };
    let Some(allocated) = pending.captured.post_id.parse::<u64>().ok()
        .and_then(|id|DateTime::from_timestamp((id >> 32) as i64,0)) else { return Ok(false); };
    let now = Utc::now();
    if prepared > submitted || submitted-prepared > chrono::Duration::minutes(30)
        || allocated.timestamp() < prepared.timestamp() || allocated > submitted+chrono::Duration::minutes(30)
        || allocated > now || captured < submitted || captured > now || checked < captured || checked > now
    { return Ok(false); }
    let prior = candidate.evidence_json.as_deref().and_then(|raw|serde_json::from_str::<serde_json::Value>(raw).ok());
    let old = prior.as_ref().and_then(|e|e.get("verificationDiagnostic").and_then(|d|d.get("pendingMetadataCandidate"))
        .or_else(||e.get("post").and_then(|p|p.get("verificationDiagnostic")).and_then(|d|d.get("pendingMetadataCandidate"))));
    if let Some(old) = old {
        let Ok(previous) = serde_json::from_value::<PendingMetadataCandidate>(old.clone()) else { return Ok(false); };
        let mut identity = value.clone();
        let mut previous_identity = old.clone();
        let Some(fields) = identity["captured"].as_object_mut() else { return Ok(false); };
        fields.remove("metadata");
        let Some(fields) = previous_identity["captured"].as_object_mut() else { return Ok(false); };
        fields.remove("metadata");
        if identity != previous_identity || pending.captured.metadata.attempts <= previous.captured.metadata.attempts {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn needs_review(evidence: Option<&str>) -> bool {
    evidence
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .is_some_and(|value| value["verificationStatus"]["state"] == "needsReview")
}

const HANDOFF_RESUME_REASON: &str =
    "Tự tiếp tục xác minh bài đã gửi sau khi lượt mới trả máy; không đăng lại";
const HANDOFF_RECHECK_REASON: &str =
    "Giữ lịch tự kiểm tra lại bài đã gửi sau khi lượt mới trả máy; không đăng lại";

const LEGACY_IDENTITY_REVIEW_REASON: &str = "Thiếu bằng chứng tài khoản hoặc thời điểm của lần Đăng cũ; cần kiểm tra bài trên máy, không đăng lại";

fn submission_identity_complete(intent: Option<&str>) -> bool {
    intent
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .is_some_and(|value| {
            value["expectedAccount"].as_str().is_some_and(|account| {
                let handle = account.trim().trim_start_matches('@');
                !handle.is_empty()
                    && handle.len() <= 24
                    && !handle.ends_with('.')
                    && handle
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.')
            }) && value["submittedAt"]
                .as_str()
                .is_some_and(|submitted| DateTime::parse_from_rfc3339(submitted).is_ok())
        })
}

/// A row's evidence, parsed on first use and at most once. Absent or unparsable evidence reads as
/// `None`, as every helper that takes it treated it when it parsed the raw text itself.
struct RowEvidence<'a> {
    raw: Option<&'a str>,
    parsed: std::cell::OnceCell<Option<serde_json::Value>>,
}

impl<'a> RowEvidence<'a> {
    fn new(raw: Option<&'a str>) -> Self {
        Self {
            raw,
            parsed: std::cell::OnceCell::new(),
        }
    }

    fn get(&self) -> Option<&serde_json::Value> {
        self.parsed
            .get_or_init(|| self.raw.and_then(|raw| serde_json::from_str(raw).ok()))
            .as_ref()
    }
}

/// `needs_review` on parsed evidence.
fn review_parked(evidence: Option<&serde_json::Value>) -> bool {
    evidence.is_some_and(|value| value["verificationStatus"]["state"] == "needsReview")
}

/// `has_post_intent` on parsed evidence.
fn post_intent_in(evidence: Option<&serde_json::Value>) -> bool {
    evidence.is_some_and(|value| {
        matches!(
            value.get("effectIntent").and_then(|v| v.as_str()),
            Some("post" | "post_carousel")
        )
    })
}

/// `publish_sheet::evidence_has_post_link` on parsed evidence.
fn post_link_in(evidence: Option<&serde_json::Value>) -> bool {
    evidence
        .and_then(|evidence| {
            evidence
                .get("post")
                .and_then(|post| post.get("postUrl"))
                .or_else(|| evidence.get("postUrl"))
        })
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .is_some_and(crate::tiktok_share::looks_like_a_post_link)
}

fn explicitly_resumed_verification(candidate: &PendingPublishVerification) -> bool {
    explicitly_resumed_in(
        candidate,
        &RowEvidence::new(candidate.evidence_json.as_deref()),
    )
}

fn explicitly_resumed_in(candidate: &PendingPublishVerification, evidence: &RowEvidence) -> bool {
    use sha2::Digest;
    let Some(intent) = candidate.effect_intent.as_deref() else {
        return false;
    };
    if !has_post_intent(Some(intent)) || !submission_identity_complete(Some(intent)) {
        return false;
    }
    evidence.get().is_some_and(|evidence| {
        let marker = &evidence["verificationResume"];
        let identity_matches = marker["assignmentId"] == candidate.assignment_id
            && marker["intentSha256"] == format!("{:x}", sha2::Sha256::digest(intent.as_bytes()));
        identity_matches
            && ((marker["version"] == 1 && candidate.stop_marker.is_none())
                || (marker["version"] == 2
                    && marker["campaignId"] == candidate.campaign_id
                    && marker["stopGeneration"].as_str()
                        == stop_generation(candidate.stop_marker.as_deref()).as_deref()
                    && marker["authorizationId"]
                        .as_str()
                        .is_some_and(|s| Uuid::parse_str(s).is_ok())
                    && marker["requestedAt"]
                        .as_str()
                        .is_some_and(|s| DateTime::parse_from_rfc3339(s).is_ok())))
    })
}

/// The automatic resume stamp of `exclusion`, bound to this assignment and its Post intent.
fn handoff_resumed(
    candidate: &PendingPublishVerification,
    exclusion: &str,
    evidence: &RowEvidence,
) -> bool {
    use sha2::Digest;
    let Some(intent) = candidate.effect_intent.as_deref() else {
        return false;
    };
    has_post_intent(Some(intent))
        && submission_identity_complete(Some(intent))
        && evidence.get().is_some_and(|evidence| {
            let stamp = &evidence["verificationHandoffResume"];
            stamp["version"] == 1
                && stamp["assignmentId"] == candidate.assignment_id
                && stamp["exclusionRequestId"] == exclusion
                && stamp["intentSha256"] == format!("{:x}", sha2::Sha256::digest(intent.as_bytes()))
        })
}

/// The latest exclusion of this posted row came from a handoff whose closer drained it and
/// closed the app, and nothing that handoff started still needs the phone: no handoff or Start
/// preparation is in flight on it, and no other assignment there can start fresh publish work.
/// From then on no running Start relies on the row's release proof, so verification may go on.
/// Returns that exclusion's request ID.
fn handoff_resume_due(
    conn: &Connection,
    candidate: &PendingPublishVerification,
    state: &str,
) -> anyhow::Result<Option<String>> {
    let (intent, evidence) = (
        candidate.effect_intent.as_deref(),
        candidate.evidence_json.as_deref(),
    );
    // Verification debt only: an operator Stop, a parked review or a settled link stays out. A
    // park the 30-minute re-check still owns comes back to it; any other park stays out.
    let recheck_open = || {
        evidence
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .is_some_and(|evidence| automatic_recheck_open(&evidence, Utc::now()))
    };
    if candidate.stop_marker.is_some()
        || !has_post_intent(intent)
        || !submission_identity_complete(intent)
        || !may_verify(state, intent, evidence)
        || evidence_has_post_link(evidence)
        || (needs_review(evidence) && !recheck_open())
        || publish_close_pending(conn, &candidate.campaign_id)?
        || observer_authorized(conn, candidate)?
    {
        return Ok(None);
    }
    // Only what the automatic worker took before the exclusion.
    let current: bool = conn.query_row(
        "SELECT COALESCE(json_extract(request_json,'$.verificationContractVersion')=1,0)
         FROM publish_campaigns WHERE id=?1",
        [&candidate.campaign_id],
        |r| r.get(0),
    )?;
    if !current && !explicitly_resumed_verification(candidate) {
        return Ok(None);
    }
    let Some(exclusion) = conn
        .query_row(
            "SELECT request_id FROM publish_exclude_requests WHERE assignment_id=?1
             ORDER BY rowid DESC LIMIT 1",
            [&candidate.assignment_id],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    else {
        return Ok(None);
    };
    // An operator's own exclusion has no release proof; a handoff still closing keeps its marker.
    let released = conn
        .query_row(
            "SELECT value FROM settings WHERE key=?1",
            [format!(
                "publish.handoff-release.{}",
                candidate.assignment_id
            )],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .is_some_and(|proof| proof["requestId"] == exclusion.as_str());
    if !released {
        return Ok(None);
    }
    let needed: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM publish_assignments o JOIN settings s
                ON s.key='publish.handoff-pending.'||o.id WHERE o.udid=?1)
         OR EXISTS(SELECT 1 FROM settings l JOIN publish_start_requests r
                ON r.request_id=json_extract(l.value,'$.owner')
             WHERE substr(l.key,1,length(?3))=?3 AND json_valid(l.value)
             AND json_extract(l.value,'$.udid')=?1 AND r.state IN ('accepted','preparing'))
         OR EXISTS(SELECT 1 FROM publish_dispatch_jobs j WHERE j.udid=?1 AND j.assignment_id<>?2
             AND j.state IN ('queued','running','paused'))
         OR EXISTS(SELECT 1 FROM publish_work_claims w WHERE w.udid=?1
             AND w.stage IN ('transfer','compose'))
         OR EXISTS(SELECT 1 FROM publish_assignments o JOIN publish_campaigns c ON c.id=o.campaign_id
             WHERE o.udid=?1 AND o.id<>?2 AND o.effect_intent IS NULL
             AND o.state IN ('queued','preparing','ready','transferring','imported','posting')
             AND c.state NOT IN ('scheduled','cancelled','missed')
             AND NOT EXISTS(SELECT 1 FROM publish_exclude_requests e WHERE e.assignment_id=o.id))",
        params![
            candidate.udid,
            candidate.assignment_id,
            "publish.account-lock.preparing."
        ],
        |r| r.get(0),
    )?;
    if needed {
        return Ok(None);
    }
    // A phone now holding another account cannot prove this post; a person decides.
    let account: String = conn.query_row(
        "SELECT COALESCE((SELECT handle FROM device_meta WHERE udid=?1),'')",
        [&candidate.udid],
        |r| r.get(0),
    )?;
    let account = crate::publish::normalize_publish_account(&account);
    let expected = intent
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|value| {
            value["expectedAccount"]
                .as_str()
                .map(crate::publish::normalize_publish_account)
        });
    if account.is_empty() || expected.as_deref() != Some(account.as_str()) {
        return Ok(None);
    }
    // With another unresolved post on the phone, an observer of either one needs every one of
    // their pipelines finished (shared device debt); resuming sooner would stall the new
    // round's own verification instead of starting this one.
    let mut campaigns: Vec<String> =
        Database::publish_device_guard_from_connection(conn, &candidate.udid)?
            .blocking
            .into_iter()
            .filter(|hold| hold.assignment_id != candidate.assignment_id)
            .map(|hold| hold.campaign_id)
            .collect();
    if !campaigns.is_empty() {
        campaigns.push(candidate.campaign_id.clone());
        for campaign in campaigns {
            let active: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM publish_pipeline_runs WHERE campaign_id=?1)",
                [&campaign],
                |r| r.get(0),
            )?;
            if active {
                return Ok(None);
            }
        }
    }
    Ok(Some(exclusion))
}

fn stop_generation(marker: Option<&str>) -> Option<String> {
    marker
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v["generation"].as_str().map(str::to_owned))
}

fn stop_marker(conn: &Connection, campaign: &str) -> anyhow::Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT value FROM settings WHERE key=?1",
            [format!("operation.stop.publish:{campaign}")],
            |r| r.get(0),
        )
        .optional()?)
}

fn observer_authorized(
    conn: &Connection,
    candidate: &PendingPublishVerification,
) -> anyhow::Result<bool> {
    observer_authorized_in(
        conn,
        candidate,
        &RowEvidence::new(candidate.evidence_json.as_deref()),
    )
}

fn observer_authorized_in(
    conn: &Connection,
    candidate: &PendingPublishVerification,
    evidence: &RowEvidence,
) -> anyhow::Result<bool> {
    let excluded: Option<String> = conn.query_row(
        "SELECT MAX(updated_at) FROM publish_exclude_requests WHERE assignment_id=?1",
        [&candidate.assignment_id],
        |r| r.get(0),
    )?;
    if let Some(excluded) = excluded {
        let resumed_at = evidence
            .get()
            .and_then(|e| {
                e["verificationResume"]["requestedAt"]
                    .as_str()
                    .map(str::to_owned)
            })
            .and_then(|at| DateTime::parse_from_rfc3339(&at).ok());
        let excluded_at = DateTime::parse_from_rfc3339(&excluded)?;
        if !explicitly_resumed_in(candidate, evidence)
            || resumed_at.is_none_or(|at| at <= excluded_at)
        {
            // Otherwise only the automatic resume of exactly the latest exclusion, written
            // once its handoff handed the phone back, lifts it; never under a Stop marker.
            let latest: Option<String> = conn
                .query_row(
                    "SELECT request_id FROM publish_exclude_requests WHERE assignment_id=?1
                     ORDER BY rowid DESC LIMIT 1",
                    [&candidate.assignment_id],
                    |r| r.get(0),
                )
                .optional()?;
            if candidate.stop_marker.is_some()
                || !latest.is_some_and(|request| handoff_resumed(candidate, &request, evidence))
            {
                return Ok(false);
            }
        }
    }
    let marker = stop_marker(conn, &candidate.campaign_id)?;
    Ok(marker == candidate.stop_marker
        && (marker.is_none()
            || (stop_generation(marker.as_deref()).is_some()
                && explicitly_resumed_in(candidate, evidence))))
}

fn explicit_non_stop_review(evidence: Option<&str>) -> bool {
    explicit_non_stop_review_in(
        evidence
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .as_ref(),
    )
}

fn explicit_non_stop_review_in(evidence: Option<&serde_json::Value>) -> bool {
    evidence.is_some_and(|v| {
        let review = &v["verificationStatus"];
        (review["state"] == "needsReview"
            && review["cause"] != "operatorStopped"
            && review["cause"] != "verificationDeadline"
            && review["cause"] != VERIFICATION_NO_PROGRESS_CAUSE
            && review["cause"] != PROCESSING_WAIT_CAUSE
            && review["cause"] != METADATA_RETRY_CAUSE
            && review["cause"] != METADATA_UNREACHABLE_CAUSE
            && review["cause"] != INFRASTRUCTURE_RETRY_CAUSE)
            || v.get("verificationReviewBeforeStop").is_some_and(|prior| {
                prior["cause"] != VERIFICATION_NO_PROGRESS_CAUSE
                    && prior["cause"] != PROCESSING_WAIT_CAUSE
                    && prior["cause"] != METADATA_RETRY_CAUSE
                    && prior["cause"] != METADATA_UNREACHABLE_CAUSE
                    && prior["cause"] != INFRASTRUCTURE_RETRY_CAUSE
            })
    })
}

fn first_scheduled_check(intent: Option<&str>) -> Option<DateTime<Utc>> {
    let intent: serde_json::Value = serde_json::from_str(intent?).ok()?;
    let first_delay = if matches!(
        intent["package"].as_str(),
        Some("com.zhiliaoapp.musically" | "com.ss.android.ugc.trill")
    ) {
        0
    } else {
        SCHEDULED_FIRST_CHECK_SECONDS
    };
    DateTime::parse_from_rfc3339(intent["submittedAt"].as_str()?)
        .ok()?
        .with_timezone(&Utc)
        .checked_add_signed(chrono::Duration::seconds(first_delay))
}

/// Reopen only the retired age-limit reviews with a complete immutable Post identity.
/// Missing identity, an operator pause or other explicit review must stay parked.
fn obsolete_deadline_review(intent: Option<&str>, evidence: Option<&str>) -> bool {
    has_post_intent(intent)
        && submission_identity_complete(intent)
        && needs_review(evidence)
        && evidence
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .is_some_and(|value| {
                let status = &value["verificationStatus"];
                status["cause"] == "verificationDeadline"
                    && matches!(status["reviewAfterMinutes"].as_i64(), Some(30 | 240))
            })
}

fn store_idle_observation(
    conn: &Connection,
    candidate: &PendingPublishVerification,
    account: &str,
    package: &str,
    snapshot_sha256: &str,
) -> anyhow::Result<bool> {
    use sha2::Digest;
    anyhow::ensure!(
        snapshot_sha256.len() == 64 && snapshot_sha256.bytes().all(|b| b.is_ascii_hexdigit()),
        "idle observation hash missing"
    );
    let Some(intent) = candidate.effect_intent.as_deref() else {
        return Ok(false);
    };
    let parsed: serde_json::Value = serde_json::from_str(intent)?;
    let now = Utc::now();
    if !has_post_intent(Some(intent))
        || !submission_identity_complete(Some(intent))
        || parsed["submittedAt"]
            .as_str()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .is_none_or(|at| now.signed_duration_since(at) < chrono::Duration::hours(4))
        || !parsed["expectedAccount"].as_str().is_some_and(|a| {
            a.trim_start_matches('@')
                .eq_ignore_ascii_case(account.trim_start_matches('@'))
        })
        || parsed["package"].as_str() != Some(package)
    {
        return Ok(false);
    }
    let evidence: serde_json::Value = candidate
        .evidence_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let proof = serde_json::json!({"assignmentId":candidate.assignment_id,"campaignId":candidate.campaign_id,"udid":candidate.udid,"account":account,"package":package,"observedAt":now.to_rfc3339(),"sourceSha256":snapshot_sha256,"intentSha256":format!("{:x}",sha2::Sha256::digest(intent.as_bytes())),"stopMarker":candidate.stop_marker,"authorizationId":evidence["verificationResume"]["authorizationId"],"observerRevision":candidate.revision});
    conn.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![format!("publish.idle.{}",candidate.assignment_id),proof.to_string()])?;
    Ok(true)
}

fn recovery_row(
    conn: &Connection,
    id: &str,
) -> anyhow::Result<Option<(PendingPublishVerification, String, bool)>> {
    Ok(conn.query_row("SELECT a.campaign_id,a.bundle_id,a.udid,a.revision,a.effect_intent,a.evidence_json,a.state,c.run_at IS NOT NULL,(SELECT value FROM settings WHERE key='operation.stop.publish:'||a.campaign_id),EXISTS(SELECT 1 FROM publish_pipeline_runs p WHERE p.campaign_id=a.campaign_id) FROM publish_assignments a JOIN publish_campaigns c ON c.id=a.campaign_id WHERE a.id=?1",[id],|r|Ok((PendingPublishVerification {assignment_id:id.into(),campaign_id:r.get(0)?,bundle_id:r.get(1)?,udid:r.get(2)?,revision:r.get(3)?,effect_intent:r.get(4)?,evidence_json:r.get(5)?,scheduled:r.get(7)?,stop_marker:r.get(8)?},r.get(6)?,r.get(9)?))).optional()?)
}

fn publish_close_pending(conn: &Connection, campaign: &str) -> anyhow::Result<bool> {
    let marker = stop_marker(conn, campaign)?;
    let result: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key=?1",
            [format!("operation.stop.result:publish:{campaign}")],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(result) = result {
        if marker
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .is_some_and(|m| m["version"] == 3 && m["closePending"] == true)
        {
            return Ok(serde_json::from_str::<serde_json::Value>(&result)
                .ok()
                .is_none_or(|r| {
                    r["state"] != "closed" || r["stopMarker"].as_str() != marker.as_deref()
                }));
        }
        return Ok(serde_json::from_str::<serde_json::Value>(&result)
            .ok()
            .is_none_or(|v| v["state"] != "closed"));
    }
    Ok(stop_marker(conn, campaign)?
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .is_some_and(|v| v["closePending"] == true))
}

fn already_pending(
    conn: &Connection,
    candidate: &PendingPublishVerification,
) -> anyhow::Result<bool> {
    already_pending_in(
        conn,
        candidate,
        &RowEvidence::new(candidate.evidence_json.as_deref()),
    )
}

fn already_pending_in(
    conn: &Connection,
    candidate: &PendingPublishVerification,
    evidence: &RowEvidence,
) -> anyhow::Result<bool> {
    if review_parked(evidence.get()) || !observer_authorized_in(conn, candidate, evidence)? {
        return Ok(false);
    }
    let current: bool = conn.query_row("SELECT COALESCE(json_extract(request_json,'$.verificationContractVersion')=1,0) FROM publish_campaigns WHERE id=?1",[&candidate.campaign_id],|r|r.get(0))?;
    Ok(explicitly_resumed_in(candidate, evidence)
        || (current
            && candidate.stop_marker.is_none()
            && has_post_intent(candidate.effect_intent.as_deref())
            && submission_identity_complete(candidate.effect_intent.as_deref())))
}

fn resume_refusal(
    candidate: &PendingPublishVerification,
    state: &str,
    active: bool,
) -> Option<&'static str> {
    resume_refusal_in(
        candidate,
        state,
        active,
        &RowEvidence::new(candidate.evidence_json.as_deref()),
    )
}

fn resume_refusal_in(
    candidate: &PendingPublishVerification,
    state: &str,
    active: bool,
    evidence: &RowEvidence,
) -> Option<&'static str> {
    if active {
        Some("activePipeline")
    } else if !matches!(state, "verifying" | "uncertain" | "succeeded")
        || !has_post_intent(candidate.effect_intent.as_deref())
    {
        Some("notSubmitted")
    } else if !submission_identity_complete(candidate.effect_intent.as_deref()) {
        Some("submissionIdentityMissing")
    } else if explicit_non_stop_review_in(evidence.get()) {
        Some("explicitReview")
    } else {
        None
    }
}

impl Database {
    /// Confirmation authorizes observation only, never changes the Post intent or dispatch jobs.
    pub fn resume_publish_verification(
        &self,
        assignment_id: &str,
        confirmed: bool,
        expected_revision: i64,
    ) -> anyhow::Result<PublishResumeVerificationResult> {
        use PublishResumeVerificationState as State;
        let result = |state, reason: Option<&str>| PublishResumeVerificationResult {
            assignment_id: assignment_id.into(),
            state,
            reason: reason.map(str::to_owned),
        };
        if !confirmed {
            return Ok(result(State::Ineligible, Some("confirmationRequired")));
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some((mut candidate, state, active_pipeline)) = recovery_row(&tx, assignment_id)?
        else {
            return Ok(result(State::Ineligible, Some("assignmentMissing")));
        };
        if evidence_has_post_link(candidate.evidence_json.as_deref()) {
            return Ok(result(State::AlreadyVerified, None));
        }
        if publish_close_pending(&tx, &candidate.campaign_id)? {
            return Ok(result(State::Ineligible, Some("stopInProgress")));
        }
        if candidate.revision != expected_revision {
            // Lost ACK replay is tied to the exact revision the operator confirmed.
            let replay = candidate
                .evidence_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                .is_some_and(|v| {
                    v["verificationResume"]["version"] == 2
                        && v["verificationResume"]["expectedRevision"].as_i64()
                            == Some(expected_revision)
                });
            if replay
                && explicitly_resumed_verification(&candidate)
                && !needs_review(candidate.evidence_json.as_deref())
            {
                return Ok(result(State::AlreadyPending, None));
            }
            return Ok(result(State::Stale, Some("revisionChanged")));
        }
        if let Some(reason) = resume_refusal(&candidate, &state, active_pipeline) {
            return Ok(result(State::Ineligible, Some(reason)));
        }
        if already_pending(&tx, &candidate)? {
            return Ok(result(State::AlreadyPending, None));
        }
        let now = Utc::now().to_rfc3339();
        // Legacy stop markers never grant access; upgrade their generation only after
        // an explicit confirmation. Keep the stop marker and cancelled campaign intact.
        if candidate.stop_marker.is_some()
            && stop_generation(candidate.stop_marker.as_deref()).is_none()
        {
            let mut marker = candidate
                .stop_marker
                .as_deref()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                .filter(serde_json::Value::is_object)
                .unwrap_or_else(|| serde_json::json!({}));
            marker["generation"] = serde_json::json!(Uuid::new_v4().to_string());
            marker["version"] = serde_json::json!(2);
            let raw = marker.to_string();
            tx.execute(
                "UPDATE settings SET value=?2 WHERE key=?1",
                params![
                    format!("operation.stop.publish:{}", candidate.campaign_id),
                    raw
                ],
            )?;
            candidate.stop_marker = Some(raw);
        }
        use sha2::Digest;
        let mut evidence = candidate
            .evidence_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| serde_json::json!({}));
        // Only this confirmed revision resets the automatic budget. Preserve the
        // parked reason/budget for audit, including a review followed by Stop.
        evidence["verificationBeforeResume"] = serde_json::json!({
            "status":evidence["verificationStatus"],"budget":evidence["verificationBudget"],
            "reviewBeforeStop":evidence["verificationReviewBeforeStop"],"resumedAt":now,
        });
        let mut budget = new_verification_budget(candidate.effect_intent.as_deref());
        // Resume opens a fresh processing wait rather than one already spent since Post.
        budget["processingWaitFrom"] = serde_json::json!(now);
        // The proof frontier belongs to the exact intent, not to the budget: keep it so a
        // stage-2 post still earns its restart after resume. Only the counters reset.
        let prior = &evidence["verificationBudget"];
        if prior["version"] == 1 && prior["intentSha256"] == budget["intentSha256"] {
            budget["publicationStage"] =
                serde_json::json!(prior["publicationStage"].as_u64().unwrap_or(0).min(2));
            if prior["provenCandidateUrl"].is_string() {
                budget["provenCandidateUrl"] = prior["provenCandidateUrl"].clone();
            }
        }
        evidence["verificationBudget"] = budget;
        if let Some(object) = evidence.as_object_mut() {
            object.remove("verificationReviewBeforeStop");
        }
        evidence["verificationResume"] = serde_json::json!({"version":2,"assignmentId":assignment_id,"campaignId":candidate.campaign_id,
            "intentSha256":format!("{:x}",sha2::Sha256::digest(candidate.effect_intent.as_deref().unwrap_or_default().as_bytes())),
            "stopGeneration":stop_generation(candidate.stop_marker.as_deref()),"authorizationId":Uuid::new_v4().to_string(),"requestedAt":now,"expectedRevision":expected_revision});
        let attempts = evidence["verificationStatus"]["attempts"]
            .as_u64()
            .unwrap_or(0);
        evidence["verificationStatus"] = serde_json::json!({"state":"pending","cause":"operatorResumed","reasonCode":"operatorResumed","reason":"Tiếp tục xác minh bài đã gửi; không đăng lại","attempts":attempts,"checkIntervalSeconds":VERIFICATION_CHECK_SECONDS,"nextCheckAt":now,"deadlineAt":null});
        tx.execute("UPDATE publish_assignments SET state='verifying',error_code='post_verification_pending',evidence_json=?2,revision=revision+1,updated_at=?3 WHERE id=?1 AND revision=?4",params![assignment_id,evidence.to_string(),now,expected_revision])?;
        tx.execute(
            "DELETE FROM settings WHERE key=?1",
            [format!("publish.stop-release.{assignment_id}")],
        )?;
        reconcile_verification_snapshot(&tx, &candidate.campaign_id, &now)?;
        tx.commit()?;
        Ok(result(State::Accepted, None))
    }

    /// Advisory only: mutations repeat these checks under an immediate transaction.
    pub fn publish_recovery_capabilities(
        &self,
        campaign_id: &str,
    ) -> anyhow::Result<Vec<PublishRecoveryCapabilities>> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let ids = tx
            .prepare("SELECT id FROM publish_assignments WHERE campaign_id=?1 ORDER BY ordinal")?
            .query_map([campaign_id], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut capabilities = Vec::new();
        // The UI polls this every 2 s over rows whose evidence runs to ~200 KB (r134, 16/10/2026):
        // each row's evidence is parsed once for every check below, and the campaign request once
        // per call, which this one read snapshot keeps the same for every row.
        let mut sheet_enabled = None;
        for id in ids {
            let Some((candidate, state, active_pipeline)) = recovery_row(&tx, &id)? else {
                continue;
            };
            let evidence = RowEvidence::new(candidate.evidence_json.as_deref());
            let linked = post_link_in(evidence.get());
            let resumed = explicitly_resumed_in(&candidate, &evidence) && !linked;
            let check_reason = if linked {
                Some("alreadyVerified")
            } else if (candidate.stop_marker.is_some() && !resumed)
                || !observer_authorized_in(&tx, &candidate, &evidence)?
            {
                Some("operatorStopped")
            } else if !may_verify_in(&state, candidate.effect_intent.as_deref(), &evidence) {
                Some("noCandidate")
            } else if !submission_identity_complete(candidate.effect_intent.as_deref()) {
                Some("submissionIdentityMissing")
            } else {
                None
            };
            // Exact claim_publish_assignment_retry predicate, including the live parent
            // and assignment job guard; do not make retry look safer than its DB claim.
            let retry: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM publish_assignments a JOIN publish_campaigns c ON c.id=a.campaign_id WHERE a.id=?1 AND a.state='failed_before_dispatch' AND a.effect_intent IS NULL AND c.state IN ('failed_before_dispatch','verifying','uncertain','posting') AND NOT EXISTS(SELECT 1 FROM publish_dispatch_jobs j WHERE j.assignment_id=a.id AND j.state IN ('queued','running','paused')) AND NOT EXISTS(SELECT 1 FROM publish_exclude_requests e WHERE e.assignment_id=a.id))",[&id],|r|r.get(0))?;
            let capability = |reason: Option<&str>| PublishRecoveryCapability {
                allowed: reason.is_none(),
                reason: reason.map(str::to_owned),
            };
            let resume_reason = if linked {
                Some("alreadyVerified")
            } else if publish_close_pending(&tx, &candidate.campaign_id)? {
                Some("stopInProgress")
            } else if already_pending_in(&tx, &candidate, &evidence)? {
                Some("alreadyPending")
            } else {
                resume_refusal_in(&candidate, &state, active_pipeline, &evidence)
            };
            let sheet_required = match sheet_enabled {
                Some(enabled) => enabled,
                None => *sheet_enabled.insert(
                    tx.query_row(
                        "SELECT request_json FROM publish_campaigns WHERE id=?1",
                        [campaign_id],
                        |r| r.get::<_, String>(0),
                    )
                    .map_err(anyhow::Error::from)
                    .and_then(|raw| {
                        Ok(serde_json::from_str::<crate::PublishCampaignRequest>(&raw)?
                            .sheet_enabled)
                    })?,
                ),
            };
            capabilities.push(PublishRecoveryCapabilities {
                sheet_required,
                exclusion: tx.query_row("SELECT e.assignment_id,e.state,a.revision,e.reason
                    FROM publish_exclude_requests e JOIN publish_assignments a ON a.id=e.assignment_id
                    WHERE e.assignment_id=?1 ORDER BY e.updated_at DESC LIMIT 1", [&id], |r| Ok(PublishExcludeReceipt {
                        assignment_id:r.get(0)?,state:r.get(1)?,revision:r.get(2)?,reason:r.get(3)?,
                    })).optional()?,
                recovery: tx
                    .query_row(
                        "SELECT payload FROM publish_recovery_state WHERE assignment_id=?1",
                        [&id],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()?
                    .map(|s| serde_json::from_str(&s))
                    .transpose()?,
                assignment_id: id,
                revision: candidate.revision,
                check_link: capability(check_reason),
                resume_verification: capability(resume_reason),
                retry_before_post: capability(if retry {
                    None
                } else if active_pipeline {
                    Some("activePipeline")
                } else {
                    Some("notRetryableBeforePost")
                }),
                verification_resumed: resumed,
            });
        }
        Ok(capabilities)
    }

    /// Cancel a queue-only campaign without pretending to have closed an app. The
    /// immediate transaction fences a concurrent dispatcher claim before classifying it.
    pub fn stop_unstarted_publish_operation(
        &self,
        campaign_id: &str,
    ) -> anyhow::Result<Option<crate::ipc_contract::OperationStopResult>> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let pristine: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM publish_campaigns c WHERE c.id=?1
             AND c.state IN ('queued','scheduled')
             AND EXISTS(SELECT 1 FROM publish_assignments a WHERE a.campaign_id=c.id)
             AND NOT EXISTS(SELECT 1 FROM publish_assignments a WHERE a.campaign_id=c.id
                 AND (a.state NOT IN ('queued','scheduled') OR a.effect_intent IS NOT NULL OR a.evidence_json IS NOT NULL))
             AND NOT EXISTS(SELECT 1 FROM publish_pipeline_runs r WHERE r.campaign_id=c.id)
             AND NOT EXISTS(SELECT 1 FROM publish_attempts p WHERE p.campaign_id=c.id)
             AND NOT EXISTS(SELECT 1 FROM publish_dispatch_jobs j WHERE j.campaign_id=c.id
                 AND (j.started_at_ms IS NOT NULL OR j.state NOT IN ('queued','paused')))
             AND NOT EXISTS(SELECT 1 FROM publish_account_reservations r JOIN publish_assignments a ON a.id=r.assignment_id WHERE a.campaign_id=c.id)
             AND NOT EXISTS(SELECT 1 FROM publish_recovery_state r JOIN publish_assignments a ON a.id=r.assignment_id WHERE a.campaign_id=c.id))",
            [campaign_id], |r| r.get(0),
        )?;
        if !pristine {
            return Ok(None);
        }
        let devices = tx
            .prepare(
                "SELECT DISTINCT udid FROM publish_assignments WHERE campaign_id=?1 ORDER BY udid",
            )?
            .query_map([campaign_id], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let now = Utc::now().to_rfc3339();
        tx.execute("UPDATE publish_campaigns SET state='cancelled',revision=revision+1,updated_at=?2 WHERE id=?1",params![campaign_id,now])?;
        tx.execute("UPDATE publish_assignments SET state='cancelled',revision=revision+1,updated_at=?2 WHERE campaign_id=?1",params![campaign_id,now])?;
        tx.execute("UPDATE publish_dispatch_jobs SET state='cancelled',reason='cancelled_before_start',revision=revision+1,finished_at_ms=?2 WHERE campaign_id=?1",params![campaign_id,Utc::now().timestamp_millis()])?;
        let marker = serde_json::json!({"version":2,"requestedAt":now,"generation":Uuid::new_v4().to_string(),"closePending":false,"neverStarted":true}).to_string();
        let result = crate::ipc_contract::OperationStopResult {
            operation_id: format!("publish:{campaign_id}"),
            state: "closed".into(),
            stop_marker: Some(marker.clone()),
            devices: devices
                .into_iter()
                .map(|udid| crate::ipc_contract::StopDeviceResult {
                    udid,
                    closed: true,
                    message: "Đã hủy lượt chưa bắt đầu; không có phiên thiết bị cần đóng".into(),
                })
                .collect(),
        };
        tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![format!("operation.stop.publish:{campaign_id}"),marker])?;
        tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![format!("operation.stop.result:publish:{campaign_id}"),serde_json::to_string(&result)?])?;
        tx.execute("INSERT INTO publish_events(campaign_id,revision,kind,payload_json,created_at) SELECT id,revision,'state',?2,?3 FROM publish_campaigns WHERE id=?1",
            params![campaign_id,serde_json::json!({"state":"cancelled","source":"stopBeforeStart"}).to_string(),now])?;
        tx.commit()?;
        Ok(Some(result))
    }

    /// Arms the durable close barrier in the SAME transaction as observer revocation.
    /// A crash before the command writes its async result still cannot permit resume.
    pub fn begin_publish_operation_stop(&self, campaign_id: &str) -> anyhow::Result<Vec<String>> {
        self.stop_publish_operation_inner(campaign_id, true)
    }

    /// Explicit operator stop: preserve Post intents/receipts, invalidate pending observers,
    /// and park automatic verification. Never make submitted rows retryable for Post.
    pub fn stop_publish_operation(&self, campaign_id: &str) -> anyhow::Result<Vec<String>> {
        self.stop_publish_operation_inner(campaign_id, false)
    }

    fn stop_publish_operation_inner(
        &self,
        campaign_id: &str,
        will_close: bool,
    ) -> anyhow::Result<Vec<String>> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM publish_campaigns WHERE id=?1)",
            [campaign_id],
            |r| r.get(0),
        )?;
        anyhow::ensure!(exists, "Không tìm thấy tác vụ đăng bài");
        let mut statement=tx.prepare("SELECT id,udid,state,evidence_json,effect_intent FROM publish_assignments WHERE campaign_id=?1")?;
        let rows = statement
            .query_map([campaign_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        let now = Utc::now().to_rfc3339();
        let mut devices = Vec::new();
        for (id, udid, state, raw, intent) in rows {
            if !devices.contains(&udid) {
                devices.push(udid);
            }
            if state == "succeeded" && evidence_has_post_link(raw.as_deref()) {
                continue;
            }
            let mut evidence = raw
                .as_deref()
                .map(serde_json::from_str::<serde_json::Value>)
                .transpose()?
                .unwrap_or_else(|| serde_json::json!({}));
            if explicit_non_stop_review(raw.as_deref())
                && evidence.get("verificationReviewBeforeStop").is_none()
            {
                evidence["verificationReviewBeforeStop"] = evidence["verificationStatus"].clone();
            }
            evidence["verificationStatus"] = serde_json::json!({"state":"needsReview","cause":"operatorStopped","reasonCode":"operatorStopped","reason":"Người dùng đã dừng tác vụ; giữ kết quả đã đăng, không tự gửi lại","checkedAt":now,"nextCheckAt":null});
            let next = if state == "succeeded" {
                "succeeded"
            } else if intent.is_some()
                || matches!(state.as_str(), "posting" | "verifying" | "uncertain")
            {
                "uncertain"
            } else {
                "cancelled"
            };
            tx.execute("UPDATE publish_assignments SET state=?2,evidence_json=?3,revision=revision+1,updated_at=?4 WHERE id=?1",params![id,next,evidence.to_string(),now])?;
        }
        tx.execute("UPDATE publish_campaigns SET state=CASE WHEN state='succeeded' THEN state ELSE 'cancelled' END,revision=revision+1,updated_at=?2 WHERE id=?1",params![campaign_id,now])?;
        tx.execute("UPDATE publish_dispatch_jobs SET state='cancelled' WHERE campaign_id=?1 AND state='queued'",[campaign_id])?;
        tx.execute("UPDATE publish_recovery_state SET payload=json_set(payload,'$.state','stopped','$.nextRetryAt',NULL,'$.reconnectDeadline',NULL) WHERE assignment_id IN(SELECT id FROM publish_assignments WHERE campaign_id=?1)",[campaign_id])?;
        let result: Option<String> = tx
            .query_row(
                "SELECT value FROM settings WHERE key=?1",
                [format!("operation.stop.result:publish:{campaign_id}")],
                |r| r.get(0),
            )
            .optional()?;
        let already_closed = result
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .is_some_and(|v| v["state"] == "closed");
        let close_pending =
            will_close || (!already_closed && publish_close_pending(&tx, campaign_id)?);
        tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![format!("operation.stop.publish:{campaign_id}"),serde_json::json!({"version":if will_close {3}else{2},"requestedAt":now,"generation":Uuid::new_v4().to_string(),"closePending":close_pending}).to_string()])?;
        tx.commit()?;
        Ok(devices)
    }

    pub fn publish_operation_stopped(&self, campaign_id: &str) -> anyhow::Result<bool> {
        Ok(self
            .get_setting(&format!("operation.stop.publish:{campaign_id}"))?
            .is_some())
    }

    /// Called only after the exact device closer has ended its exclusive context.
    /// The stop marker fences a delayed closer from releasing a resumed attempt.
    pub fn record_publish_stopped_device_release(
        &self,
        campaign_id: &str,
        udid: &str,
        expected_marker: &str,
    ) -> anyhow::Result<bool> {
        use sha2::Digest;
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if stop_marker(&tx, campaign_id)?.as_deref() != Some(expected_marker) {
            return Ok(false);
        }
        let rows = tx.prepare("SELECT id,effect_intent,evidence_json FROM publish_assignments WHERE campaign_id=?1 AND udid=?2")?
            .query_map(params![campaign_id,udid], |r| Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,Option<String>>(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (id, intent, evidence) in rows {
            if evidence_has_post_link(evidence.as_deref()) {
                continue;
            }
            let stopped = evidence
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                .is_some_and(|e| e["verificationStatus"]["cause"] == "operatorStopped");
            if !stopped {
                return Ok(false);
            }
            let proof = serde_json::json!({"assignmentId":id,"udid":udid,"stopMarker":expected_marker,
                "intentSha256":intent.as_ref().map(|s|format!("{:x}",sha2::Sha256::digest(s.as_bytes())))});
            tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![format!("publish.stop-release.{id}"),proof.to_string()])?;
            tx.execute(
                "DELETE FROM publish_account_reservations WHERE assignment_id=?1",
                [&id],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }
    pub fn publish_verification_is_current(
        &self,
        candidate: &PendingPublishVerification,
    ) -> anyhow::Result<bool> {
        let conn = self.conn()?;
        let Some((current, state, _)) = recovery_row(&conn, &candidate.assignment_id)? else {
            return Ok(false);
        };
        Ok(current.campaign_id == candidate.campaign_id
            && current.revision == candidate.revision
            && current.effect_intent == candidate.effect_intent
            && current.evidence_json == candidate.evidence_json
            && may_verify(
                &state,
                current.effect_intent.as_deref(),
                current.evidence_json.as_deref(),
            )
            && observer_authorized(&conn, candidate)?)
    }

    /// Reserve one destructive app restart for this immutable Post intent.
    /// A crash after the claim leaves the receipt observational instead of
    /// repeatedly closing TikTok on every verification tick.
    pub fn claim_publish_processing_restart(
        &self,
        candidate: &PendingPublishVerification,
    ) -> anyhow::Result<bool> {
        use sha2::Digest;
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some((current, state, active)) = recovery_row(&tx, &candidate.assignment_id)? else {
            return Ok(false);
        };
        if active
            || !may_verify(&state, current.effect_intent.as_deref(), current.evidence_json.as_deref())
            || current.campaign_id != candidate.campaign_id
            || current.udid != candidate.udid
            || current.revision != candidate.revision
            || current.effect_intent != candidate.effect_intent
            || current.evidence_json != candidate.evidence_json
            || !observer_authorized(&tx, candidate)?
        {
            return Ok(false);
        }
        let Some(intent) = candidate.effect_intent.as_deref() else { return Ok(false) };
        let intent_sha256 = format!("{:x}", sha2::Sha256::digest(intent.as_bytes()));
        let key = format!("publish.processing_restart.{}.{}",
            candidate.assignment_id, intent_sha256);
        let proof = serde_json::json!({"assignmentId":candidate.assignment_id,
            "campaignId":candidate.campaign_id,"udid":candidate.udid,
            "intentSha256":intent_sha256,
            "claimedAt":Utc::now().to_rfc3339(),"observerRevision":candidate.revision});
        let claimed = tx.execute(
            "INSERT OR IGNORE INTO settings(key,value) VALUES(?1,?2)",
            params![key, proof.to_string()],
        )? == 1;
        tx.commit()?;
        Ok(claimed)
    }

    pub fn observe_stale_publish_idle_for_candidate(
        &self,
        candidate: &PendingPublishVerification,
        account: &str,
        package: &str,
        snapshot_sha256: &str,
    ) -> anyhow::Result<bool> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some((current, state, active)) = recovery_row(&tx, &candidate.assignment_id)? else {
            return Ok(false);
        };
        if active
            || !matches!(state.as_str(), "uncertain" | "verifying")
            || current.revision != candidate.revision
            || current.campaign_id != candidate.campaign_id
            || current.effect_intent != candidate.effect_intent
            || current.evidence_json != candidate.evidence_json
            || !observer_authorized(&tx, candidate)?
        {
            return Ok(false);
        }
        let changed = store_idle_observation(&tx, candidate, account, package, snapshot_sha256)?;
        tx.commit()?;
        Ok(changed)
    }

    /// A stale submission is still unresolved, but a fresh own-profile observation
    /// proves this old receipt is no longer an active upload blocking the phone.
    /// This does not settle publication, create a link or reopen its Send claim.
    pub fn observe_stale_publish_idle(
        &self,
        assignment_id: &str,
        account: &str,
        package: &str,
        snapshot_sha256: &str,
    ) -> anyhow::Result<bool> {
        // Compatibility for synchronous callers. Stopped work requires the snapshot
        // API; callers spanning a device await must never acquire fresh authority here.
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some((candidate, state, active)) = recovery_row(&tx, assignment_id)? else {
            return Ok(false);
        };
        if active
            || candidate.stop_marker.is_some()
            || !matches!(state.as_str(), "uncertain" | "verifying")
        {
            return Ok(false);
        }
        let changed = store_idle_observation(&tx, &candidate, account, package, snapshot_sha256)?;
        tx.commit()?;
        Ok(changed)
    }

    /// Stamp the first background observation before a submitted assignment becomes visible.
    /// Metadata comes from its durable campaign/intent, not an IPC flag or carried evidence.
    pub fn with_initial_scheduled_verification(
        &self,
        campaign_id: &str,
        assignment_id: &str,
        evidence_json: &str,
    ) -> anyhow::Result<String> {
        let (scheduled, intent): (bool, Option<String>) = self.conn()?.query_row(
            "SELECT c.run_at IS NOT NULL,a.effect_intent FROM publish_assignments a JOIN publish_campaigns c ON c.id=a.campaign_id WHERE a.id=?1 AND c.id=?2",
            params![assignment_id,campaign_id],|row|Ok((row.get(0)?,row.get(1)?)),
        )?;
        if !scheduled || !submission_identity_complete(intent.as_deref()) {
            return Ok(evidence_json.to_owned());
        }
        let mut evidence: serde_json::Value = serde_json::from_str(evidence_json)?;
        evidence["verificationStatus"] = serde_json::json!({
            "state":"pending",
            "reason":"Đã gửi bài hẹn giờ; Android mở lại TikTok để lấy link, nếu chưa được sẽ thử lại sau 2 phút",
            "cause":"deferredScheduledLink", "attempts":0, "readFailures":0,
            "checkIntervalSeconds":VERIFICATION_CHECK_SECONDS, "reviewAfterMinutes":null, "deadlineAt":null, "nextCheckAt":first_scheduled_check(intent.as_deref()).map(|at|at.to_rfc3339()),
        });
        Ok(evidence.to_string())
    }

    /// Automatic checks omit explicit review except the retired age limits.
    pub fn pending_publish_verifications(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<PendingPublishVerification>> {
        self.publish_verification_candidates(limit, None, false, false)
    }

    pub fn pending_current_publish_verifications(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<PendingPublishVerification>> {
        self.publish_verification_candidates(limit, None, false, true)
    }

    /// Automatic recovery restricted to one campaign; unlike explicit link checks,
    /// this does not reopen review rows or admit legacy verification contracts.
    pub fn pending_current_publish_verifications_for_campaign(
        &self,
        campaign_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<PendingPublishVerification>> {
        self.publish_verification_candidates(limit, Some(campaign_id), false, true)
    }

    /// An explicit warm link check may settle review rows without opening the Post path.
    pub fn publish_verifications_for_campaign(
        &self,
        campaign_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<PendingPublishVerification>> {
        self.publish_verification_candidates(limit, Some(campaign_id), true, false)
    }

    fn publish_verification_candidates(
        &self,
        limit: usize,
        campaign_id: Option<&str>,
        include_review: bool,
        current_contract_only: bool,
    ) -> anyhow::Result<Vec<PendingPublishVerification>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = limit.min(1000);
        let now = Utc::now();
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            "SELECT a.id,a.campaign_id,a.bundle_id,a.udid,a.revision,a.effect_intent,a.evidence_json,a.state,c.run_at IS NOT NULL,
             COALESCE(json_extract(c.request_json,'$.verificationContractVersion')=1,0),
             (SELECT value FROM settings WHERE key='operation.stop.publish:'||a.campaign_id)
             FROM publish_assignments a JOIN publish_campaigns c ON c.id=a.campaign_id
             LEFT JOIN publish_dispatch_turns t ON t.udid=a.udid
             WHERE a.state IN ('verifying','uncertain','succeeded') AND (?1 IS NULL OR a.campaign_id=?1)
             AND (?2=0 OR json_extract(c.request_json,'$.verificationContractVersion')=1
                 OR (json_valid(a.evidence_json) AND json_extract(a.evidence_json,'$.verificationResume.version') IN (1,2)))
             ORDER BY CASE WHEN instr(a.evidence_json,'\"priorPostPriority\"')>0 AND json_valid(a.evidence_json)
                 AND EXISTS(SELECT 1 FROM publish_dispatch_jobs p
                     WHERE p.assignment_id=json_extract(a.evidence_json,'$.priorPostPriority.assignmentId')
                     AND p.state IN ('queued','running')) THEN 0 ELSE 1 END,
             COALESCE(t.last_turn,0),a.updated_at,a.campaign_id,a.ordinal"
        )?;
        let rows = statement.query_map(params![campaign_id, current_contract_only], |row| {
            Ok((
                PendingPublishVerification {
                    assignment_id: row.get(0)?,
                    campaign_id: row.get(1)?,
                    bundle_id: row.get(2)?,
                    udid: row.get(3)?,
                    scheduled: row.get(8)?,
                    revision: row.get(4)?,
                    effect_intent: row.get(5)?,
                    evidence_json: row.get(6)?,
                    stop_marker: row.get(10)?,
                },
                row.get::<_, String>(7)?,
                row.get::<_, bool>(9)?,
            ))
        })?;
        // Prefer due rows so a long not-due schedule tail cannot starve the worker / expire
        // path under the 1000-candidate ceiling.
        let mut due = Vec::new();
        let mut later = Vec::new();
        for row in rows {
            let (candidate, state, current_contract) = row?;
            if !observer_authorized(&conn, &candidate)? {
                continue;
            }
            if candidate.stop_marker.is_some()
                && (stop_generation(candidate.stop_marker.as_deref()).is_none()
                    || !explicitly_resumed_verification(&candidate))
            {
                continue;
            }
            if current_contract_only
                && !current_contract
                && !explicitly_resumed_verification(&candidate)
            {
                continue;
            }
            if !(include_review
                || !needs_review(candidate.evidence_json.as_deref())
                || obsolete_deadline_review(
                    candidate.effect_intent.as_deref(),
                    candidate.evidence_json.as_deref(),
                ))
                || !may_verify(
                    &state,
                    candidate.effect_intent.as_deref(),
                    candidate.evidence_json.as_deref(),
                )
            {
                continue;
            }
            if candidate.is_due(now) {
                due.push(candidate);
                if due.len() == limit {
                    break;
                }
            } else if later.len() < limit {
                later.push(candidate);
            }
        }
        let mut pending = due;
        if pending.len() < limit {
            let room = limit - pending.len();
            pending.extend(later.into_iter().take(room));
        } else {
            pending.truncate(limit);
        }
        Ok(pending)
    }

    /// A new automation session must not cold-start TikTok over a pending upload.
    pub fn has_pending_publish_for_device(&self, udid: &str) -> anyhow::Result<bool> {
        Ok(!self.publish_device_guard(udid)?.blocking.is_empty())
    }

    /// Read-only split between a protected upload and an old completed upload's
    /// missing-link obligation. Never edits state, evidence, retry scope or outbox.
    pub fn publish_device_guard(&self, udid: &str) -> anyhow::Result<PublishDeviceGuard> {
        let conn = self.conn()?;
        Self::publish_device_guard_from_connection(&conn, udid)
    }

    pub(super) fn publish_device_guard_from_connection(
        conn: &Connection,
        udid: &str,
    ) -> anyhow::Result<PublishDeviceGuard> {
        let mut statement = conn.prepare(
            "SELECT a.state,a.effect_intent,a.evidence_json,a.id,a.campaign_id,a.updated_at,
             EXISTS(SELECT 1 FROM publish_pipeline_runs p WHERE p.campaign_id=a.campaign_id)
             FROM publish_assignments a
             WHERE a.udid=?1 AND a.state IN ('posting','verifying','uncertain','succeeded')
             ORDER BY a.updated_at,a.id",
        )?;
        let rows = statement.query_map([udid], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, bool>(6)?,
            ))
        })?;
        let mut guard = PublishDeviceGuard::default();
        let now = Utc::now();
        for row in rows {
            let (state, intent, evidence, assignment_id, campaign_id, updated_at, active_pipeline) =
                row?;
            if state == "posting"
                || state == "verifying"
                || state == "uncertain"
                || may_verify(&state, intent.as_deref(), evidence.as_deref())
            {
                let reason = evidence
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                    .and_then(|e| {
                        e["verificationStatus"]["reason"]
                            .as_str()
                            .map(str::to_owned)
                    })
                    .unwrap_or_else(|| "Bài đang tải hoặc chưa xác định được kết quả Đăng".into());
                let handoff_released = super::publish_start::handoff_release_current(conn, &assignment_id)?;
                let mut completed = handoff_released || completed_upload_with_link_debt(
                    &state,
                    evidence.as_deref(),
                    &updated_at,
                    active_pipeline,
                    now,
                );
                // A closer proves this device drained even while a sibling is
                // still releasing its own lease in the cancelled pipeline.
                if !completed && state != "posting" {
                    use sha2::Digest;
                    let marker = stop_marker(conn, &campaign_id)?;
                    let raw: Option<String> = conn
                        .query_row(
                            "SELECT value FROM settings WHERE key=?1",
                            [format!("publish.stop-release.{assignment_id}")],
                            |r| r.get(0),
                        )
                        .optional()?;
                    let stopped = evidence
                        .as_deref()
                        .and_then(|r| serde_json::from_str::<serde_json::Value>(r).ok())
                        .is_some_and(|e| e["verificationStatus"]["cause"] == "operatorStopped");
                    completed = stopped
                        && marker.is_some()
                        && raw
                            .and_then(|r| serde_json::from_str::<serde_json::Value>(&r).ok())
                            .is_some_and(|p| {
                                let digest = intent
                                    .as_ref()
                                    .map(|s| format!("{:x}", sha2::Sha256::digest(s.as_bytes())));
                                p["assignmentId"] == assignment_id
                                    && p["udid"] == udid
                                    && p["stopMarker"].as_str() == marker.as_deref()
                                    && p["intentSha256"].as_str() == digest.as_deref()
                            });
                }
                if !completed
                    && !active_pipeline
                    && matches!(state.as_str(), "uncertain" | "verifying")
                {
                    use sha2::Digest;
                    let raw: Option<String> = conn
                        .query_row(
                            "SELECT value FROM settings WHERE key=?1",
                            [format!("publish.idle.{assignment_id}")],
                            |r| r.get(0),
                        )
                        .optional()?;
                    let marker = stop_marker(conn, &campaign_id)?;
                    let authorization = evidence
                        .as_deref()
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                        .and_then(|v| {
                            v["verificationResume"]["authorizationId"]
                                .as_str()
                                .map(str::to_owned)
                        });
                    completed = raw
                        .and_then(|r| serde_json::from_str::<serde_json::Value>(&r).ok())
                        .is_some_and(|o| {
                            let digest = intent
                                .as_ref()
                                .map(|i| format!("{:x}", sha2::Sha256::digest(i.as_bytes())));
                            o["assignmentId"] == assignment_id
                                && o["udid"] == udid
                                && o["intentSha256"].as_str() == digest.as_deref()
                                && o["stopMarker"].as_str() == marker.as_deref()
                                && o["authorizationId"].as_str() == authorization.as_deref()
                                && o["observedAt"]
                                    .as_str()
                                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                                    .is_some_and(|at| {
                                        let age = now.signed_duration_since(at);
                                        age >= chrono::Duration::zero()
                                            && age < chrono::Duration::days(1)
                                    })
                        });
                }
                let hold = PublishDeviceHold {
                    assignment_id,
                    campaign_id,
                    updated_at,
                    reason,
                };
                if completed {
                    guard.link_review.push(hold);
                } else {
                    guard.blocking.push(hold);
                }
            }
        }
        Ok(guard)
    }

    /// Resume retired age-limit reviews and park receipts missing submission identity.
    /// This is a worker mutation, never a UI read. Existing uploads and imports remain owned.
    pub fn expire_due_publish_verifications(&self) -> anyhow::Result<Vec<String>> {
        self.expire_publish_verifications(false)
    }

    pub fn expire_current_publish_verifications(&self) -> anyhow::Result<Vec<String>> {
        self.expire_publish_verifications(true)
    }

    fn expire_publish_verifications(
        &self,
        current_contract_only: bool,
    ) -> anyhow::Result<Vec<String>> {
        // Exclusions lifted here become ordinary candidates below and in the same tick's queue.
        let mut campaigns = self
            .resume_publish_verifications_after_handoff(|_, _| true)
            .unwrap_or_else(|error| {
                tracing::warn!("tự tiếp tục xác minh sau khi trả máy: {error:#}");
                Vec::new()
            });
        let now = Utc::now();
        for candidate in
            self.publish_verification_candidates(1000, None, true, current_contract_only)?
        {
            let reopen = obsolete_deadline_review(
                candidate.effect_intent.as_deref(),
                candidate.evidence_json.as_deref(),
            );
            // Other reviews stay parked, except the bounded automatic re-check.
            if needs_review(candidate.evidence_json.as_deref()) && !reopen {
                if self.recheck_parked_publish_verification(&candidate, now)?
                    && !campaigns.contains(&candidate.campaign_id)
                {
                    campaigns.push(candidate.campaign_id);
                }
                continue;
            }
            if !reopen && submission_identity_complete(candidate.effect_intent.as_deref()) {
                continue;
            }
            let reason = if reopen {
                "Tiếp tục tự kiểm tra liên kết mỗi 2 phút; bài đã gửi chưa được xác minh"
            } else {
                LEGACY_IDENTITY_REVIEW_REASON
            };
            if self.record_publish_verification_pending(&candidate, reason)?
                && !campaigns.contains(&candidate.campaign_id)
            {
                campaigns.push(candidate.campaign_id);
            }
        }
        Ok(campaigns)
    }

    /// The 30-minute re-check alone (`automatic_recheck_due`), for the parked rows of one
    /// campaign that `admit(campaign, udid)` accepts: dev acceptance keeps expiry off, but
    /// measures the re-check inside its pinned scope, one scoped campaign at a time. Worker
    /// mutation, never a UI read. Returns the campaigns changed.
    pub fn recheck_parked_publish_verifications(
        &self,
        campaign_id: &str,
        admit: impl Fn(&str, &str) -> bool,
    ) -> anyhow::Result<Vec<String>> {
        let now = Utc::now();
        let mut campaigns = Vec::new();
        for candidate in
            self.publish_verification_candidates(1000, Some(campaign_id), true, true)?
        {
            if !admit(&candidate.campaign_id, &candidate.udid)
                || !needs_review(candidate.evidence_json.as_deref())
            {
                continue;
            }
            // One unreadable row must not hold back the others.
            match self.recheck_parked_publish_verification(&candidate, now) {
                Ok(true) if !campaigns.contains(&candidate.campaign_id) => {
                    campaigns.push(candidate.campaign_id)
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(
                    "chưa tự kiểm tra lại lượt {} trên {}: {error:#}",
                    candidate.assignment_id,
                    candidate.udid
                ),
            }
        }
        Ok(campaigns)
    }

    /// Reopen one parked row for its re-check once it is due; false when it is not.
    fn recheck_parked_publish_verification(
        &self,
        candidate: &PendingPublishVerification,
        now: DateTime<Utc>,
    ) -> anyhow::Result<bool> {
        let due = candidate
            .evidence_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .is_some_and(|evidence| automatic_recheck_due(&evidence, now));
        Ok(due
            && submission_identity_complete(candidate.effect_intent.as_deref())
            && self.reopen_publish_verification_for_recheck(candidate)?)
    }

    /// Lift each handoff exclusion off a posted row whose phone the handoff handed back
    /// (`handoff_resume_due`), for the rows `admit(campaign, udid)` accepts. Worker mutation,
    /// never a UI read. Returns the campaigns changed.
    pub fn resume_publish_verifications_after_handoff(
        &self,
        admit: impl Fn(&str, &str) -> bool,
    ) -> anyhow::Result<Vec<String>> {
        let rows: Vec<(String, String, String)> = {
            let conn = self.conn()?;
            // Cheap prefilter; each row is decided again inside its own write transaction. A park
            // comes through only with a re-check cause and reason inside its window, so parks
            // the re-check never reopens do not take the writer lane on every tick.
            let mut statement = conn.prepare(
                "SELECT a.id,a.campaign_id,a.udid FROM publish_assignments a
                 WHERE a.state IN ('verifying','uncertain','succeeded') AND a.effect_intent IS NOT NULL
                 AND NOT EXISTS(SELECT 1 FROM settings s WHERE s.key='operation.stop.publish:'||a.campaign_id)
                 AND EXISTS(SELECT 1 FROM settings s WHERE s.key='publish.handoff-release.'||a.id
                     AND json_valid(s.value) AND json_extract(s.value,'$.requestId')=
                     (SELECT e.request_id FROM publish_exclude_requests e WHERE e.assignment_id=a.id
                      ORDER BY e.rowid DESC LIMIT 1))
                 AND json_valid(a.evidence_json)
                 AND COALESCE(json_extract(a.evidence_json,'$.verificationStatus.state'),'')<>'verified'
                 AND (COALESCE(json_extract(a.evidence_json,'$.verificationStatus.state'),'')<>'needsReview'
                     OR (json_extract(a.evidence_json,'$.verificationStatus.cause') IN (SELECT value FROM json_each(?1))
                         AND json_extract(a.evidence_json,'$.verificationStatus.reasonCode') IN (SELECT value FROM json_each(?2))
                         AND julianday(COALESCE(json_extract(a.evidence_json,'$.verificationBudget.recheckSince'),
                             json_extract(a.evidence_json,'$.verificationStatus.checkedAt'))) > julianday('now')-?3))
                 AND COALESCE(json_extract(a.evidence_json,'$.verificationHandoffResume.exclusionRequestId'),'')<>
                     (SELECT e.request_id FROM publish_exclude_requests e WHERE e.assignment_id=a.id
                      ORDER BY e.rowid DESC LIMIT 1)
                 ORDER BY a.updated_at,a.id LIMIT 1000",
            )?;
            let rows = statement
                .query_map(
                    params![
                        serde_json::to_string(&AUTOMATIC_RECHECK_CAUSES)?,
                        serde_json::to_string(&AUTOMATIC_RECHECK_REASONS)?,
                        AUTOMATIC_RECHECK_WINDOW_HOURS as f64 / 24.0
                    ],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };
        let mut campaigns = Vec::new();
        for (assignment, campaign, udid) in rows {
            if !admit(&campaign, &udid) {
                continue;
            }
            // One unreadable row must not hold back the others.
            match self.resume_publish_verification_after_handoff(&assignment) {
                Ok(true) if !campaigns.contains(&campaign) => campaigns.push(campaign),
                Ok(_) => {}
                Err(error) => tracing::warn!(
                    "chưa tự tiếp tục xác minh lượt {assignment} trên {udid}: {error:#}"
                ),
            }
        }
        Ok(campaigns)
    }

    /// Record the automatic resume under CAS, before any observation. Verification only: the
    /// Post intent, state, jobs, account reservation and Sheet obligation stay as they are.
    fn resume_publish_verification_after_handoff(
        &self,
        assignment_id: &str,
    ) -> anyhow::Result<bool> {
        use sha2::Digest;
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some((candidate, state, _)) = recovery_row(&tx, assignment_id)? else {
            return Ok(false);
        };
        let Some(exclusion) = handoff_resume_due(&tx, &candidate, &state)? else {
            return Ok(false);
        };
        let Some(mut evidence) = candidate
            .evidence_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .filter(serde_json::Value::is_object)
        else {
            return Ok(false);
        };
        let intent = candidate.effect_intent.as_deref();
        let at = Utc::now().to_rfc3339();
        // `handoff_resume_due` admits a park only while its 30-minute re-check still owns it.
        let parked = needs_review(candidate.evidence_json.as_deref());
        evidence["verificationHandoffResume"] = serde_json::json!({
            "version":1,"assignmentId":assignment_id,"campaignId":candidate.campaign_id,
            "udid":candidate.udid,"exclusionRequestId":exclusion,
            "intentSha256":format!("{:x}",sha2::Sha256::digest(intent.unwrap_or_default().as_bytes())),
            "resumedAt":at,"reason":if parked {HANDOFF_RECHECK_REASON} else {HANDOFF_RESUME_REASON},
            "statusBefore":evidence["verificationStatus"].clone(),
        });
        // Counters stay: this is no new budget. Only the time windows that ran while the row was
        // excluded, unobserved, start again: the processing wait and TikTok's unanswered window.
        // A row in the 30-minute re-check spent both before its park, so they stay spent.
        let initial = new_verification_budget(intent);
        if let Some(budget) = evidence
            .get_mut("verificationBudget")
            .and_then(serde_json::Value::as_object_mut)
            .filter(|b| b.get("version").and_then(serde_json::Value::as_u64) == Some(1))
            .filter(|b| b.get("intentSha256") == Some(&initial["intentSha256"]))
            .filter(|b| !parked && !b.contains_key("recheckSince"))
        {
            budget.insert("processingWaitFrom".into(), serde_json::json!(at));
            budget.remove("metadataUnreachableSince");
        }
        if parked {
            // A park keeps its status (needsReview, cause, reasonCode, checkedAt,
            // automaticRecheckAt) and budget: the stamp alone lifts the exclusion, and the
            // re-check reopens the row at its own cadence inside the window of its first park.
        } else if evidence["verificationStatus"].is_object() {
            // Keep cause/reasonCode/attempts: the next pass reads them (restart and re-check rules).
            let status = &mut evidence["verificationStatus"];
            status["state"] = serde_json::json!("pending");
            status["reason"] = serde_json::json!(HANDOFF_RESUME_REASON);
            status["nextCheckAt"] = serde_json::json!(at);
        } else {
            evidence["verificationStatus"] = serde_json::json!({
                "state":"pending","cause":"handoffReleased","reasonCode":"handoffReleased",
                "reason":HANDOFF_RESUME_REASON,"attempts":0,"readFailures":0,
                "checkIntervalSeconds":VERIFICATION_CHECK_SECONDS,"nextCheckAt":at,"deadlineAt":null,
            });
        }
        let changed = tx.execute(
            "UPDATE publish_assignments SET evidence_json=?1,revision=revision+1,updated_at=?2
             WHERE id=?3 AND campaign_id=?4 AND revision=?5",
            params![
                evidence.to_string(),
                at,
                assignment_id,
                candidate.campaign_id,
                candidate.revision
            ],
        )?;
        if changed != 1 {
            return Ok(false);
        }
        reconcile_verification_snapshot(&tx, &candidate.campaign_id, &at)?;
        tx.commit()?;
        if parked {
            tracing::info!(
                "Giữ lịch tự kiểm tra lại mỗi {} phút của bài đã gửi trên {} sau khi lượt mới trả máy (lượt {}, loại {}); không đăng lại",
                AUTOMATIC_RECHECK_MINUTES,
                candidate.udid,
                assignment_id,
                exclusion
            );
        } else {
            tracing::info!(
                "Tự tiếp tục xác minh bài đã gửi trên {} sau khi lượt mới trả máy (lượt {}, loại {}); không đăng lại",
                candidate.udid,
                assignment_id,
                exclusion
            );
        }
        Ok(true)
    }

    /// One more read-only observation of a parked row (`automatic_recheck_due`), under the same
    /// CAS and observer authority as any observation. Its exhausted counters stay as they are,
    /// so the next unproven observation parks it again; only a verified link or new proof of
    /// the post moves it on. Never touches the Post intent, media or Sheet obligation.
    fn reopen_publish_verification_for_recheck(
        &self,
        candidate: &PendingPublishVerification,
    ) -> anyhow::Result<bool> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some((current, state, active)) = recovery_row(&tx, &candidate.assignment_id)? else {
            return Ok(false);
        };
        if active
            || current.campaign_id != candidate.campaign_id
            || current.revision != candidate.revision
            || current.effect_intent != candidate.effect_intent
            || current.evidence_json != candidate.evidence_json
            || !may_verify(
                &state,
                current.effect_intent.as_deref(),
                current.evidence_json.as_deref(),
            )
            || !observer_authorized(&tx, candidate)?
        {
            return Ok(false);
        }
        let Some(mut evidence) = candidate
            .evidence_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .filter(serde_json::Value::is_object)
        else {
            return Ok(false);
        };
        let now = Utc::now();
        if !automatic_recheck_due(&evidence, now) {
            return Ok(false);
        }
        // Only a budget of this exact intent: a re-check never hands out fresh counters.
        let initial = new_verification_budget(candidate.effect_intent.as_deref());
        if evidence["verificationBudget"]["version"] != 1
            || evidence["verificationBudget"]["intentSha256"] != initial["intentSha256"]
        {
            return Ok(false);
        }
        let parked = evidence["verificationStatus"].clone();
        let budget = &mut evidence["verificationBudget"];
        if !budget["recheckSince"].is_string() {
            budget["recheckSince"] = parked["checkedAt"].clone();
        }
        let count = budget["recheckCount"]
            .as_u64()
            .unwrap_or(0)
            .saturating_add(1);
        budget["recheckCount"] = serde_json::json!(count);
        let at = now.to_rfc3339();
        evidence["verificationStatus"] = serde_json::json!({
            "state":"pending","cause":AUTOMATIC_RECHECK_CAUSE,"parkedCause":parked["cause"],
            "reasonCode":parked["reasonCode"],"observationReason":parked["observationReason"],
            "reason":format!("Tự kiểm tra lại lần {count}: lần dừng trước không phải bằng chứng bài sai (TikTok còn xử lý bài hoặc chưa trả lời, hoặc lượt đọc trên máy bị cắt ngắn). App chỉ đọc lại mỗi {AUTOMATIC_RECHECK_MINUTES} phút trong {AUTOMATIC_RECHECK_WINDOW_HOURS} giờ, tối đa {AUTOMATIC_RECHECK_COPY_PASSES} lượt có bấm Sao chép liên kết; không đăng lại."),
            "checkedAt":parked["checkedAt"],"submittedAt":parked["submittedAt"],
            "attempts":parked["attempts"],"readFailures":parked["readFailures"],
            "checkIntervalSeconds":AUTOMATIC_RECHECK_MINUTES * 60,"reviewAfterMinutes":null,
            "nextCheckAt":at,"deadlineAt":null,
        });
        let changed = tx.execute(
            "UPDATE publish_assignments SET state='verifying',error_code='post_verification_pending',evidence_json=?1,revision=revision+1,updated_at=?2
             WHERE id=?3 AND campaign_id=?4 AND revision=?5 AND state IN ('verifying','uncertain','succeeded')",
            params![evidence.to_string(), at, candidate.assignment_id, candidate.campaign_id, candidate.revision],
        )?;
        if changed != 1 {
            return Ok(false);
        }
        reconcile_verified_campaign(&tx, &candidate.campaign_id, &at)?;
        reconcile_verification_snapshot(&tx, &candidate.campaign_id, &at)?;
        tx.commit()?;
        tracing::info!(
            "Tự kiểm tra lại lần {} bài đã gửi trên {} (lượt {}, dừng vì {}/{}); chỉ đọc, không đăng lại",
            count,
            candidate.udid,
            candidate.assignment_id,
            parked["cause"].as_str().unwrap_or_default(),
            parked["reasonCode"].as_str().unwrap_or_default()
        );
        Ok(true)
    }

    /// Keep a failed observation; scheduled posts retain periodic checks until link proof.
    /// The assignment, campaign, event and snapshot use one CAS transaction.
    pub fn record_publish_verification_pending(
        &self,
        candidate: &PendingPublishVerification,
        reason: &str,
    ) -> anyhow::Result<bool> {
        self.record_publish_verification_observation(candidate, reason, "pending")
    }

    pub fn record_publish_verification_observation(
        &self,
        candidate: &PendingPublishVerification,
        reason: &str,
        reason_code: &str,
    ) -> anyhow::Result<bool> {
        self.record_publish_verification_diagnostic(candidate, reason, reason_code, None)
    }

    pub fn record_publish_verification_diagnostic(
        &self,
        candidate: &PendingPublishVerification,
        reason: &str,
        reason_code: &str,
        diagnostic: Option<&serde_json::Value>,
    ) -> anyhow::Result<bool> {
        self.record_publish_verification_diagnostic_inner(
            candidate,
            reason,
            reason_code,
            diagnostic,
            false,
        )
    }

    /// Only an explicit warm check may enroll an old publication into periodic
    /// verification. The marker binds its unchanged Post identity, not its epoch.
    pub fn record_manual_publish_verification_diagnostic(
        &self,
        candidate: &PendingPublishVerification,
        reason: &str,
        reason_code: &str,
        diagnostic: Option<&serde_json::Value>,
    ) -> anyhow::Result<bool> {
        self.record_publish_verification_diagnostic_inner(
            candidate,
            reason,
            reason_code,
            diagnostic,
            true,
        )
    }

    fn record_publish_verification_diagnostic_inner(
        &self,
        candidate: &PendingPublishVerification,
        reason: &str,
        reason_code: &str,
        diagnostic: Option<&serde_json::Value>,
        manual: bool,
    ) -> anyhow::Result<bool> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: Option<(String,Option<String>,Option<String>)> = transaction.query_row(
            "SELECT a.state,a.effect_intent,a.evidence_json FROM publish_assignments a WHERE a.id=?1 AND a.campaign_id=?2 AND a.revision=?3",
            params![candidate.assignment_id,candidate.campaign_id,candidate.revision],
            |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))
        ).optional()?;
        let Some((state, intent, prior)) = current else {
            return Ok(false);
        };
        if !may_verify(&state, intent.as_deref(), prior.as_deref())
            || intent != candidate.effect_intent
            || prior != candidate.evidence_json
            || !observer_authorized(&transaction, candidate)?
        {
            return Ok(false);
        }
        let mut evidence = prior
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| serde_json::json!({}));
        if evidence.get("priorEvidenceJson").is_none() {
            evidence["priorEvidenceJson"] = serde_json::to_value(&prior)?;
        }
        let metadata_observation = retained_metadata_observation(&transaction,candidate,diagnostic)?;
        let metadata_unanswered = metadata_observation && diagnostic
            .and_then(|d| d.get("pendingMetadataCandidate"))
            .and_then(|value| serde_json::from_value::<crate::publish_submission::PendingMetadataCandidate>(value.clone()).ok())
            .is_some_and(|pending| public_check_unanswered(&pending.captured.metadata));
        // Judged against the previous pass's diagnostic, before this one replaces it.
        let exempt = exempt_from_no_progress(&evidence, reason_code, diagnostic);
        if let Some(diagnostic) = diagnostic {
            let previous = evidence.get("verificationDiagnostic").and_then(|d|d.get("pendingMetadataCandidate"))
                .or_else(|| evidence.get("post").and_then(|p|p.get("verificationDiagnostic")).and_then(|d|d.get("pendingMetadataCandidate"))).cloned();
            evidence["verificationDiagnostic"] = diagnostic.clone();
            if !metadata_observation {
                if let Some(previous) = previous {
                    evidence["verificationDiagnostic"]["pendingMetadataCandidate"] = previous;
                }
            }
        }
        let checked = Utc::now();
        let identity_missing = !submission_identity_complete(intent.as_deref());
        let legacy: bool = transaction.query_row(
            "SELECT COALESCE(json_extract(request_json,'$.verificationContractVersion'),0)<>1 FROM publish_campaigns WHERE id=?1",
            [&candidate.campaign_id],|row|row.get(0))?;
        let legacy_review = legacy
            && matches!(
                reason_code,
                "draftsObserved" | "composerOrUpload" | "submissionIdentityMissing"
            );
        let prior_review = needs_review(prior.as_deref())
            && !obsolete_deadline_review(intent.as_deref(), prior.as_deref());
        let processing_until = processing_pending(reason_code, diagnostic)
            .then(|| processing_wait_until(&evidence["verificationBudget"], intent.as_deref()))
            .flatten();
        let processing_wait = processing_until.is_some_and(|until| checked < until);
        let processing_exhausted =
            !prior_review && processing_until.is_some_and(|until| checked >= until);
        let infrastructure = infrastructure_failure(reason_code);
        let budget = if prior_review {
            // A one-shot manual check can settle proof, but cannot renew automatic work.
            evidence["verificationBudget"].clone()
        } else if metadata_observation {
            let initial = new_verification_budget(intent.as_deref());
            let prior = &evidence["verificationBudget"];
            let mut budget = if prior["version"] == 1 && prior["intentSha256"] == initial["intentSha256"] {
                prior.clone()
            } else { initial };
            if metadata_unanswered {
                // TikTok throttling is not a failed proof: bounded by elapsed time below.
                budget["metadataUnreachableObservations"] = serde_json::json!(budget["metadataUnreachableObservations"].as_u64().unwrap_or(0).saturating_add(1));
                if !budget["metadataUnreachableSince"].is_string() {
                    budget["metadataUnreachableSince"] = serde_json::json!(checked.to_rfc3339());
                }
            } else {
                budget["metadataObservations"] = serde_json::json!(budget["metadataObservations"].as_u64().unwrap_or(0).saturating_add(1));
            }
            budget["metadataLimit"] = serde_json::json!(METADATA_RETRY_LIMIT);
            budget["observations"] = serde_json::json!(budget["observations"].as_u64().unwrap_or(0).saturating_add(1));
            budget
        } else {
            advance_verification_budget(
                &evidence,
                intent.as_deref(),
                diagnostic,
                exempt,
                processing_wait,
                infrastructure,
            )
        };
        let infrastructure_exhausted = !prior_review
            && infrastructure
            && budget["infrastructureObservations"].as_u64().unwrap_or(0)
                >= INFRASTRUCTURE_RETRY_LIMIT;
        let metadata_exhausted = budget["metadataObservations"].as_u64().unwrap_or(0) >= METADATA_RETRY_LIMIT;
        let metadata_unreachable = !prior_review
            && metadata_unanswered
            && budget["metadataUnreachableSince"].as_str()
                .and_then(|since| DateTime::parse_from_rfc3339(since).ok())
                .is_some_and(|since| checked >= since.with_timezone(&Utc) + chrono::Duration::minutes(METADATA_UNREACHABLE_MINUTES));
        let budget_exhausted = metadata_exhausted || metadata_unreachable || budget["noProgressObservations"].as_u64().unwrap_or(0)
            >= VERIFICATION_NO_PROGRESS_LIMIT;
        evidence["verificationBudget"] = budget;
        // An automatic pass after the first re-check that tapped Copy spends the re-check's share
        // cap (`AUTOMATIC_RECHECK_COPY_PASSES`), whichever branch kept its budget. Network-only
        // metadata reads tap nothing, and an operator's manual check is not counted.
        let copied = diagnostic
            .is_some_and(|value| value["copyAttempts"].as_u64().is_some_and(|taps| taps > 0));
        if !manual
            && !prior_review
            && copied
            && evidence["verificationBudget"]["recheckSince"].is_string()
        {
            let passes = &mut evidence["verificationBudget"]["recheckCopyPasses"];
            *passes = serde_json::json!(passes.as_u64().unwrap_or(0).saturating_add(1));
        }
        let review = identity_missing
            || legacy_review
            || prior_review
            || budget_exhausted
            || processing_exhausted
            || infrastructure_exhausted
            || reason_code == "metadataCandidateInvalid";
        if manual
            && legacy
            && !review
            && has_post_intent(intent.as_deref())
            && !explicitly_resumed_verification(candidate)
        {
            use sha2::Digest;
            evidence["verificationResume"] = serde_json::json!({"version":1,"assignmentId":candidate.assignment_id,
                "intentSha256":format!("{:x}",sha2::Sha256::digest(intent.as_deref().unwrap_or_default().as_bytes())),
                "requestedAt":checked.to_rfc3339()});
        }
        let now = checked.to_rfc3339();
        let observation_reason: String = reason.chars().take(512).collect();
        let reason = if identity_missing {
            LEGACY_IDENTITY_REVIEW_REASON.to_owned()
        } else if review && needs_review(prior.as_deref()) {
            // Preserve the reason for a non-deadline explicit review.
            prior
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                .and_then(|value| {
                    value["verificationStatus"]["reason"]
                        .as_str()
                        .map(str::to_owned)
                })
                .filter(|text| !text.trim().is_empty())
                .unwrap_or(observation_reason.clone())
        } else if metadata_exhausted && !legacy_review {
            format!("Đã giữ liên kết ứng viên; tự kiểm tra dữ liệu công khai đã dừng sau {METADATA_RETRY_LIMIT} lượt chưa xác minh được. {observation_reason} Xác nhận Tiếp tục xác minh bài đã gửi để mở ngân sách mới, không đăng lại.")
        } else if metadata_unreachable && !legacy_review {
            format!("Đã giữ liên kết ứng viên; kiểm tra công khai của TikTok không phản hồi suốt {METADATA_UNREACHABLE_MINUTES} phút (TikTok giới hạn tần suất hoặc lỗi máy chủ), dữ liệu bài chưa bị từ chối. {observation_reason} Không đăng lại; xác nhận Tiếp tục xác minh bài đã gửi để thử lại.")
        } else if processing_exhausted && !legacy_review {
            format!("TikTok vẫn chưa trả link sau {PROCESSING_WAIT_MINUTES} phút chờ xử lý bài ({reason_code}). {observation_reason} Cần kiểm tra; xác nhận Tiếp tục xác minh bài đã gửi để chờ thêm, không đăng lại.")
        } else if infrastructure_exhausted && !legacy_review {
            format!("Tự kiểm tra đã dừng sau {INFRASTRUCTURE_RETRY_LIMIT} lượt lỗi clipboard của helper trên máy ({reason_code}); đây là lỗi công cụ, không phải bằng chứng bài sai. {observation_reason} Không đăng lại; xác nhận Tiếp tục xác minh bài đã gửi để mở ngân sách mới.")
        } else if budget_exhausted && !legacy_review {
            format!("Tự kiểm tra đã dừng sau {VERIFICATION_NO_PROGRESS_LIMIT} lượt liên tiếp không có bằng chứng mới của đúng bài ({reason_code}). {observation_reason} Cần kiểm tra; xác nhận Tiếp tục xác minh bài đã gửi để mở ngân sách mới, không đăng lại.")
        } else {
            observation_reason.clone()
        };
        let submitted = intent
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .and_then(|value| value.get("submittedAt").cloned());
        let attempts = evidence["verificationStatus"]["attempts"]
            .as_u64()
            .unwrap_or(0)
            .saturating_add(1);
        let read_failures = if reason_code == "readFailed" {
            evidence["verificationStatus"]["readFailures"]
                .as_u64()
                .unwrap_or(0)
                .saturating_add(1)
        } else {
            0
        };
        // Each observation is bounded by the driver deadline; between attempts the
        // phone is released. Persist this cadence, including read failures and restart.
        let next_check = (!review).then(|| {
            (checked + chrono::Duration::seconds(VERIFICATION_CHECK_SECONDS)).to_rfc3339()
        });
        let cause = if identity_missing {
            "submissionIdentityMissing"
        } else if prior_review {
            evidence["verificationStatus"]["cause"]
                .as_str()
                .unwrap_or("explicitReview")
        } else if legacy_review {
            reason_code
        } else if metadata_exhausted {
            METADATA_RETRY_CAUSE
        } else if metadata_unreachable {
            METADATA_UNREACHABLE_CAUSE
        } else if processing_exhausted {
            PROCESSING_WAIT_CAUSE
        } else if infrastructure_exhausted {
            INFRASTRUCTURE_RETRY_CAUSE
        } else if budget_exhausted {
            VERIFICATION_NO_PROGRESS_CAUSE
        } else {
            reason_code
        }
        .to_owned();
        evidence["verificationStatus"] = serde_json::json!({
            "state":if review {"needsReview"} else {"pending"},
            "reason":reason,"checkedAt":now,"submittedAt":submitted,
            "reviewAfterMinutes":null,"checkIntervalSeconds":VERIFICATION_CHECK_SECONDS,
            "cause":cause,
            "observationReason":observation_reason,"reasonCode":reason_code,
            "attempts":attempts,"readFailures":read_failures,"nextCheckAt":next_check,
            "deadlineAt":null
        });
        // A park the worker reopens itself says so instead of asking for the operator. It
        // stays needsReview with no nextCheckAt; `expire_publish_verifications` reopens it.
        let recheck_at = checked + chrono::Duration::minutes(AUTOMATIC_RECHECK_MINUTES);
        if review
            && !prior_review
            && !legacy_review
            && automatic_recheck_review(&evidence)
            && automatic_recheck_until(&evidence).is_some_and(|until| recheck_at < until)
        {
            let parked = evidence["verificationStatus"]["reason"]
                .as_str()
                .unwrap_or_default();
            evidence["verificationStatus"]["reason"] = serde_json::json!(format!(
                "{parked} Lý do dừng không phải bằng chứng bài sai (TikTok còn xử lý bài hoặc chưa trả lời, hoặc lượt đọc trên máy bị cắt ngắn): app tự kiểm tra lại mỗi {AUTOMATIC_RECHECK_MINUTES} phút trong {AUTOMATIC_RECHECK_WINDOW_HOURS} giờ, chỉ đọc, không đăng lại, không cần thao tác."
            ));
            evidence["verificationStatus"]["automaticRecheckAt"] =
                serde_json::json!(recheck_at.to_rfc3339());
        }
        let next_state = if review { "uncertain" } else { "verifying" };
        let error = if review {
            "post_verification_needs_review"
        } else {
            "post_verification_pending"
        };
        let changed=transaction.execute(
            "UPDATE publish_assignments SET state=?1,error_code=?2,evidence_json=?3,revision=revision+1,updated_at=?4
             WHERE id=?5 AND campaign_id=?6 AND revision=?7 AND state IN ('verifying','uncertain','succeeded')",
            params![next_state,error,evidence.to_string(),now,candidate.assignment_id,candidate.campaign_id,candidate.revision]
        )?;
        if changed != 1 {
            return Ok(false);
        }
        reconcile_verified_campaign(&transaction, &candidate.campaign_id, &now)?;
        reconcile_verification_snapshot(&transaction, &candidate.campaign_id, &now)?;
        transaction.commit()?;
        Ok(true)
    }

    /// Commit own-post proof and the Sheet obligation together, only for the revision read.
    /// A concurrent active Post, cancellation of an assignment, or stale check cannot win.
    pub fn record_verified_publish_with_sheet_row(
        &self,
        candidate: &PendingPublishVerification,
        evidence_json: &str,
        post_url: &str,
        poster: &str,
        partners: &[String],
    ) -> anyhow::Result<bool> {
        let parsed = url::Url::parse(post_url)?;
        anyhow::ensure!(
            parsed.scheme() == "https"
                && matches!(
                    parsed.host_str(),
                    Some("www.tiktok.com" | "tiktok.com" | "m.tiktok.com")
                )
                && parsed.username().is_empty()
                && parsed.password().is_none()
                && parsed.port_or_known_default() == Some(443),
            "verification requires canonical TikTok HTTPS link"
        );
        let targets = crate::interaction::parse_tiktok_links(post_url);
        anyhow::ensure!(
            targets.len() == 1
                && targets[0]
                    .target
                    .as_ref()
                    .is_some_and(|target| target.normalized_url == post_url),
            "verification requires canonical post identity without tracking"
        );
        let mut evidence: serde_json::Value = serde_json::from_str(evidence_json)?;
        let proof = evidence.get("post").unwrap_or(&evidence);
        anyhow::ensure!(
            proof.get("postUrl").and_then(|v| v.as_str()) == Some(post_url)
                && proof.get("publicationVerified").and_then(|v| v.as_bool()) == Some(true),
            "verification evidence must carry the verified canonical post link"
        );
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: Option<(String,Option<String>,Option<String>)> = transaction.query_row(
            "SELECT state,effect_intent,evidence_json FROM publish_assignments WHERE id=?1 AND campaign_id=?2 AND revision=?3",
            params![candidate.assignment_id,candidate.campaign_id,candidate.revision],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))
        ).optional()?;
        let Some((state, intent, prior)) = current else {
            return Ok(false);
        };
        if !may_verify(&state, intent.as_deref(), prior.as_deref())
            || intent != candidate.effect_intent
            || prior != candidate.evidence_json
            || !observer_authorized(&transaction, candidate)?
        {
            return Ok(false);
        }
        // Verification may run hours later or after a scheduled run. The submitted day
        // belongs to the one-shot claim, never to this observer's clock.
        let submission_identity = intent
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
        if let Some(account) = submission_identity
            .as_ref()
            .and_then(|value| value.get("expectedAccount"))
            .and_then(serde_json::Value::as_str)
        {
            anyhow::ensure!(
                targets[0].target.as_ref().is_some_and(|target| target
                    .author
                    .eq_ignore_ascii_case(account.trim_start_matches('@'))),
                "verified link does not match the account recorded before Post"
            );
        }
        if let Some(identity) = submission_identity {
            let proof = if evidence
                .get("post")
                .is_some_and(serde_json::Value::is_object)
            {
                &mut evidence["post"]
            } else {
                &mut evidence
            };
            for field in ["submittedAt", "expectedAccount"] {
                if let Some(value) = identity.get(field) {
                    proof[field] = value.clone();
                }
            }
        }
        let original = prior
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .and_then(|value| value.get("priorEvidenceJson").cloned());
        evidence["priorEvidenceJson"] = original.unwrap_or(serde_json::to_value(prior)?);
        let now = Utc::now().to_rfc3339();
        evidence["verificationStatus"] = serde_json::json!({"state":"verified","checkedAt":now});
        let changed = transaction.execute(
            "UPDATE publish_assignments SET state='succeeded',evidence_json=?1,error_code=NULL,revision=revision+1,updated_at=?2
             WHERE id=?3 AND campaign_id=?4 AND revision=?5 AND state IN ('verifying','uncertain','succeeded')",
            params![evidence.to_string(),now,candidate.assignment_id,candidate.campaign_id,candidate.revision]
        )?;
        if changed != 1 {
            return Ok(false);
        }
        queue_sheet_row(
            &transaction,
            &candidate.assignment_id,
            &candidate.campaign_id,
            post_url,
            poster,
            partners,
        )?;
        reconcile_verified_campaign(&transaction, &candidate.campaign_id, &now)?;
        reconcile_verification_snapshot(&transaction, &candidate.campaign_id, &now)?;
        transaction.commit()?;
        Ok(true)
    }
}

fn reconcile_verification_snapshot(
    conn: &Connection,
    campaign_id: &str,
    now: &str,
) -> anyhow::Result<()> {
    let previous: Option<(String, String)> = conn
        .query_row(
            "SELECT input_digest,report_json FROM publish_execution_snapshots WHERE campaign_id=?1",
            [campaign_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let request_json: String = conn.query_row(
        "SELECT request_json FROM publish_campaigns WHERE id=?1",
        [campaign_id],
        |row| row.get(0),
    )?;
    let request: crate::PublishCampaignRequest = serde_json::from_str(&request_json)?;
    let detail = Database::get_publish_campaign_from_connection(conn, campaign_id)?
        .context("verification campaign disappeared")?;
    let input_digest = previous
        .as_ref()
        .map(|(digest, _)| digest.clone())
        .unwrap_or(publish_campaign_input_digest(&request, &detail)?);
    let (status, retry_scope) = reconciled_sheet_delivery_status(conn, campaign_id)?;
    // Repeated per-assignment settles must not recursively stringify the prior wrapper.
    let previous_report = previous.as_ref().map(|(_, report)| {
        serde_json::from_str::<serde_json::Value>(report)
            .ok()
            .filter(|value| {
                value.get("source").and_then(|v| v.as_str())
                    == Some("post_verification_reconciliation")
            })
            .and_then(|value| value.get("previousReportJson").cloned())
            .unwrap_or_else(|| serde_json::Value::String(report.clone()))
    });
    let draft = crate::PublishExecutionSnapshotDraft {
        input_digest,
        status,
        retry_scope,
        report_json: serde_json::json!({
            "campaignId":campaign_id,"status":status,"retryScope":retry_scope,
            "source":"post_verification_reconciliation","sheetEnabled":request.sheet_enabled,
            "targetSnapshot":request.target_snapshot,
            "previousReportJson":previous_report,
        }),
    };
    super::publish::store_publish_execution_snapshot(conn, campaign_id, &draft, now)?;
    Ok(())
}

fn reconcile_verified_campaign(
    conn: &Connection,
    campaign_id: &str,
    now: &str,
) -> anyhow::Result<()> {
    let (current, current_error): (String, Option<String>) = conn.query_row(
        "SELECT state,error_code FROM publish_campaigns WHERE id=?1",
        [campaign_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if matches!(current.as_str(), "posting" | "cancelled" | "missed") {
        return Ok(());
    }
    let mut statement =
        conn.prepare("SELECT state,evidence_json FROM publish_assignments WHERE campaign_id=?1")?;
    let rows = statement
        .query_map([campaign_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let review_needed = rows
        .iter()
        .any(|(state, evidence)| state == "uncertain" && needs_review(evidence.as_deref()));
    let state = if rows
        .iter()
        .any(|(state, _)| matches!(state.as_str(), "posting" | "uncertain"))
    {
        "uncertain"
    } else if rows.iter().any(|(state, evidence)| {
        state == "verifying"
            || (state == "succeeded" && !evidence_has_post_link(evidence.as_deref()))
    }) {
        "verifying"
    } else if !rows.is_empty() && rows.iter().all(|(state, _)| state == "succeeded") {
        "succeeded"
    } else {
        "failed_before_dispatch"
    };
    let error = match state {
        "succeeded" => None,
        "verifying" => Some("post_verification_pending"),
        "uncertain" if review_needed => Some("post_verification_needs_review"),
        "uncertain" => Some("post_or_cleanup_failed"),
        _ => Some("post_refused_before_dispatch"),
    };
    if state == current && current_error.as_deref() == error {
        return Ok(());
    }
    conn.execute("UPDATE publish_campaigns SET state=?1,error_code=?2,revision=revision+1,updated_at=?3 WHERE id=?4",params![state,error,now,campaign_id])?;
    conn.execute("INSERT INTO publish_events(campaign_id,revision,kind,payload_json,created_at) SELECT id,revision,'state',?1,?2 FROM publish_campaigns WHERE id=?3",params![serde_json::json!({"state":state,"errorCode":error,"source":"post_verification"}).to_string(),now,campaign_id])?;
    Ok(())
}

/// What a publish job may do about one earlier post of another campaign still holding its
/// phone (`PublishDeviceGuard::blocking`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PriorPostClass {
    /// TikTok already holds the post (`publish_upload_settled`): a job whose approval carries
    /// `priorPosts` may pause its verification, close TikTok and post.
    Releasable,
    /// Not proven yet, but still observed automatically: wait, send nothing.
    Pending,
    /// Only a person can move it on: stop before Post now.
    NeedsOperator,
}

/// One earlier post on the phone, read at one revision.
#[derive(Debug, Clone)]
pub struct PriorPostHold {
    pub assignment_id: String,
    pub campaign_id: String,
    pub revision: i64,
    pub updated_at: String,
    pub class: PriorPostClass,
    /// Short Vietnamese reason, quoted in the operator's text.
    pub reason: String,
    pub intent_sha256: Option<String>,
    pub publication_stage: u64,
    pub package: Option<String>,
}

/// Which earlier posts the automatic verifier will still observe: the dev acceptance scope, and
/// whether automatic re-checks of parked rows run.
pub struct PriorPostScope<'a> {
    pub observable: &'a (dyn Fn(&str, &str) -> bool + Sync),
    pub automatic_recheck: bool,
}

fn observe_every_post(_: &str, _: &str) -> bool {
    true
}

impl PriorPostScope<'static> {
    /// Production: the verifier observes every campaign and re-checks parked rows.
    pub fn production() -> Self {
        Self {
            observable: &observe_every_post,
            automatic_recheck: true,
        }
    }
}

/// The class of one blocking hold on `udid`, re-read on `conn` (inside the caller's transaction
/// when it writes). A missing row is skipped. Never edits anything.
pub(super) fn classify_prior_post_hold(
    conn: &Connection,
    udid: &str,
    hold: &PublishDeviceHold,
    scope: &PriorPostScope<'_>,
) -> anyhow::Result<Option<PriorPostHold>> {
    use sha2::Digest;
    let Some((candidate, state, _)) = recovery_row(conn, &hold.assignment_id)? else {
        return Ok(None);
    };
    let (intent, evidence) = (
        candidate.effect_intent.as_deref(),
        candidate.evidence_json.as_deref(),
    );
    let parsed: serde_json::Value = evidence
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default();
    let worker: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM publish_dispatch_jobs WHERE assignment_id=?1 AND state='running')
         OR EXISTS(SELECT 1 FROM publish_work_claims WHERE owner=?1)",
        [&candidate.assignment_id],
        |r| r.get(0),
    )?;
    let handoff_in_flight: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM publish_assignments o JOIN settings s
             ON s.key='publish.handoff-pending.'||o.id WHERE o.udid=?1)",
        [udid],
        |r| r.get(0),
    )?;
    let current_contract: bool = conn.query_row(
        "SELECT COALESCE(json_extract(request_json,'$.verificationContractVersion')=1,0)
         FROM publish_campaigns WHERE id=?1",
        [&candidate.campaign_id],
        |r| r.get(0),
    )?;
    let observed_by_worker = current_contract || explicitly_resumed_verification(&candidate);
    let review_reason = || {
        parsed["verificationStatus"]["reason"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| "bài đó cần kiểm tra".into())
    };
    use PriorPostClass::*;
    let (class, reason) = if candidate.stop_marker.is_some()
        || publish_close_pending(conn, &candidate.campaign_id)?
    {
        (NeedsOperator, "lượt đó đã bị Dừng".to_owned())
    } else if !(scope.observable)(&candidate.campaign_id, udid) {
        (
            NeedsOperator,
            "lượt đó ngoài phạm vi nghiệm thu đang mở".to_owned(),
        )
    } else if state == "posting" {
        (Pending, "bài đó đang được gửi lên TikTok".to_owned())
    } else if !has_post_intent(intent) || !submission_identity_complete(intent) {
        (
            NeedsOperator,
            "thiếu bằng chứng tài khoản hoặc thời điểm Đăng của bài đó".to_owned(),
        )
    } else if worker {
        (Pending, "đang kiểm tra bài đó".to_owned())
    } else if handoff_in_flight {
        (Pending, "đang nhả máy cho lượt khác".to_owned())
    } else if !observer_authorized(conn, &candidate)? {
        // Excluded with no release proof that still holds and no resume stamp: frozen.
        (
            NeedsOperator,
            "bài đó đã bị tạm dừng xác minh và chưa được tiếp tục".to_owned(),
        )
    } else if !observed_by_worker {
        (
            NeedsOperator,
            "bài đó thuộc lượt theo cơ chế xác minh cũ".to_owned(),
        )
    } else {
        // A released post resumes on its own only on a phone still saved with its account
        // (`handoff_resume_due`); otherwise it is not released, only waited for.
        let account: String = conn.query_row(
            "SELECT COALESCE((SELECT handle FROM device_meta WHERE udid=?1),'')",
            [udid],
            |r| r.get(0),
        )?;
        let account = crate::publish::normalize_publish_account(&account);
        let expected = intent
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .and_then(|value| {
                value["expectedAccount"]
                    .as_str()
                    .map(crate::publish::normalize_publish_account)
            });
        let resumable = !account.is_empty() && expected.as_deref() == Some(account.as_str());
        let settled = publish_upload_settled(intent, evidence);
        // Still observed with no person: a pending check, or a parked row the worker re-checks.
        let automatic = !needs_review(evidence)
            || (scope.automatic_recheck
                && automatic_recheck_review(&parsed)
                && automatic_recheck_until(&parsed).is_some_and(|until| Utc::now() < until));
        if settled && resumable {
            (Releasable, "app đã thấy bài đó trên Hồ sơ".to_owned())
        } else if !automatic {
            (NeedsOperator, review_reason())
        } else if settled {
            (
                Pending,
                "máy chưa lưu đúng tài khoản của bài đó nên chỉ chờ xác minh".to_owned(),
            )
        } else if needs_review(evidence) {
            (Pending, "chờ lần tự kiểm tra lại bài đó".to_owned())
        } else if parsed["verificationStatus"]["reasonCode"] == "tiktokProcessing" {
            (Pending, "TikTok báo bài đang được xử lý".to_owned())
        } else {
            (Pending, "chưa thấy bài trên Hồ sơ".to_owned())
        }
    };
    Ok(Some(PriorPostHold {
        assignment_id: candidate.assignment_id,
        campaign_id: candidate.campaign_id,
        revision: candidate.revision,
        updated_at: hold.updated_at.clone(),
        class,
        reason,
        intent_sha256: intent.map(|raw| format!("{:x}", sha2::Sha256::digest(raw.as_bytes()))),
        publication_stage: parsed["verificationBudget"]["publicationStage"]
            .as_u64()
            .unwrap_or(0),
        package: intent
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .and_then(|value| value["package"].as_str().map(str::to_owned)),
    }))
}

/// Every blocking post of another campaign on `udid`, classified on `conn`.
pub(super) fn prior_post_outlook_on(
    conn: &Connection,
    udid: &str,
    own_campaign: Option<&str>,
    scope: &PriorPostScope<'_>,
) -> anyhow::Result<Vec<PriorPostHold>> {
    let mut holds = Vec::new();
    for hold in Database::publish_device_guard_from_connection(conn, udid)?.blocking {
        if own_campaign == Some(hold.campaign_id.as_str()) {
            continue;
        }
        if let Some(classified) = classify_prior_post_hold(conn, udid, &hold, scope)? {
            holds.push(classified);
        }
    }
    Ok(holds)
}

/// Ask the verifier to check a waiting earlier post now, once per (that post, this job). Only a
/// row waiting for its next automatic check, with no observer running on it, under CAS. Moves
/// one check earlier and marks the row so the verifier lists it first while the job waits; never
/// reopens a parked row or changes its budget.
pub(super) fn prioritize_prior_post(
    conn: &Connection,
    hold: &PriorPostHold,
    for_assignment: &str,
    for_campaign: &str,
) -> anyhow::Result<bool> {
    let observing: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM publish_work_claims WHERE owner=?1)",
        [&hold.assignment_id],
        |r| r.get(0),
    )?;
    if observing || hold.class != PriorPostClass::Pending {
        return Ok(false);
    }
    let Some((candidate, state, _)) = recovery_row(conn, &hold.assignment_id)? else {
        return Ok(false);
    };
    let Some(mut evidence) = candidate
        .evidence_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .filter(serde_json::Value::is_object)
    else {
        return Ok(false);
    };
    if candidate.revision != hold.revision
        || evidence["verificationStatus"]["state"] != "pending"
        || evidence["priorPostPriority"]["assignmentId"] == for_assignment
        || !may_verify(
            &state,
            candidate.effect_intent.as_deref(),
            candidate.evidence_json.as_deref(),
        )
        || !observer_authorized(conn, &candidate)?
    {
        return Ok(false);
    }
    let now = Utc::now().to_rfc3339();
    evidence["verificationStatus"]["nextCheckAt"] = serde_json::json!(now);
    evidence["priorPostPriority"] = serde_json::json!({
        "assignmentId": for_assignment, "campaignId": for_campaign, "requestedAt": now,
    });
    let changed = conn.execute(
        "UPDATE publish_assignments SET evidence_json=?1,revision=revision+1
         WHERE id=?2 AND campaign_id=?3 AND revision=?4",
        params![
            evidence.to_string(),
            candidate.assignment_id,
            candidate.campaign_id,
            candidate.revision
        ],
    )?;
    Ok(changed == 1)
}

impl Database {
    /// Read-only: the earlier posts on `udid` a job of `own_campaign` (or a new request) would
    /// meet at its turn, and their classes. Used by preflight notes and the dispatcher's gate.
    pub fn prior_post_outlook(
        &self,
        udid: &str,
        own_campaign: Option<&str>,
        scope: &PriorPostScope<'_>,
    ) -> anyhow::Result<Vec<PriorPostHold>> {
        prior_post_outlook_on(&self.conn()?, udid, own_campaign, scope)
    }
}
