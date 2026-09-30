//! The real preparation boundary must not spend a second read on a partial tree.
use super::*;
use crate::ui_automation::{
    ObservationCompleteness, ObservationRequest, ObservationSource, ObservedAppContext,
    SemanticNode, UiObservation,
};
use std::sync::atomic::AtomicUsize;

const PACKAGE: &str = "com.ss.android.ugc.trill";

struct PreparationSession {
    observations: AtomicUsize,
    editor_reads: AtomicUsize,
    extra_caption_reads: AtomicUsize,
    taps: AtomicUsize,
    unproved_caption: Option<&'static str>,
}

#[async_trait::async_trait]
impl UiSession for PreparationSession {
    async fn observe(&self, request: &ObservationRequest) -> anyhow::Result<UiObservation> {
        let caption = request.query.id.as_deref() == Some("com.ss.android.ugc.trill:id/eej");
        let mut matches = Vec::new();
        let mut source = ObservationSource::AccessibilityHierarchy;
        let mut unknown_match_count = 0;
        if self.taps.load(Ordering::Relaxed) == 0 && caption {
            tokio::time::sleep(Duration::from_millis(200)).await;
            matches.push(SemanticNode {
                node_id: 1,
                id: Some(format!("{PACKAGE}:id/eej")),
                class_name: Some("android.widget.EditText".into()),
                package: Some(PACKAGE.into()),
                role: Some("textbox".into()),
                text: Some("approved caption".into()),
                visible: Some(true),
                enabled: Some(true),
                password: Some(false),
                showing_hint: Some(false),
                ..Default::default()
            });
        } else if caption {
            // The primary observation already showed no caption evidence. An
            // independently requested caption probe costs the remaining window.
            self.extra_caption_reads.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(600)).await;
        } else {
            let read = self.editor_reads.fetch_add(1, Ordering::Relaxed);
            if read == 0 {
                // Live #8, 30/09/2026: first post-Back source returned Unknown
                // after about 6.3 s. A positive editor arrives in the next read.
                tokio::time::sleep(Duration::from_millis(
                    if self.unproved_caption == Some("caption-transition") {
                        700
                    } else {
                        6300
                    },
                ))
                .await;
                if let Some(mode) = self.unproved_caption {
                    if mode == "upstream-unknown-candidate" {
                        unknown_match_count = 1;
                    }
                    matches.push(SemanticNode {
                        node_id: 3,
                        id: Some(format!("{PACKAGE}:id/eej")),
                        class_name: Some("android.widget.EditText".into()),
                        package: Some(PACKAGE.into()),
                        role: if mode == "missing-role" {
                            None
                        } else {
                            Some("textbox".into())
                        },
                        text: Some("approved caption".into()),
                        visible: Some(true),
                        enabled: Some(true),
                        password: Some(false),
                        showing_hint: Some(false),
                        ..Default::default()
                    });
                    if mode == "unreadable-native-id" {
                        source = ObservationSource::NativeQuery;
                        matches.push(SemanticNode {
                            node_id: 4,
                            package: Some(PACKAGE.into()),
                            role: Some("textbox".into()),
                            text: Some("other candidate".into()),
                            visible: Some(true),
                            enabled: Some(true),
                            password: Some(false),
                            showing_hint: Some(false),
                            ..Default::default()
                        });
                    }
                }
            } else {
                tokio::time::sleep(Duration::from_millis(900)).await;
                matches.push(SemanticNode {
                    node_id: 2,
                    id: Some(format!("{PACKAGE}:id/so9")),
                    class_name: Some("android.widget.TextView".into()),
                    package: Some(PACKAGE.into()),
                    text: Some("Sound A".into()),
                    visible: Some(true),
                    password: Some(false),
                    ..Default::default()
                });
            }
        }
        let generation = self.observations.fetch_add(1, Ordering::Relaxed) + 1;
        Ok(UiObservation {
            observation_id: format!("preparation-{generation}"),
            generation: generation as u64,
            device_id: "preparation-phone".into(),
            session_epoch: "owned-preparation".into(),
            app: ObservedAppContext {
                package: Some(PACKAGE.into()),
                ..Default::default()
            },
            started_at_ms: 0,
            ended_at_ms: 1,
            source,
            completeness: ObservationCompleteness::Partial,
            matches,
            unknown_match_count,
        })
    }
    fn gui_session_epoch(&self) -> String {
        "owned-preparation".into()
    }
    async fn locate_all(&self, query: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
        anyhow::ensure!(
            query == ElementQuery::ResourceIdSuffix(":id/aun"),
            "unexpected lookup"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        Ok(vec![ElementBox {
            x: 32.,
            y: 95.,
            width: 105.,
            height: 105.,
            description: Some("Back".into()),
            enabled: true,
            clickable: true,
        }])
    }
    async fn tap(&self, _: crate::TapPoint) -> anyhow::Result<()> {
        self.taps.fetch_add(1, Ordering::Relaxed);
        tokio::time::sleep(Duration::from_millis(200)).await;
        Ok(())
    }
    async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
        anyhow::bail!("unexpected swipe")
    }
    async fn type_text(&self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("unexpected typing")
    }
    async fn home(&self) -> anyhow::Result<()> {
        anyhow::bail!("unexpected Home")
    }
    async fn back(&self) -> anyhow::Result<()> {
        anyhow::bail!("unexpected hardware Back")
    }
    async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("unexpected target")
    }
    async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("unexpected visibility")
    }
    fn stream_url(&self) -> Option<String> {
        None
    }
}

