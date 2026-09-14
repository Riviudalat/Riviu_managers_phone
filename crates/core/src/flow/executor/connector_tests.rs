use super::*;
use serde_json::json;
use tokio::io::AsyncReadExt;

fn connector_document(actions: Vec<crate::FlowNode>) -> FlowDocumentV2 {
    let mut document = FlowDocumentV2::empty("Connector integration");
    document.revision = 1;
    document.nodes.splice(1..1, actions);
    document.edges = document
        .nodes
        .windows(2)
        .map(|pair| crate::FlowEdge::flow(pair[0].id, pair[1].id))
        .collect();
    document
}
fn connector_node(kind: ActionKind, config: serde_json::Value) -> crate::FlowNode {
    let mut node = crate::FlowNode::new(kind, config);
    if matches!(
        kind,
        ActionKind::FileWrite | ActionKind::HttpRequest | ActionKind::SheetWrite
    ) {
        node.postcondition = Some(EvidenceSpec::ConnectorResult {
            name: node.config["name"].as_str().unwrap().into(),
        });
    }
    node
}
fn fixture_from_document(document: &FlowDocumentV2) -> ExecutorFixture {
    // The engine's core dependency and this lib-test core are separate crate
    // instances. Cross their boundary using the same serialized IPC contract.
    let document = serde_json::from_value(serde_json::to_value(document).unwrap()).unwrap();
    let catalog: Vec<_> =
        serde_json::from_value(serde_json::to_value(crate::release_one_catalog()).unwrap())
            .unwrap();
    let compiled = riviu_script_engine::compile_flow(&document, &catalog).unwrap();
    let plan = serde_json::from_value(serde_json::to_value(compiled.plan).unwrap()).unwrap();
    ExecutorFixture::new(plan, Arc::new(FixtureFrames::new(&[40])))
}

#[tokio::test]
async fn file_connector_transform_and_log_use_durable_outputs_without_device_work() {
    let relative = format!("connector-integration-{}/values.json", Uuid::new_v4());
    let document = connector_document(vec![
        connector_node(
            ActionKind::FileWrite,
            json!({"name":"written","path":relative,"format":"json","value":"{\"status\":\"ready\"}"}),
        ),
        connector_node(
            ActionKind::FileRead,
            json!({"name":"loaded","path":relative,"format":"json"}),
        ),
        connector_node(
            ActionKind::Transform,
            json!({"name":"status","source":"loaded","operation":"jsonGet","path":"/status"}),
        ),
        connector_node(
            ActionKind::Log,
            json!({"message":"connector completed","variable":"status"}),
        ),
    ]);
    let fixture = fixture_from_document(&document);
    fixture
        .executor
        .run_device(fixture.device_run_id, fixture.plan.clone())
        .await
        .unwrap();
    let detail = fixture.detail();
    assert!(detail
        .attempts
        .iter()
        .all(|a| a.state == FlowAttemptState::Succeeded));
    let logged = detail
        .attempts
        .iter()
        .find(|a| a.action_kind == ActionKind::Log)
        .unwrap();
    assert_eq!(logged.evidence_result.as_ref().unwrap()["value"], "ready");
    let written = detail
        .attempts
        .iter()
        .find(|a| a.action_kind == ActionKind::FileWrite)
        .unwrap();
    assert_eq!(
        written.evidence_result.as_ref().unwrap()["operation"],
        "fileWrite"
    );
    assert!(written.canonical_input.is_some());
    let fullpath = fixture.database.flow_connector_root().join(&relative);
    std::fs::write(&fullpath, "{\"status\":\"changed after completion\"}").unwrap();
    let context = fixture
        .database
        .get_flow_attempt_execution_context(logged.id)
        .unwrap()
        .unwrap();
    let restored = crate::flow::data::variables_before(
        &context.plan,
        &context.device_attempts,
        fixture.device_run_id,
        logged.node_id,
    )
    .unwrap();
    assert_eq!(restored["status"], "ready");
    assert_eq!(restored["loaded"], "{\"status\":\"ready\"}");
    assert!(fixture.driver.operations.lock().is_empty());
    fixture.shutdown().await;
    std::fs::remove_file(fullpath).unwrap();
}

#[tokio::test]
async fn http_timeout_after_dispatch_stays_uncertain_when_executor_is_reentered() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/request", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let observed = requests.clone();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 4096];
        assert!(stream.read(&mut bytes).await.unwrap() > 0);
        observed.fetch_add(1, Ordering::SeqCst);
        if tokio::time::timeout(Duration::from_millis(650), listener.accept())
            .await
            .is_ok()
        {
            observed.fetch_add(1, Ordering::SeqCst);
        }
    });
    let document = connector_document(vec![
        connector_node(
            ActionKind::HttpRequest,
            json!({"name":"result","url":url,"method":"POST","body":"{\"command\":\"fixture\"}","timeoutMs":100}),
        ),
        connector_node(
            ActionKind::Log,
            json!({"message":"should remain queued","variable":"result"}),
        ),
    ]);
    let fixture = fixture_from_document(&document);
    let result = fixture
        .executor
        .run_device(fixture.device_run_id, fixture.plan.clone())
        .await;
    assert!(result.is_err());
    let detail = fixture.detail();
    let http = detail
        .attempts
        .iter()
        .find(|a| a.action_kind == ActionKind::HttpRequest)
        .unwrap();
    assert_eq!(http.state, FlowAttemptState::Uncertain);
    assert!(http.canonical_input.is_some());
    assert!(http.evidence_result.is_none());
    assert_eq!(
        detail
            .attempts
            .iter()
            .find(|a| a.action_kind == ActionKind::Log)
            .unwrap()
            .state,
        FlowAttemptState::Queued
    );
    assert!(fixture
        .executor
        .run_device(fixture.device_run_id, fixture.plan.clone())
        .await
        .is_err());
    server.await.unwrap();
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert!(fixture.driver.operations.lock().is_empty());
    fixture.shutdown().await;
}

#[tokio::test]
async fn resolved_file_path_escape_fails_without_an_effect_and_leaves_followups_queued() {
    let document = connector_document(vec![
        connector_node(
            ActionKind::SetVariable,
            json!({"name":"path","value":"../outside.txt"}),
        ),
        connector_node(
            ActionKind::FileWrite,
            json!({"name":"written","path":"${path}","format":"text","value":"fixture"}),
        ),
        connector_node(
            ActionKind::Log,
            json!({"message":"should remain queued","variable":"written"}),
        ),
    ]);
    let fixture = fixture_from_document(&document);
    assert!(fixture
        .executor
        .run_device(fixture.device_run_id, fixture.plan.clone())
        .await
        .is_err());
    let detail = fixture.detail();
    let write = detail
        .attempts
        .iter()
        .find(|a| a.action_kind == ActionKind::FileWrite)
        .unwrap();
    assert_eq!(write.state, FlowAttemptState::FailedBeforeDispatch);
    assert_eq!(
        detail
            .attempts
            .iter()
            .find(|a| a.action_kind == ActionKind::Log)
            .unwrap()
            .state,
        FlowAttemptState::Queued
    );
    assert!(fixture.driver.operations.lock().is_empty());
    fixture.shutdown().await;
}
