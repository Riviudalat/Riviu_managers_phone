use super::*;

fn data_node(kind: ActionKind, config: CompiledActionConfig) -> CompiledFlowNode {
    CompiledFlowNode {
        id: Uuid::new_v4(),
        kind,
        config,
        postcondition: None,
    }
}

fn log_node(message: &str, variable: Option<&str>) -> CompiledFlowNode {
    data_node(
        ActionKind::Log,
        CompiledActionConfig::Log {
            message: message.into(),
            variable: variable.map(str::to_owned),
        },
    )
}

fn locator() -> crate::QualifiedElementLocator {
    crate::QualifiedElementLocator {
        strategy: crate::ElementLocatorStrategy::AccessibilityId,
        value: "caption".into(),
    }
}

fn read_plan(nodes: Vec<CompiledFlowNode>, hierarchy: bool) -> CompiledFlowPlanV2 {
    let mut nodes = nodes;
    nodes.insert(0, launch_node());
    let mut capabilities = vec!["app.launch", "accessibility.readText"];
    if hierarchy {
        capabilities.push("accessibility.hierarchy");
    }
    plan(
        nodes,
        ContextPlan {
            requires_exclusive: true,
            requires_ui_session: true,
            requires_stream: false,
            requires_fresh_text_session: true,
            initial_bundle_id: Some(TARGET.into()),
        },
        &capabilities,
    )
}

fn wire_branch(plan: &mut CompiledFlowPlanV2, branch: Uuid, matched: Uuid, not_matched: Uuid) {
    plan.successors = plan
        .execution_order
        .windows(2)
        .map(|pair| (pair[0], BTreeMap::from([("flow".into(), pair[1])])))
        .collect();
    plan.successors.insert(
        branch,
        BTreeMap::from([
            ("matched".into(), matched),
            ("notMatched".into(), not_matched),
        ]),
    );
    // Both branch leaves terminate; neither leaf leads to the other.
    plan.successors.remove(&matched);
    plan.successors.remove(&not_matched);
}

