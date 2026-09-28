use super::{model::*, runtime::resolve_navigation};
use crate::{SwipeGesture, TapPoint, UiSession};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

#[derive(Default)]
struct Session {
    reads: AtomicUsize,
    change: bool,
    generations: Option<[u64; 2]>,
    read_error: Option<ReadError>,
}

enum ReadError {
    Immediate,
    AfterDeadline,
    AfterCancellation(tokio::sync::watch::Sender<bool>),
}
#[async_trait::async_trait]
impl UiSession for Session {
    fn stream_url(&self) -> Option<String> {
        None
    }
    fn gui_session_epoch(&self) -> String {
        "epoch".into()
    }
    fn supports_element_bounds(&self) -> bool {
        true
    }
    async fn window_size(&self) -> anyhow::Result<(f64, f64)> {
        Ok((400., 800.))
    }
    async fn active_app_bundle(&self) -> anyhow::Result<String> {
        Ok("com.android.settings".into())
    }
    async fn ui_language(&self) -> Option<String> {
        Some("en-US".into())
    }
    async fn app_version(&self, _: &str) -> Option<String> {
        Some("unknown".into())
    }
    async fn hierarchy_source_snapshot(&self) -> anyhow::Result<crate::HierarchySourceSnapshot> {
        let read = self.reads.fetch_add(1, Ordering::SeqCst);
        let text = if self.change && read > 0 {
            "Factory reset"
        } else {
            "About phone"
        };
        let generation = self
            .generations
            .map_or(read as u64 + 1, |values| values[read]);
        Ok(crate::HierarchySourceSnapshot{generation,xml:format!("<hierarchy><node package=\"com.android.settings\" text=\"Settings\" bounds=\"[0,0][400,50]\"/><node package=\"com.android.settings\" text=\"{text}\" bounds=\"[10,100][200,150]\" enabled=\"true\" clickable=\"true\"/></hierarchy>")})
    }
    async fn observe(
        &self,
        request: &super::ObservationRequest,
    ) -> anyhow::Result<super::UiObservation> {
        match self.read_error.as_ref().unwrap() {
            ReadError::Immediate => {}
            ReadError::AfterDeadline => {
                // Model synchronous decoding finishing after the enclosing timer was polled.
                std::thread::sleep(Duration::from_millis(request.remaining_ms + 5));
            }
            ReadError::AfterCancellation(sender) => sender.send(true).unwrap(),
        }
        anyhow::bail!("observation transport failed")
    }
    async fn tap(&self, _: TapPoint) -> anyhow::Result<()> {
        panic!("resolver must never tap")
    }
    async fn swipe(&self, _: SwipeGesture) -> anyhow::Result<()> {
        panic!("resolver must never swipe")
    }
    async fn type_text(&self, _: &str) -> anyhow::Result<()> {
        panic!("resolver must never type")
    }
    async fn home(&self) -> anyhow::Result<()> {
        panic!("resolver must never navigate")
    }
    async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
        panic!("resolver must never tap")
    }
    async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
        unreachable!()
    }
}
#[tokio::test]
async fn target_is_reobserved_and_a_changed_target_is_rejected() {
    for (change, generations, accepted) in [
        (false, [2, 3], true),
        (true, [2, 3], false),
        (false, [2, 2], false),
        (false, [2, 1], false),
    ] {
        let session = Session {
            change,
            generations: Some(generations),
            ..Default::default()
        };
        let result = resolve_navigation(&session, "aboutDevice", Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(
            result.is_some(),
            accepted,
            "change={change}, generations={generations:?}"
        );
        assert_eq!(session.reads.load(Ordering::SeqCst), 2);
    }
}

#[tokio::test]
async fn observation_read_errors_respect_completion_deadline_and_cancellation() {
    use super::{
        runtime::wait_for_observation, ObservationExpectation, ObservationRequest,
        ObservationWaitStatus,
    };
    let mut outcomes = Vec::new();
    for boundary in ["immediate", "deadline", "cancel"] {
        let (sender, mut cancel) = tokio::sync::watch::channel(false);
        let session = Session {
            read_error: Some(match boundary {
                "deadline" => ReadError::AfterDeadline,
                "cancel" => ReadError::AfterCancellation(sender.clone()),
                _ => ReadError::Immediate,
            }),
            ..Default::default()
        };
        let request = ObservationRequest {
            query: Default::default(),
            scope: None,
            fields: Default::default(),
            remaining_ms: if boundary == "deadline" { 20 } else { 1000 },
        };
        let result = wait_for_observation(
            &session,
            &request,
            &ObservationExpectation::Exists,
            tokio::time::Instant::now() + Duration::from_millis(request.remaining_ms),
            &mut cancel,
            Duration::from_millis(1),
        )
        .await;
        outcomes.push(
            result
                .map(|result| result.status)
                .map_err(|error| error.to_string()),
        );
    }
    assert_eq!(
        outcomes,
        [
            Err("observation transport failed".to_owned()),
            Ok(ObservationWaitStatus::DeadlineExceeded),
            Ok(ObservationWaitStatus::Cancelled),
        ]
    );
}

#[test]
fn rust_contract_matches_python_wire_fixture() {
    let request: GuiRequest =
        serde_json::from_str(include_str!("../../fixtures/gui-request.json")).unwrap();
    assert_eq!(request.protocol_version, 1);
    assert_eq!(request.app.system_locale, "en-US");
    assert_eq!(request.nodes[0].id, 3);
    let encoded = serde_json::to_value(request).unwrap();
    assert!(encoded.get("observationId").is_some());
}
