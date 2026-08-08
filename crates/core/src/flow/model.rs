use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const FLOW_SCHEMA_VERSION: u32 = 2;

pub type FlowId = Uuid;
pub type NodeId = Uuid;
pub type EdgeId = Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowDocumentV2 {
    pub schema_version: u32,
    pub id: FlowId,
    pub name: String,
    pub revision: u64,
    pub entry_node_id: NodeId,
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
    pub viewport: FlowViewport,
}

impl FlowDocumentV2 {
    pub fn empty(name: impl Into<String>) -> Self {
        let mut start = FlowNode::new(ActionKind::Start, Value::Object(Default::default()));
        start.position = CanvasPoint { x: 0.0, y: 80.0 };
        let mut end = FlowNode::new(ActionKind::End, Value::Object(Default::default()));
        end.position = CanvasPoint { x: 320.0, y: 80.0 };
        let entry_node_id = start.id;
        let edge = FlowEdge::flow(start.id, end.id);

        Self {
            schema_version: FLOW_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            name: name.into(),
            revision: 0,
            entry_node_id,
            nodes: vec![start, end],
            edges: vec![edge],
            viewport: FlowViewport::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowNode {
    pub id: NodeId,
    pub kind: ActionKind,
    pub position: CanvasPoint,
    pub config: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postcondition: Option<EvidenceSpec>,
}

impl FlowNode {
    pub fn new(kind: ActionKind, config: Value) -> Self {
        Self {
            id: Uuid::new_v4(),
            kind,
            position: CanvasPoint::default(),
            config,
            postcondition: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowEdge {
    pub id: EdgeId,
    pub source_node_id: NodeId,
    pub source_port: String,
    pub target_node_id: NodeId,
    pub target_port: String,
}

impl FlowEdge {
    pub fn flow(source_node_id: NodeId, target_node_id: NodeId) -> Self {
        Self {
            id: Uuid::new_v4(),
            source_node_id,
            source_port: "flow".into(),
            target_node_id,
            target_port: "flow".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CanvasPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowViewport {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

impl Default for FlowViewport {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum ActionKind {
    Start,
    End,
    LaunchApp,
    TerminateApp,
    Wait,
    Tap,
    Swipe,
    TypeText,
    Screenshot,
    Home,
    AssertVisible,
    TapVision,
    IfVision,
    RawHttp,
    RawWda,
    Shell,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum EvidenceSpec {
    ActiveAppEquals {
        bundle_id: String,
    },
    ProcessAbsent {
        bundle_id: String,
    },
    FrameDigestChanged {
        minimum_distance: u32,
    },
    FrameRegionChanged {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        minimum_distance: u32,
    },
    QualifiedFramePredicate {
        detector_id: String,
    },
    AccessibilityVisible {
        accessibility_id: String,
    },
    TextReadBackEquals {
        locator: QualifiedElementLocator,
        value: String,
    },
    ArtifactDecodedAndHashed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ElementLocatorStrategy {
    AccessibilityId,
    ClassName,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualifiedElementLocator {
    pub strategy: ElementLocatorStrategy,
    pub value: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ResourceClass {
    PureDesktop,
    Bridge,
    UiSession,
    UiWithStream,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SideEffectClass {
    None,
    IdempotentSet,
    AmbiguousUi,
    ArtifactWrite,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceRequirement {
    None,
    ActiveApp,
    Process,
    Frame,
    TextOrQualifiedFrame,
    Artifact,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceKind {
    ActiveAppEquals,
    ProcessAbsent,
    FrameDigestChanged,
    FrameRegionChanged,
    QualifiedFramePredicate,
    AccessibilityVisible,
    TextReadBackEquals,
    ArtifactDecodedAndHashed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ReconciliationPolicy {
    None,
    ReadActiveApp,
    ReadProcess,
    ReadFrame,
    ReadText,
    ReadArtifact,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RetryPolicy {
    Never,
    BeforeDispatchOnly,
    IdempotentAfterRead,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextPlan {
    pub requires_exclusive: bool,
    pub requires_ui_session: bool,
    pub requires_stream: bool,
    pub requires_fresh_text_session: bool,
    pub initial_bundle_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompiledFlowPlanV2 {
    pub schema_version: u32,
    pub flow_id: FlowId,
    pub revision: u64,
    pub nodes: BTreeMap<NodeId, CompiledFlowNode>,
    /// A stable topological order over every node. Retained for deterministic
    /// recovery ordering, canonical hashing, and executing legacy linear plans.
    /// It no longer implies that all nodes run: with branches only the nodes on
    /// the taken path execute (see `successors`).
    pub execution_order: Vec<NodeId>,
    /// Explicit per-node adjacency keyed by output port (`flow` for linear nodes,
    /// `matched`/`notMatched` for `IfVision`). Empty for legacy plans compiled
    /// before branching existed — the executor then falls back to
    /// `execution_order`. `skip_serializing_if` keeps those legacy plans'
    /// canonical JSON (and thus their frozen plan hash) byte-identical.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub successors: BTreeMap<NodeId, BTreeMap<String, NodeId>>,
    pub context_plan: ContextPlan,
    pub action_definition_versions: BTreeMap<ActionKind, u32>,
    pub required_capabilities: BTreeSet<String>,
}

impl CompiledFlowPlanV2 {
    /// The Start node — always first in the topological order.
    pub fn entry_node(&self) -> Option<NodeId> {
        self.execution_order.first().copied()
    }

    /// The next node on the taken path after `node_id`. An `IfVision` node routes
    /// by `chosen_port` (`matched`/`notMatched`); every other kind uses its lone
    /// `flow` port. Legacy plans with no `successors` fall back to the linear
    /// `execution_order`. Returns `None` at `End`, a dead end, or an as-yet
    /// undecided branch (missing `chosen_port`).
    pub fn successor_on_path(&self, node_id: NodeId, chosen_port: Option<&str>) -> Option<NodeId> {
        if self.successors.is_empty() {
            let index = self.execution_order.iter().position(|id| *id == node_id)?;
            return self.execution_order.get(index + 1).copied();
        }
        let kind = self.nodes.get(&node_id)?.kind;
        if kind == ActionKind::End {
            return None;
        }
        let ports = self.successors.get(&node_id)?;
        match kind {
            ActionKind::IfVision => ports.get(chosen_port?).copied(),
            _ => ports.get("flow").copied(),
        }
    }

    /// Graph-predecessors of `node_id`: the source of every edge pointing at it.
    /// A rejoin node has several; only the branch actually taken will have run.
    /// Legacy plans fall back to the single linear predecessor.
    pub fn predecessors(&self, node_id: NodeId) -> Vec<NodeId> {
        if self.successors.is_empty() {
            let Some(index) = self.execution_order.iter().position(|id| *id == node_id) else {
                return Vec::new();
            };
            return index
                .checked_sub(1)
                .map(|previous| vec![self.execution_order[previous]])
                .unwrap_or_default();
        }
        self.successors
            .iter()
            .filter(|(_, ports)| ports.values().any(|target| *target == node_id))
            .map(|(source, _)| *source)
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowSummary {
    pub id: FlowId,
    pub name: String,
    pub latest_revision: u64,
    pub archived: bool,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowRevisionRecord {
    pub document: FlowDocumentV2,
    pub compiled_plan: CompiledFlowPlanV2,
    pub plan_hash: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FlowArtifactRecord {
    pub id: Uuid,
    pub attempt_id: Uuid,
    pub relative_path: String,
    pub label: String,
    pub kind: String,
    pub size: u64,
    pub sha256: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub enum FlowAttemptState {
    Queued,
    IntentCommitted,
    EffectDispatched,
    Verifying,
    Succeeded,
    FailedBeforeDispatch,
    FailedVerified,
    Uncertain,
    Cancelled,
    Interrupted,
}

impl FlowAttemptState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded
                | Self::FailedBeforeDispatch
                | Self::FailedVerified
                | Self::Uncertain
                | Self::Cancelled
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub enum FlowAggregateState {
    Queued,
    Running,
    Succeeded,
    Partial,
    Failed,
    Cancelled,
}

impl FlowAggregateState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Partial | Self::Failed | Self::Cancelled
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase")]
pub enum FlowDeviceRunState {
    Queued,
    Preflight,
    Running,
    Succeeded,
    Failed,
    Skipped,
    Cancelled,
}

impl FlowDeviceRunState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Skipped | Self::Cancelled
        )
    }

    pub fn is_success(self) -> bool {
        self == Self::Succeeded
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "camelCase", deny_unknown_fields)]
pub enum FlowTargetSelection {
    One { udid: String },
    Selected { udids: Vec<String> },
    AllEligible,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowSelectionSnapshot {
    pub requested: FlowTargetSelection,
    pub target_udids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowErrorRecord {
    pub code: String,
    pub message: String,
    pub node_id: Option<NodeId>,
    pub field: Option<String>,
    pub udid: Option<String>,
    pub attempt_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowContextReleaseProof {
    pub udid: String,
    pub owner: crate::DeviceWorkOwner,
    pub had_session: bool,
    pub had_stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum FlowPreflightScope {
    TargetFree,
    TargetQualified { bundle_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowCapabilitySnapshot {
    pub scope: FlowPreflightScope,
    pub device: Option<crate::DeviceCapabilitySnapshot>,
    pub agent_status: Option<crate::AgentStatus>,
    pub capability_ids: BTreeSet<String>,
}

impl FlowCapabilitySnapshot {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self
            .capability_ids
            .iter()
            .any(|id| id.is_empty() || id.trim() != id || id.chars().any(char::is_control))
        {
            return Err("Flow capability ID is invalid");
        }
        match (&self.scope, &self.device, &self.agent_status) {
            (FlowPreflightScope::TargetFree, None, None) if self.capability_ids.is_empty() => {
                Ok(())
            }
            (FlowPreflightScope::TargetFree, _, _) => {
                Err("target-free Flow preflight cannot claim a target or capability")
            }
            (FlowPreflightScope::TargetQualified { bundle_id }, Some(device), agent_status)
                if !bundle_id.is_empty()
                    && bundle_id.trim() == bundle_id
                    && !bundle_id.chars().any(char::is_control)
                    && device.target_app.bundle_id == *bundle_id
                    && agent_status.as_ref().is_none_or(|status| {
                        !status.udid.is_empty()
                            && status.udid.trim() == status.udid
                            && status.features.iter().all(|feature| {
                                !feature.is_empty()
                                    && feature.trim() == feature
                                    && !feature.chars().any(char::is_control)
                            })
                    }) =>
            {
                Ok(())
            }
            (FlowPreflightScope::TargetQualified { .. }, _, _) => {
                Err("target-qualified Flow preflight must bind its exact device target")
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowRunRecord {
    pub id: Uuid,
    pub flow_id: FlowId,
    pub flow_revision: u64,
    pub plan_sha256: String,
    pub selection: FlowSelectionSnapshot,
    pub state: FlowAggregateState,
    pub event_revision: u64,
    pub error: Option<FlowErrorRecord>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowDeviceRunRecord {
    pub id: Uuid,
    pub run_id: Uuid,
    pub udid: String,
    pub state: FlowDeviceRunState,
    pub capability_snapshot: Option<FlowCapabilitySnapshot>,
    pub release_proof: Option<FlowContextReleaseProof>,
    pub error: Option<FlowErrorRecord>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowNodeAttemptRecord {
    pub id: Uuid,
    pub device_run_id: Uuid,
    pub node_id: NodeId,
    pub action_kind: ActionKind,
    pub attempt_no: u32,
    pub side_effect_class: SideEffectClass,
    pub state: FlowAttemptState,
    pub canonical_input: Option<Value>,
    pub evidence_baseline: Option<Value>,
    pub evidence_result: Option<Value>,
    /// For an `IfVision` node, the output port the runtime match selected
    /// (`matched`/`notMatched`). First-class so recovery can rebuild the taken
    /// path without re-running the vision predicate. `None` for every other kind
    /// and for attempts recorded before branching existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chosen_port: Option<String>,
    pub retry_allowed: bool,
    pub error: Option<FlowErrorRecord>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowEventRecord {
    pub id: i64,
    pub run_id: Uuid,
    pub revision: u64,
    pub kind: String,
    pub payload: Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowRunDetail {
    pub run: FlowRunRecord,
    pub device_runs: Vec<FlowDeviceRunRecord>,
    pub attempts: Vec<FlowNodeAttemptRecord>,
    pub artifacts: Vec<FlowArtifactRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("expected revision {expected}, actual revision {actual}")]
pub struct RevisionConflict {
    pub expected: u64,
    pub actual: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("flow {flow_id} does not exist")]
pub struct FlowNotFound {
    pub flow_id: FlowId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowArchiveMutation {
    pub flow_id: FlowId,
    pub document_revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompiledFlowNode {
    pub id: NodeId,
    pub kind: ActionKind,
    pub config: CompiledActionConfig,
    pub postcondition: Option<EvidenceSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageCoordinateTarget {
    pub x: f64,
    pub y: f64,
    pub image_width: u32,
    pub image_height: u32,
    pub orientation: crate::ScreenOrientation,
    pub profile_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CompiledActionConfig {
    Empty,
    LaunchApp {
        bundle_id: String,
    },
    TerminateApp {
        bundle_id: String,
    },
    Wait {
        duration_ms: u64,
    },
    Tap {
        target: CompiledTapTarget,
    },
    Swipe {
        from: ImageCoordinateTarget,
        to: ImageCoordinateTarget,
        duration_ms: u64,
    },
    TypeText {
        text: String,
        read_back_locator: QualifiedElementLocator,
    },
    Screenshot {
        label: String,
        format: String,
    },
    AssertVisible {
        accessibility_id: String,
    },
    TapVision {
        /// Template image (a small crop of the target), PNG, base64-encoded.
        template_png_base64: String,
        /// Match threshold in [0,1]; the NCC score must reach it to tap.
        threshold: f64,
        /// Optional search region (screen fractions) to speed up and disambiguate.
        region: Option<VisionRegion>,
    },
    IfVision {
        /// Template image (a small crop of the target), PNG, base64-encoded.
        template_png_base64: String,
        /// Match threshold in [0,1]; a score reaching it routes to `matched`.
        threshold: f64,
        /// Optional search region (screen fractions) to speed up and disambiguate.
        region: Option<VisionRegion>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "mode",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CompiledTapTarget {
    Point { target: ImageCoordinateTarget },
    AccessibilityId { value: String },
}

/// A rectangular search region for a vision node, in screen fractions (0..=1).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisionRegion {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

/// Maximum base64-encoded vision template size (~256 KB decoded).
pub const MAX_VISION_TEMPLATE_ENCODED_BYTES: usize = 350_000;

/// Decode a base64 PNG vision template into an RGB image, enforcing a size cap.
/// Shared by the Flow compiler (fail-fast validation) and the executor (runtime
/// match), so both agree on exactly what a valid template is.
pub fn decode_vision_template(base64_png: &str) -> Result<image::RgbImage, String> {
    use base64::Engine as _;
    if base64_png.is_empty() {
        return Err("vision template is empty".to_string());
    }
    if base64_png.len() > MAX_VISION_TEMPLATE_ENCODED_BYTES {
        return Err("vision template exceeds the size limit".to_string());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_png)
        .map_err(|error| format!("vision template is not valid base64: {error}"))?;
    let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        .map_err(|error| format!("vision template is not a valid PNG: {error}"))?;
    Ok(decoded.to_rgb8())
}

/// Validate a vision search region: coordinates finite, within [0,1], and forming
/// a positive-area box. Shared by the compiler.
pub fn validate_vision_region(region: &VisionRegion) -> Result<(), String> {
    let all_finite = region.x0.is_finite()
        && region.y0.is_finite()
        && region.x1.is_finite()
        && region.y1.is_finite();
    if !all_finite {
        return Err("region coordinates must be finite".to_string());
    }
    let in_unit = [region.x0, region.y0, region.x1, region.y1]
        .iter()
        .all(|value| (0.0..=1.0).contains(value));
    if !in_unit {
        return Err("region coordinates must be within 0..=1".to_string());
    }
    if region.x0 >= region.x1 || region.y0 >= region.y1 {
        return Err("region must have positive width and height".to_string());
    }
    Ok(())
}

pub fn qualified_geometry_profile_id(
    snapshot: &crate::DeviceCapabilitySnapshot,
) -> Result<String, &'static str> {
    let geometry = snapshot.geometry.as_ref().ok_or("geometry is absent")?;
    let dimensions_are_valid = geometry.pixel_width > 0
        && geometry.pixel_height > 0
        && geometry.logical_width.is_finite()
        && geometry.logical_height.is_finite()
        && geometry.scale_x.is_finite()
        && geometry.scale_y.is_finite()
        && geometry.logical_width > 0.0
        && geometry.logical_height > 0.0
        && geometry.scale_x > 0.0
        && geometry.scale_y > 0.0;
    if !dimensions_are_valid {
        return Err("geometry is invalid");
    }

    let material = serde_json::json!({
        "selectedArtifactSha256": &snapshot.selected_artifact_sha256,
        "installedAgent": &snapshot.installed_agent,
        "agentVersion": &snapshot.agent_version,
        "protocolVersion": snapshot.protocol_version,
        "driverAdapterVersion": &snapshot.driver_adapter_version,
        "transport": snapshot.transport,
        "productType": &snapshot.product_type,
        "iosVersion": &snapshot.ios_version,
        "targetApp": &snapshot.target_app,
        "geometry": geometry,
    });
    let bytes = serde_json::to_vec(&canonicalize_json(material))
        .map_err(|_| "geometry serialization failed")?;
    Ok(hex_sha256(&bytes))
}

fn canonicalize_json(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let sorted: BTreeMap<_, _> = object
                .into_iter()
                .map(|(key, value)| (key, canonicalize_json(value)))
                .collect();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_json).collect()),
        scalar => scalar,
    }
}

pub fn canonical_compiled_plan_json(
    plan: &CompiledFlowPlanV2,
) -> Result<String, serde_json::Error> {
    let value = serde_json::to_value(plan)?;
    serde_json::to_string(&canonicalize_json(value))
}

pub fn canonical_execution_hash_material_json(
    plan: &CompiledFlowPlanV2,
) -> Result<String, serde_json::Error> {
    let mut value = serde_json::to_value(plan)?;
    value
        .as_object_mut()
        .expect("compiled plan is an object")
        .remove("revision");
    serde_json::to_string(&canonicalize_json(value))
}

pub fn compiled_plan_sha256(plan: &CompiledFlowPlanV2) -> Result<String, serde_json::Error> {
    let canonical = canonical_execution_hash_material_json(plan)?;
    Ok(hex_sha256(canonical.as_bytes()))
}

pub fn validate_artifact_label(label: &str, format: &str) -> Result<(), &'static str> {
    if label.trim() != label || label.is_empty() || label.chars().count() > 96 {
        return Err("ArtifactLabelLength");
    }
    if label
        .chars()
        .any(|character| character.is_control() || character == '/' || character == '\\')
        || label.contains("..")
    {
        return Err("ArtifactLabelCharacters");
    }

    let extension = label
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase());
    let windows_stem = label
        .split_once('.')
        .map_or(label, |(windows_stem, _)| windows_stem);
    let upper = windows_stem.to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"];
    if windows_stem.is_empty()
        || reserved.contains(&upper.as_str())
        || (upper.len() == 4
            && upper.is_ascii()
            && matches!(&upper[..3], "COM" | "LPT")
            && matches!(
                &upper[3..],
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"
            ))
    {
        return Err("ArtifactLabelReserved");
    }

    match (format, extension.as_deref()) {
        ("jpeg", None | Some("jpg") | Some("jpeg")) | ("png", None | Some("png")) => Ok(()),
        ("jpeg" | "png", Some(_)) => Err("ArtifactLabelExtension"),
        _ => Err("ArtifactFormat"),
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
