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
    TransientThenAbsent,
    SessionChanges,
    DriverRepair,
}
#[async_trait::async_trait]
impl UiSession for Session {
    fn stream_url(&self) -> Option<String> {
        None
    }
    fn gui_session_epoch(&self) -> String {
        if matches!(
            self.read_error,
            Some(ReadError::SessionChanges | ReadError::DriverRepair)
        ) && self.reads.load(Ordering::SeqCst) > 0
        {
            "replacement".into()
        } else {
            "epoch".into()
        }
    }
    fn gui_scope(&self) -> Option<super::GuiScope> {
        matches!(self.read_error, Some(ReadError::DriverRepair)).then(|| super::GuiScope {
            run_id: "run".into(),
            assignment_id: None,
            device_id: "phone".into(),
            deadline_ms: None,
        })
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
        let read = self.reads.fetch_add(1, Ordering::SeqCst);
        match self.read_error.as_ref().unwrap() {
            ReadError::Immediate => {}
            ReadError::AfterDeadline => {
                // Model synchronous decoding finishing after the enclosing timer was polled.
                std::thread::sleep(Duration::from_millis(request.remaining_ms + 5));
            }
            ReadError::AfterCancellation(sender) => sender.send(true).unwrap(),
            ReadError::TransientThenAbsent if read == 0 => {
                return Err(crate::driver::UiError::new(
                    crate::driver::UiErrorKind::Timeout,
                    "observe",
                    "read response timed out",
                )
                .into());
            }
            ReadError::TransientThenAbsent | ReadError::SessionChanges => {
                return Ok(super::UiObservation {
                    device_id: "phone".into(),
                    app: super::ObservedAppContext {
                        package: Some("com.android.settings".into()),
                        ..Default::default()
                    },
                    session_epoch: self.gui_session_epoch(),
                    observation_id: format!("observation-{read}"),
                    generation: read as u64 + 1,
                    started_at_ms: 1,
                    ended_at_ms: 2,
                    source: super::ObservationSource::AccessibilityHierarchy,
                    completeness: if read == 1 {
                        super::ObservationCompleteness::Partial
                    } else {
                        super::ObservationCompleteness::Complete
                    },
                    matches: Vec::new(),
                    unknown_match_count: 0,
                });
            }
            ReadError::DriverRepair => {
                assert_eq!(
                    read, 0,
                    "a repaired wait must use the no-recovery read next"
                );
                return Err(crate::driver::SessionEpochChanged {
                    previous_epoch: "epoch".into(),
                    current_epoch: "replacement".into(),
                    device_id: "phone".into(),
                    package: "com.android.settings".into(),
                    fresh_observation: Box::new(repaired_observation(
                        1,
                        super::ObservationCompleteness::Partial,
                    )),
                }
                .into());
            }
        }
        anyhow::bail!("observation transport failed")
    }
    async fn observe_without_recovery(
        &self,
        _: &super::ObservationRequest,
    ) -> anyhow::Result<super::UiObservation> {
        assert!(matches!(self.read_error, Some(ReadError::DriverRepair)));
        let read = self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(repaired_observation(
            read as u64 + 1,
            super::ObservationCompleteness::Complete,
        ))
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

fn repaired_observation(
    generation: u64,
    completeness: super::ObservationCompleteness,
) -> super::UiObservation {
    super::UiObservation {
        device_id: "phone".into(),
        app: super::ObservedAppContext {
            package: Some("com.android.settings".into()),
            ..Default::default()
        },
        session_epoch: "replacement".into(),
        observation_id: format!("repaired-{generation}"),
        generation,
        started_at_ms: 1,
        ended_at_ms: 2,
        source: super::ObservationSource::AccessibilityHierarchy,
        completeness,
        matches: Vec::new(),
        unknown_match_count: 0,
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

#[tokio::test]
async fn observation_wait_retries_only_the_read_and_partial_absence_stays_unknown() {
    use super::{
        runtime::wait_for_observation, ObservationExpectation, ObservationRequest,
        ObservationWaitStatus,
    };
    let (_sender, mut cancel) = tokio::sync::watch::channel(false);
    let session = Session {
        read_error: Some(ReadError::TransientThenAbsent),
        ..Default::default()
    };
    let request = ObservationRequest {
        query: Default::default(),
        scope: None,
        fields: Default::default(),
        remaining_ms: 1000,
    };
    let result = wait_for_observation(
        &session,
        &request,
        &ObservationExpectation::Absent,
        tokio::time::Instant::now() + Duration::from_millis(request.remaining_ms),
        &mut cancel,
        Duration::from_millis(1),
    )
    .await
    .expect("one transient read must not abort the caller's wait");
    assert_eq!(result.status, ObservationWaitStatus::Satisfied);
    assert_eq!(session.reads.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn observation_wait_rejects_a_replacement_session_even_on_its_first_read() {
    use super::{runtime::wait_for_observation, ObservationExpectation, ObservationRequest};
    let (_sender, mut cancel) = tokio::sync::watch::channel(false);
    let session = Session {
        read_error: Some(ReadError::SessionChanges),
        ..Default::default()
    };
    let request = ObservationRequest {
        query: Default::default(),
        scope: None,
        fields: Default::default(),
        remaining_ms: 1000,
    };
    let error = wait_for_observation(
        &session,
        &request,
        &ObservationExpectation::Absent,
        tokio::time::Instant::now() + Duration::from_millis(request.remaining_ms),
        &mut cancel,
        Duration::from_millis(1),
    )
    .await
    .expect_err("session recovery must invalidate the wait's original binding");
    assert_eq!(error.to_string(), "observation_session_changed");
    assert_eq!(session.reads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn observation_wait_rebinds_driver_repair_then_polls_without_another_repair() {
    use super::{
        runtime::wait_for_observation, ObservationExpectation, ObservationRequest,
        ObservationScope, ObservationWaitStatus,
    };
    let (_sender, mut cancel) = tokio::sync::watch::channel(false);
    let session = Session {
        read_error: Some(ReadError::DriverRepair),
        ..Default::default()
    };
    let request = ObservationRequest {
        query: Default::default(),
        scope: Some(ObservationScope {
            package: Some("com.android.settings".into()),
            root: None,
        }),
        fields: Default::default(),
        remaining_ms: 1000,
    };
    let result = wait_for_observation(
        &session,
        &request,
        &ObservationExpectation::Absent,
        tokio::time::Instant::now() + Duration::from_millis(request.remaining_ms),
        &mut cancel,
        Duration::from_millis(1),
    )
    .await
    .unwrap();
    assert_eq!(result.status, ObservationWaitStatus::Satisfied);
    assert_eq!(
        session.reads.load(Ordering::SeqCst),
        2,
        "repaired Partial evidence cannot prove absence; the next no-recovery read must prove it"
    );
}

#[tokio::test(start_paused = true)]
async fn bounded_read_stops_pending_reads_and_keeps_permanent_errors() {
    use super::runtime::{read_before_deadline, ReadWaitResult};
    use std::sync::{atomic::AtomicBool, Arc};
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    let task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1)).await;
        stopping.store(true, Ordering::Relaxed);
    });
    let result = read_before_deadline(
        std::future::pending::<anyhow::Result<()>>(),
        tokio::time::Instant::now() + Duration::from_secs(2),
        &stop,
    )
    .await
    .unwrap();
    task.await.unwrap();
    assert!(matches!(result, ReadWaitResult::Cancelled));
    stop.store(false, Ordering::Relaxed);
    let error = read_before_deadline(
        async { Err::<(), _>(anyhow::anyhow!("observation_binding_invalid")) },
        tokio::time::Instant::now() + Duration::from_secs(2),
        &stop,
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "observation_binding_invalid");
}

#[tokio::test]
async fn bounded_read_does_not_accept_late_ready_or_error_after_stop() {
    use super::runtime::{read_before_deadline, ReadWaitResult};
    use std::sync::atomic::AtomicBool;
    for failure in [false, true] {
        let stop = AtomicBool::new(false);
        let result = read_before_deadline(
            async {
                stop.store(true, Ordering::Relaxed);
                if failure {
                    anyhow::bail!("read response timed out");
                }
                Ok(())
            },
            tokio::time::Instant::now() + Duration::from_secs(2),
            &stop,
        )
        .await
        .unwrap();
        assert!(matches!(result, ReadWaitResult::Cancelled));
    }
}

#[test]
fn read_recovery_classification_does_not_retry_actions_or_http_validation() {
    use crate::driver::{classify_read_failure, ReadFailureKind, UiError, UiErrorKind};
    for (kind, op, expected) in [
        (UiErrorKind::Timeout, "observe", ReadFailureKind::Transient),
        (UiErrorKind::Transport, "locate", ReadFailureKind::Transient),
        (UiErrorKind::Session, "readText", ReadFailureKind::Transient),
        (
            UiErrorKind::Timeout,
            "actions.swipe",
            ReadFailureKind::Permanent,
        ),
        (UiErrorKind::Transport, "tap", ReadFailureKind::Permanent),
        (UiErrorKind::Http, "observe", ReadFailureKind::Permanent),
    ] {
        assert_eq!(
            classify_read_failure(&UiError::new(kind, op, "fixture failure").into()),
            expected,
            "kind={kind:?}, op={op}"
        );
    }
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
