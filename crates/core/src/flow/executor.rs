use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{
    capture_baseline, capture_process_baseline, compiled_plan_sha256, contracts,
    decode_and_hash_artifact, decode_vision_template, evaluate_postcondition,
    qualified_geometry_profile_id, verify_process_absence, ActionKind, CompiledActionConfig,
    CompiledFlowNode, CompiledFlowPlanV2, CompiledTapTarget, EvidenceBaseline, EvidenceError,
    EvidenceSpec, FlowArtifactRecord, FlowArtifactStore, FlowAttemptState, FlowCancellation,
    FlowCapabilitySnapshot, FlowContextReleaseProof, FlowDeviceContext, FlowDeviceRunState,
    FlowErrorRecord, FlowPreflightScope, NodeId, SideEffectClass,
};
use crate::db::{AttemptTransitionPatch, Database};
use crate::{
    AgentState, AgentStatus, ContextReleaseProof, DeviceCapabilitySnapshot, DeviceControlError,
    DeviceControlPlane, DeviceWorkOwner, GenerationFrameSource, InteractionSessionKind,
    ProcessAbsenceProof, TapPoint, UiCapacityReservation,
};

const EVIDENCE_TIMEOUT: Duration = Duration::from_secs(5);
const WAIT_SLICE: Duration = Duration::from_millis(250);

#[derive(Clone)]
pub(crate) struct FlowExecutorDeps {
    pub(crate) run_id: Uuid,
    pub(crate) udid: String,
    pub(crate) database: Arc<Database>,
    pub(crate) control: Arc<DeviceControlPlane>,
    pub(crate) frames: Arc<dyn GenerationFrameSource>,
    pub(crate) artifacts: FlowArtifactStore,
    pub(crate) cancellation: FlowCancellation,
}

#[derive(Clone)]
pub(crate) struct FlowExecutor {
    deps: FlowExecutorDeps,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct FlowExecutionError {
    code: &'static str,
    message: String,
    node_id: Option<NodeId>,
    attempt_id: Option<Uuid>,
    release_proof: Option<ContextReleaseProof>,
    retry_safe_evidence: Option<Box<serde_json::Value>>,
}

impl FlowExecutionError {
    pub(crate) fn code(&self) -> &'static str {
        self.code
    }

    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            node_id: None,
            attempt_id: None,
            release_proof: None,
            retry_safe_evidence: None,
        }
    }

    fn attributed(mut self, node_id: NodeId, attempt_id: Uuid) -> Self {
        self.node_id = Some(node_id);
        self.attempt_id = Some(attempt_id);
        self
    }

    fn released(mut self, proof: ContextReleaseProof) -> Self {
        self.release_proof = Some(proof);
        self
    }

    fn without_attribution(mut self) -> Self {
        self.node_id = None;
        self.attempt_id = None;
        self
    }

    fn retry_safe(mut self, evidence: serde_json::Value) -> Self {
        self.retry_safe_evidence = Some(Box::new(evidence));
        self
    }

    fn record(&self, udid: &str) -> FlowErrorRecord {
        FlowErrorRecord {
            code: self.code.to_string(),
            message: self.message.clone(),
            node_id: self.node_id,
            field: None,
            udid: Some(udid.to_string()),
            attempt_id: self.attempt_id,
        }
    }

    fn device(error: DeviceControlError) -> Self {
        Self::new("DeviceControl", error.to_string())
    }

    fn evidence(error: EvidenceError) -> Self {
        Self::new(error.code(), error.to_string())
    }

    fn other(error: impl std::fmt::Display) -> Self {
        Self::new("FlowExecution", error.to_string())
    }
}

struct FlowDevicePreflight {
    device_snapshot: Option<DeviceCapabilitySnapshot>,
    persisted_snapshot: FlowCapabilitySnapshot,
    profile_id: String,
}

enum ActionOutput {
    None,
    ProcessAbsent(ProcessAbsenceProof),
    Screenshot {
        label: String,
        format: String,
        bytes: Arc<Vec<u8>>,
    },
    /// An IfVision node decided which output port to take (`matched`/`notMatched`).
    /// Carries no postcondition — the decision itself is the outcome.
    Branch {
        port: String,
    },
}

struct ActionDispatchFailure {
    error: FlowExecutionError,
    stream_upgrade: Option<super::FlowDeviceUpgradeFailure>,
    request_reached_device: Option<bool>,
    deterministic_read_failure: bool,
}

impl ActionDispatchFailure {
    fn non_delivery(error: FlowExecutionError) -> Self {
        Self {
            error,
            stream_upgrade: None,
            request_reached_device: Some(false),
            deterministic_read_failure: false,
        }
    }

    fn deterministic_read(error: FlowExecutionError) -> Self {
        Self {
            error,
            stream_upgrade: None,
            request_reached_device: None,
            deterministic_read_failure: true,
        }
    }
}

impl From<FlowExecutionError> for ActionDispatchFailure {
    fn from(error: FlowExecutionError) -> Self {
        Self {
            error,
            stream_upgrade: None,
            request_reached_device: None,
            deterministic_read_failure: false,
        }
    }
}

impl FlowExecutor {
    pub(crate) fn new(deps: FlowExecutorDeps) -> Self {
        Self { deps }
    }

    pub(crate) async fn run_device(
        &self,
        device_run_id: Uuid,
        plan: CompiledFlowPlanV2,
    ) -> Result<(), FlowExecutionError> {
        self.validate_run_identity(device_run_id, &plan)?;
        let mut context = FlowDeviceContext::no_device_resources(&self.deps.udid);

        self.deps
            .database
            .transition_flow_device_run(
                device_run_id,
                FlowDeviceRunState::Queued,
                FlowDeviceRunState::Preflight,
                None,
            )
            .map_err(FlowExecutionError::other)?;

        if let Err(error) = validate_compiled_plan(&plan) {
            return self
                .finish_failed(device_run_id, &mut context, &mut None, error)
                .await;
        }

        if plan.context_plan.requires_exclusive {
            let acquire = self
                .deps
                .control
                .acquire_exclusive(&self.deps.udid, DeviceWorkOwner::Script);
            let cancelled = self.deps.cancellation.cancelled();
            tokio::pin!(acquire);
            tokio::pin!(cancelled);
            let acquired = tokio::select! {
                biased;
                _ = &mut cancelled => Err(FlowExecutionError::new(
                    "Cancelled",
                    "flow was cancelled while waiting for device ownership",
                )),
                result = &mut acquire => result.map_err(FlowExecutionError::device),
            };
            match acquired {
                Ok(exclusive) => context = FlowDeviceContext::Exclusive(exclusive),
                Err(error) => {
                    return self
                        .finish_failed(device_run_id, &mut context, &mut None, error)
                        .await;
                }
            }
        }

        let preflight_result = match target_bundle_id(&plan) {
            Some(target_bundle_id) => {
                let snapshot = match self
                    .deps
                    .control
                    .inspect_flow_device(
                        context.exclusive().map_err(FlowExecutionError::device)?,
                        target_bundle_id,
                    )
                    .await
                {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        return self
                            .finish_failed(
                                device_run_id,
                                &mut context,
                                &mut None,
                                FlowExecutionError::device(error),
                            )
                            .await;
                    }
                };
                self.build_preflight(&plan, target_bundle_id, snapshot)
            }
            None => self.build_target_free_preflight(&plan),
        };
        let preflight = match preflight_result {
            Ok(preflight) => preflight,
            Err(error) => {
                return self
                    .finish_failed(device_run_id, &mut context, &mut None, error)
                    .await;
            }
        };

        self.deps
            .database
            .transition_flow_device_run(
                device_run_id,
                FlowDeviceRunState::Preflight,
                FlowDeviceRunState::Running,
                Some(preflight.persisted_snapshot.clone()),
            )
            .map_err(FlowExecutionError::other)?;

        let initialized_attempts = self
            .deps
            .database
            .initialize_flow_device_attempts(device_run_id)
            .map_err(FlowExecutionError::other)?;

        let mut reservation = None;
        if plan.context_plan.requires_ui_session {
            match context.reserve_capacity(&self.deps.control).await {
                Ok(capacity) => reservation = Some(capacity),
                Err(failure) => {
                    return self
                        .finish_failed(
                            device_run_id,
                            &mut context,
                            &mut reservation,
                            FlowExecutionError::device(failure.error),
                        )
                        .await;
                }
            }
        }

        // Walk the graph from Start, following each node's single flow port and
        // each IfVision node's runtime-chosen branch. Only nodes on the taken
        // path execute; off-path attempts stay Queued and are ignored by the
        // path-aware success projection.
        let mut first_launch_pending = plan.context_plan.requires_ui_session;
        let mut current = plan.entry_node();
        let mut steps = 0usize;
        while let Some(node_id) = current {
            steps += 1;
            if steps > plan.nodes.len() + 1 {
                let error = FlowExecutionError::new(
                    "CompiledPlanCorrupt",
                    "taken path exceeds the compiled node count",
                );
                return self
                    .finish_failed(device_run_id, &mut context, &mut reservation, error)
                    .await;
            }
            if self.deps.cancellation.is_cancelled() {
                let error = FlowExecutionError::new("Cancelled", "flow was cancelled");
                return self
                    .finish_failed(device_run_id, &mut context, &mut reservation, error)
                    .await;
            }
            let node = plan.nodes.get(&node_id).ok_or_else(|| {
                FlowExecutionError::new(
                    "CompiledPlanCorrupt",
                    format!("taken path references missing node {node_id}"),
                )
            })?;
            let special_launch = first_launch_pending && node.kind == ActionKind::LaunchApp;
            let attempt = initialized_attempts
                .iter()
                .find(|attempt| attempt.node_id == node_id)
                .ok_or_else(|| {
                    FlowExecutionError::new(
                        "CompiledPlanCorrupt",
                        format!("initialized attempt is missing for node {node_id}"),
                    )
                })?;
            let chosen_port = match self
                .execute_node(
                    device_run_id,
                    node,
                    attempt,
                    &plan,
                    &preflight,
                    &mut context,
                    &mut reservation,
                    special_launch,
                )
                .await
            {
                Ok(port) => {
                    if special_launch {
                        first_launch_pending = false;
                    }
                    port
                }
                Err(error) => {
                    return self
                        .finish_failed(device_run_id, &mut context, &mut reservation, error)
                        .await;
                }
            };
            current = plan.successor_on_path(node_id, chosen_port.as_deref());
        }

        if first_launch_pending {
            let error = FlowExecutionError::new(
                "CompiledPlanCorrupt",
                "UI plan did not execute its initial Launch App node",
            );
            return self
                .finish_failed(device_run_id, &mut context, &mut reservation, error)
                .await;
        }

