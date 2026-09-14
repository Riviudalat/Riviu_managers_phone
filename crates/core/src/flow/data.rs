//! Per-device variables are rebuilt from succeeded attempts on the recorded path.
//! The attempt ledger is the only store: off-path nodes and later retries cannot
//! supply a value, and reopening a run never re-reads an already completed input.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{
    CompiledActionConfig, CompiledFlowPlanV2, FlowAttemptState, FlowNodeAttemptRecord, NodeId,
};

pub(crate) fn visible_in_snapshot(
    snapshot: crate::HierarchySourceSnapshot,
    package: &str,
    locator: &super::QualifiedElementLocator,
) -> anyhow::Result<bool> {
    let tree = crate::ui_automation::tree::Tree::parse(snapshot)?;
    anyhow::ensure!(
        tree.nodes
            .iter()
            .any(|node| node.attr("package") == package),
        "hierarchy has no target application"
    );
    let mut found = false;
    for (index, node) in tree.nodes.iter().enumerate() {
        let matches = match locator.strategy {
            super::ElementLocatorStrategy::AccessibilityId => {
                node.attr("content-desc") == locator.value
            }
            super::ElementLocatorStrategy::ClassName => node.attr("class") == locator.value,
        };
        if node.attr("package") != package || !matches || !tree.ancestors_visible(index) {
            continue;
        }
        if node.visibility() == Some(false) {
            continue;
        }
        anyhow::ensure!(
            node.visibility() == Some(true),
            "matching element visibility is unknown"
        );
        anyhow::ensure!(
            node.rect().is_some(),
            "matching element bounds are unreadable"
        );
        found = true;
    }
    Ok(found)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FlowCompareOperator {
    Equals,
    NotEquals,
    Contains,
    StartsWith,
    IsEmpty,
    NotEmpty,
}

impl FlowCompareOperator {
    pub fn evaluate(self, actual: &str, expected: &str) -> bool {
        match self {
            Self::Equals => actual == expected,
            Self::NotEquals => actual != expected,
            Self::Contains => actual.contains(expected),
            Self::StartsWith => actual.starts_with(expected),
            Self::IsEmpty => actual.is_empty(),
            Self::NotEmpty => !actual.is_empty(),
        }
    }
}

pub fn validate_flow_variable_name(name: &str) -> Result<(), &'static str> {
    let mut chars = name.chars();
    if name.len() > 64
        || !chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err("variable name must be 1..64 ASCII letters, digits or underscores, starting with a letter or underscore");
    }
    Ok(())
}

pub(crate) fn validate_data_output(
    config: &CompiledActionConfig,
    output: Option<&serde_json::Value>,
) -> Result<(), &'static str> {
    let text = |key: &str| {
        output
            .and_then(|o| o.get(key))
            .and_then(serde_json::Value::as_str)
    };
    let fields = output
        .and_then(serde_json::Value::as_object)
        .map(|o| o.len());
    let (valid, reason) = match config {
        CompiledActionConfig::SetVariable { name, value } => (
            text("kind") == Some("flowVariable")
                && text("name") == Some(name)
                && text("value") == Some(value)
                && fields == Some(3),
            "literal variable result does not match its compiled writer",
        ),
        CompiledActionConfig::ReadText { name, .. }
        | CompiledActionConfig::CopyVariable { name, .. } => (
            text("kind") == Some("flowVariable")
                && text("name") == Some(name)
                && text("value").is_some_and(|v| v.chars().count() <= 4096)
                && fields == Some(3),
            "text variable result is missing or invalid",
        ),
        CompiledActionConfig::OcrReadText {
            name,
            min_confidence,
            ..
        } => {
            let ocr = output
                .and_then(|o| o.get("ocr"))
                .cloned()
                .and_then(|o| serde_json::from_value::<crate::ui_automation::OcrResponse>(o).ok());
            let valid = ocr.is_some_and(|o| {
                o.text == text("value").unwrap_or_default()
                    && o.screenshot_sha256.len() == 64
                    && !o.engine.is_empty()
                    && o.lines.len() <= 128
                    && o.lines.iter().all(|line| {
                        line.confidence.is_finite()
                            && (*min_confidence..=1.0).contains(&line.confidence)
                    })
            });
            (
                text("kind") == Some("flowVariable")
                    && text("name") == Some(name)
                    && text("value").is_some_and(|s| s.len() <= 4096)
                    && fields == Some(4)
                    && valid,
                "OCR result proof is missing or invalid",
            )
        }
        CompiledActionConfig::Transform(c) => (
            text("kind") == Some("flowVariable")
                && text("name") == Some(&c.name)
                && text("value").is_some_and(|s| s.chars().count() <= 4096)
                && fields == Some(3),
            "transformation output invalid",
        ),
        CompiledActionConfig::Log { message, variable } => {
            let value_valid = if variable.is_some() {
                text("value").is_some_and(|v| v.chars().count() <= 4096)
            } else {
                output.and_then(|o| o.get("value")) == Some(&serde_json::Value::Null)
                    && output.and_then(|o| o.get("variable")) == Some(&serde_json::Value::Null)
            };
            (
                text("kind") == Some("flowLog")
                    && text("message") == Some(message)
                    && text("variable") == variable.as_deref()
                    && value_valid
                    && fields == Some(4),
                "log result does not match its compiled message and variable",
            )
        }
        config if super::extended::output_variable(config).is_some() => {
            let expected = match config {
                CompiledActionConfig::FileRead(_) => "fileRead",
                CompiledActionConfig::FileWrite(_) => "fileWrite",
                CompiledActionConfig::HttpRequest(_) => "httpRequest",
                CompiledActionConfig::SheetRead(_) => "sheetRead",
                CompiledActionConfig::SheetWrite(_) => "sheetWrite",
                _ => "invalid",
            };
            (
                text("kind") == Some("flowConnector")
                    && text("operation") == Some(expected)
                    && text("name") == super::extended::output_variable(config)
                    && text("value").is_some_and(|v| v.chars().count() <= 4096)
                    && text("value").zip(text("receipt")).is_some_and(|(v, r)| {
                        super::connectors::validate_output_receipt(config, v, r)
                    })
                    && fields == Some(5),
                "connector result receipt is invalid",
            )
        }
        _ => (
            output.is_none(),
            "an evidence-free action cannot carry success evidence",
        ),
    };
    if valid {
        Ok(())
    } else {
        Err(reason)
    }
}

