//! Durable, read-only recovery of a submitted post. Never re-enters the composer.
use super::*;

#[tauri::command]
pub async fn publish_check_links(
    state: State<'_, AppState>,
    campaign_id: String,
    udid: Option<String>,
) -> Result<serde_json::Value, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    if state.dev_acceptance.active()
        && (!state
            .dev_acceptance
            .scoped_campaign_ids()
            .contains(&campaign_id)
            || !state
                .dev_acceptance
                .allows_any(crate::dev_acceptance::AcceptanceCapability::PublishVerification)
            || udid.as_ref().is_some_and(|id| {
                !state.dev_acceptance.allows(
                    crate::dev_acceptance::AcceptanceCapability::PublishVerification,
                    &campaign_id,
                    id,
                )
            }))
    {
        return Err(acceptance_scope_denied());
    }
    let rows = state
        .db
        .publish_verifications_for_campaign(&campaign_id, 1000)
        .map_err(preflight::err)?;
    let capabilities = state
        .db
        .publish_recovery_capabilities(&campaign_id)
        .map_err(preflight::err)?;
    let mut outcomes = Vec::new();
    let mut statuses = Vec::new();
    for capability in capabilities {
        let detail = state
            .db
            .get_publish_assignment_detail(&campaign_id, &capability.assignment_id)
            .map_err(preflight::err)?;
        let Some(assignment) = detail.and_then(|d| d.assignments.into_iter().next()) else {
            continue;
        };
        if udid.as_ref().is_some_and(|id| id != &assignment.udid) {
            continue;
        }
        let allowed = state.dev_acceptance.allows(
            crate::dev_acceptance::AcceptanceCapability::PublishVerification,
            &campaign_id,
            &assignment.udid,
        );
        let row = eligible_check_candidate(&capability, &rows, allowed);
        let (status, error) = if let Some(row) = row {
            match verify_pending_assignment_inner(
                &state.control,
                &state.db,
                &state.events,
                row,
                true,
            )
            .await
            {
                Ok(outcome) => outcome,
                Err(error) => (CheckStatus::Ineligible, Some(error.to_string())),
            }
        } else if !allowed {
            (
                CheckStatus::Ineligible,
                Some("acceptanceScopeDenied".into()),
            )
        } else {
            unavailable_check_outcome(capability.check_link.reason)
        };
        statuses.push(status);
        outcomes.push(serde_json::json!({"assignmentId":assignment.id,"udid":assignment.udid,"verified":status==CheckStatus::Verified,"error":error,"status":status}));
    }
    let status = check_summary(&statuses);
    Ok(
        serde_json::json!({"campaignId":campaign_id,"state":status,"reason":if statuses.is_empty(){Some("noCandidate")}else{None},"outcomes":outcomes}),
    )
}

pub(super) fn eligible_check_candidate<'a>(
    capability: &riviu_core::db::PublishRecoveryCapabilities,
    candidates: &'a [riviu_core::db::PendingPublishVerification],
    allowed: bool,
) -> Option<&'a riviu_core::db::PendingPublishVerification> {
    (allowed && capability.check_link.allowed)
        .then(|| {
            candidates
                .iter()
                .find(|row| row.assignment_id == capability.assignment_id)
        })
        .flatten()
}

/// The phone condition holding a due automatic check, named for the operator.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishVerificationWait {
    pub code: &'static str,
    pub message: &'static str,
    /// When the background verifier tries this phone again after a session-open failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishRecoveryCapabilitiesView {
    #[serde(flatten)]
    pub capabilities: riviu_core::db::PublishRecoveryCapabilities,
    /// The background verifier observes this row on its own; otherwise only an
    /// operator action (Kiểm tra liên kết / Tiếp tục kiểm tra link) can.
    pub automatic_verification: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_wait: Option<PublishVerificationWait>,
}

/// One background-verifier row, reduced on the reader lane to what the view maps.
struct AutomaticVerification {
    assignment_id: String,
    udid: String,
    /// Due now and not holding a retained metadata candidate; both parse the row's evidence.
    due: bool,
}

