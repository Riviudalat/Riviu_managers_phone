use crate::command_error::CommandError;
use crate::state::AppState;
use riviu_core::interaction_campaign::{
    open_interaction_context, open_target_confirmed, InteractionDevice, TargetProof,
};
use riviu_core::{
    PublicCleanupCapability, PublicCleanupCapabilityStatus, PublicCleanupKind,
    PublicCleanupRunRecord, PublicCleanupRunState, PublicCleanupSourceAction, ResolvedTikTokTarget,
    ToggleCleanupEvidence, ToggleCleanupObservation, ToggleCleanupVerdict,
};
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicCleanupPreflightRequest {
    pub campaign_id: String,
    pub assignment_id: Option<String>,
    pub kind: PublicCleanupKind,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicCleanupExecuteRequest {
    pub request_id: String,
    pub campaign_id: String,
    pub assignment_id: String,
    pub kind: PublicCleanupKind,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicCleanupExecutionReport {
    pub capability: PublicCleanupCapability,
    pub run: Option<PublicCleanupRunRecord>,
    pub evidence: Option<ToggleCleanupEvidence>,
    pub session_cleanup_warning: Option<String>,
}

struct ReadyCleanup {
    capability: PublicCleanupCapability,
    source: PublicCleanupSourceAction,
    target: ResolvedTikTokTarget,
}

enum CleanupPreflight {
    Refused(PublicCleanupCapability),
    Ready(Box<ReadyCleanup>),
}

fn refused(
    kind: PublicCleanupKind,
    status: PublicCleanupCapabilityStatus,
    reason: impl Into<String>,
    device_udid: Option<String>,
) -> PublicCleanupCapability {
    PublicCleanupCapability {
        kind,
        status,
        reason: reason.into(),
        device_udid,
        effect_boundary_crossed: false,
    }
}

fn preflight(
    state: &AppState,
    request: &PublicCleanupPreflightRequest,
) -> Result<CleanupPreflight, CommandError> {
    // These three refusals are independent of the selected phone. Keep them before every DB or
    // device lookup: asking whether deletion/unfollow is supported must remain a zero-lease,
    // zero-tap operation while the controls are unmeasured.
    let static_capability = riviu_core::static_public_cleanup_capability(request.kind);
    if !static_capability.status.can_execute() {
        return Ok(CleanupPreflight::Refused(static_capability));
    }

    let assignment_id = request.assignment_id.as_deref().ok_or_else(|| {
        CommandError::invalid_argument("Like/Save cleanup requires an assignmentId")
    })?;
    let source = state
        .db
        .interaction_public_cleanup_source(&request.campaign_id, assignment_id, request.kind)
        .map_err(CommandError::operation)?;
    let Some(source) = source else {
        return Ok(CleanupPreflight::Refused(refused(
            request.kind,
            PublicCleanupCapabilityStatus::SourceNotConfirmed,
            "No matching campaign-owned public action exists",
            None,
        )));
    };
    if !source.source_confirmed {
        return Ok(CleanupPreflight::Refused(refused(
            request.kind,
            PublicCleanupCapabilityStatus::SourceNotConfirmed,
            "The source action is not Confirmed; cleanup cannot remove an effect this campaign did not prove it created",
            Some(source.device_udid),
        )));
    }
    if !state.control.reports_element_bounds(&source.device_udid) {
        return Ok(CleanupPreflight::Refused(refused(
            request.kind,
            PublicCleanupCapabilityStatus::HierarchyRequired,
            "The measured Unlike/Unsave adapter requires hierarchy state on this device",
            Some(source.device_udid),
        )));
    }
    let (campaign_request, _) = state
        .db
        .get_interaction_campaign_request(&request.campaign_id)
        .map_err(CommandError::operation)?
        .ok_or_else(|| CommandError::code("CleanupSourceMissing", "campaign does not exist"))?;
    let target = campaign_request
        .targets
        .into_iter()
        .find(|target| target.target_key == source.target_key)
        .ok_or_else(|| {
            CommandError::code(
                "CleanupTargetMissing",
                "the immutable campaign target no longer resolves",
            )
        })?;
    let mut capability = static_capability;
    capability.device_udid = Some(source.device_udid.clone());
    Ok(CleanupPreflight::Ready(Box::new(ReadyCleanup {
        capability,
        source,
        target,
    })))
}

#[tauri::command]
pub fn public_cleanup_preflight(
    state: State<'_, AppState>,
    request: PublicCleanupPreflightRequest,
) -> Result<PublicCleanupCapability, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    Ok(match preflight(&state, &request)? {
        CleanupPreflight::Refused(capability) => capability,
        CleanupPreflight::Ready(ready) => ready.capability,
    })
}

fn pre_effect_evidence(error: impl Into<String>) -> ToggleCleanupEvidence {
    ToggleCleanupEvidence {
        verdict: ToggleCleanupVerdict::FailedBeforeEffect,
        initial: None,
        final_observation: None,
        effect_boundary_crossed: false,
        error: Some(error.into()),
    }
}

fn terminal_state(evidence: &ToggleCleanupEvidence) -> PublicCleanupRunState {
    match evidence.verdict {
        ToggleCleanupVerdict::Cleared => PublicCleanupRunState::Cleared,
        ToggleCleanupVerdict::AlreadyClear => PublicCleanupRunState::AlreadyClear,
        ToggleCleanupVerdict::NoControl
        | ToggleCleanupVerdict::StateUnreadable
        | ToggleCleanupVerdict::TargetChangedBeforeEffect
        | ToggleCleanupVerdict::FailedBeforeEffect => PublicCleanupRunState::FailedBeforeEffect,
        ToggleCleanupVerdict::TargetChangedAfterEffect
        | ToggleCleanupVerdict::NotConfirmed
        | ToggleCleanupVerdict::UncertainAfterEffect => PublicCleanupRunState::Uncertain,
    }
}

fn verdict_code(verdict: ToggleCleanupVerdict) -> &'static str {
    match verdict {
        ToggleCleanupVerdict::Cleared => "cleanup_cleared",
        ToggleCleanupVerdict::AlreadyClear => "cleanup_already_clear",
        ToggleCleanupVerdict::NoControl => "cleanup_no_control",
        ToggleCleanupVerdict::StateUnreadable => "cleanup_state_unreadable",
        ToggleCleanupVerdict::TargetChangedBeforeEffect => "cleanup_target_changed_before_effect",
        ToggleCleanupVerdict::FailedBeforeEffect => "cleanup_failed_before_effect",
        ToggleCleanupVerdict::TargetChangedAfterEffect => "cleanup_target_changed_after_effect",
        ToggleCleanupVerdict::NotConfirmed => "cleanup_not_confirmed",
        ToggleCleanupVerdict::UncertainAfterEffect => "cleanup_uncertain_after_effect",
    }
}

