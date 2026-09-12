//! Restart-safe verification reads and compare-and-swap publication settlement.
use super::publish_sheet::{
    evidence_has_post_link, queue_sheet_row, reconciled_sheet_delivery_status,
};
use super::*;

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

const VERIFICATION_REVIEW_AFTER_MINUTES: i64 = 30;
/// Scheduled posts release the phone before the profile is ready; give the fleet
/// longer than an immediate post so a busy VerificationQueue can still catch up.
const SCHEDULED_REVIEW_AFTER_MINUTES: i64 = 240;
const SCHEDULED_FIRST_CHECK_SECONDS: i64 = 120;
const SCHEDULED_CHECK_SECONDS: i64 = 300;

fn deadline_review_reason(scheduled: bool) -> String {
    if scheduled {
        format!(
            "Hết {SCHEDULED_REVIEW_AFTER_MINUTES} phút tự kiểm tra bài hẹn giờ; Sheet chưa có link. Tự kiểm tra đã dừng — chọn Kiểm tra liên kết (không đăng lại)."
        )
    } else {
        format!(
            "Hết {VERIFICATION_REVIEW_AFTER_MINUTES} phút tự kiểm tra; Sheet chưa có link. Tự kiểm tra đã dừng — chọn Kiểm tra liên kết (không đăng lại)."
        )
    }
}

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

fn review_deadline_reached(intent: Option<&str>, now: DateTime<Utc>, scheduled: bool) -> bool {
    intent
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|value| value["submittedAt"].as_str().map(str::to_owned))
        .and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
        .is_some_and(|submitted| {
            now.signed_duration_since(submitted)
                >= chrono::Duration::minutes(if scheduled {
                    SCHEDULED_REVIEW_AFTER_MINUTES
                } else {
                    VERIFICATION_REVIEW_AFTER_MINUTES
                })
        })
}

fn first_scheduled_check(intent: Option<&str>) -> Option<DateTime<Utc>> {
    let intent: serde_json::Value = serde_json::from_str(intent?).ok()?;
    DateTime::parse_from_rfc3339(intent["submittedAt"].as_str()?)
        .ok()?
        .with_timezone(&Utc)
        .checked_add_signed(chrono::Duration::seconds(SCHEDULED_FIRST_CHECK_SECONDS))
}

/// Only reviews caused by the old thirty-minute policy are reopened automatically.
fn scheduled_deadline_review(
    scheduled: bool,
    intent: Option<&str>,
    evidence: Option<&str>,
) -> bool {
    scheduled
        && submission_identity_complete(intent)
        && needs_review(evidence)
        && !review_deadline_reached(intent, Utc::now(), true)
        && evidence
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .is_some_and(|value| {
                value["verificationStatus"]["cause"] == "verificationDeadline"
                    && value["verificationStatus"]["reviewAfterMinutes"]
                        == VERIFICATION_REVIEW_AFTER_MINUTES
            })
}