#[tauri::command]
pub async fn publish_recovery_capabilities(
    state: State<'_, AppState>,
    campaign_id: String,
) -> Result<Vec<PublishRecoveryCapabilitiesView>, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let id = campaign_id.clone();
    let (capabilities, automatic, fleet_full) = read_for_view(
        &state.db,
        "publish_recovery_capabilities",
        Some(&campaign_id),
        move |db| {
            let capabilities = db.publish_recovery_capabilities(&id)?;
            // The same rows and phone predicate the background verifier uses; read-only.
            let automatic = db.pending_current_publish_verifications_for_campaign(&id, 1000)?;
            let now = chrono::Utc::now();
            let automatic = automatic
                .into_iter()
                .map(|row| AutomaticVerification {
                    due: row.is_due(now) && matches!(row.retained_metadata_candidate(), Ok(None)),
                    assignment_id: row.assignment_id,
                    udid: row.udid,
                })
                .collect::<Vec<_>>();
            let fleet_full = automatic.iter().any(|row| row.due)
                && db
                    .publish_phone_work_in_use()
                    .is_ok_and(|used| used >= riviu_core::db::PUBLISH_PHONE_WORK_CAP);
            Ok((capabilities, automatic, fleet_full))
        },
        |(capabilities, automatic, _)| {
            format!(
                "{} dòng, {} dòng chờ kiểm tra tự động",
                capabilities.len(),
                automatic.len()
            )
        },
    )
    .await
    .map_err(preflight::err)?;
    Ok(capabilities
        .into_iter()
        .map(|capabilities| {
            let row = automatic
                .iter()
                .find(|row| row.assignment_id == capabilities.assignment_id);
            let verification_wait = row.filter(|row| row.due).and_then(|row| {
                let wait = VerificationQueue::phone_wait(
                    state.registry.get(&row.udid).as_ref(),
                    state.control.current_work_owner(&row.udid).is_some(),
                    fleet_full,
                );
                // On an admitted phone, the last session-open failure is the cause.
                let session = wait
                    .is_none_or(|wait| wait.admits())
                    .then(|| SESSION_WAITS.lock().get(&row.udid).cloned())
                    .flatten();
                match session {
                    Some(session) => Some(PublishVerificationWait {
                        code: session.category,
                        message: session.message,
                        retry_at: Some(session.retry_at.to_rfc3339()),
                    }),
                    None => wait.map(|wait| PublishVerificationWait {
                        code: wait.code(),
                        message: wait.message(),
                        retry_at: None,
                    }),
                }
            });
            PublishRecoveryCapabilitiesView {
                capabilities,
                automatic_verification: row.is_some(),
                verification_wait,
            }
        })
        .collect())
}

#[tauri::command]
pub fn publish_resume_verification(
    state: State<'_, AppState>,
    assignment_id: String,
    confirmed: bool,
    expected_revision: i64,
) -> Result<riviu_core::db::PublishResumeVerificationResult, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    if state.dev_acceptance.active() {
        ensure_assignment_allowed(&state.db, &assignment_id, |campaign, udid| {
            state.dev_acceptance.allows(
                crate::dev_acceptance::AcceptanceCapability::PublishVerification,
                campaign,
                udid,
            )
        })?;
    }
    resume_verification_and_announce(
        &state.db,
        &state.events,
        &assignment_id,
        confirmed,
        expected_revision,
    )
    .map_err(preflight::err)
}

pub(super) fn acceptance_scope_denied() -> CommandError {
    CommandError::code(
        "AcceptanceScopeDenied",
        "Bài hoặc máy nằm ngoài phạm vi nghiệm thu thủ công đã khóa",
    )
}

pub(super) fn ensure_assignment_allowed(
    db: &Database,
    assignment_id: &str,
    allowed: impl FnOnce(&str, &str) -> bool,
) -> Result<(), CommandError> {
    let campaign = db
        .publication_campaign_id(assignment_id)
        .map_err(preflight::err)?
        .ok_or_else(|| CommandError::invalid_argument("Không tìm thấy bài"))?;
    let detail = db
        .get_publish_assignment_detail(&campaign, assignment_id)
        .map_err(preflight::err)?
        .ok_or_else(|| CommandError::invalid_argument("Không tìm thấy bài"))?;
    let assignment = detail
        .assignments
        .iter()
        .find(|assignment| assignment.id == assignment_id)
        .ok_or_else(|| CommandError::invalid_argument("Không tìm thấy bài"))?;
    if allowed(&campaign, &assignment.udid) {
        Ok(())
    } else {
        Err(acceptance_scope_denied())
    }
}