fn settle(
    state: &AppState,
    run_id: &str,
    claimed_revision: i64,
    armed_revision: Option<i64>,
    evidence: &ToggleCleanupEvidence,
) -> Result<PublicCleanupRunRecord, CommandError> {
    let terminal = terminal_state(evidence);
    let revision = if evidence.effect_boundary_crossed {
        armed_revision.ok_or_else(|| {
            CommandError::code(
                "CleanupJournalInvariant",
                "effect boundary crossed without an armed cleanup journal",
            )
        })?
    } else {
        claimed_revision
    };
    let evidence_json = serde_json::to_string(evidence).map_err(CommandError::operation)?;
    let error = (!matches!(
        terminal,
        PublicCleanupRunState::Cleared | PublicCleanupRunState::AlreadyClear
    ))
    .then(|| verdict_code(evidence.verdict));
    let changed = state
        .db
        .settle_public_cleanup(run_id, revision, terminal, Some(&evidence_json), error)
        .map_err(CommandError::operation)?;
    if !changed {
        return Err(CommandError::code(
            "CleanupOwnershipLost",
            "cleanup journal ownership changed while the device operation was running",
        ));
    }
    state
        .db
        .get_public_cleanup_run(run_id)
        .map_err(CommandError::operation)?
        .ok_or_else(|| {
            CommandError::code(
                "CleanupJournalMissing",
                "cleanup journal disappeared after settlement",
            )
        })
}

fn report_existing(
    capability: PublicCleanupCapability,
    run: PublicCleanupRunRecord,
) -> PublicCleanupExecutionReport {
    let evidence = run
        .evidence
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok());
    PublicCleanupExecutionReport {
        capability,
        run: Some(run),
        evidence,
        session_cleanup_warning: None,
    }
}

