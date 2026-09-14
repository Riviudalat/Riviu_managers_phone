use super::*;
use crate::ui_automation::{
    GuiReasoner, GuiRequest, GuiResponse, OcrLine, OcrRequest, OcrResponse,
};

#[derive(Clone, Copy)]
enum OcrFixtureMode {
    Valid,
    WrongBinding,
    AdvanceGeneration,
    Missing,
}

struct FixtureOcr {
    mode: OcrFixtureMode,
    frames: Arc<FixtureFrames>,
    requests: Mutex<Vec<OcrRequest>>,
}

#[async_trait]
impl GuiReasoner for FixtureOcr {
    async fn resolve(&self, _request: GuiRequest) -> anyhow::Result<GuiResponse> {
        anyhow::bail!("vision provider is not part of this OCR flow")
    }

    async fn ocr(&self, request: OcrRequest) -> anyhow::Result<OcrResponse> {
        request.validate()?;
        self.requests.lock().push(request.clone());
        if matches!(self.mode, OcrFixtureMode::AdvanceGeneration) {
            self.frames.generation.fetch_add(1, Ordering::SeqCst);
        }
        let lines = if matches!(self.mode, OcrFixtureMode::Missing) {
            vec![]
        } else {
            vec![OcrLine {
                text: "Xin chào Việt Nam".into(),
                confidence: 0.96,
                bounds: request.region(),
            }]
        };
        Ok(OcrResponse {
            protocol_version: 1,
            request_id: request.request_id,
            observation_id: request.observation_id,
            session_epoch: request.session_epoch,
            generation: request.generation
                + u64::from(matches!(self.mode, OcrFixtureMode::WrongBinding)),
            screenshot_sha256: request.screenshot.sha256,
            status: if lines.is_empty() {
                "unresolved"
            } else {
                "resolved"
            }
            .into(),
            text: lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            lines,
            engine: "recorded fixture OCR".into(),
            elapsed_ms: 1,
        })
    }
}

fn ocr_node() -> CompiledFlowNode {
    CompiledFlowNode {
        id: Uuid::new_v4(),
        kind: ActionKind::OcrReadText,
        config: CompiledActionConfig::OcrReadText {
            name: "ocrCaption".into(),
            languages: vec!["vi".into(), "en".into()],
            region: Some(crate::flow::VisionRegion {
                x0: 0.25,
                y0: 0.25,
                x1: 1.0,
                y1: 1.0,
            }),
            min_confidence: 0.7,
        },
        postcondition: None,
    }
}

fn ocr_fixture(
    mode: OcrFixtureMode,
    nodes: Vec<CompiledFlowNode>,
) -> (ExecutorFixture, Arc<FixtureOcr>) {
    let mut all = vec![launch_node()];
    all.extend(nodes);
    let frames = Arc::new(FixtureFrames::new(&[40]));
    let reasoner = Arc::new(FixtureOcr {
        mode,
        frames: frames.clone(),
        requests: Mutex::new(vec![]),
    });
    let mut fixture = ExecutorFixture::new(
        plan(
            all,
            ContextPlan {
                requires_exclusive: true,
                requires_ui_session: true,
                requires_stream: true,
                requires_fresh_text_session: false,
                initial_bundle_id: Some(TARGET.into()),
            },
            &["app.launch", "stream"],
        ),
        frames,
    );
    fixture.executor.deps.reasoner = Some(reasoner.clone());
    (fixture, reasoner)
}