pub(super) fn resume_verification_and_announce(
    db: &Database,
    events: &riviu_core::events::EventBus,
    assignment_id: &str,
    confirmed: bool,
    expected_revision: i64,
) -> anyhow::Result<riviu_core::db::PublishResumeVerificationResult> {
    let campaign = db.publication_campaign_id(assignment_id)?;
    let result = db.resume_publish_verification(assignment_id, confirmed, expected_revision)?;
    if result.state == riviu_core::db::PublishResumeVerificationState::Accepted {
        if let Some(campaign) = campaign {
            execution::announce(events, db, &campaign);
        }
    }
    Ok(result)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum CheckStatus {
    Verified,
    Pending,
    Busy,
    Stopped,
    Stale,
    NoCandidate,
    Ineligible,
}

pub(super) fn unavailable_check_outcome(reason: Option<String>) -> (CheckStatus, Option<String>) {
    let status = match reason.as_deref() {
        Some("alreadyVerified") => CheckStatus::Verified,
        Some("operatorStopped") => CheckStatus::Stopped,
        Some("activePipeline") => CheckStatus::Busy,
        Some("noCandidate") | None => CheckStatus::NoCandidate,
        _ => CheckStatus::Ineligible,
    };
    (
        status,
        if status == CheckStatus::Verified {
            None
        } else {
            reason
        },
    )
}

pub(super) fn check_summary(statuses: &[CheckStatus]) -> CheckStatus {
    if statuses.is_empty() {
        return CheckStatus::NoCandidate;
    }
    for status in [
        CheckStatus::Stale,
        CheckStatus::Stopped,
        CheckStatus::Busy,
        CheckStatus::Pending,
        CheckStatus::Ineligible,
        CheckStatus::NoCandidate,
    ] {
        if statuses.contains(&status) {
            return status;
        }
    }
    CheckStatus::Verified
}

pub(super) fn committed_observation_outcome(
    db: &Database,
    candidate: &riviu_core::db::PendingPublishVerification,
) -> anyhow::Result<(CheckStatus, Option<String>)> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    enum State {
        Pending,
        NeedsReview,
        Verified,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Status {
        state: State,
        reason: Option<String>,
        next_check_at: Option<String>,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Evidence {
        verification_status: Status,
    }
    let detail = db
        .get_publish_assignment_detail(&candidate.campaign_id, &candidate.assignment_id)?
        .context("assignment missing after observation")?;
    let assignment = detail
        .assignments
        .into_iter()
        .next()
        .context("assignment missing after observation")?;
    let evidence: Evidence = serde_json::from_str(
        assignment
            .evidence_json
            .as_deref()
            .context("observation evidence missing")?,
    )?;
    let status = evidence.verification_status;
    Ok(match status.state {
        State::Verified => (CheckStatus::Verified, None),
        State::Pending
            if status
                .next_check_at
                .as_deref()
                .is_some_and(|s| chrono::DateTime::parse_from_rfc3339(s).is_ok()) =>
        {
            (CheckStatus::Pending, None)
        }
        _ => (
            CheckStatus::Ineligible,
            status
                .reason
                .or_else(|| Some("verificationNeedsReview".into())),
        ),
    })
}

#[derive(Debug)]
pub(super) struct VerificationObservation {
    pub code: &'static str,
    pub reason: String,
    pub diagnostic: Option<serde_json::Value>,
}
impl std::fmt::Display for VerificationObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}
impl std::error::Error for VerificationObservation {}

/// The verifier's own UI session did not open (helper not running or refused).
/// No screen was read, so it is not an observation and spends no verification budget.
/// The cause keeps the driver's fixed helper vocabulary for the log and the device row.
#[derive(Debug, Clone)]
pub(crate) struct SessionUnavailable {
    /// `shell_root`, `shell_uid`, `android_user`, `recovery_required` or `session_open`.
    pub(crate) category: &'static str,
    /// Vietnamese operator reason; never device output.
    pub(crate) message: &'static str,
    detail: String,
}
impl SessionUnavailable {
    /// The control plane serializes the driver chain, so only the driver's own typed
    /// markers name a category; anything else stays the generic session-open cause.
    pub(super) fn from_control_error(error: &riviu_core::DeviceControlError) -> Self {
        use riviu_android_driver::riviu_agent::{
            HelperRecoveryRequired, HelperShellIdentityRefused,
        };
        let detail: String = error.to_string().chars().take(512).collect();
        // Identity refusals first: they may also carry the recovery fence.
        let (category, message) = match HelperShellIdentityRefused::from_chain_text(&detail) {
            Some(refusal) => (refusal.category(), refusal.operator_message()),
            None if detail.contains(&HelperRecoveryRequired.to_string()) => (
                "recovery_required",
                "Cần đối soát phiên helper đang giữ; chưa nhận phiên thay thế. App tự đối soát helper khi máy kết nối lại; nếu kéo dài, xem máy này trong Cần xử lý",
            ),
            None => (
                "session_open",
                "Chưa mở được phiên kiểm tra trên máy (helper hoặc UiAutomator chưa chạy được) — app tự thử lại; nếu kéo dài, xem máy này trong Cần xử lý",
            ),
        };
        Self {
            category,
            message,
            detail,
        }
    }
}
impl std::fmt::Display for SessionUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail)
    }
}
impl std::error::Error for SessionUnavailable {}

