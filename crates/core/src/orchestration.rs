use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{AutomationDefinitionRecord, AutomationKind, ResolvedTargetSnapshot, TargetRef};

pub const ORCHESTRATION_SCHEMA_VERSION: u32 = 1;
pub const MAX_ORCHESTRATION_DELAY_MS: u64 = 86_400_000;
const MAX_ORCHESTRATION_NODES: usize = 100;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrchestrationPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutomationProfileRef {
    pub definition_id: Uuid,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OrchestrationBranch {
    Done,
    Partial,
    Failed,
    Uncertain,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum OrchestrationNodeAction {
    Start,
    Delay {
        duration_ms: u64,
    },
    RunNurture {
        profile: AutomationProfileRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_override: Option<TargetRef>,
    },
    RunInteraction {
        profile: AutomationProfileRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_override: Option<TargetRef>,
    },
    RunPublish {
        profile: AutomationProfileRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_override: Option<TargetRef>,
    },
    End,
}

impl OrchestrationNodeAction {
    pub fn profile(&self) -> Option<&AutomationProfileRef> {
        match self {
            Self::RunNurture { profile, .. }
            | Self::RunInteraction { profile, .. }
            | Self::RunPublish { profile, .. } => Some(profile),
            Self::Start | Self::Delay { .. } | Self::End => None,
        }
    }

    pub fn target_override(&self) -> Option<&TargetRef> {
        match self {
            Self::RunNurture {
                target_override, ..
            }
            | Self::RunInteraction {
                target_override, ..
            }
            | Self::RunPublish {
                target_override, ..
            } => target_override.as_ref(),
            Self::Start | Self::Delay { .. } | Self::End => None,
        }
    }

    fn expected_kind(&self) -> Option<AutomationKind> {
        match self {
            Self::RunNurture { .. } => Some(AutomationKind::Nurture),
            Self::RunInteraction { .. } => Some(AutomationKind::Interaction),
            Self::RunPublish { .. } => Some(AutomationKind::Publish),
            Self::Start | Self::Delay { .. } | Self::End => None,
        }
    }

    fn accepts_branch(&self, branch: OrchestrationBranch) -> bool {
        match self {
            Self::Start | Self::Delay { .. } => branch == OrchestrationBranch::Done,
            Self::RunNurture { .. } | Self::RunInteraction { .. } | Self::RunPublish { .. } => true,
            Self::End => false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrchestrationNode {
    pub id: Uuid,
    pub position: OrchestrationPoint,
    #[serde(flatten)]
    pub action: OrchestrationNodeAction,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrchestrationEdge {
    pub source_node_id: Uuid,
    pub source_port: OrchestrationBranch,
    pub target_node_id: Uuid,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestrationDocumentV1 {
    pub schema_version: u32,
    pub id: Uuid,
    pub revision: u64,
    pub name: String,
    pub entry_node_id: Uuid,
    pub nodes: Vec<OrchestrationNode>,
    pub edges: Vec<OrchestrationEdge>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrchestrationIssue {
    pub code: String,
    pub node_id: Option<Uuid>,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledOrchestrationV1 {
    pub document: OrchestrationDocumentV1,
    pub execution_order: Vec<Uuid>,
    pub canonical_json: String,
    pub sha256: String,
    pub profiles: BTreeMap<Uuid, AutomationProfileRef>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrchestrationSummary {
    pub id: Uuid,
    pub name: String,
    pub latest_revision: u64,
    pub archived: bool,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrchestrationRevisionRecord {
    pub compiled: CompiledOrchestrationV1,
    pub created_at: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OrchestrationRunState {
    Queued,
    Running,
    Done,
    Partial,
    Failed,
    Uncertain,
    Cancelled,
}

impl OrchestrationRunState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Done | Self::Partial | Self::Failed | Self::Uncertain | Self::Cancelled
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OrchestrationAttemptState {
    Queued,
    Dispatching,
    WaitingChild,
    Done,
    Partial,
    Failed,
    Uncertain,
    Cancelled,
}

impl OrchestrationAttemptState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Done | Self::Partial | Self::Failed | Self::Uncertain | Self::Cancelled
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrchestrationRunRecord {
    pub id: Uuid,
    pub document_id: Uuid,
    pub document_revision: u64,
    pub document_sha256: String,
    pub target: ResolvedTargetSnapshot,
    pub node_targets: BTreeMap<Uuid, ResolvedTargetSnapshot>,
    pub state: OrchestrationRunState,
    pub current_node_id: Option<Uuid>,
    pub error_code: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrchestrationNurtureChildState {
    Dispatching,
    Running,
    Done,
    Partial,
    Failed,
    Uncertain,
}

impl OrchestrationNurtureChildState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Done | Self::Partial | Self::Failed | Self::Uncertain
        )
    }

    pub fn outcome(self) -> Option<ChildCampaignOutcome> {
        match self {
            Self::Dispatching | Self::Running => None,
            Self::Done => Some(ChildCampaignOutcome::Done),
            Self::Partial => Some(ChildCampaignOutcome::Partial),
            Self::Failed => Some(ChildCampaignOutcome::Failed),
            Self::Uncertain => Some(ChildCampaignOutcome::Uncertain),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrchestrationNurtureChildRecord {
    pub id: Uuid,
    pub attempt_id: Uuid,
    pub idempotency_key: String,
    pub run_id: Option<Uuid>,
    pub requested_udids: Vec<String>,
    pub started_udids: Vec<String>,
    pub state: OrchestrationNurtureChildState,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrchestrationAttemptRecord {
    pub snapshot: OrchestrationAttemptSnapshot,
    pub run_id: Uuid,
    pub attempt_no: u32,
    pub state: OrchestrationAttemptState,
    pub child_kind: Option<AutomationKind>,
    pub child_campaign_id: Option<Uuid>,
    pub branch: Option<OrchestrationBranch>,
    pub error_code: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrchestrationRunDetail {
    pub run: OrchestrationRunRecord,
    pub attempts: Vec<OrchestrationAttemptRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("orchestration {document_id} does not exist")]
pub struct OrchestrationNotFound {
    pub document_id: Uuid,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("orchestration revision conflict: expected {expected}, actual {actual}")]
pub struct OrchestrationRevisionConflict {
    pub expected: u64,
    pub actual: u64,
}

pub fn compile_orchestration(
    document: &OrchestrationDocumentV1,
    profiles: &[AutomationDefinitionRecord],
) -> Result<CompiledOrchestrationV1, Vec<OrchestrationIssue>> {
    let mut issues = Vec::new();
    let mut push = |code: &str, node_id: Option<Uuid>, message: String| {
        issues.push(OrchestrationIssue {
            code: code.into(),
            node_id,
            message,
        });
    };

    if document.schema_version != ORCHESTRATION_SCHEMA_VERSION {
        push(
            "SchemaVersionUnsupported",
            None,
            format!("schemaVersion {} is not supported", document.schema_version),
        );
    }
    if document.name.trim().is_empty() {
        push("NameEmpty", None, "name must not be empty".into());
    }
    if document.nodes.len() < 2 || document.nodes.len() > MAX_ORCHESTRATION_NODES {
        push(
            "NodeCountOutOfRange",
            None,
            format!("node count must be 2..={MAX_ORCHESTRATION_NODES}"),
        );
    }

    let mut node_map = HashMap::new();
    for node in &document.nodes {
        if node_map.insert(node.id, node).is_some() {
            push(
                "DuplicateNodeId",
                Some(node.id),
                "node id is duplicated".into(),
            );
        }
        if !node.position.x.is_finite() || !node.position.y.is_finite() {
            push(
                "InvalidPosition",
                Some(node.id),
                "node position must be finite".into(),
            );
        }
        if let OrchestrationNodeAction::Delay { duration_ms } = node.action {
            if !(1_000..=MAX_ORCHESTRATION_DELAY_MS).contains(&duration_ms) {
                push(
                    "DelayOutOfRange",
                    Some(node.id),
                    format!("delay must be 1000..={MAX_ORCHESTRATION_DELAY_MS} ms"),
                );
            }
        }
    }

    let starts: Vec<_> = document
        .nodes
        .iter()
        .filter(|node| matches!(node.action, OrchestrationNodeAction::Start))
        .collect();
    let ends: Vec<_> = document
        .nodes
        .iter()
        .filter(|node| matches!(node.action, OrchestrationNodeAction::End))
        .collect();
    if starts.len() != 1 {
        push(
            "StartCountInvalid",
            None,
            "document must contain exactly one Start".into(),
        );
    }
    if ends.len() != 1 {
        push(
            "EndCountInvalid",
            None,
            "document must contain exactly one End".into(),
        );
    }
    match node_map.get(&document.entry_node_id) {
        Some(node) if matches!(node.action, OrchestrationNodeAction::Start) => {}
        Some(_) => push(
            "EntryNotStart",
            Some(document.entry_node_id),
            "entry node must be Start".into(),
        ),
        None => push(
            "EntryMissing",
            Some(document.entry_node_id),
            "entry node does not exist".into(),
        ),
    }

    let mut profile_refs = BTreeMap::new();
    for node in &document.nodes {
        let (Some(profile_ref), Some(expected_kind)) =
            (node.action.profile(), node.action.expected_kind())
        else {
            continue;
        };
        profile_refs.insert(node.id, profile_ref.clone());
        let exact = profiles.iter().find(|record| {
            record.definition.id == profile_ref.definition_id
                && record.revision.revision == profile_ref.revision
        });
        let Some(record) = exact else {
            push(
                "ProfileRevisionMissing",
                Some(node.id),
                format!(
                    "profile {} revision {} does not exist",
                    profile_ref.definition_id, profile_ref.revision
                ),
            );
            continue;
        };
        if record.definition.archived {
            push(
                "ProfileArchived",
                Some(node.id),
                format!("profile {} is archived", profile_ref.definition_id),
            );
        }
        if record.definition.kind != expected_kind {
            push(
                "ProfileKindMismatch",
                Some(node.id),
                format!(
                    "profile kind {:?} does not match {:?}",
                    record.definition.kind, expected_kind
                ),
            );
        }
    }

    let mut edge_keys = HashSet::new();
    let mut indegree: HashMap<Uuid, usize> =
        document.nodes.iter().map(|node| (node.id, 0)).collect();
    let mut adjacency: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for edge in &document.edges {
        let Some(source) = node_map.get(&edge.source_node_id) else {
            push(
                "EdgeSourceMissing",
                Some(edge.source_node_id),
                "edge source does not exist".into(),
            );
            continue;
        };
        if !node_map.contains_key(&edge.target_node_id) {
            push(
                "EdgeTargetMissing",
                Some(edge.target_node_id),
                "edge target does not exist".into(),
            );
            continue;
        }
        if !source.action.accepts_branch(edge.source_port) {
            push(
                "InvalidSourcePort",
                Some(edge.source_node_id),
                format!("source does not expose {:?}", edge.source_port),
            );
        }
        if !edge_keys.insert((edge.source_node_id, edge.source_port)) {
            push(
                "DuplicateSourcePort",
                Some(edge.source_node_id),
                format!("source port {:?} has more than one edge", edge.source_port),
            );
        }
        adjacency
            .entry(edge.source_node_id)
            .or_default()
            .push(edge.target_node_id);
        if let Some(count) = indegree.get_mut(&edge.target_node_id) {
            *count += 1;
        }
    }

    for node in &document.nodes {
        let required_ports: &[OrchestrationBranch] = match node.action {
            OrchestrationNodeAction::Start | OrchestrationNodeAction::Delay { .. } => {
                &[OrchestrationBranch::Done]
            }
            OrchestrationNodeAction::RunNurture { .. }
            | OrchestrationNodeAction::RunInteraction { .. }
            | OrchestrationNodeAction::RunPublish { .. } => &[
                OrchestrationBranch::Done,
                OrchestrationBranch::Partial,
                OrchestrationBranch::Failed,
                OrchestrationBranch::Uncertain,
            ],
            OrchestrationNodeAction::End => &[],
        };
        for branch in required_ports {
            if !edge_keys.contains(&(node.id, *branch)) {
                push(
                    "RequiredSourcePortMissing",
                    Some(node.id),
                    format!("source must route its {branch:?} outcome"),
                );
            }
        }
    }

    for node in &document.nodes {
        if node.id != document.entry_node_id && indegree.get(&node.id).copied().unwrap_or(0) == 0 {
            push(
                "NodeUnreachable",
                Some(node.id),
                "node has no incoming path".into(),
            );
        }
    }
    let reachable = reachable_nodes(document.entry_node_id, &adjacency);
    for node in &document.nodes {
        if !reachable.contains(&node.id) {
            push(
                "NodeUnreachable",
                Some(node.id),
                "node is not reachable from Start".into(),
            );
        }
    }

    let execution_order = topological_order(&document.nodes, &adjacency, &indegree);
    if execution_order.len() != document.nodes.len() {
        push(
            "GraphCycle",
            None,
            "orchestration graph contains a cycle".into(),
        );
    }

    if !issues.is_empty() {
        return Err(issues);
    }
    let canonical_json = canonical_orchestration_document_json(document).map_err(|error| {
        vec![OrchestrationIssue {
            code: "CanonicalizationFailed".into(),
            node_id: None,
            message: error.to_string(),
        }]
    })?;
    let sha256 =
        orchestration_revision_sha256(&canonical_json, &profile_refs).map_err(|error| {
            vec![OrchestrationIssue {
                code: "CanonicalizationFailed".into(),
                node_id: None,
                message: error.to_string(),
            }]
        })?;
    Ok(CompiledOrchestrationV1 {
        document: document.clone(),
        execution_order,
        canonical_json,
        sha256,
        profiles: profile_refs,
    })
}

pub fn orchestration_revision_sha256(
    canonical_document_json: &str,
    profiles: &BTreeMap<Uuid, AutomationProfileRef>,
) -> anyhow::Result<String> {
    let material = serde_json::json!({
        "document": serde_json::from_str::<Value>(canonical_document_json)?,
        "profiles": profiles,
    });
    let canonical = serde_json::to_vec(&canonicalize(material))?;
    Ok(format!("{:x}", Sha256::digest(canonical)))
}

fn reachable_nodes(entry: Uuid, adjacency: &HashMap<Uuid, Vec<Uuid>>) -> HashSet<Uuid> {
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([entry]);
    while let Some(node_id) = queue.pop_front() {
        if !seen.insert(node_id) {
            continue;
        }
        if let Some(targets) = adjacency.get(&node_id) {
            queue.extend(targets.iter().copied());
        }
    }
    seen
}

fn topological_order(
    nodes: &[OrchestrationNode],
    adjacency: &HashMap<Uuid, Vec<Uuid>>,
    indegree: &HashMap<Uuid, usize>,
) -> Vec<Uuid> {
    let order: HashMap<Uuid, usize> = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id, index))
        .collect();
    let mut remaining = indegree.clone();
    let mut ready: Vec<Uuid> = nodes
        .iter()
        .filter(|node| remaining.get(&node.id).copied().unwrap_or(0) == 0)
        .map(|node| node.id)
        .collect();
    let mut result = Vec::with_capacity(nodes.len());
    while !ready.is_empty() {
        ready.sort_by_key(|node_id| order.get(node_id).copied().unwrap_or(usize::MAX));
        let node_id = ready.remove(0);
        result.push(node_id);
        if let Some(targets) = adjacency.get(&node_id) {
            for target in targets {
                let count = remaining.get_mut(target).expect("known target");
                *count -= 1;
                if *count == 0 {
                    ready.push(*target);
                }
            }
        }
    }
    result
}

pub fn canonical_orchestration_document_json(
    document: &OrchestrationDocumentV1,
) -> anyhow::Result<String> {
    let value = serde_json::to_value(document)?;
    Ok(serde_json::to_string(&canonicalize(value))?)
}

fn canonicalize(value: Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, value)| (key, canonicalize(value)))
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize).collect()),
        other => other,
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildCampaignOutcome {
    Done,
    Partial,
    Failed,
    Uncertain,
}

pub fn branch_for_child_outcome(outcome: ChildCampaignOutcome) -> OrchestrationBranch {
    match outcome {
        ChildCampaignOutcome::Done => OrchestrationBranch::Done,
        ChildCampaignOutcome::Partial => OrchestrationBranch::Partial,
        ChildCampaignOutcome::Failed => OrchestrationBranch::Failed,
        ChildCampaignOutcome::Uncertain => OrchestrationBranch::Uncertain,
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrchestrationAttemptSnapshot {
    pub document_id: Uuid,
    pub document_revision: u64,
    pub document_sha256: String,
    pub canonical_document_json: String,
    pub node_id: Uuid,
    pub attempt_id: Uuid,
    pub idempotency_key: String,
    pub profile: Option<AutomationProfileRef>,
    pub target: ResolvedTargetSnapshot,
}

pub fn snapshot_orchestration_attempt(
    compiled: &CompiledOrchestrationV1,
    node_id: Uuid,
    attempt_id: Uuid,
    target: ResolvedTargetSnapshot,
) -> anyhow::Result<OrchestrationAttemptSnapshot> {
    let node = compiled
        .document
        .nodes
        .iter()
        .find(|node| node.id == node_id)
        .ok_or_else(|| anyhow::anyhow!("orchestration node {node_id} does not exist"))?;
    let material = format!(
        "riviu-orchestration-attempt-v1:{}:{}:{}:{}",
        compiled.document.id, compiled.document.revision, node_id, attempt_id
    );
    Ok(OrchestrationAttemptSnapshot {
        document_id: compiled.document.id,
        document_revision: compiled.document.revision,
        document_sha256: compiled.sha256.clone(),
        canonical_document_json: compiled.canonical_json.clone(),
        node_id,
        attempt_id,
        idempotency_key: format!("{:x}", Sha256::digest(material.as_bytes())),
        profile: node.action.profile().cloned(),
        target,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use uuid::Uuid;

    use super::*;
    use crate::{
        AutomationDefinition, AutomationDefinitionRecord, AutomationDefinitionRevision,
        AutomationKind, ExcludedDeviceReason, ResolvedTargetDevice, ResolvedTargetExclusion,
        ResolvedTargetSnapshot, TargetRef,
    };

    const DOCUMENT_ID: &str = "10000000-0000-0000-0000-000000000001";
    const START_ID: &str = "10000000-0000-0000-0000-000000000002";
    const RUN_ID: &str = "10000000-0000-0000-0000-000000000003";
    const END_ID: &str = "10000000-0000-0000-0000-000000000004";
    const PROFILE_ID: &str = "10000000-0000-0000-0000-000000000005";

    fn id(value: &str) -> Uuid {
        Uuid::parse_str(value).expect("fixture UUID")
    }

    fn profile(kind: AutomationKind, revision: u64, archived: bool) -> AutomationDefinitionRecord {
        AutomationDefinitionRecord {
            definition: AutomationDefinition {
                id: id(PROFILE_ID),
                name: "Pinned profile".into(),
                kind,
                latest_revision: revision,
                archived,
                created_at: "2026-09-03T00:00:00Z".into(),
                updated_at: "2026-09-03T00:00:00Z".into(),
            },
            revision: AutomationDefinitionRevision {
                definition_id: id(PROFILE_ID),
                revision,
                target_ref: TargetRef::All,
                config: json!({"commentEnabled": false}),
                created_at: "2026-09-03T00:00:00Z".into(),
            },
        }
    }

    fn document(action: OrchestrationNodeAction) -> OrchestrationDocumentV1 {
        let is_campaign = matches!(
            &action,
            OrchestrationNodeAction::RunNurture { .. }
                | OrchestrationNodeAction::RunInteraction { .. }
                | OrchestrationNodeAction::RunPublish { .. }
        );
        let mut edges = vec![
            OrchestrationEdge {
                source_node_id: id(START_ID),
                source_port: OrchestrationBranch::Done,
                target_node_id: id(RUN_ID),
            },
            OrchestrationEdge {
                source_node_id: id(RUN_ID),
                source_port: OrchestrationBranch::Done,
                target_node_id: id(END_ID),
            },
        ];
        if is_campaign {
            edges.extend(
                [
                    OrchestrationBranch::Partial,
                    OrchestrationBranch::Failed,
                    OrchestrationBranch::Uncertain,
                ]
                .into_iter()
                .map(|source_port| OrchestrationEdge {
                    source_node_id: id(RUN_ID),
                    source_port,
                    target_node_id: id(END_ID),
                }),
            );
        }
        OrchestrationDocumentV1 {
            schema_version: ORCHESTRATION_SCHEMA_VERSION,
            id: id(DOCUMENT_ID),
            revision: 7,
            name: "Morning fleet".into(),
            entry_node_id: id(START_ID),
            nodes: vec![
                OrchestrationNode {
                    id: id(START_ID),
                    position: OrchestrationPoint { x: 0.0, y: 0.0 },
                    action: OrchestrationNodeAction::Start,
                },
                OrchestrationNode {
                    id: id(RUN_ID),
                    position: OrchestrationPoint { x: 240.0, y: 0.0 },
                    action,
                },
                OrchestrationNode {
                    id: id(END_ID),
                    position: OrchestrationPoint { x: 480.0, y: 0.0 },
                    action: OrchestrationNodeAction::End,
                },
            ],
            edges,
        }
    }

    fn interaction_action(revision: u64) -> OrchestrationNodeAction {
        OrchestrationNodeAction::RunInteraction {
            profile: AutomationProfileRef {
                definition_id: id(PROFILE_ID),
                revision,
            },
            target_override: None,
        }
    }

    #[test]
    fn document_is_a_distinct_schema_and_canonical_hash_is_stable() {
        let document = document(interaction_action(3));
        let first =
            compile_orchestration(&document, &[profile(AutomationKind::Interaction, 3, false)])
                .expect("valid orchestration");
        let second =
            compile_orchestration(&document, &[profile(AutomationKind::Interaction, 3, false)])
                .expect("repeat compile");

        let value = serde_json::to_value(&document).expect("serialize document");
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["nodes"][1]["kind"], "runInteraction");
        assert!(serde_json::from_value::<crate::FlowDocumentV2>(value).is_err());
        assert_eq!(first.canonical_json, second.canonical_json);
        assert_eq!(first.sha256, second.sha256);
        assert_eq!(
            first.execution_order,
            [id(START_ID), id(RUN_ID), id(END_ID)]
        );
        assert!(!first.canonical_json.contains("commentEnabled"));
    }

    #[test]
    fn campaign_nodes_reject_mixed_device_flow_fields_instead_of_dropping_them() {
        let document = document(interaction_action(3));
        let mut value = serde_json::to_value(document).expect("serialize orchestration");
        value["nodes"][1]["config"] = json!({"kind": "tap", "x": 100, "y": 200});
        value["nodes"][1]["deviceUdid"] = json!("phone-a");

        assert!(
            serde_json::from_value::<OrchestrationDocumentV1>(value).is_err(),
            "campaign nodes must not silently absorb low-level device-flow fields"
        );
    }

    #[test]
    fn profile_kind_revision_and_archive_are_fail_closed() {
        let document = document(interaction_action(3));
        for (profiles, code) in [
            (Vec::new(), "ProfileRevisionMissing"),
            (
                vec![profile(AutomationKind::Interaction, 2, false)],
                "ProfileRevisionMissing",
            ),
            (
                vec![profile(AutomationKind::Nurture, 3, false)],
                "ProfileKindMismatch",
            ),
            (
                vec![profile(AutomationKind::Interaction, 3, true)],
                "ProfileArchived",
            ),
        ] {
            let issues = compile_orchestration(&document, &profiles).expect_err("invalid profile");
            assert!(issues.iter().any(|issue| issue.code == code), "{issues:?}");
        }
    }

    #[test]
    fn delay_and_campaign_ports_are_validated() {
        let too_long = document(OrchestrationNodeAction::Delay {
            duration_ms: MAX_ORCHESTRATION_DELAY_MS + 1,
        });
        assert!(compile_orchestration(&too_long, &[])
            .expect_err("bounded delay")
            .iter()
            .any(|issue| issue.code == "DelayOutOfRange"));

        let mut wrong_port = document(interaction_action(3));
        wrong_port.edges[0].source_port = OrchestrationBranch::Uncertain;
        assert!(compile_orchestration(
            &wrong_port,
            &[profile(AutomationKind::Interaction, 3, false)]
        )
        .expect_err("start cannot branch uncertain")
        .iter()
        .any(|issue| issue.code == "InvalidSourcePort"));

        let mut missing_outcomes = document(interaction_action(3));
        missing_outcomes
            .edges
            .retain(|edge| edge.source_port == OrchestrationBranch::Done);
        let issues = compile_orchestration(
            &missing_outcomes,
            &[profile(AutomationKind::Interaction, 3, false)],
        )
        .expect_err("campaign must route every typed outcome");
        assert!(
            issues
                .iter()
                .any(|issue| issue.code == "RequiredSourcePortMissing"),
            "{issues:?}"
        );
    }

    #[test]
    fn attempt_snapshot_binds_target_profile_and_one_idempotency_key() {
        let compiled = compile_orchestration(
            &document(interaction_action(3)),
            &[profile(AutomationKind::Interaction, 3, false)],
        )
        .expect("valid orchestration");
        let snapshot = ResolvedTargetSnapshot {
            target_ref: TargetRef::Group {
                group_id: "warm".into(),
            },
            included: vec![ResolvedTargetDevice {
                udid: "phone-a".into(),
                alias: "Máy A".into(),
                number: Some(1),
            }],
            excluded: vec![ResolvedTargetExclusion {
                device: ResolvedTargetDevice {
                    udid: "phone-b".into(),
                    alias: "Máy B".into(),
                    number: Some(2),
                },
                reason: ExcludedDeviceReason::NotInRoster,
            }],
            roster_sha256: "a".repeat(64),
        };
        let attempt_id = id("10000000-0000-0000-0000-000000000006");
        let first =
            snapshot_orchestration_attempt(&compiled, id(RUN_ID), attempt_id, snapshot.clone())
                .expect("snapshot");
        let second = snapshot_orchestration_attempt(&compiled, id(RUN_ID), attempt_id, snapshot)
            .expect("repeat snapshot");

        assert_eq!(first.idempotency_key, second.idempotency_key);
        assert_eq!(first.profile.expect("profile").revision, 3);
        assert_eq!(first.target.included[0].number, Some(1));
        assert!(!first.canonical_document_json.contains("commentEnabled"));
    }

    #[test]
    fn child_outcomes_map_without_collapsing_partial_or_uncertain() {
        assert_eq!(
            branch_for_child_outcome(ChildCampaignOutcome::Done),
            OrchestrationBranch::Done
        );
        assert_eq!(
            branch_for_child_outcome(ChildCampaignOutcome::Partial),
            OrchestrationBranch::Partial
        );
        assert_eq!(
            branch_for_child_outcome(ChildCampaignOutcome::Failed),
            OrchestrationBranch::Failed
        );
        assert_eq!(
            branch_for_child_outcome(ChildCampaignOutcome::Uncertain),
            OrchestrationBranch::Uncertain
        );
    }
}
