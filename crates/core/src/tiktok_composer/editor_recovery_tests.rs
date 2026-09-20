use super::*;
use std::sync::atomic::AtomicUsize;

const PACKAGE: &str = "com.zhiliaoapp.musically";

struct EditorSession {
    taps: AtomicUsize,
    reads: AtomicUsize,
    xml: String,
    permanent_failure: bool,
    fixed_generation: bool,
    change_epoch: bool,
    stop_on_read: Option<std::sync::Arc<AtomicBool>>,
    frames: AtomicUsize,
    ocr: Option<crate::ui_automation::SharedReasoner>,
}

fn editor_xml(duplicate: bool, loading: bool) -> String {
    // Global 45.7.3/en, ab9, 20/09: Next text p31 inside p2y, with Your Story
    // on the left. Only these measured non-personal nodes are retained here.
    let next = format!(
        r#"<node package="{PACKAGE}" resource-id="{PACKAGE}:id/p31" text="Next" bounds="[749,1985][843,2038]" enabled="true" displayed="true"/>"#
    );
    format!(
        r#"<hierarchy><node package="{PACKAGE}" bounds="[0,0][1080,2220]" displayed="true"><node package="{PACKAGE}" text="Your Story" bounds="[35,1985][526,2038]" enabled="true" displayed="true"/>{next}{}{}</node></hierarchy>"#,
        if duplicate { next.as_str() } else { "" },
        if loading {
            r#"<node package="com.zhiliaoapp.musically" text="Loading..." bounds="[368,100][711,216]" displayed="true"/>"#
        } else {
            ""
        }
    )
}

impl EditorSession {
    fn ready() -> Self {
        Self {
            taps: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
            xml: editor_xml(false, false),
            permanent_failure: false,
            fixed_generation: false,
            change_epoch: false,
            stop_on_read: None,
            frames: AtomicUsize::new(0),
            ocr: None,
        }
    }
}

#[async_trait::async_trait]
impl UiSession for EditorSession {
    fn gui_reasoner(&self) -> Option<crate::ui_automation::SharedReasoner> {
        self.ocr.clone()
    }
    async fn active_app_bundle(&self) -> anyhow::Result<String> {
        Ok(PACKAGE.into())
    }
    async fn screenshot_png(&self) -> anyhow::Result<Vec<u8>> {
        self.frames.fetch_add(1, Ordering::Relaxed);
        let mut b = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(1080, 2220).write_to(&mut b, image::ImageFormat::Png)?;
        Ok(b.into_inner())
    }
    fn stream_url(&self) -> Option<String> {
        None
    }
    async fn tap(&self, _: crate::TapPoint) -> anyhow::Result<()> {
        self.taps.fetch_add(1, Ordering::Relaxed);
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
    async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("unexpected target")
    }
    async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("unexpected visibility")
    }
    fn supports_element_bounds(&self) -> bool {
        true
    }
    fn gui_session_epoch(&self) -> String {
        if self.change_epoch {
            self.reads.load(Ordering::Relaxed).to_string()
        } else {
            "owned-session".into()
        }
    }
    async fn locate(&self, _: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
        if self.ocr.is_some() {
            anyhow::ensure!(
                self.frames.load(Ordering::Relaxed) >= 4,
                "queried accessibility while editor still loading"
            );
            return Ok(Some(picker_next()));
        }
        Err(crate::driver::AccessibilityReadUnavailable {
            message: "waiting for the root AccessibilityNodeInfo after picker Next".into(),
        }
        .into())
    }
    async fn hierarchy_source_snapshot(&self) -> anyhow::Result<crate::HierarchySourceSnapshot> {
        if self.ocr.is_some() {
            anyhow::ensure!(
                self.frames.load(Ordering::Relaxed) >= 4,
                "queried accessibility while editor still loading"
            );
        }
        let read = self.reads.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some(stop) = &self.stop_on_read {
            stop.store(true, Ordering::Relaxed);
        }
        if self.permanent_failure {
            return Err(crate::driver::AccessibilityReadUnavailable {
                message: "root still unavailable".into(),
            }
            .into());
        }
        Ok(crate::HierarchySourceSnapshot {
            generation: if self.fixed_generation {
                1
            } else {
                read as u64
            },
            xml: self.xml.clone(),
        })
    }
}