#[tokio::test(start_paused = true)]
async fn partial_editor_read_does_not_spend_a_second_caption_probe_before_recovery() {
    let session = PreparationSession {
        observations: AtomicUsize::new(0),
        editor_reads: AtomicUsize::new(0),
        extra_caption_reads: AtomicUsize::new(0),
        taps: AtomicUsize::new(0),
        unproved_caption: None,
    };
    let labels = crate::tiktok_labels::controls_for(PACKAGE, "en", "38.3.2").unwrap();
    let mut plan = ComposerPlan::resolve(&labels).unwrap();
    // This fixture names the same measured caption field, so its semantic
    // snapshot can independently prove the exact approved text.
    plan.publish.as_mut().unwrap().caption = ElementQuery::ResourceIdSuffix(":id/eej");
    let mut composer = Composer::new(&session, plan, |element: &ElementBox| element.centre());
    let started = Instant::now();
    composer
        .prepare_sound_editor(
            SoundPickerPlan::resolve(PACKAGE, "en", "38.3.2").unwrap(),
            "Sound A",
            "approved caption",
            &AtomicBool::new(false),
        )
        .await
        .expect("partial first source must recover within the original eight-second budget");
    assert!(started.elapsed() <= Duration::from_secs(8));
    assert_eq!(
        session.taps.load(Ordering::Relaxed),
        1,
        "Unknown cannot authorize a second Back"
    );
    assert_eq!(session.extra_caption_reads.load(Ordering::Relaxed), 0);
    assert_eq!(session.editor_reads.load(Ordering::Relaxed), 2);
}

#[tokio::test(start_paused = true)]
async fn caption_transition_during_back_lookup_does_not_retry_on_the_editor() {
    let session = PreparationSession {
        observations: AtomicUsize::new(0),
        editor_reads: AtomicUsize::new(0),
        extra_caption_reads: AtomicUsize::new(0),
        taps: AtomicUsize::new(0),
        unproved_caption: Some("caption-transition"),
    };
    let labels = crate::tiktok_labels::controls_for(PACKAGE, "en", "38.3.2").unwrap();
    let mut plan = ComposerPlan::resolve(&labels).unwrap();
    plan.publish.as_mut().unwrap().caption = ElementQuery::ResourceIdSuffix(":id/eej");
    let mut composer = Composer::new(&session, plan, |element: &ElementBox| element.centre());
    let started = Instant::now();
    composer
        .prepare_sound_editor(
            SoundPickerPlan::resolve(PACKAGE, "en", "38.3.2").unwrap(),
            "Sound A",
            "approved caption",
            &AtomicBool::new(false),
        )
        .await
        .expect("a completed first Back must retain the selected editor");
    assert_eq!(
        session.taps.load(Ordering::Relaxed),
        1,
        "caption proof taken before the Back lookup cannot authorize a second Back on the editor"
    );
    assert!(started.elapsed() <= Duration::from_secs(8));
    assert_eq!(
        session.extra_caption_reads.load(Ordering::Relaxed),
        0,
        "the retry predicate is one batched editor/caption observation"
    );
}

#[tokio::test(start_paused = true)]
async fn batch_caption_with_missing_role_or_native_identity_cannot_retry_back() {
    for mode in [
        "missing-role",
        "unreadable-native-id",
        "upstream-unknown-candidate",
    ] {
        let session = PreparationSession {
            observations: AtomicUsize::new(0),
            editor_reads: AtomicUsize::new(0),
            extra_caption_reads: AtomicUsize::new(0),
            taps: AtomicUsize::new(0),
            unproved_caption: Some(mode),
        };
        let labels = crate::tiktok_labels::controls_for(PACKAGE, "en", "38.3.2").unwrap();
        let mut plan = ComposerPlan::resolve(&labels).unwrap();
        plan.publish.as_mut().unwrap().caption = ElementQuery::ResourceIdSuffix(":id/eej");
        let mut composer = Composer::new(&session, plan, |element: &ElementBox| element.centre());
        composer
            .prepare_sound_editor(
                SoundPickerPlan::resolve(PACKAGE, "en", "38.3.2").unwrap(),
                "Sound A",
                "approved caption",
                &AtomicBool::new(false),
            )
            .await
            .expect("unproved caption permits observations but no additional Back");
        assert_eq!(session.taps.load(Ordering::Relaxed), 1, "mode {mode}");
        assert_eq!(
            session.extra_caption_reads.load(Ordering::Relaxed),
            0,
            "mode {mode}"
        );
    }
}
