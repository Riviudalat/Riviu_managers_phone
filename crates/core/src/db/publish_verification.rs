//! Restart-safe verification reads and compare-and-swap publication settlement.
use super::publish_sheet::{
    evidence_has_post_link, queue_sheet_row, reconciled_sheet_delivery_status,
};
use super::*;

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
    match state {
        "verifying" | "uncertain" => {
            has_post_intent(intent) || has_post_intent(evidence) || needs_review(evidence)
        }
        // Historical success was only a feed-return observation. Recover its missing link
        // without rewriting the original evidence or ever entering the Post path again.
        "succeeded" => !evidence_has_post_link(evidence),
        _ => false,
    }
}

// Upload-idle proof has a separate age threshold; link polling has no total deadline.
const UPLOAD_IDLE_AFTER_MINUTES: i64 = 240;
const SCHEDULED_FIRST_CHECK_SECONDS: i64 = 120;
const VERIFICATION_CHECK_SECONDS: i64 = 300;

pub(super) fn needs_review(evidence: Option<&str>) -> bool {
    evidence
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .is_some_and(|value| value["verificationStatus"]["state"] == "needsReview")
}

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

fn explicitly_resumed_verification(candidate: &PendingPublishVerification) -> bool {
    use sha2::Digest;
    let Some(intent) = candidate.effect_intent.as_deref() else {
        return false;
    };
    if !has_post_intent(Some(intent)) || !submission_identity_complete(Some(intent)) {
        return false;
    }
    candidate
        .evidence_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .is_some_and(|evidence| {
            let marker = &evidence["verificationResume"];
            let identity_matches = marker["assignmentId"] == candidate.assignment_id
                && marker["intentSha256"]
                    == format!("{:x}", sha2::Sha256::digest(intent.as_bytes()));
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
    let marker = stop_marker(conn, &candidate.campaign_id)?;
    Ok(marker == candidate.stop_marker
        && (marker.is_none()
            || (stop_generation(marker.as_deref()).is_some()
                && explicitly_resumed_verification(candidate))))
}

fn explicit_non_stop_review(evidence: Option<&str>) -> bool {
    evidence
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .is_some_and(|v| {
            let review = &v["verificationStatus"];
            (review["state"] == "needsReview"
                && review["cause"] != "operatorStopped"
                && review["cause"] != "verificationDeadline")
                || v.get("verificationReviewBeforeStop").is_some()
        })
}

fn first_scheduled_check(intent: Option<&str>) -> Option<DateTime<Utc>> {
    let intent: serde_json::Value = serde_json::from_str(intent?).ok()?;
    DateTime::parse_from_rfc3339(intent["submittedAt"].as_str()?)
        .ok()?
        .with_timezone(&Utc)
        .checked_add_signed(chrono::Duration::seconds(SCHEDULED_FIRST_CHECK_SECONDS))
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
    let result: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key=?1",
            [format!("operation.stop.result:publish:{campaign}")],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(result) = result {
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
    if needs_review(candidate.evidence_json.as_deref()) {
        return Ok(false);
    }
    let current: bool = conn.query_row("SELECT COALESCE(json_extract(request_json,'$.verificationContractVersion')=1,0) FROM publish_campaigns WHERE id=?1",[&candidate.campaign_id],|r|r.get(0))?;
    Ok(explicitly_resumed_verification(candidate)
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
    if active {
        Some("activePipeline")
    } else if !matches!(state, "verifying" | "uncertain" | "succeeded")
        || !has_post_intent(candidate.effect_intent.as_deref())
    {
        Some("notSubmitted")
    } else if !submission_identity_complete(candidate.effect_intent.as_deref()) {
        Some("submissionIdentityMissing")
    } else if explicit_non_stop_review(candidate.evidence_json.as_deref()) {
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
        evidence["verificationResume"] = serde_json::json!({"version":2,"assignmentId":assignment_id,"campaignId":candidate.campaign_id,
            "intentSha256":format!("{:x}",sha2::Sha256::digest(candidate.effect_intent.as_deref().unwrap_or_default().as_bytes())),
            "stopGeneration":stop_generation(candidate.stop_marker.as_deref()),"authorizationId":Uuid::new_v4().to_string(),"requestedAt":now,"expectedRevision":expected_revision});
        let attempts = evidence["verificationStatus"]["attempts"]
            .as_u64()
            .unwrap_or(0);
        evidence["verificationStatus"] = serde_json::json!({"state":"pending","cause":"operatorResumed","reasonCode":"operatorResumed","reason":"Tiếp tục xác minh bài đã gửi; không đăng lại","attempts":attempts,"checkIntervalSeconds":VERIFICATION_CHECK_SECONDS,"nextCheckAt":now,"deadlineAt":null});
        tx.execute("UPDATE publish_assignments SET state='verifying',error_code='post_verification_pending',evidence_json=?2,revision=revision+1,updated_at=?3 WHERE id=?1 AND revision=?4",params![assignment_id,evidence.to_string(),now,expected_revision])?;
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
        for id in ids {
            let Some((candidate, state, active_pipeline)) = recovery_row(&tx, &id)? else {
                continue;
            };
            let resumed = explicitly_resumed_verification(&candidate)
                && !evidence_has_post_link(candidate.evidence_json.as_deref());
            let check_reason = if evidence_has_post_link(candidate.evidence_json.as_deref()) {
                Some("alreadyVerified")
            } else if candidate.stop_marker.is_some() && !resumed {
                Some("operatorStopped")
            } else if !may_verify(
                &state,
                candidate.effect_intent.as_deref(),
                candidate.evidence_json.as_deref(),
            ) {
                Some("noCandidate")
            } else if !submission_identity_complete(candidate.effect_intent.as_deref()) {
                Some("submissionIdentityMissing")
            } else {
                None
            };
            // Exact claim_publish_assignment_retry predicate, including the live parent
            // and assignment job guard; do not make retry look safer than its DB claim.
            let retry: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM publish_assignments a JOIN publish_campaigns c ON c.id=a.campaign_id WHERE a.id=?1 AND a.state='failed_before_dispatch' AND a.effect_intent IS NULL AND c.state IN ('failed_before_dispatch','verifying','uncertain') AND NOT EXISTS(SELECT 1 FROM publish_pipeline_runs r WHERE r.campaign_id=c.id) AND NOT EXISTS(SELECT 1 FROM publish_dispatch_jobs j WHERE j.assignment_id=a.id AND j.state IN ('running','paused')))",[&id],|r|r.get(0))?;
            let capability = |reason: Option<&str>| PublishRecoveryCapability {
                allowed: reason.is_none(),
                reason: reason.map(str::to_owned),
            };
            let resume_reason = if evidence_has_post_link(candidate.evidence_json.as_deref()) {
                Some("alreadyVerified")
            } else if publish_close_pending(&tx, &candidate.campaign_id)? {
                Some("stopInProgress")
            } else if already_pending(&tx, &candidate)? {
                Some("alreadyPending")
            } else {
                resume_refusal(&candidate, &state, active_pipeline)
            };
            capabilities.push(PublishRecoveryCapabilities {
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
            (will_close && !already_closed) || publish_close_pending(&tx, campaign_id)?;
        tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![format!("operation.stop.publish:{campaign_id}"),serde_json::json!({"version":2,"requestedAt":now,"generation":Uuid::new_v4().to_string(),"closePending":close_pending}).to_string()])?;
        tx.commit()?;
        Ok(devices)
    }

    pub fn publish_operation_stopped(&self, campaign_id: &str) -> anyhow::Result<bool> {
        Ok(self
            .get_setting(&format!("operation.stop.publish:{campaign_id}"))?
            .is_some())
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
            "reason":"Đã gửi bài hẹn giờ; tự kiểm tra liên kết sau 2 phút rồi mỗi 5 phút đến khi xác minh được link",
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
             ORDER BY COALESCE(t.last_turn,0),a.updated_at,a.campaign_id,a.ordinal"
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
                let mut completed = completed_upload_with_link_debt(
                    &state,
                    evidence.as_deref(),
                    &updated_at,
                    active_pipeline,
                    now,
                );
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
                    let marker = stop_marker(&conn, &campaign_id)?;
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
        let mut campaigns = Vec::new();
        for candidate in
            self.publish_verification_candidates(1000, None, false, current_contract_only)?
        {
            let reopen = obsolete_deadline_review(
                candidate.effect_intent.as_deref(),
                candidate.evidence_json.as_deref(),
            );
            if !reopen && submission_identity_complete(candidate.effect_intent.as_deref()) {
                continue;
            }
            let reason = if reopen {
                "Tiếp tục tự kiểm tra liên kết mỗi 5 phút; bài đã gửi chưa được xác minh"
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
        if let Some(diagnostic) = diagnostic {
            evidence["verificationDiagnostic"] = diagnostic.clone();
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
        let review = identity_missing
            || legacy_review
            || (needs_review(prior.as_deref())
                && !obsolete_deadline_review(intent.as_deref(), prior.as_deref()));
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
        } else if review {
            evidence["verificationStatus"]["cause"]
                .as_str()
                .unwrap_or("explicitReview")
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