struct EditorOcr {
    calls: AtomicUsize,
}
#[async_trait::async_trait]
impl crate::ui_automation::GuiReasoner for EditorOcr {
    async fn resolve(
        &self,
        _: crate::ui_automation::GuiRequest,
    ) -> anyhow::Result<crate::ui_automation::GuiResponse> {
        unreachable!()
    }
    async fn ocr(
        &self,
        r: crate::ui_automation::OcrRequest,
    ) -> anyhow::Result<crate::ui_automation::OcrResponse> {
        use crate::ui_automation::{OcrLine, OcrRect, OcrResponse};
        let loading = self.calls.fetch_add(1, Ordering::Relaxed) < 2;
        let mut lines = vec![
            OcrLine {
                text: "Your Story".into(),
                bounds: OcrRect {
                    x: 180,
                    y: 1985,
                    width: 230,
                    height: 53,
                },
                confidence: 0.96,
            },
            OcrLine {
                text: "Next".into(),
                bounds: OcrRect {
                    x: 749,
                    y: 1985,
                    width: 94,
                    height: 53,
                },
                confidence: 0.96,
            },
        ];
        if loading && r.roi.is_none() {
            lines.push(OcrLine {
                text: "Loading...".into(),
                bounds: OcrRect {
                    x: 368,
                    y: 100,
                    width: 300,
                    height: 60,
                },
                confidence: 0.96,
            });
        }
        Ok(OcrResponse {
            protocol_version: 1,
            request_id: r.request_id,
            observation_id: r.observation_id,
            session_epoch: r.session_epoch,
            generation: r.generation,
            screenshot_sha256: r.screenshot.sha256,
            status: "resolved".into(),
            text: lines
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            lines,
            engine: "fixture".into(),
            elapsed_ms: 1,
        })
    }
}
#[tokio::test(start_paused = true)]
async fn editor_arrival_is_proved_by_two_frames_while_suggested_sound_loads() {
    let mut s = EditorSession::ready();
    s.ocr = Some(std::sync::Arc::new(EditorOcr {
        calls: AtomicUsize::new(0),
    }));
    let mut c = Composer::new(&s, plan(), |e: &ElementBox| e.centre());
    assert!(c
        .advance_to_edit_step(&picker_next(), &AtomicBool::new(false))
        .await
        .unwrap());
    assert_eq!(s.frames.load(Ordering::Relaxed), 2);
    assert_eq!(s.reads.load(Ordering::Relaxed), 0);
    assert_eq!(s.taps.load(Ordering::Relaxed), 1);
}

fn plan() -> ComposerPlan {
    ComposerPlan::resolve(&crate::tiktok_labels::controls_for(PACKAGE, "en", "45.7.3").unwrap())
        .unwrap()
}
fn picker_next() -> ElementBox {
    ElementBox {
        x: 550.,
        y: 1936.,
        width: 498.,
        height: 116.,
        description: Some("Next (1)".into()),
        enabled: true,
        clickable: true,
    }
}

#[tokio::test(start_paused = true)]
async fn editor_recovers_a_failed_element_read_with_two_new_owned_snapshots() {
    let session = EditorSession::ready();
    let mut composer = Composer::new(&session, plan(), |e: &ElementBox| e.centre());
    assert!(composer
        .advance_to_edit_step(&picker_next(), &AtomicBool::new(false))
        .await
        .unwrap());
    assert_eq!(session.reads.load(Ordering::Relaxed), 2);
    assert_eq!(
        session.taps.load(Ordering::Relaxed),
        1,
        "only picker Next, never editor Next or Post"
    );
}

#[tokio::test(start_paused = true)]
async fn editor_recovery_refuses_duplicate_loading_stale_or_replaced_observations() {
    for mode in 0..5 {
        let mut session = EditorSession::ready();
        match mode {
            0 => session.xml = editor_xml(true, false),
            1 => session.xml = editor_xml(false, true),
            2 => session.fixed_generation = true,
            3 => session.change_epoch = true,
            _ => session.permanent_failure = true,
        }
        let mut composer = Composer::new(&session, plan(), |e: &ElementBox| e.centre());
        let started = Instant::now();
        let result = composer
            .advance_to_edit_step(&picker_next(), &AtomicBool::new(false))
            .await;
        assert!(!matches!(result, Ok(true)), "mode {mode}");
        assert!(started.elapsed() <= Duration::from_secs(31));
        assert_eq!(session.taps.load(Ordering::Relaxed), 1);
    }
}

#[tokio::test(start_paused = true)]
async fn editor_recovery_checks_stop_after_the_read_and_before_accepting_arrival() {
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let mut session = EditorSession::ready();
    session.stop_on_read = Some(stop.clone());
    let mut composer = Composer::new(&session, plan(), |e: &ElementBox| e.centre());
    assert!(!composer
        .advance_to_edit_step(&picker_next(), &stop)
        .await
        .unwrap());
    assert_eq!(session.taps.load(Ordering::Relaxed), 1);
}

#[tokio::test(start_paused = true)]
async fn editor_recovery_does_not_tap_picker_after_stop() {
    let session = EditorSession::ready();
    let mut composer = Composer::new(&session, plan(), |e: &ElementBox| e.centre());
    assert!(!composer
        .advance_to_edit_step(&picker_next(), &AtomicBool::new(true))
        .await
        .unwrap());
    assert_eq!(session.taps.load(Ordering::Relaxed), 0);
}