/// The background verifier's last session-open failure on a phone and its next try.
#[derive(Debug, Clone)]
struct SessionWait {
    category: &'static str,
    message: &'static str,
    retry_at: chrono::DateTime<chrono::Utc>,
}

/// Per phone, for the device row. Memory only, pruned to pending phones by the worker
/// every tick; it names a wait and never admits an observation, Post or Sheet write.
static SESSION_WAITS: std::sync::LazyLock<parking_lot::Mutex<HashMap<String, SessionWait>>> =
    std::sync::LazyLock::new(Default::default);

/// Keeps `udid`'s session-open failure; true when its cause changed, to log it once.
pub(crate) fn note_session_unavailable(
    udid: &str,
    unavailable: &SessionUnavailable,
    retry_at: chrono::DateTime<chrono::Utc>,
) -> bool {
    let wait = SessionWait {
        category: unavailable.category,
        message: unavailable.message,
        retry_at,
    };
    SESSION_WAITS
        .lock()
        .insert(udid.to_owned(), wait)
        .is_none_or(|previous| previous.category != unavailable.category)
}

/// The session opened (or the phone left the queue); true when a failure was kept.
pub(crate) fn clear_session_wait(udid: &str) -> bool {
    SESSION_WAITS.lock().remove(udid).is_some()
}

pub(crate) fn retain_session_waits(pending: &std::collections::HashSet<&str>) {
    SESSION_WAITS
        .lock()
        .retain(|udid, _| pending.contains(udid.as_str()));
}

/// Only this observer's publication is hydrated. Other devices' media and the
/// campaign event history do not belong in each concurrent verification task.
pub(super) fn verification_input(
    db: &Database,
    candidate: &riviu_core::db::PendingPublishVerification,
) -> anyhow::Result<
    Option<(
        riviu_core::PublishAssignmentRecord,
        riviu_core::PublishBundle,
    )>,
> {
    let detail = db
        .get_publish_assignment_detail(&candidate.campaign_id, &candidate.assignment_id)?
        .context("campaign missing")?;
    let assignment = detail
        .assignments
        .into_iter()
        .find(|a| a.id == candidate.assignment_id)
        .context("assignment missing")?;
    if db.publish_assignment_revision(&candidate.assignment_id)? != candidate.revision
        || assignment.effect_intent != candidate.effect_intent
        || assignment.evidence_json != candidate.evidence_json
    {
        return Ok(None);
    }
    let bundle = detail
        .bundles
        .into_iter()
        .find(|b| b.id == candidate.bundle_id)
        .context("bundle missing")?;
    Ok(Some((assignment, bundle)))
}