async fn execute_ready(
    state: &AppState,
    request_id: &str,
    ready: ReadyCleanup,
) -> Result<PublicCleanupExecutionReport, CommandError> {
    let journal = state
        .db
        .ensure_public_cleanup_run(request_id, &ready.source, &ready.target)
        .map_err(CommandError::operation)?;
    let Some(claimed_revision) = state
        .db
        .claim_public_cleanup(&journal.id)
        .map_err(CommandError::operation)?
    else {
        let current = state
            .db
            .get_public_cleanup_run(&journal.id)
            .map_err(CommandError::operation)?
            .ok_or_else(|| {
                CommandError::code("CleanupJournalMissing", "cleanup journal does not exist")
            })?;
        return Ok(report_existing(ready.capability, current));
    };

    let InteractionDevice {
        context,
        target_package,
    } = match open_interaction_context(&state.control, &ready.source.device_udid).await {
        Ok(device) => device,
        Err(error) => {
            let evidence = pre_effect_evidence(error.to_string());
            let run = settle(state, &journal.id, claimed_revision, None, &evidence)?;
            return Ok(PublicCleanupExecutionReport {
                capability: ready.capability,
                run: Some(run),
                evidence: Some(evidence),
                session_cleanup_warning: None,
            });
        }
    };
    let result = async {
        let session = state
            .control
            .streaming_session(&context)
            .map_err(anyhow::Error::from)?;
        anyhow::ensure!(
            session.supports_element_bounds(),
            "cleanup session did not provide hierarchy bounds"
        );
        let language = session
            .ui_language()
            .await
            .ok_or_else(|| anyhow::anyhow!("cleanup session did not report UI language"))?;
        let app_version = session
            .app_version(&target_package)
            .await
            .ok_or_else(|| anyhow::anyhow!("cleanup session did not report TikTok version"))?;
        let labels =
            riviu_core::tiktok_labels::controls_for(&target_package, &language, &app_version)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "no measured TikTok controls for {target_package} {language} {app_version}"
                    )
                })?;
        anyhow::ensure!(
            matches!(
                open_target_confirmed(
                    &state.nurture_engine,
                    &ready.source.device_udid,
                    session.as_ref(),
                    &ready.target,
                    &target_package,
                )
                .await?,
                TargetProof::Identified
            ),
            "canonical target author was not identified"
        );

        let mut armed_revision = None;
        let arm = |_: &ToggleCleanupObservation| {
            let revision = state
                .db
                .arm_public_cleanup(
                    &journal.id,
                    claimed_revision,
                    match ready.source.kind {
                        PublicCleanupKind::Like => "unlike_confirmed_source",
                        PublicCleanupKind::Save => "unsave_confirmed_source",
                        _ => unreachable!("preflight only admits Like/Save"),
                    },
                )?
                .ok_or_else(|| anyhow::anyhow!("cleanup journal ownership was lost before tap"))?;
            armed_revision = Some(revision);
            Ok(())
        };
        let evidence = match ready.source.kind {
            PublicCleanupKind::Like => {
                riviu_core::tiktok_like::unlike_post_with_gate(session.as_ref(), labels, arm).await
            }
            PublicCleanupKind::Save => {
                riviu_core::tiktok_save::unsave_post_with_gate(session.as_ref(), labels, arm).await
            }
            _ => unreachable!("preflight only admits Like/Save"),
        };
        Ok::<_, anyhow::Error>((evidence, armed_revision))
    }
    .await;
    let closed = state.control.close_ui_context(context).await;

    let (evidence, armed_revision) = match result {
        Ok(result) => result,
        Err(error) => (pre_effect_evidence(format!("{error:#}")), None),
    };
    let run = settle(
        state,
        &journal.id,
        claimed_revision,
        armed_revision,
        &evidence,
    )?;
    Ok(PublicCleanupExecutionReport {
        capability: ready.capability,
        run: Some(run),
        evidence: Some(evidence),
        session_cleanup_warning: closed.err().map(|error| error.to_string()),
    })
}

#[tauri::command]
pub async fn public_cleanup_execute(
    state: State<'_, AppState>,
    request: PublicCleanupExecuteRequest,
) -> Result<PublicCleanupExecutionReport, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let preflight_request = PublicCleanupPreflightRequest {
        campaign_id: request.campaign_id,
        assignment_id: Some(request.assignment_id),
        kind: request.kind,
    };
    match preflight(&state, &preflight_request)? {
        CleanupPreflight::Refused(capability) => Ok(PublicCleanupExecutionReport {
            capability,
            run: None,
            evidence: None,
            session_cleanup_warning: None,
        }),
        CleanupPreflight::Ready(ready) => execute_ready(&state, &request.request_id, *ready).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_mapping_never_makes_an_after_effect_verdict_retryable() {
        for verdict in [
            ToggleCleanupVerdict::TargetChangedAfterEffect,
            ToggleCleanupVerdict::NotConfirmed,
            ToggleCleanupVerdict::UncertainAfterEffect,
        ] {
            let evidence = ToggleCleanupEvidence {
                verdict,
                initial: None,
                final_observation: None,
                effect_boundary_crossed: true,
                error: None,
            };
            assert_eq!(terminal_state(&evidence), PublicCleanupRunState::Uncertain);
            assert!(!terminal_state(&evidence).retry_is_safe());
        }
    }

    #[test]
    fn unsupported_paths_return_before_any_source_or_device_lookup() {
        let source = include_str!("public_cleanup_commands.rs");
        let body = source
            .split("fn preflight(")
            .nth(1)
            .expect("preflight function")
            .split("#[tauri::command]")
            .next()
            .expect("preflight body");
        let refusal = body
            .find("if !static_capability.status.can_execute()")
            .expect("static refusal");
        for operation in [
            "interaction_public_cleanup_source",
            "reports_element_bounds",
            "get_interaction_campaign_request",
        ] {
            assert!(
                refusal < body.find(operation).expect("preflight operation"),
                "{operation} moved before the zero-device static refusal"
            );
        }
        assert!(!body[..refusal].contains("open_interaction_context"));
    }
}
