use riviu_core::{
    ui_automation::inspector::ElementSelector, ActionKind, EvidenceSpec, FlowDocumentV2, FlowEdge,
    FlowNode,
};
#[test]
fn captured_selector_and_expected_element_compile_as_replayable_flow() {
    let selector = ElementSelector {
        package: "app.test".into(),
        description: Some("Profile".into()),
        text: None,
        resource_id: None,
        class_name: None,
        schema_version: None,
        text_prefix: None,
        description_prefix: None,
        scope: None,
        action_target: None,
        ordinal: None,
    };
    let start = FlowNode::new(ActionKind::Start, serde_json::json!({}));
    let mut launch = FlowNode::new(
        ActionKind::LaunchApp,
        serde_json::json!({"bundleId":"app.test"}),
    );
    launch.postcondition = Some(EvidenceSpec::ActiveAppEquals {
        bundle_id: "app.test".into(),
    });
    let mut tap = FlowNode::new(ActionKind::Tap, serde_json::json!({"selector":&selector}));
    tap.postcondition = Some(EvidenceSpec::ElementVisible {
        selector: Box::new(ElementSelector {
            text: Some("Edit profile".into()),
            description: None,
            ..selector.clone()
        }),
    });
    // The recorded "get the post link" shape: refresh by a swipe that keeps its screen, open
    // the first grid tile by position, arm the clipboard, read it back into a variable.
    let profile = selector;
    let mut refresh = FlowNode::new(
        ActionKind::Swipe,
        serde_json::json!({"fromFraction":{"x":0.5,"y":0.45},"toFraction":{"x":0.5,"y":0.8},"durationMs":400}),
    );
    refresh.postcondition = Some(EvidenceSpec::ElementVisible {
        selector: Box::new(profile.clone()),
    });
    let grid = serde_json::json!({"maxDepth":1,"resourceId":"app.test:id/hoo","className":"android.widget.GridView"});
    let tile = serde_json::json!({"package":"app.test","resourceId":"app.test:id/emr","className":"android.widget.FrameLayout","schemaVersion":2,"scope":grid,"ordinal":1});
    let mut open_tile = FlowNode::new(ActionKind::Tap, serde_json::json!({ "selector": tile }));
    open_tile.postcondition = Some(EvidenceSpec::ElementVisible {
        selector: Box::new(profile),
    });
    let arm = FlowNode::new(ActionKind::ReadClipboard, serde_json::json!({"mode":"arm"}));
    let read = FlowNode::new(
        ActionKind::ReadClipboard,
        serde_json::json!({"mode":"read","name":"postLink"}),
    );
    let end = FlowNode::new(ActionKind::End, serde_json::json!({}));
    let document = linear(vec![start, launch, tap, refresh, open_tile, arm, read, end]);
    assert!(
        riviu_script_engine::compile_flow(&document, &riviu_core::release_one_catalog()).is_ok()
    );

    let mut unnamed = document.clone();
    unnamed.nodes[6].config = serde_json::json!({"mode":"read"});
    assert!(
        riviu_script_engine::compile_flow(&unnamed, &riviu_core::release_one_catalog()).is_err()
    );
}

fn linear(nodes: Vec<FlowNode>) -> FlowDocumentV2 {
    let mut document:FlowDocumentV2=serde_json::from_value(serde_json::json!({"schemaVersion":2,"id":uuid::Uuid::new_v4(),"name":"Recorded Profile","revision":1,"entryNodeId":nodes[0].id,"nodes":nodes,"edges":[],"viewport":{"x":0,"y":0,"zoom":1}})).unwrap();
    document.edges=document.nodes.windows(2).map(|p|serde_json::from_value::<FlowEdge>(serde_json::json!({"id":uuid::Uuid::new_v4(),"sourceNodeId":p[0].id,"sourcePort":"flow","targetNodeId":p[1].id,"targetPort":"flow"})).unwrap()).collect();
    document
}