        let proof = context
            .close(&self.deps.control)
            .await
            .map_err(FlowExecutionError::device)?;
        drop(reservation.take());
        self.deps
            .database
            .mark_device_terminal(
                device_run_id,
                &[FlowDeviceRunState::Running],
                FlowDeviceRunState::Succeeded,
                None,
                flow_release_proof(proof),
            )
            .map_err(FlowExecutionError::other)?;
        Ok(())
    }

    pub(crate) async fn resume_device(
        &self,
        device_run_id: Uuid,
        plan: CompiledFlowPlanV2,
        first_attempt_id: Uuid,
    ) -> Result<(), FlowExecutionError> {
        self.validate_run_identity(device_run_id, &plan)?;
        let initial = self
            .deps
            .database
            .get_flow_attempt_execution_context(first_attempt_id)
            .map_err(FlowExecutionError::other)?
            .ok_or_else(|| {
                FlowExecutionError::new("RunIdentityMismatch", "retry attempt is absent")
            })?;
        if initial.device.id != device_run_id
            || initial.run.id != self.deps.run_id
            || initial.attempt.state != FlowAttemptState::Queued
            || initial.plan != plan
        {
            return Err(FlowExecutionError::new(
                "RunIdentityMismatch",
                "retry attempt does not match its immutable run plan",
            ));
        }
        if initial
            .device_attempts
            .iter()
            .any(|attempt| attempt.state == FlowAttemptState::Uncertain)
        {
            return Err(FlowExecutionError::new(
                "RetryNotAllowed",
                "an uncertain device attempt blocks successor dispatch",
            ));
        }
        // Rebuild the taken path from Start up to the retry node, replaying each
        // already-decided branch from its recorded chosen_port. Every node on
        // that prefix must be durably succeeded, and we capture the app the
        // prefix last launched so the live session can be re-attached.
        let retry_node_id = initial.attempt.node_id;
        let mut prior_launch_bundle: Option<String> = None;
        let mut cursor = plan.entry_node();
        let mut prefix_steps = 0usize;
        loop {
            let Some(node_id) = cursor else {
                return Err(FlowExecutionError::new(
                    "RunIdentityMismatch",
                    "retry node is not on the taken path",
                ));
            };
            if node_id == retry_node_id {
                break;
            }
            prefix_steps += 1;
            if prefix_steps > plan.nodes.len() + 1 {
                return Err(FlowExecutionError::new(
                    "CompiledPlanCorrupt",
                    "taken path exceeds the compiled node count",
                ));
            }
            let latest = initial
                .device_attempts
                .iter()
                .filter(|attempt| attempt.node_id == node_id)
                .max_by_key(|attempt| attempt.attempt_no)
                .ok_or_else(|| {
                    FlowExecutionError::new(
                        "RetryNotAllowed",
                        "retry predecessor has no durable attempt",
                    )
                })?;
            if latest.state != FlowAttemptState::Succeeded {
                return Err(FlowExecutionError::new(
                    "RetryNotAllowed",
                    "retry predecessor is not durably succeeded",
                ));
            }
            if let Some(CompiledFlowNode {
                config: CompiledActionConfig::LaunchApp { bundle_id },
                ..
            }) = plan.nodes.get(&node_id)
            {
                prior_launch_bundle = Some(bundle_id.clone());
            }
            cursor = plan.successor_on_path(node_id, latest.chosen_port.as_deref());
        }

        let mut context = FlowDeviceContext::no_device_resources(&self.deps.udid);
        self.deps
            .database
            .transition_flow_device_run(
                device_run_id,
                FlowDeviceRunState::Queued,
                FlowDeviceRunState::Preflight,
                None,
            )
            .map_err(FlowExecutionError::other)?;
        if let Err(error) = validate_compiled_plan(&plan) {
            return self
                .finish_failed(device_run_id, &mut context, &mut None, error)
                .await;
        }

        if plan.context_plan.requires_exclusive {
            let acquire = self
                .deps
                .control
                .acquire_exclusive(&self.deps.udid, DeviceWorkOwner::Script);
            let cancelled = self.deps.cancellation.cancelled();
            tokio::pin!(acquire);
            tokio::pin!(cancelled);
            let acquired = tokio::select! {
                biased;
                _ = &mut cancelled => Err(FlowExecutionError::new(
                    "Cancelled",
                    "flow retry was cancelled while waiting for device ownership",
                )),
                result = &mut acquire => result.map_err(FlowExecutionError::device),
            };
            match acquired {
                Ok(exclusive) => context = FlowDeviceContext::Exclusive(exclusive),
                Err(error) => {
                    return self
                        .finish_failed(device_run_id, &mut context, &mut None, error)
                        .await;
                }
            }
        }

        let preflight_result = match target_bundle_id(&plan) {
            Some(target_bundle_id) => {
                let snapshot = match self
                    .deps
                    .control
                    .inspect_flow_device(
                        context.exclusive().map_err(FlowExecutionError::device)?,
                        target_bundle_id,
                    )
                    .await
                {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        return self
                            .finish_failed(
                                device_run_id,
                                &mut context,
                                &mut None,
                                FlowExecutionError::device(error),
                            )
                            .await;
                    }
                };
                self.build_preflight(&plan, target_bundle_id, snapshot)
            }
            None => self.build_target_free_preflight(&plan),
        };
        let preflight = match preflight_result {
            Ok(preflight) => preflight,
            Err(error) => {
                return self
                    .finish_failed(device_run_id, &mut context, &mut None, error)
                    .await;
            }
        };
        self.deps
            .database
            .transition_flow_device_run(
                device_run_id,
                FlowDeviceRunState::Preflight,
                FlowDeviceRunState::Running,
                Some(preflight.persisted_snapshot.clone()),
            )
            .map_err(FlowExecutionError::other)?;

        let mut reservation = None;
        if plan.context_plan.requires_ui_session {
            match context.reserve_capacity(&self.deps.control).await {
                Ok(capacity) => reservation = Some(capacity),
                Err(failure) => {
                    return self
                        .finish_failed(
                            device_run_id,
                            &mut context,
                            &mut reservation,
                            FlowExecutionError::device(failure.error),
                        )
                        .await;
                }
            }
        }

        let mut first_launch_pending = plan.context_plan.requires_ui_session;
        if let Some(bundle_id) = prior_launch_bundle.as_deref() {
            let observed_bundle = match context.active_app_bundle(&self.deps.control).await {
                Ok(bundle_id) => bundle_id,
                Err(error) => {
                    return self
                        .finish_failed(
                            device_run_id,
                            &mut context,
                            &mut reservation,
                            FlowExecutionError::device(error),
                        )
                        .await;
                }
            };
            if observed_bundle != bundle_id {
                return self
                    .finish_failed(
                        device_run_id,
                        &mut context,
                        &mut reservation,
                        FlowExecutionError::new(
                            "ActiveAppMismatch",
                            format!("UI resume expected {bundle_id}, observed {observed_bundle}"),
                        ),
                    )
                    .await;
            }
            let kind = if plan.context_plan.requires_fresh_text_session {
                InteractionSessionKind::FreshText
            } else {
                InteractionSessionKind::Ordinary
            };
            if let Err(error) = context
                .upgrade_existing_session(&self.deps.control, bundle_id, kind)
                .await
            {
                return self
                    .finish_failed(
                        device_run_id,
                        &mut context,
                        &mut reservation,
                        FlowExecutionError::device(error),
                    )
                    .await;
            }
            if let Err(error) = self.verify_required_live_capabilities(&plan, &context) {
                return self
                    .finish_failed(device_run_id, &mut context, &mut reservation, error)
                    .await;
            }
            if plan.context_plan.requires_stream {
                let capacity = reservation.take().ok_or_else(|| {
                    FlowExecutionError::new(
                        "ContextOrder",
                        "retry stream capacity was not reserved before session attach",
                    )
                })?;
                if let Err(mut failure) = context.upgrade_stream(&self.deps.control, capacity).await
                {
                    let mut error =
                        FlowExecutionError::new("DeviceControl", failure.error.to_string());
                    if let Some(proof) = failure
                        .release_failed_stream(&self.deps.control)
                        .await
                        .map_err(FlowExecutionError::device)?
                    {
                        error = error.released(proof);
                    }
                    return self
                        .finish_failed(device_run_id, &mut context, &mut reservation, error)
                        .await;
                }
            }
            first_launch_pending = false;
        }

        // Execute the retry node and continue down the taken path, deciding
        // branches as they run. Nodes past the retry point never ran, but any
        // already-succeeded node is skipped defensively (idempotent resume).
        let mut current = Some(retry_node_id);
        let mut steps = 0usize;
        while let Some(node_id) = current {
            steps += 1;
            if steps > plan.nodes.len() + 1 {
                let error = FlowExecutionError::new(
                    "CompiledPlanCorrupt",
                    "taken path exceeds the compiled node count",
                );
                return self
                    .finish_failed(device_run_id, &mut context, &mut reservation, error)
                    .await;
            }
            if self.deps.cancellation.is_cancelled() {
                let error = FlowExecutionError::new("Cancelled", "flow retry was cancelled");
                return self
                    .finish_failed(device_run_id, &mut context, &mut reservation, error)
                    .await;
            }
            let node = plan.nodes.get(&node_id).ok_or_else(|| {
                FlowExecutionError::new(
                    "CompiledPlanCorrupt",
                    format!("taken path references missing node {node_id}"),
                )
            })?;
            let attempt = initial
                .device_attempts
                .iter()
                .filter(|attempt| attempt.node_id == node_id)
                .max_by_key(|attempt| attempt.attempt_no)
                .ok_or_else(|| {
                    FlowExecutionError::new(
                        "RunIdentityMismatch",
                        format!("retry successor has no attempt for node {node_id}"),
                    )
                })?;
            if attempt.state == FlowAttemptState::Succeeded {
                current = plan.successor_on_path(node_id, attempt.chosen_port.as_deref());
                continue;
            }
            let special_launch = first_launch_pending && node.kind == ActionKind::LaunchApp;
            let chosen_port = match self
                .execute_node(
                    device_run_id,
                    node,
                    attempt,
                    &plan,
                    &preflight,
                    &mut context,
                    &mut reservation,
                    special_launch,
                )
                .await
            {
                Ok(port) => {
                    if special_launch {
                        first_launch_pending = false;
                    }
                    port
                }
                Err(error) => {
                    return self
                        .finish_failed(device_run_id, &mut context, &mut reservation, error)
                        .await;
                }
            };
            current = plan.successor_on_path(node_id, chosen_port.as_deref());
        }

        if first_launch_pending {
            let error = FlowExecutionError::new(
                "CompiledPlanCorrupt",
                "UI retry did not establish its target session",
            );
            return self
                .finish_failed(device_run_id, &mut context, &mut reservation, error)
                .await;
        }
        let proof = context
            .close(&self.deps.control)
            .await
            .map_err(FlowExecutionError::device)?;
        drop(reservation.take());
        self.deps
            .database
            .mark_device_terminal(
                device_run_id,
                &[FlowDeviceRunState::Running],
                FlowDeviceRunState::Succeeded,
                None,
                flow_release_proof(proof),
            )
            .map_err(FlowExecutionError::other)?;
        Ok(())
    }

    fn validate_run_identity(
        &self,
        device_run_id: Uuid,
        plan: &CompiledFlowPlanV2,
    ) -> Result<(), FlowExecutionError> {
        let detail = self
            .deps
            .database
            .get_flow_run(self.deps.run_id)
            .map_err(FlowExecutionError::other)?
            .ok_or_else(|| FlowExecutionError::new("RunIdentityMismatch", "Flow run is absent"))?;
        let device = detail
            .device_runs
            .iter()
            .find(|device| device.id == device_run_id)
            .ok_or_else(|| {
                FlowExecutionError::new(
                    "RunIdentityMismatch",
                    "device run does not belong to the executor run",
                )
            })?;
        let supplied_plan_sha256 = compiled_plan_sha256(plan).map_err(FlowExecutionError::other)?;
        if device.run_id != self.deps.run_id
            || device.udid != self.deps.udid
            || detail.run.flow_id != plan.flow_id
            || detail.run.flow_revision != plan.revision
            || detail.run.plan_sha256 != supplied_plan_sha256
        {
            return Err(FlowExecutionError::new(
                "RunIdentityMismatch",
                "executor run, device, UDID, revision, or pinned plan hash differs",
            ));
        }
        Ok(())
    }

    fn build_preflight(
        &self,
        plan: &CompiledFlowPlanV2,
        target_bundle_id: &str,
        snapshot: DeviceCapabilitySnapshot,
    ) -> Result<FlowDevicePreflight, FlowExecutionError> {
        if snapshot.target_app.bundle_id != target_bundle_id {
            return Err(FlowExecutionError::new(
                "TargetIdentityMismatch",
                "capability inspection returned a different target bundle",
            ));
        }
        let bridge_only = !plan.context_plan.requires_ui_session;
        let status = self.deps.control.cached_agent_status(&self.deps.udid);
        let agent_status = (!bridge_only && status.state != AgentState::Unknown).then_some(status);
        if !bridge_only {
            let agent = agent_status.as_ref().ok_or_else(|| {
                FlowExecutionError::new("AgentNotReady", "Agent status is unavailable")
            })?;
            if agent.state != AgentState::Ready
                || !agent.auth_ready
                || !snapshot.protected_auth_ready
                || snapshot.installed_agent.bundle_id.trim().is_empty()
                || snapshot.installed_agent.executable_name.trim().is_empty()
                || snapshot.agent_version.trim().is_empty()
                || agent.artifact_id.trim().is_empty()
                || agent.artifact_version.trim().is_empty()
                || agent.bundle_id.trim().is_empty()
            {
                return Err(FlowExecutionError::new(
                    "AgentNotReady",
                    "Agent identity or protected health is incomplete",
                ));
            }
        }

        let profile_id = if bridge_only {
            String::new()
        } else {
            qualified_geometry_profile_id(&snapshot)
                .map_err(|message| FlowExecutionError::new("GeometryUnavailable", message))?
        };
        let capability_ids = static_flow_capability_ids(
            agent_status.as_ref(),
            &self.deps.control.driver_contract_ids(),
            !bridge_only && snapshot.protected_auth_ready,
        );
        let missing = plan
            .required_capabilities
            .iter()
            .filter(|capability| {
                !matches!(
                    capability.as_str(),
                    "accessibility.visible" | "accessibility.readText"
                ) && !capability_ids.contains(*capability)
            })
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(FlowExecutionError::new(
                "CapabilityUnavailable",
                format!("missing static Flow capabilities: {}", missing.join(", ")),
            ));
        }
        Ok(FlowDevicePreflight {
            device_snapshot: Some(snapshot.clone()),
            persisted_snapshot: FlowCapabilitySnapshot {
                scope: FlowPreflightScope::TargetQualified {
                    bundle_id: target_bundle_id.to_string(),
                },
                device: Some(snapshot),
                agent_status,
                capability_ids,
            },
            profile_id,
        })
    }

    fn build_target_free_preflight(
        &self,
        plan: &CompiledFlowPlanV2,
    ) -> Result<FlowDevicePreflight, FlowExecutionError> {
        if !plan.required_capabilities.is_empty() {
            return Err(FlowExecutionError::new(
                "CapabilityUnavailable",
                "target-free Flow cannot require device capabilities",
            ));
        }
        Ok(FlowDevicePreflight {
            device_snapshot: None,
            persisted_snapshot: FlowCapabilitySnapshot {
                scope: FlowPreflightScope::TargetFree,
                device: None,
                agent_status: None,
                capability_ids: BTreeSet::new(),
            },
            profile_id: String::new(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_node(
        &self,
        device_run_id: Uuid,
        node: &CompiledFlowNode,
        attempt: &super::FlowNodeAttemptRecord,
        plan: &CompiledFlowPlanV2,
        preflight: &FlowDevicePreflight,
        context: &mut FlowDeviceContext,
        reservation: &mut Option<UiCapacityReservation>,
        special_launch: bool,
    ) -> Result<Option<String>, FlowExecutionError> {
        self.verify_live_node_capabilities(node, context)?;
        let baseline_deadline = tokio::time::Instant::now() + EVIDENCE_TIMEOUT;
        let baseline = self
            .capture_node_baseline(node, context, baseline_deadline)
            .await?;
        if attempt.device_run_id != device_run_id
            || attempt.node_id != node.id
            || attempt.action_kind != node.kind
            || attempt.side_effect_class != contracts(node.kind).1
            || attempt.state != FlowAttemptState::Queued
        {
            return Err(FlowExecutionError::new(
                "RunIdentityMismatch",
                "queued Flow attempt does not match its immutable device node",
            ));
        }
        let attempt_id = attempt.id;
        self.deps
            .database
            .transition_attempt(
                attempt_id,
                FlowAttemptState::Queued,
                FlowAttemptState::IntentCommitted,
                AttemptTransitionPatch {
                    canonical_input: Some(
                        serde_json::to_value(&node.config).map_err(FlowExecutionError::other)?,
                    ),
                    evidence_baseline: Some(
                        serde_json::to_value(&baseline).map_err(FlowExecutionError::other)?,
                    ),
                    ..Default::default()
                },
            )
            .map_err(FlowExecutionError::other)?;

        if !compiled_config_matches(node) {
            let error = FlowExecutionError::new(
                "CompiledPlanCorrupt",
                format!("compiled action config does not match node {}", node.id),
            )
            .attributed(node.id, attempt_id);
            self.fail_before_dispatch(attempt_id, &error)?;
            return Err(error);
        }
        if let Err(error) = self.validate_geometry(node, preflight, context) {
            let error = error.attributed(node.id, attempt_id);
            self.fail_before_dispatch(attempt_id, &error)?;
            return Err(error);
        }

        self.deps
            .database
            .transition_attempt(
                attempt_id,
                FlowAttemptState::IntentCommitted,
                FlowAttemptState::EffectDispatched,
                AttemptTransitionPatch::default(),
            )
            .map_err(FlowExecutionError::other)?;

        let output = match self
            .dispatch_action(node, plan, preflight, context, reservation, special_launch)
            .await
        {
            Ok(output) => output,
            Err(mut failure) => {
                let error = std::mem::replace(
                    &mut failure.error,
                    FlowExecutionError::new("FlowExecution", "dispatch failure was consumed"),
                )
                .attributed(node.id, attempt_id);
                if failure.request_reached_device == Some(false) {
                    self.deps
                        .database
                        .transition_attempt(
                            attempt_id,
                            FlowAttemptState::EffectDispatched,
                            FlowAttemptState::FailedBeforeDispatch,
                            AttemptTransitionPatch {
                                evidence_result: Some(serde_json::json!({
                                    "kind": "transportNonDelivery",
                                    "requestReachedDevice": false,
                                })),
                                error: Some(error.record(&self.deps.udid)),
                                ..Default::default()
                            },
                        )
                        .map_err(FlowExecutionError::other)?;
                    return Err(error);
                }
                if failure.deterministic_read_failure {
                    self.deps
                        .database
                        .transition_attempt(
                            attempt_id,
                            FlowAttemptState::EffectDispatched,
                            FlowAttemptState::Verifying,
                            AttemptTransitionPatch::default(),
                        )
                        .map_err(FlowExecutionError::other)?;
                    self.deps
                        .database
                        .transition_attempt(
                            attempt_id,
                            FlowAttemptState::Verifying,
                            FlowAttemptState::FailedVerified,
                            AttemptTransitionPatch {
                                error: Some(error.record(&self.deps.udid)),
                                ..Default::default()
                            },
                        )
                        .map_err(FlowExecutionError::other)?;
                    return Err(error);
                }
                if special_launch {
                    return self
                        .reconcile_initial_launch_failure(
                            attempt_id,
                            node,
                            context,
                            &mut failure,
                            error,
                        )
                        .await
                        .map(|()| None);
                }
                let next = if error.code == "Cancelled"
                    && contracts(node.kind).1 == SideEffectClass::None
                {
                    FlowAttemptState::Cancelled
                } else {
                    FlowAttemptState::Uncertain
                };
                self.deps
                    .database
                    .transition_attempt(
                        attempt_id,
                        FlowAttemptState::EffectDispatched,
                        next,
                        AttemptTransitionPatch {
                            error: Some(error.record(&self.deps.udid)),
                            ..Default::default()
                        },
                    )
                    .map_err(FlowExecutionError::other)?;
                return Err(error);
            }
        };
        let deadline = tokio::time::Instant::now() + EVIDENCE_TIMEOUT;

        // A branch predicate carries its chosen port; persist it alongside the
        // transition into verification so recovery can rebuild the taken path.
        let chosen_port = match &output {
            ActionOutput::Branch { port } => Some(port.clone()),
            _ => None,
        };
        self.deps
            .database
            .transition_attempt(
                attempt_id,
                FlowAttemptState::EffectDispatched,
                FlowAttemptState::Verifying,
                AttemptTransitionPatch {
                    chosen_port: chosen_port.clone(),
                    ..Default::default()
                },
            )
            .map_err(FlowExecutionError::other)?;

        self.verify_node(
            device_run_id,
            attempt_id,
            node,
            context,
            baseline,
            output,
            deadline,
        )
        .await
        .map(|()| chosen_port)
    }

    async fn reconcile_initial_launch_failure(
        &self,
        attempt_id: Uuid,
        node: &CompiledFlowNode,
        context: &FlowDeviceContext,
        failure: &mut ActionDispatchFailure,
        mut error: FlowExecutionError,
    ) -> Result<(), FlowExecutionError> {
        let CompiledActionConfig::LaunchApp { bundle_id } = &node.config else {
            return Err(error);
        };
        self.deps
            .database
            .record_nonterminal_attempt_error(
                attempt_id,
                FlowAttemptState::EffectDispatched,
                error.record(&self.deps.udid),
            )
            .map_err(FlowExecutionError::other)?;

        let observed = match failure
            .stream_upgrade
            .as_ref()
            .map(|upgrade| upgrade.reconciliation_session(&self.deps.control))
            .transpose()
        {
            Ok(Some(Some(session))) => session
                .active_app_bundle()
                .await
                .map_err(|read_error| read_error.to_string()),
            Ok(Some(None)) | Ok(None) => context
                .active_app_bundle(&self.deps.control)
                .await
                .map_err(|read_error| read_error.to_string()),
            Err(read_error) => Err(read_error.to_string()),
        };

        if let Some(upgrade) = failure.stream_upgrade.as_mut() {
            match upgrade.release_failed_stream(&self.deps.control).await {
                Ok(Some(proof)) => error = error.released(proof),
                Ok(None) => {}
                Err(cleanup_error) => {
                    let cleanup_error =
                        FlowExecutionError::device(cleanup_error).attributed(node.id, attempt_id);
                    self.deps
                        .database
                        .transition_attempt(
                            attempt_id,
                            FlowAttemptState::EffectDispatched,
                            FlowAttemptState::Uncertain,
                            AttemptTransitionPatch {
                                error: Some(cleanup_error.record(&self.deps.udid)),
                                ..Default::default()
                            },
                        )
                        .map_err(FlowExecutionError::other)?;
                    return Err(cleanup_error);
                }
            }
        }

        let observed = match observed {
            Ok(observed) => observed,
            Err(read_error) => {
                error.message = format!(
                    "{}; active-app reconciliation failed: {read_error}",
                    error.message
                );
                self.deps
                    .database
                    .transition_attempt(
                        attempt_id,
                        FlowAttemptState::EffectDispatched,
                        FlowAttemptState::Uncertain,
                        AttemptTransitionPatch {
                            error: Some(error.record(&self.deps.udid)),
                            ..Default::default()
                        },
                    )
                    .map_err(FlowExecutionError::other)?;
                return Err(error);
            }
        };
        let matched = observed == *bundle_id;
        let observed_sha256 = format!("{:x}", Sha256::digest(observed.as_bytes()));
        let evidence = serde_json::json!({
            "kind": "activeAppEquals",
            "matched": matched,
            "observedSha256": observed_sha256,
            "measurement": {"bundleId": observed},
        });
        self.deps
            .database
            .transition_attempt(
                attempt_id,
                FlowAttemptState::EffectDispatched,
                FlowAttemptState::Verifying,
                AttemptTransitionPatch::default(),
            )
            .map_err(FlowExecutionError::other)?;
        if matched {
            self.deps
                .database
                .transition_attempt(
                    attempt_id,
                    FlowAttemptState::Verifying,
                    FlowAttemptState::Succeeded,
                    AttemptTransitionPatch {
                        evidence_result: Some(evidence),
                        ..Default::default()
                    },
                )
                .map_err(FlowExecutionError::other)?;
            return Err(error.without_attribution());
        }

        self.deps
            .database
            .transition_attempt(
                attempt_id,
                FlowAttemptState::Verifying,
                FlowAttemptState::FailedVerified,
                AttemptTransitionPatch {
                    evidence_result: Some(evidence),
                    error: Some(error.record(&self.deps.udid)),
                    ..Default::default()
                },
            )
            .map_err(FlowExecutionError::other)?;
        Err(error.retry_safe(serde_json::json!({
            "kind": "activeAppEquals",
            "matched": false,
            "observedSha256": observed_sha256,
            "measurement": {
                "expectedBundleId": bundle_id,
                "observedBundleId": observed,
            },
        })))
    }

    async fn capture_node_baseline(
        &self,
        node: &CompiledFlowNode,
        context: &FlowDeviceContext,
        deadline: tokio::time::Instant,
    ) -> Result<EvidenceBaseline, FlowExecutionError> {
        let Some(specification) = node.postcondition.as_ref() else {
            return Ok(EvidenceBaseline::None);
        };
        if let EvidenceSpec::ProcessAbsent { bundle_id } = specification {
            check_evidence_request_boundary(deadline, &self.deps.cancellation)?;
            let state = context.inspect_process(&self.deps.control, bundle_id).await;
            check_evidence_request_boundary(deadline, &self.deps.cancellation)?;
            let state = state.map_err(FlowExecutionError::device)?;
            return capture_process_baseline(specification, &state)
                .map_err(FlowExecutionError::evidence);
        }
        capture_baseline(
            self.deps.frames.as_ref(),
            &self.deps.udid,
            context.generation(),
            specification,
            deadline,
            &self.deps.cancellation,
        )
        .await
        .map_err(FlowExecutionError::evidence)
    }

    fn fail_before_dispatch(
        &self,
        attempt_id: Uuid,
        error: &FlowExecutionError,
    ) -> Result<(), FlowExecutionError> {
        self.deps
            .database
            .transition_attempt(
                attempt_id,
                FlowAttemptState::IntentCommitted,
                FlowAttemptState::FailedBeforeDispatch,
                AttemptTransitionPatch {
                    error: Some(error.record(&self.deps.udid)),
                    ..Default::default()
                },
            )
            .map_err(FlowExecutionError::other)?;
        Ok(())
    }

    fn validate_geometry(
        &self,
        node: &CompiledFlowNode,
        preflight: &FlowDevicePreflight,
        context: &FlowDeviceContext,
    ) -> Result<(), FlowExecutionError> {
        let targets = match &node.config {
            CompiledActionConfig::Tap {
                target: CompiledTapTarget::Point { target },
            } => vec![target],
            CompiledActionConfig::Swipe { from, to, .. } => vec![from, to],
            _ => return Ok(()),
        };
        let generation = context.generation();
        let frame = self
            .deps
            .frames
            .latest_in_generation(&self.deps.udid, generation)
            .ok_or_else(|| {
                FlowExecutionError::new(
                    "StaleGeneration",
                    "no frame exists in the owned stream generation",
                )
            })?;
        let image = image::load_from_memory(&frame.bytes).map_err(|error| {
            FlowExecutionError::new("GeometryMismatch", format!("decode live frame: {error}"))
        })?;
        if self
            .deps
            .frames
            .latest_in_generation(&self.deps.udid, generation)
            .is_none()
        {
            return Err(FlowExecutionError::new(
                "StaleGeneration",
                "stream generation changed while decoding the geometry frame",
            ));
        }
        let geometry = preflight
            .device_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.geometry.as_ref())
            .ok_or_else(|| {
                FlowExecutionError::new("GeometryMismatch", "runtime geometry is unavailable")
            })?;
        if image.width() != geometry.pixel_width || image.height() != geometry.pixel_height {
            return Err(FlowExecutionError::new(
                "GeometryMismatch",
                "decoded frame dimensions do not match qualified runtime geometry",
            ));
        }
        let orientation = preflight
            .device_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.geometry.as_ref())
            .map(|geometry| geometry.orientation)
            .ok_or_else(|| {
                FlowExecutionError::new("GeometryMismatch", "runtime geometry is unavailable")
            })?;
        for target in targets {
            if target.image_width != image.width()
                || target.image_height != image.height()
                || target.orientation != orientation
                || target.profile_id != preflight.profile_id
            {
                return Err(FlowExecutionError::new(
                    "GeometryMismatch",
                    "compiled coordinates do not match the runtime frame profile",
                ));
            }
        }
        Ok(())
    }

    async fn dispatch_action(
        &self,
        node: &CompiledFlowNode,
        plan: &CompiledFlowPlanV2,
        preflight: &FlowDevicePreflight,
        context: &mut FlowDeviceContext,
        reservation: &mut Option<UiCapacityReservation>,
        special_launch: bool,
    ) -> Result<ActionOutput, ActionDispatchFailure> {
        match (&node.kind, &node.config) {
            (ActionKind::Start | ActionKind::End, CompiledActionConfig::Empty) => {
                Ok(ActionOutput::None)
            }
            (ActionKind::LaunchApp, CompiledActionConfig::LaunchApp { bundle_id }) => {
                if special_launch {
                    let kind = if plan.context_plan.requires_fresh_text_session {
                        InteractionSessionKind::FreshText
                    } else {
                        InteractionSessionKind::Ordinary
                    };
                    if let Err(error) = context
                        .upgrade_session(&self.deps.control, bundle_id, kind)
                        .await
                    {
                        return Err(FlowExecutionError::device(error).into());
                    }
                    self.verify_required_live_capabilities(plan, context)?;
                    if plan.context_plan.requires_stream {
                        let capacity = reservation.take().ok_or_else(|| {
                            FlowExecutionError::new(
                                "ContextOrder",
                                "stream capacity was not reserved before session startup",
                            )
                        })?;
                        if let Err(failure) =
                            context.upgrade_stream(&self.deps.control, capacity).await
                        {
                            return Err(ActionDispatchFailure {
                                error: FlowExecutionError::new(
                                    "DeviceControl",
                                    failure.error.to_string(),
                                ),
                                stream_upgrade: Some(failure),
                                request_reached_device: None,
                                deterministic_read_failure: false,
                            });
                        }
                    }
                } else {
                    context
                        .foreground_app(&self.deps.control, bundle_id)
                        .await
                        .map_err(FlowExecutionError::device)?;
                }
                Ok(ActionOutput::None)
            }
            (ActionKind::TerminateApp, CompiledActionConfig::TerminateApp { bundle_id }) => {
                let proof = context
                    .terminate_app(&self.deps.control, bundle_id)
                    .await
                    .map_err(FlowExecutionError::device)?;
                Ok(ActionOutput::ProcessAbsent(proof))
            }
            (ActionKind::Wait, CompiledActionConfig::Wait { duration_ms }) => {
                cancellable_wait(*duration_ms, &self.deps.cancellation).await?;
                Ok(ActionOutput::None)
            }
            (ActionKind::Tap, CompiledActionConfig::Tap { target }) => {
                let session = context
                    .session(&self.deps.control)
                    .map_err(FlowExecutionError::device)?;
                match target {
                    CompiledTapTarget::Point { target } => {
                        if let Err(error) = self.validate_geometry(node, preflight, context) {
                            return Err(ActionDispatchFailure::non_delivery(error));
                        }
                        session
                            .tap_image(
                                target.x,
                                target.y,
                                f64::from(target.image_width),
                                f64::from(target.image_height),
                            )
                            .await
                            .map_err(FlowExecutionError::other)?;
                    }
                    CompiledTapTarget::AccessibilityId { value } => {
                        session
                            .find_and_tap(value)
                            .await
                            .map_err(FlowExecutionError::other)?;
                    }
                }
                Ok(ActionOutput::None)
            }
            (
                ActionKind::Swipe,
                CompiledActionConfig::Swipe {
                    from,
                    to,
                    duration_ms,
                },
            ) => {
                let session = context
                    .session(&self.deps.control)
                    .map_err(FlowExecutionError::device)?;
                if let Err(error) = self.validate_geometry(node, preflight, context) {
                    return Err(ActionDispatchFailure::non_delivery(error));
                }
                session
                    .swipe_image(
                        TapPoint {
                            x: from.x,
                            y: from.y,
                        },
                        TapPoint { x: to.x, y: to.y },
                        f64::from(from.image_width),
                        f64::from(from.image_height),
                        *duration_ms,
                    )
                    .await
                    .map_err(FlowExecutionError::other)?;
                Ok(ActionOutput::None)
            }
            (ActionKind::TypeText, CompiledActionConfig::TypeText { text, .. }) => {
                context
                    .session(&self.deps.control)
                    .map_err(FlowExecutionError::device)?
                    .type_text(text)
                    .await
                    .map_err(FlowExecutionError::other)?;
                Ok(ActionOutput::None)
            }
            (ActionKind::Screenshot, CompiledActionConfig::Screenshot { label, format }) => {
                let frame = self
                    .deps
                    .frames
                    .latest_in_generation(&self.deps.udid, context.generation())
                    .ok_or_else(|| {
                        FlowExecutionError::new(
                            "StaleGeneration",
                            "screenshot stream generation is unavailable",
                        )
                    })?;
                Ok(ActionOutput::Screenshot {
                    label: label.clone(),
                    format: format.clone(),
                    bytes: frame.bytes,
                })
            }
            (ActionKind::Home, CompiledActionConfig::Empty) => {
                context
                    .session(&self.deps.control)
                    .map_err(FlowExecutionError::device)?
                    .home()
                    .await
                    .map_err(FlowExecutionError::other)?;
                Ok(ActionOutput::None)
            }
            (
                ActionKind::AssertVisible,
                CompiledActionConfig::AssertVisible { accessibility_id },
            ) => {
                let session = context
                    .session(&self.deps.control)
                    .map_err(FlowExecutionError::device)?;
                if !session.supports_accessibility_readback() {
                    return Err(FlowExecutionError::new(
                        "CapabilityUnavailable",
                        "accessibility read-back is unavailable in the live session",
                    )
                    .into());
                }
                if let Err(error) = session.assert_visible(accessibility_id).await {
                    return Err(ActionDispatchFailure::deterministic_read(
                        FlowExecutionError::other(error),
                    ));
                }
                Ok(ActionOutput::None)
            }
            (
                ActionKind::TapVision,
                CompiledActionConfig::TapVision {
                    template_png_base64,
                    threshold,
                    region,
                },
            ) => {
                let session = context
                    .session(&self.deps.control)
                    .map_err(FlowExecutionError::device)?;
                let generation = context.generation();
                let frame = self
                    .deps
                    .frames
                    .latest_in_generation(&self.deps.udid, generation)
                    .ok_or_else(|| {
                        ActionDispatchFailure::non_delivery(FlowExecutionError::new(
                            "StaleGeneration",
                            "no frame exists in the owned stream generation",
                        ))
                    })?;
                let scene = image::load_from_memory(&frame.bytes)
                    .map_err(|error| {
                        ActionDispatchFailure::non_delivery(FlowExecutionError::new(
                            "VisionDecode",
                            format!("decode live frame: {error}"),
                        ))
                    })?
                    .to_rgb8();
                // Re-check the generation didn't flip while decoding — same guard
                // validate_geometry uses for the compiled-coordinate path.
                if self
                    .deps
                    .frames
                    .latest_in_generation(&self.deps.udid, generation)
                    .is_none()
                {
                    return Err(ActionDispatchFailure::non_delivery(
                        FlowExecutionError::new(
                            "StaleGeneration",
                            "stream generation changed while decoding the vision frame",
                        ),
                    ));
                }
                let template = decode_vision_template(template_png_base64).map_err(|message| {
                    ActionDispatchFailure::non_delivery(FlowExecutionError::new(
                        "VisionTemplate",
                        message,
                    ))
                })?;
                let (frame_w, frame_h) = (scene.width(), scene.height());
                // Optional ROI (screen fractions) → pixel box; match center maps
                // back to full-frame pixel space for tap_image.
                let (offset_x, offset_y, haystack) = match region {
                    Some(roi) => {
                        let to_px = |value: f64, span: u32| {
                            (value * f64::from(span))
                                .round()
                                .clamp(0.0, f64::from(span)) as u32
                        };
                        let x0 = to_px(roi.x0, frame_w);
                        let y0 = to_px(roi.y0, frame_h);
                        let x1 = to_px(roi.x1, frame_w);
                        let y1 = to_px(roi.y1, frame_h);
                        let (width, height) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
                        if width == 0 || height == 0 {
                            return Err(ActionDispatchFailure::deterministic_read(
                                FlowExecutionError::new(
                                    "VisionRegion",
                                    "search region is empty at the live frame size",
                                ),
                            ));
                        }
                        let cropped =
                            image::imageops::crop_imm(&scene, x0, y0, width, height).to_image();
                        (f64::from(x0), f64::from(y0), cropped)
                    }
                    None => (0.0, 0.0, scene),
                };
                let haystack_gray = crate::screen_match::to_gray(&haystack);
                let needle_gray = crate::screen_match::to_gray(&template);
                match crate::screen_match::find_template(&haystack_gray, &needle_gray) {
                    Some(matched) if matched.score >= *threshold => {
                        session
                            .tap_image(
                                offset_x + matched.cx,
                                offset_y + matched.cy,
                                f64::from(frame_w),
                                f64::from(frame_h),
                            )
                            .await
                            .map_err(FlowExecutionError::other)?;
                        Ok(ActionOutput::None)
                    }
                    other => {
                        let best = other.map_or(0.0, |matched| matched.score);
                        Err(ActionDispatchFailure::deterministic_read(
                            FlowExecutionError::new(
                                "VisionNotFound",
                                format!(
                                    "template not found (best score {best:.3} < threshold {:.3})",
                                    *threshold
                                ),
                            ),
                        ))
                    }
                }
            }
            (
                ActionKind::IfVision,
                CompiledActionConfig::IfVision {
                    template_png_base64,
                    threshold,
                    region,
                },
            ) => {
                // Read-only branch predicate: find the template on the current
                // frame and route by whether it clears the threshold. Unlike
                // TapVision, "not found" is not a failure — it is the notMatched
                // branch. Only an unreadable screen is a (retriable) failure.
                let generation = context.generation();
                let frame = self
                    .deps
                    .frames
                    .latest_in_generation(&self.deps.udid, generation)
                    .ok_or_else(|| {
                        ActionDispatchFailure::non_delivery(FlowExecutionError::new(
                            "StaleGeneration",
                            "no frame exists in the owned stream generation",
                        ))
                    })?;
                let scene = image::load_from_memory(&frame.bytes)
                    .map_err(|error| {
                        ActionDispatchFailure::non_delivery(FlowExecutionError::new(
                            "VisionDecode",
                            format!("decode live frame: {error}"),
                        ))
                    })?
                    .to_rgb8();
                if self
                    .deps
                    .frames
                    .latest_in_generation(&self.deps.udid, generation)
                    .is_none()
                {
                    return Err(ActionDispatchFailure::non_delivery(
                        FlowExecutionError::new(
                            "StaleGeneration",
                            "stream generation changed while decoding the vision frame",
                        ),
                    ));
                }
                let template = decode_vision_template(template_png_base64).map_err(|message| {
                    ActionDispatchFailure::non_delivery(FlowExecutionError::new(
                        "VisionTemplate",
                        message,
                    ))
                })?;
                let (frame_w, frame_h) = (scene.width(), scene.height());
                let haystack = match region {
                    Some(roi) => {
                        let to_px = |value: f64, span: u32| {
                            (value * f64::from(span))
                                .round()
                                .clamp(0.0, f64::from(span)) as u32
                        };
                        let x0 = to_px(roi.x0, frame_w);
                        let y0 = to_px(roi.y0, frame_h);
                        let x1 = to_px(roi.x1, frame_w);
                        let y1 = to_px(roi.y1, frame_h);
                        let (width, height) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
                        if width == 0 || height == 0 {
                            return Err(ActionDispatchFailure::deterministic_read(
                                FlowExecutionError::new(
                                    "VisionRegion",
                                    "search region is empty at the live frame size",
                                ),
                            ));
                        }
                        image::imageops::crop_imm(&scene, x0, y0, width, height).to_image()
                    }
                    None => scene,
                };
                let haystack_gray = crate::screen_match::to_gray(&haystack);
                let needle_gray = crate::screen_match::to_gray(&template);
                let matched = crate::screen_match::find_template(&haystack_gray, &needle_gray)
                    .is_some_and(|found| found.score >= *threshold);
                let port = if matched { "matched" } else { "notMatched" };
                Ok(ActionOutput::Branch {
                    port: port.to_string(),
                })
            }
            _ => Err(FlowExecutionError::new(
                "CompiledPlanCorrupt",
                format!("compiled action config does not match node {}", node.id),
            )
            .into()),
        }
    }

    fn verify_live_node_capabilities(
        &self,
        node: &CompiledFlowNode,
        context: &FlowDeviceContext,
    ) -> Result<(), FlowExecutionError> {
        let needs_text = node.kind == ActionKind::TypeText;
        let needs_readback = matches!(node.kind, ActionKind::TypeText | ActionKind::AssertVisible);
        if !needs_text && !needs_readback {
            return Ok(());
        }
        let session = context
            .session(&self.deps.control)
            .map_err(FlowExecutionError::device)?;
        if needs_text && !session.supports_text_input() {
            return Err(FlowExecutionError::new(
                "CapabilityUnavailable",
                "text input is unavailable in the live session",
            ));
        }
        if needs_readback && !session.supports_accessibility_readback() {
            return Err(FlowExecutionError::new(
                "CapabilityUnavailable",
                "accessibility read-back is unavailable in the live session",
            ));
        }
        Ok(())
    }

    fn verify_required_live_capabilities(
        &self,
        plan: &CompiledFlowPlanV2,
        context: &FlowDeviceContext,
    ) -> Result<(), FlowExecutionError> {
        let needs_text = plan.required_capabilities.contains("ui.text");
        let needs_readback = plan.required_capabilities.contains("accessibility.visible")
            || plan
                .required_capabilities
                .contains("accessibility.readText");
        if !needs_text && !needs_readback {
            return Ok(());
        }
        let session = context
            .session(&self.deps.control)
            .map_err(FlowExecutionError::device)?;
        if needs_text && !session.supports_text_input() {
            return Err(FlowExecutionError::new(
                "CapabilityUnavailable",
                "text input is unavailable after session bootstrap",
            ));
        }
        if needs_readback && !session.supports_accessibility_readback() {
            return Err(FlowExecutionError::new(
                "CapabilityUnavailable",
                "accessibility read-back is unavailable after session bootstrap",
            ));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn verify_node(
        &self,
        device_run_id: Uuid,
        attempt_id: Uuid,
        node: &CompiledFlowNode,
        context: &FlowDeviceContext,
        baseline: EvidenceBaseline,
        output: ActionOutput,
        deadline: tokio::time::Instant,
    ) -> Result<(), FlowExecutionError> {
        if let ActionOutput::Screenshot {
            label,
            format,
            bytes,
        } = output
        {
            return match self.publish_screenshot(
                device_run_id,
                attempt_id,
                context.generation(),
                &label,
                &format,
                &bytes,
            ) {
                Ok(()) => Ok(()),
                Err(error) => self.fail_verification(attempt_id, node, error),
            };
        }

        let Some(specification) = node.postcondition.as_ref() else {
            self.deps
                .database
                .transition_attempt(
                    attempt_id,
                    FlowAttemptState::Verifying,
                    FlowAttemptState::Succeeded,
                    AttemptTransitionPatch::default(),
                )
                .map_err(FlowExecutionError::other)?;
            return Ok(());
        };

        let result = match output {
            ActionOutput::ProcessAbsent(proof) => {
                match verify_process_absence(specification, &baseline, &proof) {
                    Ok(result) => result,
                    Err(error) => {
                        return self.fail_verification(
                            attempt_id,
                            node,
                            FlowExecutionError::evidence(error),
                        )
                    }
                }
            }
            ActionOutput::None => {
                let session = context.session(&self.deps.control).ok();
                match evaluate_postcondition(
                    self.deps.frames.as_ref(),
                    session.as_deref(),
                    &self.deps.udid,
                    context.generation(),
                    specification,
                    &baseline,
                    deadline,
                    &self.deps.cancellation,
                )
                .await
                {
                    Ok(result) => result,
                    Err(error) => {
                        return self.fail_verification(
                            attempt_id,
                            node,
                            FlowExecutionError::evidence(error),
                        )
                    }
                }
            }
            ActionOutput::Screenshot { .. } => unreachable!("handled above"),
            ActionOutput::Branch { .. } => {
                unreachable!("branch nodes carry no postcondition and succeed before this point")
            }
        };
        let result_value = serde_json::to_value(&result).map_err(FlowExecutionError::other)?;
        if result.matched {
            self.deps
                .database
                .transition_attempt(
                    attempt_id,
                    FlowAttemptState::Verifying,
                    FlowAttemptState::Succeeded,
                    AttemptTransitionPatch {
                        evidence_result: Some(result_value),
                        ..Default::default()
                    },
                )
                .map_err(FlowExecutionError::other)?;
            Ok(())
        } else {
            let error = FlowExecutionError::new("EvidenceMismatch", "evidence did not match")
                .attributed(node.id, attempt_id);
            self.deps
                .database
                .transition_attempt(
                    attempt_id,
                    FlowAttemptState::Verifying,
                    FlowAttemptState::FailedVerified,
                    AttemptTransitionPatch {
                        evidence_result: Some(result_value),
                        error: Some(error.record(&self.deps.udid)),
                        ..Default::default()
                    },
                )
                .map_err(FlowExecutionError::other)?;
            Err(error)
        }
    }

    fn fail_verification(
        &self,
        attempt_id: Uuid,
        node: &CompiledFlowNode,
        error: FlowExecutionError,
    ) -> Result<(), FlowExecutionError> {
        let error = error.attributed(node.id, attempt_id);
        let cancelled =
            error.code == "Cancelled" && contracts(node.kind).1 == SideEffectClass::None;
        self.deps
            .database
            .transition_attempt(
                attempt_id,
                FlowAttemptState::Verifying,
                if cancelled {
                    FlowAttemptState::Cancelled
                } else {
                    FlowAttemptState::Uncertain
                },
                AttemptTransitionPatch {
                    error: Some(error.record(&self.deps.udid)),
                    ..Default::default()
                },
            )
            .map_err(FlowExecutionError::other)?;
        Err(error)
    }

    fn publish_screenshot(
        &self,
        device_run_id: Uuid,
        attempt_id: Uuid,
        generation: u64,
        label: &str,
        format: &str,
        bytes: &[u8],
    ) -> Result<(), FlowExecutionError> {
        let decoded = decode_and_hash_artifact(bytes).map_err(FlowExecutionError::evidence)?;
        if decoded.format != format {
            return Err(FlowExecutionError::new(
                "EvidenceInvalid",
                "screenshot format does not match decoded stream bytes",
            ));
        }
        let prepared = self
            .deps
            .artifacts
            .prepare_image(
                self.deps.run_id,
                device_run_id,
                attempt_id,
                label,
                format,
                bytes,
            )
            .map_err(FlowExecutionError::other)?;
        if self
            .deps
            .frames
            .latest_in_generation(&self.deps.udid, generation)
            .is_none()
        {
            self.deps
                .artifacts
                .rollback_file(&prepared)
                .map_err(FlowExecutionError::other)?;
            return Err(FlowExecutionError::new(
                "StaleGeneration",
                "stream generation changed before screenshot publication",
            ));
        }
        let relative = self
            .deps
            .artifacts
            .publish_file(&prepared)
            .map_err(FlowExecutionError::other)?;
        if self
            .deps
            .frames
            .latest_in_generation(&self.deps.udid, generation)
            .is_none()
        {
            self.deps
                .artifacts
                .rollback_file(&prepared)
                .map_err(FlowExecutionError::other)?;
            return Err(FlowExecutionError::new(
                "StaleGeneration",
                "stream generation changed before screenshot database commit",
            ));
        }
        let record = FlowArtifactRecord {
            id: prepared.id,
            attempt_id,
            relative_path: relative.to_string_lossy().into_owned(),
            label: label.to_string(),
            kind: format.to_string(),
            size: prepared.size,
            sha256: prepared.sha256.clone(),
            created_at: Utc::now(),
        };
        if let Err(error) = self
            .deps
            .database
            .publish_artifact_and_succeed(attempt_id, &record)
        {
            let rollback = self.deps.artifacts.rollback_file(&prepared);
            return Err(FlowExecutionError::other(match rollback {
                Ok(()) => error.to_string(),
                Err(rollback_error) => {
                    format!("{error}; artifact rollback failed: {rollback_error}")
                }
            }));
        }
        Ok(())
    }

    async fn finish_failed(
        &self,
        device_run_id: Uuid,
        context: &mut FlowDeviceContext,
        reservation: &mut Option<UiCapacityReservation>,
        error: FlowExecutionError,
    ) -> Result<(), FlowExecutionError> {
        let proof = if let Some(proof) = error.release_proof.clone() {
            proof
        } else {
            match context.close(&self.deps.control).await {
                Ok(proof) => proof,
                Err(_) if context.level() == 4 => return Err(error),
                Err(close_error) => return Err(FlowExecutionError::device(close_error)),
            }
        };
        drop(reservation.take());
        let cancelled = error.code == "Cancelled";
        self.deps
            .database
            .mark_device_terminal(
                device_run_id,
                &[FlowDeviceRunState::Preflight, FlowDeviceRunState::Running],
                if cancelled {
                    FlowDeviceRunState::Cancelled
                } else {
                    FlowDeviceRunState::Failed
                },
                (!cancelled).then(|| error.record(&self.deps.udid)),
                flow_release_proof(proof),
            )
            .map_err(FlowExecutionError::other)?;
        if let (Some(attempt_id), Some(evidence)) = (
            error.attempt_id,
            error.retry_safe_evidence.as_deref().cloned(),
        ) {
            self.deps
                .database
                .record_retry_safe_reconciliation(attempt_id, evidence)
                .map_err(FlowExecutionError::other)?;
        }
        Err(error)
    }
}

fn static_flow_capability_ids(
    agent: Option<&AgentStatus>,
    driver_contracts: &BTreeSet<String>,
    protected_target_inspection: bool,
) -> BTreeSet<String> {
    let features = agent
        .into_iter()
        .flat_map(|value| value.features.iter().map(String::as_str))
        .collect::<BTreeSet<_>>();
    let mut ids = BTreeSet::new();
    if agent.is_some() && protected_target_inspection {
        ids.extend(["app.launch".to_string(), "ui.home".to_string()]);
    }
    if driver_contracts.contains("verifiedProcessControl") {
        ids.insert("app.terminate".to_string());
    }
    for (feature, capability) in [
        ("stream", "stream"),
        ("tap", "ui.tap"),
        ("swipe", "ui.swipe"),
        ("text", "ui.text"),
    ] {
        if features.contains(feature) {
            ids.insert(capability.to_string());
        }
    }
    ids
}

fn target_bundle_id(plan: &CompiledFlowPlanV2) -> Option<&str> {
    if let Some(bundle_id) = plan.context_plan.initial_bundle_id.as_deref() {
        return Some(bundle_id);
    }
    plan.execution_order.iter().find_map(|node_id| {
        let node = plan.nodes.get(node_id)?;
        match &node.config {
            CompiledActionConfig::LaunchApp { bundle_id }
            | CompiledActionConfig::TerminateApp { bundle_id } => Some(bundle_id.as_str()),
            _ => match node.postcondition.as_ref() {
                Some(EvidenceSpec::ActiveAppEquals { bundle_id })
                | Some(EvidenceSpec::ProcessAbsent { bundle_id }) => Some(bundle_id.as_str()),
                _ => None,
            },
        }
    })
}

fn last_launch_bundle_before(plan: &CompiledFlowPlanV2, end: usize) -> Option<&str> {
    plan.execution_order
        .get(..end)?
        .iter()
        .rev()
        .find_map(|node_id| {
            let node = plan.nodes.get(node_id)?;
            match &node.config {
                CompiledActionConfig::LaunchApp { bundle_id } => Some(bundle_id.as_str()),
                _ => None,
            }
        })
}

fn compiled_config_matches(node: &CompiledFlowNode) -> bool {
    matches!(
        (&node.kind, &node.config),
        (
            ActionKind::Start | ActionKind::End | ActionKind::Home,
            CompiledActionConfig::Empty
        ) | (
            ActionKind::LaunchApp,
            CompiledActionConfig::LaunchApp { .. }
        ) | (
            ActionKind::TerminateApp,
            CompiledActionConfig::TerminateApp { .. }
        ) | (ActionKind::Wait, CompiledActionConfig::Wait { .. })
            | (ActionKind::Tap, CompiledActionConfig::Tap { .. })
            | (ActionKind::Swipe, CompiledActionConfig::Swipe { .. })
            | (ActionKind::TypeText, CompiledActionConfig::TypeText { .. })
            | (
                ActionKind::Screenshot,
                CompiledActionConfig::Screenshot { .. }
            )
            | (
                ActionKind::AssertVisible,
                CompiledActionConfig::AssertVisible { .. }
            )
    )
}

fn validate_compiled_plan(plan: &CompiledFlowPlanV2) -> Result<(), FlowExecutionError> {
    if plan.execution_order.len() != plan.nodes.len()
        || plan.execution_order.iter().collect::<BTreeSet<_>>().len() != plan.execution_order.len()
    {
        return Err(FlowExecutionError::new(
            "CompiledPlanCorrupt",
            "compiled execution order is incomplete or contains duplicates",
        ));
    }
    for node_id in &plan.execution_order {
        let node = plan.nodes.get(node_id).ok_or_else(|| {
            FlowExecutionError::new(
                "CompiledPlanCorrupt",
                format!("execution order references missing node {node_id}"),
            )
        })?;
        if node.id != *node_id || !compiled_config_matches(node) {
            return Err(FlowExecutionError::new(
                "CompiledPlanCorrupt",
                format!("compiled action config does not match node {}", node.id),
            ));
        }
    }
    if plan.context_plan.requires_stream && !plan.context_plan.requires_ui_session {
        return Err(FlowExecutionError::new(
            "CompiledPlanCorrupt",
            "stream context requires a UI session",
        ));
    }
    if plan.context_plan.requires_fresh_text_session && !plan.context_plan.requires_ui_session {
        return Err(FlowExecutionError::new(
            "CompiledPlanCorrupt",
            "fresh text context requires a UI session",
        ));
    }
    if plan.context_plan.requires_ui_session && !plan.context_plan.requires_exclusive {
        return Err(FlowExecutionError::new(
            "CompiledPlanCorrupt",
            "UI session context requires exclusive device ownership",
        ));
    }
    let target = target_bundle_id(plan);
    let target_free = plan.execution_order.iter().all(|node_id| {
        plan.nodes.get(node_id).is_some_and(|node| {
            matches!(
                node.kind,
                ActionKind::Start | ActionKind::Wait | ActionKind::End
            )
        })
    });
    if target.is_none()
        && (!target_free
            || plan.context_plan.requires_exclusive
            || plan.context_plan.requires_ui_session
            || plan.context_plan.requires_stream
            || plan.context_plan.requires_fresh_text_session)
    {
        return Err(FlowExecutionError::new(
            "CompiledPlanCorrupt",
            "target-free Flow may contain only Start, Wait, and End without device resources",
        ));
    }
    if target.is_some() && !plan.context_plan.requires_exclusive {
        return Err(FlowExecutionError::new(
            "CompiledPlanCorrupt",
            "target-qualified Flow requires exclusive device ownership",
        ));
    }
    if plan.context_plan.requires_ui_session {
        let initial = plan
            .context_plan
            .initial_bundle_id
            .as_deref()
            .filter(|bundle_id| exact_identifier(bundle_id))
            .ok_or_else(|| {
                FlowExecutionError::new(
                    "CompiledPlanCorrupt",
                    "UI plan has no exact initial target bundle",
                )
            })?;
        let first_launch = plan.execution_order.iter().find_map(|node_id| {
            let node = plan.nodes.get(node_id)?;
            match &node.config {
                CompiledActionConfig::LaunchApp { bundle_id } => Some(bundle_id.as_str()),
                _ => None,
            }
        });
        if first_launch != Some(initial) {
            return Err(FlowExecutionError::new(
                "CompiledPlanCorrupt",
                "first Launch App does not match the initial target bundle",
            ));
        }
    }
    Ok(())
}

fn exact_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= 255
        && !value.chars().any(char::is_control)
}

fn flow_release_proof(proof: ContextReleaseProof) -> FlowContextReleaseProof {
    FlowContextReleaseProof {
        udid: proof.udid,
        owner: proof.owner,
        had_session: proof.had_session,
        had_stream: proof.had_stream,
    }
}

fn check_evidence_request_boundary(
    deadline: tokio::time::Instant,
    cancellation: &FlowCancellation,
) -> Result<(), FlowExecutionError> {
    if cancellation.is_cancelled() {
        Err(FlowExecutionError::evidence(EvidenceError::Cancelled))
    } else if tokio::time::Instant::now() >= deadline {
        Err(FlowExecutionError::evidence(EvidenceError::Timeout))
    } else {
        Ok(())
    }
}

async fn cancellable_wait(
    duration_ms: u64,
    cancellation: &FlowCancellation,
) -> Result<(), FlowExecutionError> {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(duration_ms);
    while tokio::time::Instant::now() < deadline {
        if cancellation.is_cancelled() {
            return Err(FlowExecutionError::new("Cancelled", "flow was cancelled"));
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let slice = remaining.min(WAIT_SLICE);
        tokio::select! {
            _ = tokio::time::sleep(slice) => {}
            _ = cancellation.cancelled() => {}
        }
    }
    if cancellation.is_cancelled() {
        Err(FlowExecutionError::new("Cancelled", "flow was cancelled"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet, HashMap};
    use std::io::Cursor;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
    use parking_lot::Mutex;
    use tokio::sync::Notify;
    use uuid::Uuid;

    use super::*;
    use crate::db::Database;
    use crate::{
        compiled_plan_sha256, qualified_geometry_profile_id, ActionKind, ActiveTransport,
        AgentState, AgentStatus, AppProcessState, AttemptArtifactInspection, CompiledActionConfig,
        CompiledFlowNode, CompiledFlowPlanV2, CompiledTapTarget, ConnectionKind, ContextPlan,
        DeviceCapabilitySnapshot, DeviceControlPlane, DeviceDriver, DeviceInfo, DeviceStatus,
        DeviceWorkCoordinator, DeviceWorkOwner, EvidenceSpec, FlowArtifactStore, FlowAttemptState,
        FlowCancellation, FlowDocumentV2, FlowSelectionSnapshot, FlowTargetSelection, Frame,
        FrameSource, FrameStream, GenerationFrame, GenerationFrameEvent, GenerationFrameSource,
        GenerationFrameStream, ImageCoordinateTarget, InstalledAgentIdentity,
        InstalledTargetIdentity, InteractionSessionKind, ProcessAbsenceProof, QualifiedGeometry,
        ScreenOrientation, StreamBudgetManager, StreamHandoffProof, StreamStartProof,
        StreamStopProof, SwipeGesture, TapPoint, UiSession, FLOW_SCHEMA_VERSION,
    };

    const UDID: &str = "fixture-udid";
    const TARGET: &str = "com.apple.Preferences";

    struct RecordingFlowDriver {
        operations: Arc<Mutex<Vec<String>>>,
        work: Arc<DeviceWorkCoordinator>,
        streams: Arc<StreamBudgetManager>,
        snapshot: Mutex<DeviceCapabilitySnapshot>,
        active_bundle: Arc<Mutex<String>>,
        typed_text: Arc<Mutex<String>>,
        processes: Mutex<HashMap<String, u64>>,
        block_process_inspection: AtomicBool,
        fail_process_inspection: AtomicBool,
        process_inspection_started: AtomicBool,
        process_inspection_release: Notify,
        session_kind: Mutex<Option<InteractionSessionKind>>,
        fail_tap: Arc<AtomicBool>,
        fail_session_start: AtomicBool,
        fail_stream_start: AtomicBool,
        session_start_delay_ms: AtomicU64,
        drop_launch_effect: AtomicBool,
        fail_active_read: Arc<AtomicBool>,
        fail_assert_visible: Arc<AtomicBool>,
        omit_first_frame: AtomicBool,
        stream_generation: AtomicU64,
        supports_readback: Arc<AtomicBool>,
        inspection_calls: AtomicUsize,
        launch_calls: Arc<AtomicUsize>,
        tap_calls: Arc<AtomicUsize>,
        active_app_reads: Arc<AtomicUsize>,
        screenshot_png_calls: Arc<AtomicUsize>,
        close_session_calls: Arc<AtomicUsize>,
        stop_stream_calls: Arc<AtomicUsize>,
        assert_visible_calls: Arc<AtomicUsize>,
        release_observed: AtomicBool,
    }

    impl RecordingFlowDriver {
        fn new(
            work: Arc<DeviceWorkCoordinator>,
            streams: Arc<StreamBudgetManager>,
            snapshot: DeviceCapabilitySnapshot,
        ) -> Self {
            Self {
                operations: Arc::new(Mutex::new(Vec::new())),
                work,
                streams,
                snapshot: Mutex::new(snapshot),
                active_bundle: Arc::new(Mutex::new(String::new())),
                typed_text: Arc::new(Mutex::new(String::new())),
                processes: Mutex::new(HashMap::from([(TARGET.to_string(), 42)])),
                block_process_inspection: AtomicBool::new(false),
                fail_process_inspection: AtomicBool::new(false),
                process_inspection_started: AtomicBool::new(false),
                process_inspection_release: Notify::new(),
                session_kind: Mutex::new(None),
                fail_tap: Arc::new(AtomicBool::new(false)),
                fail_session_start: AtomicBool::new(false),
                fail_stream_start: AtomicBool::new(false),
                session_start_delay_ms: AtomicU64::new(0),
                drop_launch_effect: AtomicBool::new(false),
                fail_active_read: Arc::new(AtomicBool::new(false)),
                fail_assert_visible: Arc::new(AtomicBool::new(false)),
                omit_first_frame: AtomicBool::new(false),
                stream_generation: AtomicU64::new(1),
                supports_readback: Arc::new(AtomicBool::new(true)),
                inspection_calls: AtomicUsize::new(0),
                launch_calls: Arc::new(AtomicUsize::new(0)),
                tap_calls: Arc::new(AtomicUsize::new(0)),
                active_app_reads: Arc::new(AtomicUsize::new(0)),
                screenshot_png_calls: Arc::new(AtomicUsize::new(0)),
                close_session_calls: Arc::new(AtomicUsize::new(0)),
                stop_stream_calls: Arc::new(AtomicUsize::new(0)),
                assert_visible_calls: Arc::new(AtomicUsize::new(0)),
                release_observed: AtomicBool::new(false),
            }
        }

        fn push(&self, operation: impl Into<String>) {
            self.operations.lock().push(operation.into());
        }

        fn operations(&self) -> Vec<String> {
            if self.work.current_owner(UDID).is_none()
                && !self.release_observed.swap(true, Ordering::SeqCst)
            {
                self.push("release");
            }
            self.operations.lock().clone()
        }

        fn status(&self) -> AgentStatus {
            AgentStatus {
                udid: UDID.to_string(),
                state: AgentState::Ready,
                artifact_id: "fixture-agent".to_string(),
                artifact_version: "1.0".to_string(),
                bundle_id: "com.fixture.agent".to_string(),
                protocol_version: 2,
                features: vec![
                    "stream".to_string(),
                    "tap".to_string(),
                    "swipe".to_string(),
                    "text".to_string(),
                ],
                installed_version: Some("1.0".to_string()),
                installed_build: Some("1".to_string()),
                auth_ready: true,
                mjpeg_ready: false,
                session_ready: false,
                message: None,
            }
        }
    }

    struct RecordingSession {
        operations: Arc<Mutex<Vec<String>>>,
        active_bundle: Arc<Mutex<String>>,
        typed_text: Arc<Mutex<String>>,
        fail_tap: Arc<AtomicBool>,
        tap_calls: Arc<AtomicUsize>,
        active_app_reads: Arc<AtomicUsize>,
        screenshot_png_calls: Arc<AtomicUsize>,
        assert_visible_calls: Arc<AtomicUsize>,
        fail_active_read: Arc<AtomicBool>,
        fail_assert_visible: Arc<AtomicBool>,
        supports_text: bool,
        supports_readback: bool,
    }

    impl RecordingSession {
        fn push(&self, operation: &str) {
            self.operations.lock().push(operation.to_string());
        }
    }

    #[async_trait]
    impl UiSession for RecordingSession {
        async fn tap(&self, _point: TapPoint) -> anyhow::Result<()> {
            self.tap_calls.fetch_add(1, Ordering::SeqCst);
            self.push("tap");
            if self.fail_tap.load(Ordering::SeqCst) {
                anyhow::bail!("fixture tap failed");
            }
            Ok(())
        }

        async fn swipe(&self, _gesture: SwipeGesture) -> anyhow::Result<()> {
            self.push("swipe");
            Ok(())
        }

        async fn type_text(&self, text: &str) -> anyhow::Result<()> {
            self.push("typeText");
            *self.typed_text.lock() = text.to_string();
            Ok(())
        }

        fn supports_text_input(&self) -> bool {
            self.supports_text
        }

        async fn home(&self) -> anyhow::Result<()> {
            self.push("home");
            *self.active_bundle.lock() = "com.apple.springboard".to_string();
            Ok(())
        }

        async fn find_and_tap(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            self.push("findAndTap");
            Ok(())
        }

        async fn assert_visible(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            self.assert_visible_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_assert_visible.load(Ordering::SeqCst) {
                anyhow::bail!("fixture element is not visible");
            }
            self.push("assertVisible");
            Ok(())
        }

        async fn launch_app_foreground(&self, bundle_id: &str) -> anyhow::Result<()> {
            *self.active_bundle.lock() = bundle_id.to_string();
            Ok(())
        }

        async fn active_app_bundle(&self) -> anyhow::Result<String> {
            self.active_app_reads.fetch_add(1, Ordering::SeqCst);
            if self.fail_active_read.load(Ordering::SeqCst) {
                anyhow::bail!("fixture active-app read failed");
            }
            Ok(self.active_bundle.lock().clone())
        }

        async fn read_text(
            &self,
            _locator: &crate::QualifiedElementLocator,
            _request_timeout: Duration,
        ) -> anyhow::Result<String> {
            self.push("readText");
            Ok(self.typed_text.lock().clone())
        }

        fn supports_accessibility_readback(&self) -> bool {
            self.supports_readback
        }

        async fn screenshot_png(&self) -> anyhow::Result<Vec<u8>> {
            self.screenshot_png_calls.fetch_add(1, Ordering::SeqCst);
            anyhow::bail!("Flow must not poll the UI screenshot route")
        }

        fn stream_url(&self) -> Option<String> {
            Some("http://fixture/stream".to_string())
        }
    }

    #[async_trait]
    impl DeviceDriver for RecordingFlowDriver {
        fn cached_agent_status(&self, _udid: &str) -> AgentStatus {
            self.status()
        }

        async fn inspect_device_for_target(
            &self,
            _udid: &str,
            target_bundle_id: &str,
        ) -> anyhow::Result<DeviceCapabilitySnapshot> {
            self.inspection_calls.fetch_add(1, Ordering::SeqCst);
            let mut snapshot = self.snapshot.lock().clone();
            snapshot.target_app.bundle_id = target_bundle_id.to_string();
            Ok(snapshot)
        }

        fn supports_verified_app_termination(&self) -> bool {
            true
        }

        async fn inspect_app_process(
            &self,
            _udid: &str,
            bundle_id: &str,
        ) -> anyhow::Result<AppProcessState> {
            if self.block_process_inspection.load(Ordering::SeqCst) {
                let released = self.process_inspection_release.notified();
                self.process_inspection_started
                    .store(true, Ordering::SeqCst);
                released.await;
            }
            if self.fail_process_inspection.load(Ordering::SeqCst) {
                anyhow::bail!("fixture process inspection failed");
            }
            let pid = self.processes.lock().get(bundle_id).copied();
            Ok(AppProcessState {
                bundle_id: bundle_id.to_string(),
                pid,
                running: pid.is_some(),
            })
        }

        async fn confirm_interaction_stream_stopped(
            &self,
            _udid: &str,
        ) -> anyhow::Result<StreamHandoffProof> {
            Ok(StreamHandoffProof {
                generation: self.stream_generation.load(Ordering::SeqCst),
            })
        }

        async fn read_active_app_bundle(&self, _udid: &str) -> anyhow::Result<String> {
            self.active_app_reads.fetch_add(1, Ordering::SeqCst);
            if self.fail_active_read.load(Ordering::SeqCst) {
                anyhow::bail!("fixture active-app read failed");
            }
            Ok(self.active_bundle.lock().clone())
        }

        async fn start_interaction_session(
            self: &Self,
            _udid: &str,
            _bundle_id: &str,
            kind: InteractionSessionKind,
        ) -> anyhow::Result<Box<dyn UiSession>> {
            *self.session_kind.lock() = Some(kind);
            self.push(match kind {
                InteractionSessionKind::Ordinary => "session:ordinary",
                InteractionSessionKind::FreshText => "session:freshText",
            });
            if self.fail_session_start.load(Ordering::SeqCst) {
                anyhow::bail!("fixture session start failed");
            }
            let delay_ms = self.session_start_delay_ms.load(Ordering::SeqCst);
            if delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            Ok(Box::new(RecordingSession {
                operations: self.operations.clone(),
                active_bundle: self.active_bundle.clone(),
                typed_text: self.typed_text.clone(),
                fail_tap: self.fail_tap.clone(),
                tap_calls: self.tap_calls.clone(),
                active_app_reads: self.active_app_reads.clone(),
                screenshot_png_calls: self.screenshot_png_calls.clone(),
                assert_visible_calls: self.assert_visible_calls.clone(),
                fail_active_read: self.fail_active_read.clone(),
                fail_assert_visible: self.fail_assert_visible.clone(),
                supports_text: kind == InteractionSessionKind::FreshText,
                supports_readback: kind == InteractionSessionKind::FreshText
                    && self.supports_readback.load(Ordering::SeqCst),
            }))
        }

        async fn stop_owned_stream(&self, _udid: &str) -> anyhow::Result<StreamStopProof> {
            self.stop_stream_calls.fetch_add(1, Ordering::SeqCst);
            self.push("stopStream");
            let old_generation = self.stream_generation.fetch_add(1, Ordering::SeqCst);
            Ok(StreamStopProof {
                old_generation,
                new_generation: old_generation + 1,
                child_stopped: true,
            })
        }

        async fn start_stream_after_session(
            &self,
            _udid: &str,
        ) -> anyhow::Result<StreamStartProof> {
            assert!(matches!(
                self.operations.lock().last().map(String::as_str),
                Some("session:ordinary" | "session:freshText")
            ));
            self.push("startStream");
            if self.fail_stream_start.load(Ordering::SeqCst) {
                self.stream_generation.fetch_add(1, Ordering::SeqCst);
                anyhow::bail!("fixture stream start failed");
            }
            Ok(StreamStartProof {
                generation: self.stream_generation.load(Ordering::SeqCst),
                first_frame_observed: !self.omit_first_frame.load(Ordering::SeqCst),
                stream_url: "http://fixture/stream".to_string(),
            })
        }

        async fn list_devices(&self) -> anyhow::Result<Vec<DeviceInfo>> {
            Ok(Vec::new())
        }

        async fn refresh_device(&self, udid: &str) -> anyhow::Result<DeviceInfo> {
            Ok(DeviceInfo {
                udid: udid.to_string(),
                name: "Fixture".to_string(),
                model: "iPhone".to_string(),
                ios_version: "16.7.15".to_string(),
                battery: None,
                wda_ready: false,
                wda_expires_at: None,
                stream_url: None,
                tile_stream_state: crate::TileStreamState::Parked,
                last_error: None,
                connection: ConnectionKind::Mock,
                status: DeviceStatus::Ready,
            })
        }

        async fn install_app(&self, _udid: &str, _path: &Path) -> anyhow::Result<()> {
            Ok(())
        }

        async fn uninstall_app(&self, _udid: &str, _bundle_id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn screenshot(&self, _udid: &str, dest: &Path) -> anyhow::Result<PathBuf> {
            Ok(dest.to_path_buf())
        }

        async fn syslog_tail(&self, _udid: &str, _lines: usize) -> anyhow::Result<String> {
            Ok(String::new())
        }

        async fn launch_app(&self, _udid: &str, bundle_id: &str) -> anyhow::Result<()> {
            if self.launch_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                assert_eq!(self.work.current_owner(UDID), Some(DeviceWorkOwner::Script));
                assert_eq!(self.streams.reserved_capacity(), 1);
                self.push("park");
                self.push("reserveStream");
            }
            self.push(format!("launch:{bundle_id}"));
            if !self.drop_launch_effect.load(Ordering::SeqCst) {
                *self.active_bundle.lock() = bundle_id.to_string();
            }
            Ok(())
        }

        async fn terminate_app(
            &self,
            _udid: &str,
            bundle_id: &str,
        ) -> anyhow::Result<ProcessAbsenceProof> {
            self.push(format!("terminate:{bundle_id}"));
            let old_pid = self.processes.lock().remove(bundle_id);
            Ok(ProcessAbsenceProof {
                bundle_id: bundle_id.to_string(),
                old_pid,
            })
        }

        async fn reboot(&self, _udid: &str) -> anyhow::Result<()> {
            Ok(())
        }

        async fn start_ui_session(&self, _udid: &str) -> anyhow::Result<Box<dyn UiSession>> {
            anyhow::bail!("Flow must use the interaction session transition")
        }

        fn invalidate_ui_session(&self, _udid: &str) {
            self.close_session_calls.fetch_add(1, Ordering::SeqCst);
            self.push("closeSession");
        }

        async fn ensure_stream(&self, _udid: &str) -> anyhow::Result<String> {
            anyhow::bail!("Flow must use start_stream_after_session")
        }

        async fn prepare_device(&self, _udid: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    struct FixtureFrameStream {
        frame: Option<Frame>,
    }

    #[async_trait]
    impl FrameStream for FixtureFrameStream {
        async fn next(&mut self) -> Option<Frame> {
            self.frame.take()
        }
    }

    struct FixtureGenerationStream {
        event: Option<GenerationFrameEvent>,
    }

    #[async_trait]
    impl GenerationFrameStream for FixtureGenerationStream {
        async fn next(&mut self) -> GenerationFrameEvent {
            self.event.take().unwrap_or(GenerationFrameEvent::Closed)
        }
    }

    struct FixtureFrames {
        generation: AtomicU64,
        subscriptions: AtomicUsize,
        latest_calls: AtomicUsize,
        invalidate_on_latest_call: AtomicUsize,
        frames: Vec<Frame>,
    }

    impl FixtureFrames {
        fn new(colors: &[u8]) -> Self {
            Self {
                generation: AtomicU64::new(1),
                subscriptions: AtomicUsize::new(0),
                latest_calls: AtomicUsize::new(0),
                invalidate_on_latest_call: AtomicUsize::new(0),
                frames: colors.iter().map(|color| jpeg(*color)).collect(),
            }
        }

        fn invalidate_on_latest_call(&self, call: usize) {
            self.invalidate_on_latest_call.store(call, Ordering::SeqCst);
        }
    }

    impl FrameSource for FixtureFrames {
        fn subscribe(&self, _udid: &str) -> Box<dyn FrameStream> {
            Box::new(FixtureFrameStream {
                frame: self.frames.last().cloned(),
            })
        }

        fn latest(&self, _udid: &str) -> Option<Frame> {
            self.frames.last().cloned()
        }
    }

    impl GenerationFrameSource for FixtureFrames {
        fn subscribe_generation(
            &self,
            _udid: &str,
            generation: u64,
        ) -> Box<dyn GenerationFrameStream> {
            if self.generation.load(Ordering::SeqCst) != generation {
                return Box::new(FixtureGenerationStream {
                    event: Some(GenerationFrameEvent::Advanced {
                        expected: generation,
                        actual: self.generation.load(Ordering::SeqCst),
                    }),
                });
            }
            let index = self.subscriptions.fetch_add(1, Ordering::SeqCst);
            let frame = self
                .frames
                .get(index)
                .or_else(|| self.frames.last())
                .expect("fixture has a frame")
                .clone();
            Box::new(FixtureGenerationStream {
                event: Some(GenerationFrameEvent::Frame(GenerationFrame {
                    generation,
                    sequence: u64::try_from(self.frames.len() + index + 1)
                        .expect("fixture sequence"),
                    bytes: frame,
                })),
            })
        }

        fn latest_in_generation(&self, _udid: &str, generation: u64) -> Option<GenerationFrame> {
            let call = self.latest_calls.fetch_add(1, Ordering::SeqCst) + 1;
            if self.invalidate_on_latest_call.load(Ordering::SeqCst) == call {
                self.generation.fetch_add(1, Ordering::SeqCst);
                return None;
            }
            (self.generation.load(Ordering::SeqCst) == generation).then(|| GenerationFrame {
                generation,
                sequence: u64::try_from(self.frames.len()).expect("fixture sequence"),
                bytes: self.frames.last().expect("fixture has a frame").clone(),
            })
        }
    }

    struct ExecutorFixture {
        executor: FlowExecutor,
        device_run_id: Uuid,
        run_id: Uuid,
        plan: CompiledFlowPlanV2,
        driver: Arc<RecordingFlowDriver>,
        database: Arc<Database>,
        control: Arc<DeviceControlPlane>,
        work: Arc<DeviceWorkCoordinator>,
        streams: Arc<StreamBudgetManager>,
        cancellation: FlowCancellation,
    }

    impl ExecutorFixture {
        fn new(plan: CompiledFlowPlanV2, frames: Arc<FixtureFrames>) -> Self {
            let database_path =
                std::env::temp_dir().join(format!("riviu-flow-executor-{}.db", Uuid::new_v4()));
            let database = Arc::new(Database::open(database_path).expect("executor database"));
            let mut document = FlowDocumentV2::empty("Executor fixture");
            document.id = plan.flow_id;
            document.revision = plan.revision;
            let hash = compiled_plan_sha256(&plan).expect("compiled hash");
            let revision = database
                .save_flow_revision(None, &document, &plan, &hash)
                .expect("persist executor revision");
            let run = database
                .create_flow_run(
                    &revision,
                    FlowSelectionSnapshot {
                        requested: FlowTargetSelection::One {
                            udid: UDID.to_string(),
                        },
                        target_udids: vec![UDID.to_string()],
                    },
                )
                .expect("create executor run");
            let device = database
                .create_flow_device_run(run.id, UDID)
                .expect("create executor device");

            let work = Arc::new(DeviceWorkCoordinator::new());
            let streams = Arc::new(StreamBudgetManager::new(1).expect("stream budget"));
            let driver = Arc::new(RecordingFlowDriver::new(
                work.clone(),
                streams.clone(),
                capability_snapshot(),
            ));
            let control = Arc::new(DeviceControlPlane::new(
                driver.clone(),
                work.clone(),
                streams.clone(),
            ));
            let artifact_root = std::env::temp_dir()
                .join(format!("riviu-flow-executor-artifacts-{}", Uuid::new_v4()));
            let cancellation = FlowCancellation::default();
            let executor = FlowExecutor::new(FlowExecutorDeps {
                run_id: run.id,
                udid: UDID.to_string(),
                database: database.clone(),
                control: control.clone(),
                frames,
                artifacts: FlowArtifactStore::new(artifact_root).expect("artifact store"),
                cancellation: cancellation.clone(),
            });
            Self {
                executor,
                device_run_id: device.id,
                run_id: run.id,
                plan,
                driver,
                database,
                control,
                work,
                streams,
                cancellation,
            }
        }

        async fn shutdown(&self) {
            self.control
                .shutdown_cleanup()
                .await
                .expect("control shutdown");
        }

        fn detail(&self) -> crate::FlowRunDetail {
            self.database
                .get_flow_run(self.run_id)
                .expect("load executor run")
                .expect("executor run")
        }
    }

    #[tokio::test]
    async fn executor_rejects_a_plan_that_differs_from_the_run_pinned_hash() {
        let fixture = ExecutorFixture::new(terminate_plan(), Arc::new(FixtureFrames::new(&[40])));
        let mut substituted_plan = fixture.plan.clone();
        substituted_plan
            .required_capabilities
            .insert("fixture.unpinned".to_string());

        let error = fixture
            .executor
            .run_device(fixture.device_run_id, substituted_plan)
            .await
            .expect_err("executor must bind the caller plan to the run hash");

        assert_eq!(error.code(), "RunIdentityMismatch");
        assert!(fixture.driver.operations.lock().is_empty());
        assert!(fixture.work.current_owner(UDID).is_none());
        assert_eq!(
            fixture.detail().device_runs[0].state,
            FlowDeviceRunState::Queued
        );
        fixture.shutdown().await;
    }

    #[test]
    fn ui_resume_uses_the_last_succeeded_launch_target_before_the_retried_node() {
        let mut plan = launch_wait_plan();
        let wait_index = plan
            .execution_order
            .iter()
            .position(|node_id| plan.nodes[node_id].kind == ActionKind::Wait)
            .expect("wait node");
        let second_bundle = "com.example.second";
        let second_launch = CompiledFlowNode {
            id: Uuid::new_v4(),
            kind: ActionKind::LaunchApp,
            config: CompiledActionConfig::LaunchApp {
                bundle_id: second_bundle.to_string(),
            },
            postcondition: Some(EvidenceSpec::ActiveAppEquals {
                bundle_id: second_bundle.to_string(),
            }),
        };
        plan.execution_order.insert(wait_index, second_launch.id);
        plan.nodes.insert(second_launch.id, second_launch);

        assert_eq!(
            last_launch_bundle_before(&plan, wait_index + 1),
            Some(second_bundle)
        );
    }

    #[tokio::test]
    async fn text_flow_upgrades_once_and_starts_stream_after_fresh_session() {
        let fixture = ExecutorFixture::new(text_plan(), Arc::new(FixtureFrames::new(&[40])));
        fixture
            .executor
            .run_device(fixture.device_run_id, fixture.plan.clone())
            .await
            .expect("device run");

        assert_eq!(
            fixture.driver.operations(),
            vec![
                "park",
                "reserveStream",
                "launch:com.apple.Preferences",
                "session:freshText",
                "startStream",
                "typeText",
                "readText",
                "stopStream",
                "closeSession",
                "release",
            ]
        );
        assert_eq!(fixture.driver.launch_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.driver.stop_stream_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.driver.close_session_calls.load(Ordering::SeqCst), 1);
        let replacement = fixture
            .work
            .try_acquire(UDID, DeviceWorkOwner::Repair)
            .expect("flow released the device");
        drop(replacement);
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn ordinary_ui_flow_uses_one_ordinary_session_without_reacquiring() {
        let fixture = ExecutorFixture::new(launch_only_plan(), Arc::new(FixtureFrames::new(&[40])));
        fixture
            .executor
            .run_device(fixture.device_run_id, fixture.plan.clone())
            .await
            .expect("device run");

        assert_eq!(
            *fixture.driver.session_kind.lock(),
            Some(InteractionSessionKind::Ordinary)
        );
        assert_eq!(fixture.driver.launch_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.driver.close_session_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.driver.stop_stream_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            fixture
                .driver
                .operations()
                .iter()
                .filter(|operation| operation.as_str() == "park")
                .count(),
            1
        );
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn failed_stream_upgrade_recovers_the_session_owner_and_closes_once() {
        let fixture = ExecutorFixture::new(screenshot_plan(), Arc::new(FixtureFrames::new(&[40])));
        fixture
            .driver
            .omit_first_frame
            .store(true, Ordering::SeqCst);
        fixture
            .driver
            .fail_active_read
            .store(true, Ordering::SeqCst);

        let error = fixture
            .executor
            .run_device(fixture.device_run_id, fixture.plan.clone())
            .await
            .expect_err("stream startup must fail");

        assert_eq!(error.code(), "DeviceControl");
        assert_eq!(fixture.driver.launch_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.driver.stop_stream_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.driver.close_session_calls.load(Ordering::SeqCst), 1);
        assert!(fixture.work.current_owner(UDID).is_none());
        let detail = fixture.detail();
        let launch = detail
            .attempts
            .iter()
            .find(|attempt| attempt.action_kind == ActionKind::LaunchApp)
            .expect("launch attempt");
        assert_eq!(launch.state, FlowAttemptState::Uncertain);
        let proof = detail.device_runs[0]
            .release_proof
            .as_ref()
            .expect("stream release proof");
        assert!(proof.had_session);
        assert!(proof.had_stream);
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn failed_first_launch_session_is_reconciled_as_proved_non_delivery() {
        let fixture = ExecutorFixture::new(launch_only_plan(), Arc::new(FixtureFrames::new(&[40])));
        fixture
            .driver
            .fail_session_start
            .store(true, Ordering::SeqCst);
        fixture
            .driver
            .drop_launch_effect
            .store(true, Ordering::SeqCst);

        let error = fixture
            .executor
            .run_device(fixture.device_run_id, fixture.plan.clone())
            .await
            .expect_err("session startup must fail");

        assert_eq!(error.code(), "DeviceControl");
        assert_eq!(fixture.driver.launch_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.driver.close_session_calls.load(Ordering::SeqCst), 0);
        assert!(fixture.work.current_owner(UDID).is_none());
        let detail = fixture.detail();
        assert_eq!(detail.attempts[0].state, FlowAttemptState::FailedVerified);
        assert!(detail.attempts[0].retry_allowed);
        assert_eq!(fixture.driver.active_app_reads.load(Ordering::SeqCst), 1);
        let proof = detail.device_runs[0]
            .release_proof
            .as_ref()
            .expect("exclusive release proof");
        assert!(!proof.had_session);
        assert!(!proof.had_stream);
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn direct_stream_start_error_is_reconciled_and_released_with_exact_proof() {
        let fixture = ExecutorFixture::new(screenshot_plan(), Arc::new(FixtureFrames::new(&[40])));
        fixture
            .driver
            .fail_stream_start
            .store(true, Ordering::SeqCst);

        let error = fixture
            .executor
            .run_device(fixture.device_run_id, fixture.plan.clone())
            .await
            .expect_err("stream startup error must fail the device run");

        assert_eq!(error.code(), "DeviceControl");
        let detail = fixture.detail();
        let launch = detail
            .attempts
            .iter()
            .find(|attempt| attempt.action_kind == ActionKind::LaunchApp)
            .expect("launch attempt");
        assert_eq!(launch.state, FlowAttemptState::Succeeded);
        assert_eq!(detail.device_runs[0].state, FlowDeviceRunState::Failed);
        let proof = detail.device_runs[0]
            .release_proof
            .as_ref()
            .expect("bounded failed-start cleanup proof");
        assert!(proof.had_session);
        assert!(proof.had_stream);
        assert_eq!(fixture.control.cleanup_quarantine_count(), 0);
        assert!(fixture.work.current_owner(UDID).is_none());
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn capacity_reservation_failure_after_running_closes_with_terminal_proof() {
        let fixture = ExecutorFixture::new(launch_only_plan(), Arc::new(FixtureFrames::new(&[40])));
        let transfer = fixture
            .streams
            .begin_foreground_transfer("fixture-other", DeviceWorkOwner::Script)
            .expect("reserve competing foreground slot");
        let blocker = fixture
            .streams
            .complete_transfer(transfer, StreamStopProof::not_required())
            .expect("hold competing foreground slot");

        let error = fixture
            .executor
            .run_device(fixture.device_run_id, fixture.plan.clone())
            .await
            .expect_err("capacity exhaustion must fail the device run");

        assert_eq!(error.code(), "DeviceControl");
        let detail = fixture.detail();
        assert_eq!(detail.device_runs[0].state, FlowDeviceRunState::Failed);
        let proof = detail.device_runs[0]
            .release_proof
            .as_ref()
            .expect("exclusive release proof");
        assert!(!proof.had_session);
        assert!(!proof.had_stream);
        assert!(fixture.work.current_owner(UDID).is_none());
        drop(blocker);
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn targetless_start_wait_end_uses_typed_preflight_without_device_inspection() {
        let fixture = ExecutorFixture::new(wait_only_plan(), Arc::new(FixtureFrames::new(&[40])));
        let executor = fixture.executor.clone();
        let device_run_id = fixture.device_run_id;
        let plan = fixture.plan.clone();
        let task = tokio::spawn(async move { executor.run_device(device_run_id, plan).await });
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(fixture.work.current_owner(UDID), None);
        task.await
            .expect("target-free executor task")
            .expect("targetless Wait flow");

        let detail = fixture.detail();
        assert_eq!(detail.device_runs[0].state, FlowDeviceRunState::Succeeded);
        assert!(detail.device_runs[0].capability_snapshot.is_some());
        assert_eq!(fixture.driver.inspection_calls.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.driver.launch_calls.load(Ordering::SeqCst), 0);
        assert!(fixture.work.current_owner(UDID).is_none());
        let release = detail.device_runs[0]
            .release_proof
            .as_ref()
            .expect("typed no-resource release proof");
        assert!(!release.had_session);
        assert!(!release.had_stream);
        let persisted = serde_json::to_value(
            detail.device_runs[0]
                .capability_snapshot
                .as_ref()
                .expect("target-free preflight"),
        )
        .expect("serialize target-free preflight");
        assert_eq!(persisted["scope"]["kind"], "targetFree");
        assert!(persisted["agentStatus"].is_null());
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn static_capability_ids_are_persisted_with_the_qualified_snapshot() {
        let fixture = ExecutorFixture::new(launch_only_plan(), Arc::new(FixtureFrames::new(&[40])));

        fixture
            .executor
            .run_device(fixture.device_run_id, fixture.plan.clone())
            .await
            .expect("qualified launch flow");

        let detail = fixture.detail();
        let persisted = serde_json::to_value(
            detail.device_runs[0]
                .capability_snapshot
                .as_ref()
                .expect("durable preflight snapshot"),
        )
        .expect("serialize preflight snapshot");
        assert_eq!(persisted["scope"]["kind"], "targetQualified");
        assert_eq!(
            persisted["agentStatus"],
            serde_json::to_value(fixture.driver.status()).expect("serialize exact AgentStatus")
        );
        assert!(persisted["capabilityIds"]
            .as_array()
            .expect("capability IDs")
            .iter()
            .any(|value| value == "app.launch"));
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn first_launch_evidence_deadline_starts_after_session_bootstrap() {
        let fixture = ExecutorFixture::new(launch_only_plan(), Arc::new(FixtureFrames::new(&[40])));
        fixture
            .driver
            .session_start_delay_ms
            .store(5_100, Ordering::SeqCst);

        fixture
            .executor
            .run_device(fixture.device_run_id, fixture.plan.clone())
            .await
            .expect("bootstrap time must not consume the evidence window");

        let launch = fixture
            .detail()
            .attempts
            .into_iter()
            .find(|attempt| attempt.action_kind == ActionKind::LaunchApp)
            .expect("launch attempt");
        assert_eq!(launch.state, FlowAttemptState::Succeeded);
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn deferred_readback_failure_is_checked_after_session_before_stream_or_node_intent() {
        let fixture = ExecutorFixture::new(text_plan(), Arc::new(FixtureFrames::new(&[40])));
        fixture
            .driver
            .supports_readback
            .store(false, Ordering::SeqCst);

        let error = fixture
            .executor
            .run_device(fixture.device_run_id, fixture.plan.clone())
            .await
            .expect_err("live readback capability is mandatory");

        assert_eq!(error.code(), "CapabilityUnavailable");
        assert!(!fixture
            .driver
            .operations()
            .iter()
            .any(|operation| operation == "startStream"));
        let type_text = fixture
            .detail()
            .attempts
            .into_iter()
            .find(|attempt| attempt.action_kind == ActionKind::TypeText)
            .expect("durably queued Type Text successor");
        assert_eq!(type_text.state, FlowAttemptState::Queued);
        assert!(type_text.canonical_input.is_none());
        let proof = fixture.detail().device_runs[0]
            .release_proof
            .clone()
            .expect("session release proof");
        assert!(proof.had_session);
        assert!(!proof.had_stream);
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn first_launch_is_verified_through_the_live_session_exactly_once() {
        let fixture = ExecutorFixture::new(launch_only_plan(), Arc::new(FixtureFrames::new(&[40])));
        fixture
            .executor
            .run_device(fixture.device_run_id, fixture.plan.clone())
            .await
            .expect("device run");

        let detail = fixture.detail();
        let launch = detail
            .attempts
            .iter()
            .find(|attempt| attempt.action_kind == ActionKind::LaunchApp)
            .expect("launch attempt");
        assert_eq!(launch.state, FlowAttemptState::Succeeded);
        assert_eq!(
            launch.evidence_result.as_ref().unwrap()["measurement"]["bundleId"],
            TARGET
        );
        assert_eq!(fixture.driver.launch_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.driver.active_app_reads.load(Ordering::SeqCst), 1);
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn bridge_only_terminate_uses_process_absence_without_launch_or_session() {
        let fixture = ExecutorFixture::new(terminate_plan(), Arc::new(FixtureFrames::new(&[40])));
        fixture
            .executor
            .run_device(fixture.device_run_id, fixture.plan.clone())
            .await
            .expect("device run");

        let detail = fixture.detail();
        let terminate = detail
            .attempts
            .iter()
            .find(|attempt| attempt.action_kind == ActionKind::TerminateApp)
            .expect("terminate attempt");
        assert_eq!(terminate.state, FlowAttemptState::Succeeded);
        assert_eq!(
            terminate.evidence_result.as_ref().unwrap()["measurement"]["running"],
            false
        );
        assert_eq!(fixture.driver.launch_calls.load(Ordering::SeqCst), 0);
        assert_eq!(*fixture.driver.session_kind.lock(), None);
        assert_eq!(
            fixture.driver.operations(),
            vec![format!("terminate:{TARGET}"), "release".to_string()]
        );
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn cancellation_during_process_baseline_never_dispatches_terminate() {
        let fixture = ExecutorFixture::new(terminate_plan(), Arc::new(FixtureFrames::new(&[40])));
        fixture
            .driver
            .block_process_inspection
            .store(true, Ordering::SeqCst);
        fixture
            .driver
            .fail_process_inspection
            .store(true, Ordering::SeqCst);
        let executor = fixture.executor.clone();
        let device_run_id = fixture.device_run_id;
        let plan = fixture.plan.clone();
        let task = tokio::spawn(async move { executor.run_device(device_run_id, plan).await });

        tokio::time::timeout(Duration::from_secs(1), async {
            while !fixture
                .driver
                .process_inspection_started
                .load(Ordering::SeqCst)
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("process baseline inspection must start");
        fixture.cancellation.cancel();
        fixture.driver.process_inspection_release.notify_one();

        let error = task
            .await
            .expect("executor task")
            .expect_err("cancelled process baseline");
        assert_eq!(error.code(), "Cancelled");
        assert!(!fixture
            .driver
            .operations()
            .iter()
            .any(|operation| operation.starts_with("terminate:")));
        let terminate = fixture
            .detail()
            .attempts
            .into_iter()
            .find(|attempt| attempt.action_kind == ActionKind::TerminateApp)
            .expect("durably queued Terminate attempt");
        assert_eq!(terminate.state, FlowAttemptState::Queued);
        assert!(terminate.canonical_input.is_none());
        assert_eq!(
            fixture.detail().device_runs[0].state,
            FlowDeviceRunState::Cancelled
        );
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn bridge_only_terminate_does_not_require_ui_geometry() {
        let fixture = ExecutorFixture::new(terminate_plan(), Arc::new(FixtureFrames::new(&[40])));
        fixture.driver.snapshot.lock().geometry = None;

        fixture
            .executor
            .run_device(fixture.device_run_id, fixture.plan.clone())
            .await
            .expect("bridge-only termination");

        assert_eq!(fixture.driver.launch_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            fixture.detail().device_runs[0].state,
            FlowDeviceRunState::Succeeded
        );
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn ui_preflight_without_geometry_releases_exclusive_and_persists_failure() {
        let fixture = ExecutorFixture::new(launch_only_plan(), Arc::new(FixtureFrames::new(&[40])));
        fixture.driver.snapshot.lock().geometry = None;

        let error = fixture
            .executor
            .run_device(fixture.device_run_id, fixture.plan.clone())
            .await
            .expect_err("UI geometry is mandatory");

        assert_eq!(error.code(), "GeometryUnavailable");
        assert_eq!(fixture.driver.launch_calls.load(Ordering::SeqCst), 0);
        assert!(fixture.work.current_owner(UDID).is_none());
        let device = &fixture.detail().device_runs[0];
        assert_eq!(device.state, FlowDeviceRunState::Failed);
        let proof = device
            .release_proof
            .as_ref()
            .expect("preflight release proof");
        assert!(!proof.had_session);
        assert!(!proof.had_stream);
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn screenshot_reads_the_owned_generation_and_never_calls_ui_screenshot() {
        let fixture = ExecutorFixture::new(screenshot_plan(), Arc::new(FixtureFrames::new(&[40])));
        fixture
            .executor
            .run_device(fixture.device_run_id, fixture.plan.clone())
            .await
            .expect("device run");

        let detail = fixture.detail();
        assert_eq!(detail.artifacts.len(), 1);
        assert_eq!(detail.artifacts[0].label, "capture.jpeg");
        assert_eq!(
            fixture.driver.screenshot_png_calls.load(Ordering::SeqCst),
            0
        );
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn screenshot_generation_advance_after_file_publish_rolls_back_before_db_commit() {
        let frames = Arc::new(FixtureFrames::new(&[40]));
        frames.invalidate_on_latest_call(5);
        let fixture = ExecutorFixture::new(screenshot_plan(), frames.clone());

        let error = fixture
            .executor
            .run_device(fixture.device_run_id, fixture.plan.clone())
            .await
            .expect_err("generation advance must stop screenshot publication");

        assert_eq!(error.code(), "StaleGeneration");
        let detail = fixture.detail();
        assert!(detail.artifacts.is_empty());
        let screenshot = detail
            .attempts
            .iter()
            .find(|attempt| attempt.action_kind == ActionKind::Screenshot)
            .expect("screenshot attempt");
        assert_eq!(screenshot.state, FlowAttemptState::Uncertain);
        assert_eq!(
            fixture
                .executor
                .deps
                .artifacts
                .inspect_attempt_image(
                    fixture.run_id,
                    fixture.device_run_id,
                    screenshot.id,
                    "jpeg",
                )
                .expect("inspect rolled-back screenshot"),
            AttemptArtifactInspection::Absent,
        );
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn tap_ack_with_an_unchanged_post_frame_is_failed_verified() {
        let fixture = ExecutorFixture::new(tap_plan(4, 4), Arc::new(FixtureFrames::new(&[40, 40])));
        let error = fixture
            .executor
            .run_device(fixture.device_run_id, fixture.plan.clone())
            .await
            .expect_err("unchanged frame must fail verification");

        assert_eq!(error.code(), "EvidenceMismatch", "{error}");
        let detail = fixture.detail();
        let tap = detail
            .attempts
            .iter()
            .find(|attempt| attempt.action_kind == ActionKind::Tap)
            .expect("tap attempt");
        assert_eq!(tap.state, FlowAttemptState::FailedVerified);
        assert_eq!(tap.evidence_result.as_ref().unwrap()["matched"], false);
        assert_eq!(fixture.driver.tap_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.driver.stop_stream_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.driver.close_session_calls.load(Ordering::SeqCst), 1);
        assert!(fixture.work.current_owner(UDID).is_none());
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn geometry_mismatch_fails_before_the_tap_device_call() {
        let fixture = ExecutorFixture::new(tap_plan(8, 4), Arc::new(FixtureFrames::new(&[40, 90])));
        let error = fixture
            .executor
            .run_device(fixture.device_run_id, fixture.plan.clone())
            .await
            .expect_err("compiled geometry must match the live frame");

        assert_eq!(error.code(), "GeometryMismatch");
        assert_eq!(fixture.driver.tap_calls.load(Ordering::SeqCst), 0);
        let tap = fixture
            .detail()
            .attempts
            .into_iter()
            .find(|attempt| attempt.action_kind == ActionKind::Tap)
            .expect("tap attempt");
        assert_eq!(tap.state, FlowAttemptState::FailedBeforeDispatch);
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn decoded_frame_dimensions_must_match_qualified_runtime_geometry() {
        let mut qualified_snapshot = capability_snapshot();
        qualified_snapshot.geometry.as_mut().unwrap().pixel_width = 8;
        let qualified_profile = qualified_geometry_profile_id(&qualified_snapshot).unwrap();
        let mut qualified_plan = tap_plan(4, 4);
        let tap = qualified_plan
            .nodes
            .values_mut()
            .find(|node| node.kind == ActionKind::Tap)
            .expect("tap node");
        let CompiledActionConfig::Tap {
            target: CompiledTapTarget::Point { target },
        } = &mut tap.config
        else {
            panic!("coordinate tap fixture");
        };
        target.profile_id = qualified_profile;
        let fixture = ExecutorFixture::new(qualified_plan, Arc::new(FixtureFrames::new(&[40, 90])));
        *fixture.driver.snapshot.lock() = qualified_snapshot;

        let error = fixture
            .executor
            .run_device(fixture.device_run_id, fixture.plan.clone())
            .await
            .expect_err("decoded frame must match qualified pixel dimensions");

        assert_eq!(error.code(), "GeometryMismatch");
        assert_eq!(fixture.driver.tap_calls.load(Ordering::SeqCst), 0);
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn generation_advance_after_decode_before_coordinate_dispatch_proves_non_delivery() {
        let frames = Arc::new(FixtureFrames::new(&[40, 90]));
        frames.invalidate_on_latest_call(6);
        let fixture = ExecutorFixture::new(tap_plan(4, 4), frames);

        let error = fixture
            .executor
            .run_device(fixture.device_run_id, fixture.plan.clone())
            .await
            .expect_err("generation advance must stop coordinate dispatch");

        assert_eq!(error.code(), "StaleGeneration");
        assert_eq!(fixture.driver.tap_calls.load(Ordering::SeqCst), 0);
        let tap = fixture
            .detail()
            .attempts
            .into_iter()
            .find(|attempt| attempt.action_kind == ActionKind::Tap)
            .expect("tap attempt");
        assert_eq!(tap.state, FlowAttemptState::FailedBeforeDispatch);
        assert_eq!(
            tap.evidence_result.expect("non-delivery proof"),
            serde_json::json!({
                "kind": "transportNonDelivery",
                "requestReachedDevice": false
            })
        );
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn assert_visible_request_failure_is_deterministic_failed_verified() {
        let fixture =
            ExecutorFixture::new(assert_visible_plan(), Arc::new(FixtureFrames::new(&[40])));
        fixture
            .driver
            .fail_assert_visible
            .store(true, Ordering::SeqCst);

        let error = fixture
            .executor
            .run_device(fixture.device_run_id, fixture.plan.clone())
            .await
            .expect_err("invisible locator must fail deterministically");

        assert_eq!(error.code(), "FlowExecution");
        let assertion = fixture
            .detail()
            .attempts
            .into_iter()
            .find(|attempt| attempt.action_kind == ActionKind::AssertVisible)
            .expect("Assert Visible attempt");
        assert_eq!(assertion.state, FlowAttemptState::FailedVerified);
        assert!(!assertion.retry_allowed);
        assert_eq!(
            fixture.driver.assert_visible_calls.load(Ordering::SeqCst),
            1
        );
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn compiled_kind_config_mismatch_is_persisted_before_any_device_call() {
        let fixture = ExecutorFixture::new(
            corrupt_terminate_plan(),
            Arc::new(FixtureFrames::new(&[40])),
        );
        let error = fixture
            .executor
            .run_device(fixture.device_run_id, fixture.plan.clone())
            .await
            .expect_err("compiled plan is corrupt");

        assert_eq!(error.code(), "CompiledPlanCorrupt", "{error}");
        assert_eq!(fixture.driver.launch_calls.load(Ordering::SeqCst), 0);
        assert!(fixture
            .driver
            .operations()
            .ends_with(&["release".to_string()]));
        assert_eq!(
            fixture.detail().device_runs[0].error.as_ref().unwrap().code,
            "CompiledPlanCorrupt"
        );
        fixture.shutdown().await;
    }

    #[tokio::test]
    async fn cancellation_during_wait_closes_the_highest_context_once() {
        let fixture = ExecutorFixture::new(launch_wait_plan(), Arc::new(FixtureFrames::new(&[40])));
        let executor = fixture.executor.clone();
        let device_run_id = fixture.device_run_id;
        let plan = fixture.plan.clone();
        let task = tokio::spawn(async move { executor.run_device(device_run_id, plan).await });
        tokio::time::timeout(Duration::from_secs(1), async {
            while fixture.driver.launch_calls.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("launch before cancellation");
        fixture.cancellation.cancel();
        let error = task
            .await
            .expect("executor task")
            .expect_err("cancelled wait");

        assert_eq!(error.code(), "Cancelled");
        assert_eq!(fixture.driver.close_session_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.driver.stop_stream_calls.load(Ordering::SeqCst), 0);
        assert!(fixture.work.current_owner(UDID).is_none());
        fixture.shutdown().await;
    }

    fn text_plan() -> CompiledFlowPlanV2 {
        let launch = launch_node();
        let typed = CompiledFlowNode {
            id: Uuid::new_v4(),
            kind: ActionKind::TypeText,
            config: CompiledActionConfig::TypeText {
                text: "xin chao".to_string(),
                read_back_locator: crate::QualifiedElementLocator {
                    strategy: crate::ElementLocatorStrategy::AccessibilityId,
                    value: "SearchField".to_string(),
                },
            },
            postcondition: Some(EvidenceSpec::TextReadBackEquals {
                locator: crate::QualifiedElementLocator {
                    strategy: crate::ElementLocatorStrategy::AccessibilityId,
                    value: "SearchField".to_string(),
                },
                value: "xin chao".to_string(),
            }),
        };
        plan(
            vec![launch, typed],
            ContextPlan {
                requires_exclusive: true,
                requires_ui_session: true,
                requires_stream: true,
                requires_fresh_text_session: true,
                initial_bundle_id: Some(TARGET.to_string()),
            },
            &["app.launch", "ui.text", "stream", "accessibility.readText"],
        )
    }

    fn launch_only_plan() -> CompiledFlowPlanV2 {
        plan(
            vec![launch_node()],
            ContextPlan {
                requires_exclusive: true,
                requires_ui_session: true,
                requires_stream: false,
                requires_fresh_text_session: false,
                initial_bundle_id: Some(TARGET.to_string()),
            },
            &["app.launch"],
        )
    }

    fn launch_wait_plan() -> CompiledFlowPlanV2 {
        let wait = CompiledFlowNode {
            id: Uuid::new_v4(),
            kind: ActionKind::Wait,
            config: CompiledActionConfig::Wait {
                duration_ms: 10_000,
            },
            postcondition: None,
        };
        plan(
            vec![launch_node(), wait],
            ContextPlan {
                requires_exclusive: true,
                requires_ui_session: true,
                requires_stream: false,
                requires_fresh_text_session: false,
                initial_bundle_id: Some(TARGET.to_string()),
            },
            &["app.launch"],
        )
    }

    fn wait_only_plan() -> CompiledFlowPlanV2 {
        let start = CompiledFlowNode {
            id: Uuid::new_v4(),
            kind: ActionKind::Start,
            config: CompiledActionConfig::Empty,
            postcondition: None,
        };
        let wait = CompiledFlowNode {
            id: Uuid::new_v4(),
            kind: ActionKind::Wait,
            config: CompiledActionConfig::Wait { duration_ms: 50 },
            postcondition: None,
        };
        let end = CompiledFlowNode {
            id: Uuid::new_v4(),
            kind: ActionKind::End,
            config: CompiledActionConfig::Empty,
            postcondition: None,
        };
        plan(
            vec![start, wait, end],
            ContextPlan {
                requires_exclusive: false,
                requires_ui_session: false,
                requires_stream: false,
                requires_fresh_text_session: false,
                initial_bundle_id: None,
            },
            &[],
        )
    }

    fn assert_visible_plan() -> CompiledFlowPlanV2 {
        let assertion = CompiledFlowNode {
            id: Uuid::new_v4(),
            kind: ActionKind::AssertVisible,
            config: CompiledActionConfig::AssertVisible {
                accessibility_id: "SearchField".to_string(),
            },
            postcondition: None,
        };
        plan(
            vec![launch_node(), assertion],
            ContextPlan {
                requires_exclusive: true,
                requires_ui_session: true,
                requires_stream: false,
                requires_fresh_text_session: true,
                initial_bundle_id: Some(TARGET.to_string()),
            },
            &["app.launch", "accessibility.visible"],
        )
    }

    fn terminate_plan() -> CompiledFlowPlanV2 {
        let terminate = CompiledFlowNode {
            id: Uuid::new_v4(),
            kind: ActionKind::TerminateApp,
            config: CompiledActionConfig::TerminateApp {
                bundle_id: TARGET.to_string(),
            },
            postcondition: Some(EvidenceSpec::ProcessAbsent {
                bundle_id: TARGET.to_string(),
            }),
        };
        plan(
            vec![terminate],
            ContextPlan {
                requires_exclusive: true,
                requires_ui_session: false,
                requires_stream: false,
                requires_fresh_text_session: false,
                initial_bundle_id: None,
            },
            &["app.terminate"],
        )
    }

    fn corrupt_terminate_plan() -> CompiledFlowPlanV2 {
        let corrupt = CompiledFlowNode {
            id: Uuid::new_v4(),
            kind: ActionKind::TerminateApp,
            config: CompiledActionConfig::Wait { duration_ms: 1 },
            postcondition: Some(EvidenceSpec::ProcessAbsent {
                bundle_id: TARGET.to_string(),
            }),
        };
        plan(
            vec![corrupt],
            ContextPlan {
                requires_exclusive: true,
                requires_ui_session: false,
                requires_stream: false,
                requires_fresh_text_session: false,
                initial_bundle_id: None,
            },
            &["app.terminate"],
        )
    }

    fn screenshot_plan() -> CompiledFlowPlanV2 {
        let screenshot = CompiledFlowNode {
            id: Uuid::new_v4(),
            kind: ActionKind::Screenshot,
            config: CompiledActionConfig::Screenshot {
                label: "capture.jpeg".to_string(),
                format: "jpeg".to_string(),
            },
            postcondition: Some(EvidenceSpec::ArtifactDecodedAndHashed),
        };
        plan(
            vec![launch_node(), screenshot],
            ContextPlan {
                requires_exclusive: true,
                requires_ui_session: true,
                requires_stream: true,
                requires_fresh_text_session: false,
                initial_bundle_id: Some(TARGET.to_string()),
            },
            &["app.launch", "stream"],
        )
    }

    fn tap_plan(image_width: u32, image_height: u32) -> CompiledFlowPlanV2 {
        let profile_id = qualified_geometry_profile_id(&capability_snapshot()).unwrap();
        let tap = CompiledFlowNode {
            id: Uuid::new_v4(),
            kind: ActionKind::Tap,
            config: CompiledActionConfig::Tap {
                target: CompiledTapTarget::Point {
                    target: ImageCoordinateTarget {
                        x: 1.0,
                        y: 1.0,
                        image_width,
                        image_height,
                        orientation: ScreenOrientation::Portrait,
                        profile_id,
                    },
                },
            },
            postcondition: Some(EvidenceSpec::FrameRegionChanged {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
                minimum_distance: 1,
            }),
        };
        plan(
            vec![launch_node(), tap],
            ContextPlan {
                requires_exclusive: true,
                requires_ui_session: true,
                requires_stream: true,
                requires_fresh_text_session: false,
                initial_bundle_id: Some(TARGET.to_string()),
            },
            &["app.launch", "ui.tap", "stream"],
        )
    }

    fn launch_node() -> CompiledFlowNode {
        CompiledFlowNode {
            id: Uuid::new_v4(),
            kind: ActionKind::LaunchApp,
            config: CompiledActionConfig::LaunchApp {
                bundle_id: TARGET.to_string(),
            },
            postcondition: Some(EvidenceSpec::ActiveAppEquals {
                bundle_id: TARGET.to_string(),
            }),
        }
    }

    fn plan(
        nodes: Vec<CompiledFlowNode>,
        context_plan: ContextPlan,
        required_capabilities: &[&str],
    ) -> CompiledFlowPlanV2 {
        let execution_order = nodes.iter().map(|node| node.id).collect::<Vec<_>>();
        let action_definition_versions = nodes
            .iter()
            .map(|node| (node.kind, 1))
            .collect::<BTreeMap<_, _>>();
        let nodes = nodes
            .into_iter()
            .map(|node| (node.id, node))
            .collect::<BTreeMap<_, _>>();
        CompiledFlowPlanV2 {
            schema_version: FLOW_SCHEMA_VERSION,
            flow_id: Uuid::new_v4(),
            revision: 1,
            nodes,
            execution_order,
            successors: Default::default(),
            context_plan,
            action_definition_versions,
            required_capabilities: required_capabilities
                .iter()
                .map(|capability| (*capability).to_string())
                .collect::<BTreeSet<_>>(),
        }
    }

    fn capability_snapshot() -> DeviceCapabilitySnapshot {
        DeviceCapabilitySnapshot {
            installed_agent: InstalledAgentIdentity {
                bundle_id: "com.fixture.agent".to_string(),
                version: "1.0".to_string(),
                build: "1".to_string(),
                executable_name: "FixtureRunner".to_string(),
                signer_identity_sha256: "a".repeat(64),
            },
            selected_artifact_sha256: "b".repeat(64),
            agent_version: "1.0".to_string(),
            protocol_version: 2,
            driver_adapter_version: "fixture-driver-1".to_string(),
            transport: ActiveTransport::Mock,
            product_type: "iPhone10,1".to_string(),
            ios_version: "16.7.15".to_string(),
            target_app: InstalledTargetIdentity {
                bundle_id: TARGET.to_string(),
                version: "1".to_string(),
                build: "1".to_string(),
            },
            protected_auth_ready: true,
            geometry: Some(QualifiedGeometry {
                logical_width: 4.0,
                logical_height: 4.0,
                pixel_width: 4,
                pixel_height: 4,
                scale_x: 1.0,
                scale_y: 1.0,
                orientation: ScreenOrientation::Portrait,
            }),
        }
    }

    fn jpeg(color: u8) -> Frame {
        let image = RgbImage::from_pixel(4, 4, Rgb([color, color, color]));
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(image)
            .write_to(&mut bytes, ImageFormat::Jpeg)
            .expect("encode fixture JPEG");
        Arc::new(bytes.into_inner())
    }
}
