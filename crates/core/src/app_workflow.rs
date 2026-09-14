//! Application-level authoring graph. Native steps keep engine contracts while
//! the graph owns their order, branches, configuration and stable identities.
use crate::AutomationKind;
use anyhow::{ensure, Context};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppWorkflowNode {
    pub id: Uuid,
    pub action: String,
    pub position: crate::CanvasPoint,
    #[serde(default)]
    pub config: BTreeMap<String, Value>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppWorkflowEdge {
    pub id: Uuid,
    pub source: Uuid,
    pub port: String,
    pub target: Uuid,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppWorkflowV1 {
    pub schema_version: u8,
    pub id: Uuid,
    pub revision: u64,
    pub name: String,
    pub kind: AutomationKind,
    pub entry_node_id: Uuid,
    pub nodes: Vec<AppWorkflowNode>,
    pub edges: Vec<AppWorkflowEdge>,
    #[serde(default)]
    pub viewport: crate::FlowViewport,
    pub profile_config: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppWorkflowSummary {
    pub id: Uuid,
    pub name: String,
    pub kind: AutomationKind,
    pub latest_revision: u64,
    pub updated_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStepDefinition {
    pub action: String,
    pub label: String,
    pub category: String,
    pub ports: Vec<String>,
    pub default_config: BTreeMap<String, Value>,
}

pub fn app_step_catalog(kind: AutomationKind) -> Vec<AppStepDefinition> {
    let mut defs = vec![
        ("start", "Bắt đầu", "Điều khiển", json!({})),
        ("end", "Kết thúc", "Điều khiển", json!({})),
        ("wait", "Chờ", "Điều khiển", json!({"durationMs":1000})),
        (
            "setVariable",
            "Gán biến",
            "Dữ liệu",
            json!({"name":"value","value":""}),
        ),
        (
            "ifValue",
            "So sánh biến",
            "Dữ liệu",
            json!({"name":"value","equals":""}),
        ),
        ("log", "Ghi nhật ký", "Dữ liệu", json!({"message":""})),
    ];
    match kind {
        AutomationKind::Nurture => defs.extend([
            ("openApp", "Mở TikTok", "Thiết bị", json!({})),
            ("prepareFeed", "Chuẩn bị bảng tin", "Nuôi TikTok", json!({})),
            ("readContent", "Đọc nội dung", "Nuôi TikTok", json!({})),
            (
                "watch",
                "Xem nội dung",
                "Nuôi TikTok",
                json!({"minimumSeconds":3,"maximumSeconds":9}),
            ),
            (
                "like",
                "Thích bài",
                "Nuôi TikTok",
                json!({"probability":35}),
            ),
            ("save", "Lưu bài", "Nuôi TikTok", json!({"probability":0})),
            (
                "comment",
                "Bình luận",
                "Nuôi TikTok",
                json!({"probability":0}),
            ),
            (
                "follow",
                "Theo dõi",
                "Nuôi TikTok",
                json!({"probability":0}),
            ),
            ("nextPost", "Chuyển bài", "Nuôi TikTok", json!({})),
            (
                "repeatFeed",
                "Lặp bảng tin",
                "Điều khiển",
                json!({"maximumPosts":10,"durationMinutes":30}),
            ),
        ]),
        AutomationKind::Interaction => defs.extend([
            ("loadTargets", "Đọc danh sách bài", "Tương tác", json!({})),
            (
                "assignRoles",
                "Phân vai và thiết bị",
                "Tương tác",
                json!({}),
            ),
            ("openTarget", "Mở bài đích", "Tương tác", json!({})),
            ("findParent", "Tìm bình luận gốc", "Tương tác", json!({})),
            (
                "resolveMentions",
                "Chọn đúng tài khoản tag",
                "Tương tác",
                json!({}),
            ),
            (
                "compose",
                "Soạn nội dung",
                "Tương tác",
                json!({"maximumWords":12}),
            ),
            ("send", "Gửi bình luận", "Tương tác", json!({})),
            (
                "verifyComment",
                "Xác minh bình luận",
                "Tương tác",
                json!({}),
            ),
            ("like", "Thích bài", "Tương tác", json!({})),
            ("save", "Lưu bài", "Tương tác", json!({})),
            ("nextTarget", "Lượt tiếp theo", "Điều khiển", json!({})),
        ]),
        AutomationKind::Publish => defs.extend([
            ("loadSource", "Đọc nguồn đăng", "Đăng bài", json!({})),
            ("assignDevices", "Phân công máy", "Đăng bài", json!({})),
            (
                "preflight",
                "Kiểm tra trước khi đăng",
                "Đăng bài",
                json!({}),
            ),
            ("transfer", "Chuyển ảnh và video", "Đăng bài", json!({})),
            ("compose", "Soạn bài", "Đăng bài", json!({})),
            ("selectSound", "Chọn nhạc", "Đăng bài", json!({})),
            ("post", "Đăng bài", "Đăng bài", json!({})),
            ("verifyPost", "Xác minh bài đăng", "Đăng bài", json!({})),
            ("captureLink", "Lấy liên kết", "Đăng bài", json!({})),
            ("writeSheet", "Ghi kết quả lên Sheet", "Đăng bài", json!({})),
            ("cleanup", "Dọn tài nguyên", "Đăng bài", json!({})),
        ]),
    }
    defs.into_iter()
        .map(|(action, label, category, config)| AppStepDefinition {
            action: action.into(),
            label: label.into(),
            category: category.into(),
            ports: match action {
                "end" => vec![],
                "ifValue" => vec!["matched".into(), "notMatched".into()],
                _ => vec!["done".into()],
            },
            default_config: serde_json::from_value(config).expect("literal object"),
        })
        .collect()
}

pub fn new_app_workflow(kind: AutomationKind) -> AppWorkflowV1 {
    let catalog = app_step_catalog(kind);
    let sequence: &[&str] = match kind {
        AutomationKind::Nurture => &[
            "start",
            "openApp",
            "prepareFeed",
            "readContent",
            "watch",
            "like",
            "save",
            "comment",
            "follow",
            "nextPost",
            "repeatFeed",
            "end",
        ],
        AutomationKind::Interaction => &[
            "start",
            "loadTargets",
            "assignRoles",
            "openTarget",
            "findParent",
            "resolveMentions",
            "compose",
            "send",
            "verifyComment",
            "nextTarget",
            "end",
        ],
        AutomationKind::Publish => &[
            "start",
            "loadSource",
            "assignDevices",
            "preflight",
            "transfer",
            "compose",
            "selectSound",
            "post",
            "verifyPost",
            "captureLink",
            "writeSheet",
            "cleanup",
            "end",
        ],
    };
    let nodes: Vec<_> = sequence
        .iter()
        .enumerate()
        .map(|(i, action)| AppWorkflowNode {
            id: Uuid::new_v4(),
            action: (*action).into(),
            position: crate::CanvasPoint {
                x: 80.0 + (i % 4) as f64 * 260.0,
                y: 80.0 + (i / 4) as f64 * 150.0,
            },
            config: catalog
                .iter()
                .find(|def| def.action == *action)
                .unwrap()
                .default_config
                .clone(),
        })
        .collect();
    let edges = nodes
        .windows(2)
        .map(|pair| AppWorkflowEdge {
            id: Uuid::new_v4(),
            source: pair[0].id,
            port: "done".into(),
            target: pair[1].id,
        })
        .collect();
    AppWorkflowV1 {
        schema_version: 1,
        id: Uuid::new_v4(),
        revision: 0,
        name: match kind {
            AutomationKind::Nurture => "Nuôi TikTok",
            AutomationKind::Interaction => "Tương tác",
            AutomationKind::Publish => "Đăng bài",
        }
        .into(),
        kind,
        entry_node_id: nodes[0].id,
        nodes,
        edges,
        viewport: Default::default(),
        profile_config: crate::three_feature_config_for(kind),
    }
}

pub fn validate_app_workflow(doc: &AppWorkflowV1) -> anyhow::Result<()> {
    ensure!(doc.schema_version == 1, "Unsupported app workflow schema");
    ensure!(
        !doc.name.trim().is_empty() && doc.name.len() <= 180,
        "Workflow name is required"
    );
    ensure!(
        (2..=256).contains(&doc.nodes.len()),
        "Workflow must contain 2-256 nodes"
    );
    ensure!(doc.edges.len() <= 512, "Too many workflow edges");
    crate::validate_automation_profile_config(doc.kind, &doc.profile_config)?;
    let defs = app_step_catalog(doc.kind);
    let nodes: BTreeMap<_, _> = doc.nodes.iter().map(|node| (node.id, node)).collect();
    ensure!(nodes.len() == doc.nodes.len(), "Duplicate node identity");
    ensure!(
        nodes
            .get(&doc.entry_node_id)
            .is_some_and(|node| node.action == "start"),
        "Entry must be a Start node"
    );
    ensure!(
        doc.nodes
            .iter()
            .filter(|node| node.action == "start")
            .count()
            == 1,
        "Exactly one Start is required"
    );
    ensure!(
        doc.nodes.iter().any(|node| node.action == "end"),
        "End is required"
    );
    for node in &doc.nodes {
        ensure!(
            node.position.x.is_finite() && node.position.y.is_finite(),
            "Invalid node position"
        );
        let def = defs
            .iter()
            .find(|def| def.action == node.action)
            .context("Unknown app action")?;
        crate::validate_automation_config(&serde_json::to_value(&node.config)?)?;
        if let Some(value) = node.config.get("probability") {
            ensure!(
                value.as_u64().is_some_and(|n| n <= 100),
                "Probability must be 0-100"
            );
        }
        if node.action == "wait" {
            ensure!(
                node.config
                    .get("durationMs")
                    .and_then(Value::as_u64)
                    .is_some_and(|n| (1000..=60000).contains(&n)),
                "Wait must be 1000-60000ms"
            );
        }
        for port in &def.ports {
            ensure!(
                doc.edges
                    .iter()
                    .filter(|edge| edge.source == node.id && edge.port == *port)
                    .count()
                    == 1,
                "Connect each output port exactly once: {} / {}",
                node.action,
                port
            );
        }
    }
    let mut edge_ids = BTreeSet::new();
    for edge in &doc.edges {
        ensure!(edge_ids.insert(edge.id), "Duplicate edge identity");
        let source = nodes.get(&edge.source).context("Edge source missing")?;
        ensure!(nodes.contains_key(&edge.target), "Edge target missing");
        ensure!(
            defs.iter()
                .find(|def| def.action == source.action)
                .unwrap()
                .ports
                .contains(&edge.port),
            "Invalid output port"
        );
    }
    fn visit(
        id: Uuid,
        doc: &AppWorkflowV1,
        seen: &mut BTreeSet<Uuid>,
        stack: &mut BTreeSet<Uuid>,
    ) -> anyhow::Result<()> {
        ensure!(
            !stack.contains(&id),
            "Use a bounded repeat step instead of a graph cycle"
        );
        if !seen.insert(id) {
            return Ok(());
        }
        stack.insert(id);
        for edge in doc.edges.iter().filter(|edge| edge.source == id) {
            visit(edge.target, doc, seen, stack)?;
        }
        stack.remove(&id);
        Ok(())
    }
    let mut seen = BTreeSet::new();
    visit(doc.entry_node_id, doc, &mut seen, &mut BTreeSet::new())?;
    ensure!(seen.len() == nodes.len(), "Disconnected app steps");
    Ok(())
}

/// Compile the domain stages into the immutable native engine input. Mandatory
/// stages preserve each engine's effect protocol; editable actions are applied
/// to its real settings rather than being a second, decorative graph.
pub fn compile_app_profile(doc: &AppWorkflowV1) -> anyhow::Result<Value> {
    validate_app_workflow(doc)?;
    let mut profile = doc.profile_config.clone();
    let mut cursor = doc.entry_node_id;
    let mut ordered = Vec::new();
    let mut variables: BTreeMap<String, String> = BTreeMap::new();
    for _ in 0..=doc.nodes.len() {
        let node = doc
            .nodes
            .iter()
            .find(|n| n.id == cursor)
            .context("Node missing")?;
        if node.action == "end" {
            break;
        }
        let mut port = "done";
        match node.action.as_str() {
            "setVariable" => {
                variables.insert(
                    node.config
                        .get("name")
                        .and_then(Value::as_str)
                        .context("Variable name missing")?
                        .to_string(),
                    node.config
                        .get("value")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                );
            }
            "ifValue" => {
                let name = node
                    .config
                    .get("name")
                    .and_then(Value::as_str)
                    .context("Variable name missing")?;
                let value = variables
                    .get(name)
                    .context("Variable must be assigned before comparison")?;
                port = if value
                    == node
                        .config
                        .get("equals")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                {
                    "matched"
                } else {
                    "notMatched"
                };
            }
            "start" => {}
            _ => ordered.push(node),
        }
        cursor = doc
            .edges
            .iter()
            .find(|e| e.source == cursor && e.port == port)
            .context("Output is disconnected")?
            .target;
    }
    let actions: Vec<&str> = ordered.iter().map(|n| n.action.as_str()).collect();
    let required: &[&str] = match doc.kind {
        AutomationKind::Nurture => &[
            "openApp",
            "prepareFeed",
            "readContent",
            "watch",
            "nextPost",
            "repeatFeed",
        ],
        AutomationKind::Interaction => &[
            "loadTargets",
            "assignRoles",
            "openTarget",
            "findParent",
            "resolveMentions",
            "compose",
            "send",
            "verifyComment",
            "nextTarget",
        ],
        AutomationKind::Publish => &[
            "loadSource",
            "assignDevices",
            "preflight",
            "transfer",
            "compose",
            "selectSound",
            "post",
            "verifyPost",
            "captureLink",
        ],
    };
    let mut previous = None;
    for action in required {
        let at = actions
            .iter()
            .position(|candidate| candidate == action)
            .with_context(|| format!("Required step is missing: {action}"))?;
        ensure!(
            previous.is_none_or(|last| at > last),
            "Step order violates required preparation or verification: {action}"
        );
        previous = Some(at);
        ensure!(
            actions
                .iter()
                .filter(|candidate| *candidate == action)
                .count()
                == 1,
            "Required step must occur once: {action}"
        );
    }
    // Auxiliary steps may surround a native pipeline; they never split its
    // preparation/effect/verification transaction. The caller executes them as
    // durable orchestration delays, preserving their authored position.
    let first_native = actions.iter().position(|a| required.contains(a)).unwrap();
    let last_native = actions
        .iter()
        .rposition(|a| !matches!(*a, "wait" | "log"))
        .unwrap();
    ensure!(
        !actions[first_native..=last_native]
            .iter()
            .any(|a| matches!(*a, "wait" | "log")),
        "Place waits and logs before preparation or after the final native step"
    );
    match doc.kind {
        AutomationKind::Nurture => {
            let watch = ordered.iter().find(|n| n.action == "watch").unwrap();
            let min = watch
                .config
                .get("minimumSeconds")
                .and_then(Value::as_f64)
                .context("Minimum watch duration missing")?;
            let max = watch
                .config
                .get("maximumSeconds")
                .and_then(Value::as_f64)
                .context("Maximum watch duration missing")?;
            ensure!(
                min >= 0.25 && max >= min && max <= 3600.0,
                "Invalid watch duration"
            );
            let repeat = ordered.iter().find(|n| n.action == "repeatFeed").unwrap();
            let count = repeat
                .config
                .get("maximumPosts")
                .and_then(Value::as_u64)
                .context("Post limit missing")?;
            let minutes = repeat
                .config
                .get("durationMinutes")
                .and_then(Value::as_u64)
                .context("Duration missing")?;
            ensure!(
                (1..=10000).contains(&count) && (1..=1440).contains(&minutes),
                "Invalid feed limits"
            );
            profile["durationMinutes"] = json!(minutes);
            profile["settings"]["numVideos"] = json!(count);
            profile["settings"]["numRounds"] = json!(1);
            profile["settings"]["watchMin"] = json!(min);
            profile["settings"]["watchMax"] = json!(max);
            let watch_at = actions.iter().position(|a| *a == "watch").unwrap();
            let next_at = actions.iter().position(|a| *a == "nextPost").unwrap();
            let mut order = Vec::new();
            for (action, enabled, prob) in [
                ("like", "likeEnabled", "likeProb"),
                ("save", "saveEnabled", "saveProb"),
                ("comment", "commentEnabled", "commentProb"),
                ("follow", "followEnabled", "followProb"),
            ] {
                let step = ordered.iter().enumerate().find(|(_, n)| n.action == action);
                profile["settings"][enabled] = json!(step.is_some());
                if let Some((at, node)) = step {
                    ensure!(
                        at > watch_at && at < next_at,
                        "Place {action} after Watch and before Next Post"
                    );
                    ensure!(
                        actions
                            .iter()
                            .filter(|candidate| **candidate == action)
                            .count()
                            == 1,
                        "Duplicate action: {action}"
                    );
                    profile["settings"][prob] = node
                        .config
                        .get("probability")
                        .cloned()
                        .context("Action probability missing")?;
                    if action != "follow" {
                        order.push((at, action));
                    } else {
                        ensure!(
                            !actions[at + 1..next_at]
                                .iter()
                                .any(|a| matches!(*a, "like" | "save" | "comment")),
                            "Follow must finish the current card's actions"
                        );
                    }
                }
            }
            order.sort_by_key(|(at, _)| *at);
            profile["settings"]["workflowActionOrder"] = json!(order
                .into_iter()
                .map(|(_, action)| action)
                .collect::<Vec<_>>());
        }
        AutomationKind::Interaction => {
            let target_at = actions.iter().position(|a| *a == "openTarget").unwrap();
            let parent_at = actions.iter().position(|a| *a == "findParent").unwrap();
            let mut last = target_at;
            for action in ["like", "save"] {
                if let Some(at) = actions.iter().position(|a| *a == action) {
                    ensure!(
                        at > last && at < parent_at,
                        "Place Like then Save after Open Target and before finding the parent"
                    );
                    last = at;
                }
            }
            profile["request"]["actions"]["comment"] = json!(actions.contains(&"send"));
            profile["request"]["actions"]["like"] = json!(actions.contains(&"like"));
            profile["request"]["actions"]["save"] = json!(actions.contains(&"save"));
            let compose = ordered.iter().find(|n| n.action == "compose").unwrap();
            let max = compose
                .config
                .get("maximumWords")
                .and_then(Value::as_u64)
                .context("Maximum words missing")?;
            ensure!((1..=100).contains(&max), "Maximum words must be 1-100");
            profile["request"]["maxWords"] = json!(max);
        }
        AutomationKind::Publish => {
            profile["sheetEnabled"] = json!(actions.contains(&"writeSheet"));
            profile["deleteAfterPublish"] = json!(actions.contains(&"cleanup"));
            if let Some(at) = actions.iter().position(|a| *a == "writeSheet") {
                ensure!(
                    at > actions.iter().position(|a| *a == "captureLink").unwrap(),
                    "Capture link before writing Sheet"
                );
            }
            if let Some(at) = actions.iter().position(|a| *a == "cleanup") {
                ensure!(
                    at == actions.len() - 1,
                    "Cleanup must finish the publish pipeline"
                );
            }
        }
    }
    crate::validate_automation_profile_config(doc.kind, &profile)?;
    Ok(profile)
}

pub fn compile_app_orchestration(
    doc: &AppWorkflowV1,
    profile: crate::AutomationProfileRef,
) -> anyhow::Result<crate::OrchestrationDocumentV1> {
    compile_app_profile(doc)?;
    let mut nodes = Vec::new();
    let start = doc.entry_node_id;
    let end = doc
        .nodes
        .iter()
        .find(|n| n.action == "end")
        .context("End missing")?
        .id;
    nodes.push(json!({"id":start,"kind":"start","position":{"x":0,"y":0}}));
    let mut cursor = start;
    let mut variables: BTreeMap<String, String> = BTreeMap::new();
    let mut native_added = false;
    for _ in 0..doc.nodes.len() {
        let node = doc
            .nodes
            .iter()
            .find(|node| node.id == cursor)
            .context("Node missing")?;
        let mut port = "done";
        match node.action.as_str(){
            "end"=>break,
            "start"=>{},
            "setVariable"=>{variables.insert(node.config["name"].as_str().unwrap_or_default().into(),node.config["value"].as_str().unwrap_or_default().into());},
            "ifValue"=>{port=if variables.get(node.config["name"].as_str().unwrap_or_default()).map(String::as_str)==node.config["equals"].as_str(){"matched"}else{"notMatched"};},
            "wait"=>nodes.push(json!({"id":node.id,"kind":"delay","durationMs":node.config["durationMs"],"position":node.position})),
            "log"=>nodes.push(json!({"id":node.id,"kind":"log","message":node.config["message"],"position":node.position})),
            _ if !native_added=>{native_added=true;nodes.push(json!({"id":node.id,"kind":match doc.kind {AutomationKind::Nurture=>"runNurture",AutomationKind::Interaction=>"runInteraction",AutomationKind::Publish=>"runPublish"},"profile":profile,"position":node.position}));},
            _=>{},
        }
        cursor = doc
            .edges
            .iter()
            .find(|edge| edge.source == cursor && edge.port == port)
            .context("Output missing")?
            .target;
    }
    nodes.push(json!({"id":end,"kind":"end","position":{"x":1200,"y":0}}));
    let mut edges = Vec::new();
    for pair in nodes.windows(2) {
        let source = &pair[0];
        let target = &pair[1];
        edges.push(
            json!({"sourceNodeId":source["id"],"sourcePort":"done","targetNodeId":target["id"]}),
        );
        if source["kind"]
            .as_str()
            .unwrap_or_default()
            .starts_with("run")
        {
            for port in ["partial", "failed", "uncertain"] {
                edges.push(
                    json!({"sourceNodeId":source["id"],"sourcePort":port,"targetNodeId":end}),
                );
            }
        }
    }
    serde_json::from_value(json!({"schemaVersion":1,"id":Uuid::new_v4(),"revision":0,"name":doc.name,"entryNodeId":start,"nodes":nodes,"edges":edges})).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_compile_and_reordered_nurture_actions_change_native_order() {
        for kind in [
            AutomationKind::Nurture,
            AutomationKind::Interaction,
            AutomationKind::Publish,
        ] {
            compile_app_profile(&new_app_workflow(kind)).unwrap();
        }
        let mut doc = new_app_workflow(AutomationKind::Nurture);
        let like = doc.nodes.iter().find(|n| n.action == "like").unwrap().id;
        let save = doc.nodes.iter().find(|n| n.action == "save").unwrap().id;
        for edge in &mut doc.edges {
            if edge.source == like {
                edge.source = save
            } else if edge.source == save {
                edge.source = like
            }
            if edge.target == like {
                edge.target = save
            } else if edge.target == save {
                edge.target = like
            }
        }
        let result = compile_app_profile(&doc).unwrap();
        assert_eq!(
            result["settings"]["workflowActionOrder"],
            json!(["save", "like", "comment"])
        );
    }
    #[test]
    fn missing_confirmation_and_invalid_edges_are_rejected() {
        let mut doc = new_app_workflow(AutomationKind::Publish);
        doc.nodes.retain(|n| n.action != "verifyPost");
        assert!(compile_app_profile(&doc).is_err());
        let mut doc = new_app_workflow(AutomationKind::Nurture);
        doc.edges[0].target = doc.entry_node_id;
        assert!(validate_app_workflow(&doc).is_err());
    }
}