impl Database {
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
            "reason":format!(
                "Đã gửi bài hẹn giờ; tự kiểm tra liên kết sau 2 phút rồi định kỳ ở nền, tối đa {SCHEDULED_REVIEW_AFTER_MINUTES} phút"
            ),
            "cause":"deferredScheduledLink", "attempts":0, "readFailures":0,
            "reviewAfterMinutes":SCHEDULED_REVIEW_AFTER_MINUTES, "nextCheckAt":first_scheduled_check(intent.as_deref()).map(|at|at.to_rfc3339()),
        });
        Ok(evidence.to_string())
    }

    /// Automatic checks omit explicit review except the obsolete scheduled deadline.
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
            "SELECT a.id,a.campaign_id,a.bundle_id,a.udid,a.revision,a.effect_intent,a.evidence_json,a.state,c.run_at IS NOT NULL
             FROM publish_assignments a JOIN publish_campaigns c ON c.id=a.campaign_id
             WHERE a.state IN ('verifying','uncertain','succeeded') AND (?1 IS NULL OR a.campaign_id=?1)
             AND (?2=0 OR json_extract(c.request_json,'$.verificationContractVersion')=1)
             ORDER BY a.updated_at,a.campaign_id,a.ordinal"
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
                },
                row.get::<_, String>(7)?,
            ))
        })?;
        // Prefer due rows so a long not-due schedule tail cannot starve the worker / expire
        // path under the 1000-candidate ceiling.
        let mut due = Vec::new();
        let mut later = Vec::new();
        for row in rows {
            let (candidate, state) = row?;
            if !(include_review
                || !needs_review(candidate.evidence_json.as_deref())
                || scheduled_deadline_review(
                    candidate.scheduled,
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
            } else {
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
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            "SELECT state,effect_intent,evidence_json FROM publish_assignments
             WHERE udid=?1 AND state IN ('posting','verifying','uncertain','succeeded')",
        )?;
        let rows = statement.query_map([udid], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        for row in rows {
            let (state, intent, evidence) = row?;
            if state == "posting"
                || state == "verifying"
                || state == "uncertain"
                || may_verify(&state, intent.as_deref(), evidence.as_deref())
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Resume obsolete scheduled deadlines; enforce each submission verification budget.
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
            let reopen = scheduled_deadline_review(
                candidate.scheduled,
                candidate.effect_intent.as_deref(),
                candidate.evidence_json.as_deref(),
            );
            if !reopen
                && submission_identity_complete(candidate.effect_intent.as_deref())
                && !review_deadline_reached(
                    candidate.effect_intent.as_deref(),
                    Utc::now(),
                    candidate.scheduled,
                )
            {
                continue;
            }
            let evidence = candidate
                .evidence_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
            let reason = if reopen {
                evidence
                    .as_ref()
                    .and_then(|value| value["verificationStatus"]["reason"].as_str())
                    .or_else(|| {
                        evidence.as_ref().and_then(|value| {
                            value.get("post").unwrap_or(value)["linkCaptureReason"].as_str()
                        })
                    })
                    .unwrap_or("TikTok vẫn đang xử lý; tiếp tục kiểm tra liên kết")
                    .to_owned()
            } else {
                deadline_review_reason(candidate.scheduled)
            };
            if self.record_publish_verification_pending(&candidate, &reason)?
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
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: Option<(String,Option<String>,Option<String>,bool)> = transaction.query_row(
            "SELECT a.state,a.effect_intent,a.evidence_json,c.run_at IS NOT NULL FROM publish_assignments a JOIN publish_campaigns c ON c.id=a.campaign_id WHERE a.id=?1 AND a.campaign_id=?2 AND a.revision=?3",
            params![candidate.assignment_id,candidate.campaign_id,candidate.revision],
            |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))
        ).optional()?;
        let Some((state, intent, prior, scheduled)) = current else {
            return Ok(false);
        };
        if !may_verify(&state, intent.as_deref(), prior.as_deref())
            || intent != candidate.effect_intent
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
        let deadline_hit = review_deadline_reached(intent.as_deref(), checked, scheduled);
        let review = identity_missing
            || (needs_review(prior.as_deref())
                && !scheduled_deadline_review(scheduled, intent.as_deref(), prior.as_deref()))
            || deadline_hit;
        let now = checked.to_rfc3339();
        let observation_reason: String = reason.chars().take(512).collect();
        let reason = if identity_missing {
            LEGACY_IDENTITY_REVIEW_REASON.to_owned()
        } else if deadline_hit && !needs_review(prior.as_deref()) {
            // Crossing the budget for the first time — replace the last capture note so Sheet H
            // and Theo dõi both say auto-check stopped instead of the last transient fail.
            deadline_review_reason(scheduled)
        } else if review && needs_review(prior.as_deref()) {
            // Keep an existing needsReview reason (or the clearer deadline text if already set).
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
        let base_delay = if scheduled {
            SCHEDULED_CHECK_SECONDS
        } else {
            30
        };
        let delay = base_delay * (1_i64 << read_failures.saturating_sub(1).min(2));
        let budget_minutes = if scheduled {
            SCHEDULED_REVIEW_AFTER_MINUTES
        } else {
            VERIFICATION_REVIEW_AFTER_MINUTES
        };
        let deadline = submitted
            .as_ref()
            .and_then(serde_json::Value::as_str)
            .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
            .and_then(|at| {
                at.with_timezone(&Utc)
                    .checked_add_signed(chrono::Duration::minutes(budget_minutes))
            });
        let next_check = (!review).then(|| {
            let next = checked + chrono::Duration::seconds(delay);
            deadline.map_or(next, |end| next.min(end)).to_rfc3339()
        });
        evidence["verificationStatus"] = serde_json::json!({
            "state":if review {"needsReview"} else {"pending"},
            "reason":reason,"checkedAt":now,"submittedAt":submitted,
            "reviewAfterMinutes":if identity_missing {None} else {Some(if scheduled { SCHEDULED_REVIEW_AFTER_MINUTES } else { VERIFICATION_REVIEW_AFTER_MINUTES })},
            "cause":if identity_missing {"submissionIdentityMissing"} else if review {"verificationDeadline"} else {reason_code},
            "observationReason":observation_reason,"reasonCode":reason_code,
            "attempts":attempts,"readFailures":read_failures,"nextCheckAt":next_check,
            "deadlineAt":deadline.map(|at|at.to_rfc3339())
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
        if !may_verify(&state, intent.as_deref(), prior.as_deref()) {
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