pub(crate) fn variables_before(
    plan: &CompiledFlowPlanV2,
    attempts: &[FlowNodeAttemptRecord],
    device_run_id: uuid::Uuid,
    stop: NodeId,
) -> Result<BTreeMap<String, String>, &'static str> {
    let mut values = BTreeMap::new();
    let mut current = plan.entry_node();
    for _ in 0..=plan.nodes.len() {
        let id = current.ok_or("variable consumer is not on the recorded path")?;
        if id == stop {
            return Ok(values);
        }
        let node = plan
            .nodes
            .get(&id)
            .ok_or("recorded path references a missing node")?;
        let attempt = attempts
            .iter()
            .filter(|a| a.device_run_id == device_run_id && a.node_id == id)
            .max_by_key(|a| a.attempt_no)
            .ok_or("recorded predecessor has no attempt")?;
        if attempt.state != FlowAttemptState::Succeeded || attempt.action_kind != node.kind {
            return Err("variable predecessor is not durably succeeded");
        }
        if let Some(name) = super::extended::output_variable(&node.config) {
            let output = attempt
                .evidence_result
                .as_ref()
                .ok_or("variable output is missing")?;
            let value = output
                .get("value")
                .and_then(serde_json::Value::as_str)
                .ok_or("variable output must be text")?;
            validate_data_output(&node.config, Some(output))?;
            if let CompiledActionConfig::SetVariable {
                value: expected, ..
            } = &node.config
            {
                if value != expected {
                    return Err("literal variable differs from its compiled value");
                }
            }
            values.insert(name.to_owned(), value.to_owned());
        }
        current = plan.successor_on_path(id, attempt.chosen_port.as_deref());
    }
    Err("recorded variable path contains a cycle")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActionKind, CompiledFlowNode, ContextPlan, SideEffectClass};
    use serde_json::json;
    use uuid::Uuid;

    fn record(
        device: Uuid,
        node: &CompiledFlowNode,
        value: Option<serde_json::Value>,
        port: Option<&str>,
    ) -> FlowNodeAttemptRecord {
        FlowNodeAttemptRecord {
            id: Uuid::new_v4(),
            device_run_id: device,
            node_id: node.id,
            action_kind: node.kind,
            attempt_no: 1,
            side_effect_class: SideEffectClass::None,
            state: FlowAttemptState::Succeeded,
            canonical_input: None,
            evidence_baseline: None,
            evidence_result: value,
            retry_allowed: false,
            error: None,
            started_at: None,
            updated_at: chrono::Utc::now(),
            finished_at: None,
            chosen_port: port.map(str::to_owned),
        }
    }

    #[test]
    fn variables_follow_only_succeeded_same_device_taken_path() {
        let device = Uuid::new_v4();
        let writer = CompiledFlowNode {
            id: Uuid::new_v4(),
            kind: ActionKind::SetVariable,
            config: CompiledActionConfig::SetVariable {
                name: "name".into(),
                value: "correct".into(),
            },
            postcondition: None,
        };
        let other = CompiledFlowNode {
            id: Uuid::new_v4(),
            kind: ActionKind::SetVariable,
            config: CompiledActionConfig::SetVariable {
                name: "name".into(),
                value: "wrong".into(),
            },
            postcondition: None,
        };
        let branch = CompiledFlowNode {
            id: Uuid::new_v4(),
            kind: ActionKind::IfValue,
            config: CompiledActionConfig::IfValue {
                name: "name".into(),
                operator: FlowCompareOperator::Equals,
                value: "correct".into(),
            },
            postcondition: None,
        };
        let stop = CompiledFlowNode {
            id: Uuid::new_v4(),
            kind: ActionKind::Log,
            config: CompiledActionConfig::Log {
                message: "log".into(),
                variable: Some("name".into()),
            },
            postcondition: None,
        };
        let plan = CompiledFlowPlanV2 {
            schema_version: 2,
            flow_id: Uuid::new_v4(),
            revision: 1,
            execution_order: vec![writer.id, branch.id, other.id, stop.id],
            nodes: [writer.clone(), branch.clone(), other.clone(), stop.clone()]
                .into_iter()
                .map(|n| (n.id, n))
                .collect(),
            source_paths: Default::default(),
            successors: BTreeMap::from([
                (writer.id, BTreeMap::from([("flow".into(), branch.id)])),
                (
                    branch.id,
                    BTreeMap::from([("matched".into(), stop.id), ("notMatched".into(), other.id)]),
                ),
                (other.id, BTreeMap::from([("flow".into(), stop.id)])),
            ]),
            context_plan: ContextPlan {
                requires_exclusive: false,
                requires_ui_session: false,
                requires_stream: false,
                requires_fresh_text_session: false,
                initial_bundle_id: None,
            },
            action_definition_versions: BTreeMap::new(),
            required_capabilities: Default::default(),
        };
        let correct = json!({"kind":"flowVariable","name":"name","value":"correct"});
        let mut attempts = vec![
            record(device, &writer, Some(correct.clone()), None),
            record(device, &branch, None, Some("matched")),
            record(
                device,
                &other,
                Some(json!({"kind":"flowVariable","name":"name","value":"wrong"})),
                None,
            ),
        ];
        let mut foreign = record(
            Uuid::new_v4(),
            &writer,
            Some(json!({"kind":"flowVariable","name":"name","value":"foreign"})),
            None,
        );
        foreign.attempt_no = 10;
        attempts.push(foreign);
        assert_eq!(
            variables_before(&plan, &attempts, device, stop.id).unwrap()["name"],
            "correct"
        );
        attempts[0].state = FlowAttemptState::Uncertain;
        assert!(variables_before(&plan, &attempts, device, stop.id).is_err());
        attempts[0].state = FlowAttemptState::Succeeded;
        attempts[0].evidence_result = None;
        assert!(variables_before(&plan, &attempts, device, stop.id).is_err());
    }

    #[test]
    fn data_outputs_reject_wrong_writer_extra_fields_and_invalid_names() {
        let config = CompiledActionConfig::SetVariable {
            name: "message".into(),
            value: "hello".into(),
        };
        assert!(validate_data_output(
            &config,
            Some(&json!({"kind":"flowVariable","name":"message","value":"hello"}))
        )
        .is_ok());
        for value in [
            json!({"kind":"flowVariable","name":"message","value":"other"}),
            json!({"kind":"flowVariable","name":"message","value":"hello","extra":1}),
        ] {
            assert!(validate_data_output(&config, Some(&value)).is_err());
        }
        for name in ["", "has space", "0start", "nested.key", "${execute}"] {
            assert!(validate_flow_variable_name(name).is_err());
        }
        assert!(FlowCompareOperator::Contains.evaluate("xin chao", "chao"));
        assert!(!FlowCompareOperator::Equals.evaluate("10", "010"));
    }

    #[test]
    fn visibility_read_distinguishes_absence_from_incomplete_observation() {
        let locator = super::super::QualifiedElementLocator {
            strategy: super::super::ElementLocatorStrategy::AccessibilityId,
            value: "Target".into(),
        };
        let read = |xml: &str| {
            visible_in_snapshot(
                crate::HierarchySourceSnapshot {
                    xml: xml.into(),
                    generation: 1,
                },
                "com.example.app",
                &locator,
            )
        };
        assert!(read(r#"<hierarchy><node package="com.example.app" content-desc="Target" displayed="true" bounds="[0,0][10,10]"/></hierarchy>"#).unwrap());
        assert!(!read(r#"<hierarchy><node package="com.example.app" content-desc="Other" displayed="true" bounds="[0,0][10,10]"/></hierarchy>"#).unwrap());
        assert!(read(r#"<hierarchy><node package="com.example.app" content-desc="Target" displayed="true"/></hierarchy>"#).is_err());
        assert!(read(r#"<hierarchy><node package="com.example.app" content-desc="Target" bounds="[0,0][10,10]"/></hierarchy>"#).is_err());
        assert!(read("<hierarchy/>").is_err());
        assert!(read("<hierarchy><node").is_err());
    }
}