/// Network-only authorization: no device-wide restart/upload admission is needed.
/// The captured assignment must still be exactly the observer's immutable snapshot.
pub(super) fn authorize_metadata_observer(
    db: &Database,
    assignment: &riviu_core::PublishAssignmentRecord,
    observer: &riviu_core::db::PendingPublishVerification,
) -> anyhow::Result<()> {
    anyhow::ensure!(observer.retained_metadata_candidate()?.is_some()
        && observer.assignment_id == assignment.id
        && observer.campaign_id == assignment.campaign_id
        && observer.bundle_id == assignment.bundle_id
        && observer.udid == assignment.udid
        && observer.effect_intent == assignment.effect_intent
        && observer.evidence_json == assignment.evidence_json
        && db.publish_verification_is_current(observer)?,
        "metadata observer changed or stopped; phone fallback forbidden");
    Ok(())
}

fn metadata_unavailable(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<VerificationObservation>()
        .is_some_and(|observation| observation.code == "metadataUnavailable")
}

/// The network lookup stays the recorded observation; the phone attempt rides along as
/// evidence, so an unproven own-Profile pass spends no phone-search budget.
fn with_phone_route(
    lookup: anyhow::Error,
    phone: &anyhow::Error,
    udid: &str,
    assignment_id: &str,
) -> anyhow::Error {
    let observation = phone.downcast_ref::<VerificationObservation>();
    let busy = matches!(
        phone.downcast_ref::<riviu_core::DeviceControlError>(),
        Some(riviu_core::DeviceControlError::Busy(_))
    );
    let code = match observation {
        Some(observation) => observation.code,
        None if busy => "deviceBusy",
        None if phone.downcast_ref::<SessionUnavailable>().is_some() => "sessionUnavailable",
        None => "readFailed",
    };
    log::info!(
        "Bài {assignment_id} trên {udid}: dữ liệu công khai TikTok chưa trả lời, Hồ sơ trên máy chưa xác minh được ({code}); giữ liên kết ứng viên, không đăng lại"
    );
    match lookup.downcast::<VerificationObservation>() {
        Ok(mut host) => {
            if let Some(diagnostic) = host.diagnostic.as_mut() {
                diagnostic["phoneRoute"] = serde_json::json!({
                    "reasonCode": code,
                    "reason": phone.to_string().chars().take(512).collect::<String>(),
                    "diagnostic": observation.and_then(|observation| observation.diagnostic.clone()),
                });
            }
            host.into()
        }
        Err(lookup) => lookup,
    }
}

pub(crate) async fn verify_pending_assignment(
    control: &DeviceControlPlane,
    db: &Database,
    events: &riviu_core::events::EventBus,
    candidate: &riviu_core::db::PendingPublishVerification,
) -> anyhow::Result<bool> {
    Ok(
        verify_pending_assignment_inner(control, db, events, candidate, false)
            .await?
            .0
            == CheckStatus::Verified,
    )
}