#[tokio::test]
async fn ocr_output_flows_through_value_branch_to_durable_log_with_normalized_roi() {
    let read = ocr_node();
    let read_id = read.id;
    let branch = CompiledFlowNode {
        id: Uuid::new_v4(),
        kind: ActionKind::IfValue,
        config: CompiledActionConfig::IfValue {
            name: "ocrCaption".into(),
            operator: crate::FlowCompareOperator::Equals,
            value: "Xin chào Việt Nam".into(),
        },
        postcondition: None,
    };
    let branch_id = branch.id;
    let matched = CompiledFlowNode {
        id: Uuid::new_v4(),
        kind: ActionKind::Log,
        config: CompiledActionConfig::Log {
            message: "matched OCR".into(),
            variable: Some("ocrCaption".into()),
        },
        postcondition: None,
    };
    let matched_id = matched.id;
    let other = CompiledFlowNode {
        id: Uuid::new_v4(),
        kind: ActionKind::Log,
        config: CompiledActionConfig::Log {
            message: "other".into(),
            variable: None,
        },
        postcondition: None,
    };
    let other_id = other.id;
    let (mut fixture, reasoner) =
        ocr_fixture(OcrFixtureMode::Valid, vec![read, branch, matched, other]);
    fixture.plan.successors = fixture
        .plan
        .execution_order
        .windows(2)
        .map(|pair| (pair[0], BTreeMap::from([("flow".into(), pair[1])])))
        .collect();
    fixture.plan.successors.insert(
        branch_id,
        BTreeMap::from([
            ("matched".into(), matched_id),
            ("notMatched".into(), other_id),
        ]),
    );
    fixture.plan.successors.remove(&matched_id);
    fixture.plan.successors.remove(&other_id);
    // Keep the persisted revision and in-memory graph identical for admission.
    let frames = reasoner.frames.clone();
    let mut fixture = ExecutorFixture::new(fixture.plan.clone(), frames);
    fixture.executor.deps.reasoner = Some(reasoner.clone());
    fixture
        .executor
        .run_device(fixture.device_run_id, fixture.plan.clone())
        .await
        .unwrap();
    let detail = fixture.detail();
    let read = detail
        .attempts
        .iter()
        .find(|a| a.node_id == read_id)
        .unwrap();
    assert_eq!(read.state, FlowAttemptState::Succeeded);
    let predicate = detail
        .attempts
        .iter()
        .find(|a| a.node_id == branch_id)
        .unwrap();
    assert_eq!(predicate.chosen_port.as_deref(), Some("matched"));
    let logged = detail
        .attempts
        .iter()
        .find(|a| a.node_id == matched_id)
        .unwrap();
    assert_eq!(
        logged.evidence_result.as_ref().unwrap()["value"],
        "Xin chào Việt Nam"
    );
    assert_eq!(
        detail
            .attempts
            .iter()
            .find(|a| a.node_id == other_id)
            .unwrap()
            .state,
        FlowAttemptState::Queued
    );
    {
        let requests = reasoner.requests.lock();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].roi,
            Some(crate::ui_automation::OcrRect {
                x: 1,
                y: 1,
                width: 3,
                height: 3
            })
        );
        assert_eq!(requests[0].languages, vec!["vi", "en"]);
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn ocr_stale_binding_and_advanced_generation_never_write_a_variable() {
    for mode in [
        OcrFixtureMode::WrongBinding,
        OcrFixtureMode::AdvanceGeneration,
    ] {
        let read = ocr_node();
        let read_id = read.id;
        let (fixture, reasoner) = ocr_fixture(mode, vec![read]);
        assert!(fixture
            .executor
            .run_device(fixture.device_run_id, fixture.plan.clone())
            .await
            .is_err());
        let detail = fixture.detail();
        let attempted = detail
            .attempts
            .iter()
            .find(|a| a.node_id == read_id)
            .unwrap();
        assert_eq!(attempted.state, FlowAttemptState::FailedVerified);
        assert!(attempted
            .evidence_result
            .as_ref()
            .is_none_or(|value| value.get("kind") != Some(&serde_json::json!("flowVariable"))));
        assert_eq!(reasoner.requests.lock().len(), 1);
        fixture.shutdown().await;
    }
}

#[tokio::test]
async fn ocr_absence_records_an_empty_variable_without_provider_inference() {
    let read = ocr_node();
    let read_id = read.id;
    let (fixture, _) = ocr_fixture(OcrFixtureMode::Missing, vec![read]);
    fixture
        .executor
        .run_device(fixture.device_run_id, fixture.plan.clone())
        .await
        .unwrap();
    let detail = fixture.detail();
    let attempted = detail
        .attempts
        .iter()
        .find(|a| a.node_id == read_id)
        .unwrap();
    assert_eq!(attempted.state, FlowAttemptState::Succeeded);
    assert_eq!(attempted.evidence_result.as_ref().unwrap()["value"], "");
    fixture.shutdown().await;
}