#[tokio::test]
async fn read_text_log_keeps_completed_output_when_the_device_value_changes() {
    let read = data_node(
        ActionKind::ReadText,
        CompiledActionConfig::ReadText {
            name: "caption".into(),
            locator: locator(),
        },
    );
    let log = log_node("observed caption", Some("caption"));
    let log_id = log.id;
    let fixture = ExecutorFixture::new(
        read_plan(vec![read, log], false),
        Arc::new(FixtureFrames::new(&[40])),
    );
    *fixture.driver.typed_text.lock() = "Xin chào Đà Lạt".into();
    fixture
        .executor
        .run_device(fixture.device_run_id, fixture.plan.clone())
        .await
        .unwrap();
    *fixture.driver.typed_text.lock() = "different after completion".into();
    let detail = fixture.detail();
    let logged = detail
        .attempts
        .iter()
        .find(|a| a.node_id == log_id)
        .unwrap();
    assert_eq!(logged.state, FlowAttemptState::Succeeded);
    assert_eq!(
        logged.evidence_result.as_ref().unwrap()["value"],
        "Xin chào Đà Lạt"
    );
    let context = fixture
        .database
        .get_flow_attempt_execution_context(logged.id)
        .unwrap()
        .unwrap();
    let values = crate::flow::data::variables_before(
        &context.plan,
        &context.device_attempts,
        fixture.device_run_id,
        log_id,
    )
    .unwrap();
    assert_eq!(values["caption"], "Xin chào Đà Lạt");
    assert_eq!(
        fixture
            .driver
            .operations
            .lock()
            .iter()
            .filter(|op| op.as_str() == "readText")
            .count(),
        1
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn value_branch_only_executes_the_recorded_port_for_each_comparison_result() {
    for (literal, expected_port, selected_message) in [
        ("yes", "matched", "matched path"),
        ("no", "notMatched", "other path"),
    ] {
        let writer = data_node(
            ActionKind::SetVariable,
            CompiledActionConfig::SetVariable {
                name: "answer".into(),
                value: literal.into(),
            },
        );
        let branch = data_node(
            ActionKind::IfValue,
            CompiledActionConfig::IfValue {
                name: "answer".into(),
                operator: crate::FlowCompareOperator::Equals,
                value: "yes".into(),
            },
        );
        let matched = log_node("matched path", Some("answer"));
        let other = log_node("other path", Some("answer"));
        let branch_id = branch.id;
        let matched_id = matched.id;
        let other_id = other.id;
        let mut compiled = plan(
            vec![writer, branch, matched, other],
            ContextPlan {
                requires_exclusive: false,
                requires_ui_session: false,
                requires_stream: false,
                requires_fresh_text_session: false,
                initial_bundle_id: None,
            },
            &[],
        );
        wire_branch(&mut compiled, branch_id, matched_id, other_id);
        let fixture = ExecutorFixture::new(compiled, Arc::new(FixtureFrames::new(&[40])));
        fixture
            .executor
            .run_device(fixture.device_run_id, fixture.plan.clone())
            .await
            .unwrap();
        let detail = fixture.detail();
        let predicate = detail
            .attempts
            .iter()
            .find(|a| a.node_id == branch_id)
            .unwrap();
        assert_eq!(predicate.chosen_port.as_deref(), Some(expected_port));
        let logs: Vec<_> = detail
            .attempts
            .iter()
            .filter(|a| a.action_kind == ActionKind::Log && a.state == FlowAttemptState::Succeeded)
            .collect();
        assert_eq!(logs.len(), 1);
        assert_eq!(
            logs[0].evidence_result.as_ref().unwrap()["message"],
            selected_message
        );
        let unchosen = if expected_port == "matched" {
            other_id
        } else {
            matched_id
        };
        let unchosen = detail
            .attempts
            .iter()
            .find(|a| a.node_id == unchosen)
            .unwrap();
        assert_eq!(unchosen.state, FlowAttemptState::Queued);
        assert!(unchosen.canonical_input.is_none());
        assert!(fixture.driver.operations.lock().is_empty());
        fixture.shutdown().await;
    }
}

#[tokio::test]
async fn visibility_read_failure_never_selects_the_absent_branch() {
    let branch = data_node(
        ActionKind::IfVisible,
        CompiledActionConfig::IfVisible { locator: locator() },
    );
    let matched = log_node("matched", None);
    let other = log_node("absent", None);
    let branch_id = branch.id;
    let matched_id = matched.id;
    let other_id = other.id;
    let mut compiled = read_plan(vec![branch, matched, other], true);
    wire_branch(&mut compiled, branch_id, matched_id, other_id);
    let fixture = ExecutorFixture::new(compiled, Arc::new(FixtureFrames::new(&[40])));
    fixture
        .driver
        .supports_hierarchy
        .store(true, Ordering::SeqCst);
    fixture
        .executor
        .run_device(fixture.device_run_id, fixture.plan.clone())
        .await
        .unwrap_err();
    let detail = fixture.detail();
    let predicate = detail
        .attempts
        .iter()
        .find(|a| a.node_id == branch_id)
        .unwrap();
    assert_eq!(predicate.state, FlowAttemptState::FailedVerified);
    assert_eq!(predicate.chosen_port, None);
    assert!(detail
        .attempts
        .iter()
        .filter(|a| a.action_kind == ActionKind::Log)
        .all(|a| a.state == FlowAttemptState::Queued && a.canonical_input.is_none()));
    assert_eq!(
        fixture
            .driver
            .operations
            .lock()
            .iter()
            .filter(|op| op.as_str() == "hierarchySource")
            .count(),
        1
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn visibility_capability_failure_stops_before_the_hierarchy_request() {
    let branch = data_node(
        ActionKind::IfVisible,
        CompiledActionConfig::IfVisible { locator: locator() },
    );
    let matched = log_node("matched", None);
    let other = log_node("absent", None);
    let branch_id = branch.id;
    let matched_id = matched.id;
    let other_id = other.id;
    let mut compiled = read_plan(vec![branch, matched, other], true);
    wire_branch(&mut compiled, branch_id, matched_id, other_id);
    let fixture = ExecutorFixture::new(compiled, Arc::new(FixtureFrames::new(&[40])));
    let error = fixture
        .executor
        .run_device(fixture.device_run_id, fixture.plan.clone())
        .await
        .unwrap_err();
    assert_eq!(error.code(), "CapabilityUnavailable");
    assert!(!fixture
        .driver
        .operations
        .lock()
        .iter()
        .any(|op| op == "hierarchySource"));
    assert!(fixture
        .detail()
        .attempts
        .iter()
        .all(|a| a.action_kind != ActionKind::Log || a.state != FlowAttemptState::Succeeded));
    fixture.shutdown().await;
}