async fn verify_pending_assignment_inner(
    control: &DeviceControlPlane,
    db: &Database,
    events: &riviu_core::events::EventBus,
    candidate: &riviu_core::db::PendingPublishVerification,
    manual: bool,
) -> anyhow::Result<(CheckStatus, Option<String>)> {
    if !db.publish_verification_is_current(candidate)? {
        return Ok((CheckStatus::Stale, None));
    }
    let retained = match candidate.retained_metadata_candidate() {
        Ok(retained) => retained,
        Err(_) => {
            // Controlled terminal diagnostic; never serialize the invalid envelope.
            let reason = "Retained metadata candidate is invalid; review the original publication, no phone fallback.";
            let record = if manual { Database::record_manual_publish_verification_diagnostic }
                else { Database::record_publish_verification_diagnostic };
            if record(db, candidate, reason, "metadataCandidateInvalid", None)? {
                execution::announce(events, db, &candidate.campaign_id);
                return committed_observation_outcome(db, candidate);
            }
            return Ok((CheckStatus::Stale, None));
        }
    };
    let metadata_only = retained.is_some();
    if metadata_only && !manual && !candidate.automatic_metadata_observation_allowed(chrono::Utc::now()) {
        return Ok((CheckStatus::Pending, None));
    }
    let phone_free = control.current_work_owner(&candidate.udid).is_none();
    if !metadata_only && !phone_free {
        return Ok((CheckStatus::Busy, None));
    }
    // A candidate whose last public-metadata lookup was an availability failure (timeout,
    // 429/503, request) may also be proven on its free phone's own Profile. The phone claim
    // also excludes a concurrent metadata observer of this assignment; a busy phone keeps
    // the network-only lookup, as before.
    let phone_permit = match &retained {
        Some(pending)
            if phone_free
                && matches!(
                    pending.captured.metadata.state,
                    riviu_core::publish_submission::MetadataAttemptState::Unavailable
                ) =>
        {
            db.try_publish_work(&candidate.udid, "verify", &candidate.assignment_id)?
        }
        _ => None,
    };
    let phone_route = phone_permit.is_some();
    let permit = if phone_route {
        phone_permit
    } else if metadata_only {
        db.try_publish_metadata_work(candidate, manual)?
    } else {
        db.try_publish_work(&candidate.udid, "verify", &candidate.assignment_id)?
    };
    let Some(permit) = permit else {
        return Ok((CheckStatus::Busy, None));
    };
    let Some((assignment, bundle)) = verification_input(db, candidate)? else {
        return Ok((CheckStatus::Stale, None));
    };
    if !db.publish_verification_is_current(candidate)? {
        return Ok((CheckStatus::Stale, None));
    }
    let attempt = candidate
        .evidence_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|value| value["verificationStatus"]["attempts"].as_u64())
        .unwrap_or(0)
        .saturating_add(1);
    progress::record_progress(
        db,
        &candidate.campaign_id,
        &candidate.assignment_id,
        progress::PublishProgress::CheckingExistingPostLink { attempt },
    );
    let capture = if metadata_only {
        // The network lookup never falls back to a phone search for a different post, even
        // if a concurrent revision removes or changes the retained candidate.
        let lookup = async {
            db.publish_campaign_request(&assignment.campaign_id)?
                .context("publish campaign request not found")?.network.ensure_implemented()?;
            anyhow::ensure!(!bundle.caption.trim().is_empty(), "caption missing for own-post proof");
            execution::capture_pending_metadata(db, &assignment, &bundle, Some(candidate))
                .await?.context("retained metadata candidate missing; phone fallback forbidden")
        }.await;
        match (lookup, retained.as_ref()) {
            // Still unavailable: only the same candidate's own-Profile proof may verify it.
            (Err(error), Some(pending)) if phone_route && metadata_unavailable(&error) => {
                match execution::capture_retained_candidate_on_phone(
                    db,
                    control,
                    &assignment,
                    &bundle,
                    Some(candidate),
                    &pending.captured,
                )
                .await
                {
                    Ok(captured) => Ok(captured),
                    Err(phone) => Err(with_phone_route(
                        error,
                        &phone,
                        &candidate.udid,
                        &candidate.assignment_id,
                    )),
                }
            }
            (lookup, _) => lookup,
        }
    } else {
        execution::capture_confirmed_assignment_link(db, control, &assignment, &bundle, Some(candidate)).await
    };
    // Keep the work permit through the existing CAS/outbox or diagnostic commit.
    let _permit = permit;
    match capture {
        Ok(captured) => {
            let link = captured.url;
            let mut evidence = execution::evidence_with_post_url(
                candidate
                    .evidence_json
                    .as_deref()
                    .and_then(|v| serde_json::from_str(v).ok()),
                &link,
            );
            evidence["verificationDiagnostic"] = captured.diagnostic;
            let expanded =
                evidence["verificationDiagnostic"]["stage"] == "expandedPhotoPublicProof";
            let video_public = evidence["verificationDiagnostic"]["stage"] == "videoPublicProof";
            let candidate_public = evidence["verificationDiagnostic"]["stage"] == "candidatePublicMetadata";
            // Own-Profile proof of a post whose public metadata did not answer, in this pass
            // (it copied a candidate first) or for an already retained candidate.
            let without_metadata = !(expanded || video_public || candidate_public)
                && (phone_route
                    || evidence["verificationDiagnostic"]["capturedMetadataCandidate"].is_object());
            let post = if evidence.get("post").is_some() {
                &mut evidence["post"]
            } else {
                &mut evidence
            };
            post["publicationVerified"] = serde_json::json!(true);
            post["state"] = serde_json::json!("posted");
            post["verificationMethod"] = if candidate_public {
                serde_json::json!("candidateCanonicalPublicMetadata")
            } else if video_public {
                serde_json::json!("videoCanonicalPublicMetadata")
            } else if expanded {
                serde_json::json!("expandedPhotoCaptionCanonicalPublicMetadata")
            } else {
                serde_json::json!("ownProfileCaptionAndCanonicalLink")
            };
            let changed = db.record_verified_publish_with_sheet_row(
                candidate,
                &evidence.to_string(),
                &link,
                execution::poster_identity(),
                &bundle.partners,
            )?;
            if changed {
                if without_metadata {
                    log::info!(
                        "Bài {} trên {}: đã xác minh trên Hồ sơ của chính tài khoản (caption, thời gian, Sao chép liên kết) khi dữ liệu công khai TikTok chưa trả lời",
                        candidate.assignment_id, candidate.udid
                    );
                }
                progress::record_progress(
                    db,
                    &candidate.campaign_id,
                    &candidate.assignment_id,
                    riviu_core::tiktok_composer::PublishProgress::PostConfirmed,
                );
                execution::announce(events, db, &candidate.campaign_id);
            }
            Ok((
                if changed {
                    CheckStatus::Verified
                } else {
                    CheckStatus::Stale
                },
                None,
            ))
        }
        Err(error) => {
            if matches!(
                error.downcast_ref::<riviu_core::DeviceControlError>(),
                Some(riviu_core::DeviceControlError::Busy(_))
            ) {
                return Ok((CheckStatus::Busy, None));
            }
            if !manual && error.downcast_ref::<SessionUnavailable>().is_some() {
                // Typed, unrecorded: the worker logs and shows the cause and retries
                // this phone later; no observation budget is spent.
                return Err(error);
            }
            let reason: String = error.to_string().chars().take(512).collect();
            // The stale observer can never overwrite a newer link or cancelled assignment.
            let code = error
                .downcast_ref::<VerificationObservation>()
                .map_or("readFailed", |o| o.code);
            let record = if manual {
                Database::record_manual_publish_verification_diagnostic
            } else {
                Database::record_publish_verification_diagnostic
            };
            if record(
                db,
                candidate,
                &reason,
                code,
                error
                    .downcast_ref::<VerificationObservation>()
                    .and_then(|observation| observation.diagnostic.as_ref()),
            )? {
                let outcome = committed_observation_outcome(db, candidate)?;
                let post_unknown = candidate
                    .evidence_json
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                    .is_none_or(|value| {
                        let post = value.get("post").unwrap_or(&value);
                        !matches!(post["state"].as_str(), Some("submitted" | "posted"))
                    });
                let step = if outcome.0 == CheckStatus::Pending && post_unknown {
                    Some(progress::PublishProgress::PostUncertain { reason })
                } else if outcome.0 == CheckStatus::Pending {
                    Some(progress::PublishProgress::LinkPending { reason })
                } else if outcome.0 == CheckStatus::Ineligible {
                    Some(progress::PublishProgress::LinkNeedsReview {
                        reason: outcome.1.clone().unwrap_or(reason),
                    })
                } else {
                    None
                };
                if let Some(step) = step {
                    progress::record_progress(
                        db,
                        &candidate.campaign_id,
                        &candidate.assignment_id,
                        step,
                    );
                }
                execution::announce(events, db, &candidate.campaign_id);
                return Ok(outcome);
            }
            Ok((CheckStatus::Stale, None))
        }
    }
}
