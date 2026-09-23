use super::*;
use crate::ui_automation::{
    GuiReasoner, GuiRequest, GuiResponse, OcrLine, OcrRect, OcrRequest, OcrResponse,
};
use std::sync::atomic::AtomicUsize;

struct Ocr {
    bad_binding: bool,
    missing_tab: bool,
    stop: Option<Arc<AtomicBool>>,
}
#[async_trait::async_trait]
impl GuiReasoner for Ocr {
    async fn resolve(&self, _: GuiRequest) -> anyhow::Result<GuiResponse> {
        unreachable!()
    }
    async fn ocr(&self, r: OcrRequest) -> anyhow::Result<OcrResponse> {
        let mut lines: Vec<_> = ["Hot", "For You", "Favorites", "Recent"]
            .iter()
            .enumerate()
            .map(|(i, text)| OcrLine {
                text: (*text).into(),
                confidence: 0.96,
                bounds: OcrRect {
                    x: 40 + i as u32 * 190,
                    y: 1120,
                    width: 120,
                    height: 32,
                },
            })
            .collect();
        if self.missing_tab {
            lines.pop();
        }
        if let Some(stop) = &self.stop {
            stop.store(true, Ordering::Relaxed);
            tokio::time::sleep(POLL).await;
        }
        Ok(OcrResponse {
            protocol_version: r.protocol_version,
            request_id: r.request_id,
            observation_id: r.observation_id,
            session_epoch: r.session_epoch,
            generation: r.generation,
            screenshot_sha256: if self.bad_binding {
                "f".repeat(64)
            } else {
                r.screenshot.sha256
            },
            status: "resolved".into(),
            text: lines
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            lines,
            engine: "fixture local OCR".into(),
            elapsed_ms: 1,
        })
    }
}
struct Session {
    taps: AtomicUsize,
    backs: AtomicUsize,
    reads: AtomicUsize,
    frames: AtomicUsize,
    title: &'static str,
    read_failure: bool,
    selected_marker: bool,
    changed_app: bool,
    unreadable_tab: bool,
    missing_tab_readback: bool,
    empty_tab_before_navigation: bool,
    ocr: crate::ui_automation::SharedReasoner,
    loading_entry: bool,
    png: Vec<u8>,
}
impl Session {
    fn new() -> Self {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(1080, 2220)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        Self {
            taps: AtomicUsize::new(0),
            backs: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
            frames: AtomicUsize::new(0),
            title: "One",
            read_failure: true,
            selected_marker: false,
            changed_app: false,
            unreadable_tab: false,
            missing_tab_readback: false,
            empty_tab_before_navigation: false,
            loading_entry: false,
            ocr: Arc::new(Ocr {
                bad_binding: false,
                missing_tab: false,
                stop: None,
            }),
            png: bytes.into_inner(),
        }
    }
}
fn sound_xml(selected: bool) -> String {
    let node = |id: &str, text: &str, bounds: &str, selected: bool| {
        format!(
            r#"<node package="com.zhiliaoapp.musically" resource-id="com.zhiliaoapp.musically{id}" text="{text}" bounds="{bounds}" displayed="true" enabled="true" clickable="true" selected="{selected}"/>"#
        )
    };
    format!(
        "<hierarchy>{}{}{}{}{}{}</hierarchy>",
        node(":id/wrv", "Hot", "[40,1100][140,1150]", true),
        node(":id/viewpager_container", "", "[0,1200][1080,1900]", false),
        node(
            ":id/vertical_item_music_new_rl",
            "",
            "[0,1300][1080,1480]",
            false
        ),
        node(":id/title", "One", "[220,1320][650,1360]", false),
        node(":id/z3k", "Artist", "[220,1370][700,1410]", false),
        if selected {
            node(":id/nms", "", "[180,1330][200,1360]", false)
        } else {
            String::new()
        }
    )
}
#[async_trait::async_trait]
impl UiSession for Session {
    async fn tap(&self, _: crate::TapPoint) -> anyhow::Result<()> {
        self.taps.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    async fn back(&self) -> anyhow::Result<()> {
        self.backs.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
        unreachable!()
    }
    async fn type_text(&self, _: &str) -> anyhow::Result<()> {
        unreachable!()
    }
    async fn home(&self) -> anyhow::Result<()> {
        unreachable!()
    }
    async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
        unreachable!()
    }
    async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
        unreachable!()
    }
    fn stream_url(&self) -> Option<String> {
        None
    }
    fn gui_session_epoch(&self) -> String {
        "owned-session".into()
    }
    fn gui_reasoner(&self) -> Option<crate::ui_automation::SharedReasoner> {
        Some(self.ocr.clone())
    }
    async fn active_app_bundle(&self) -> anyhow::Result<String> {
        Ok(if self.changed_app {
            "com.other.app"
        } else {
            "com.zhiliaoapp.musically"
        }
        .into())
    }
    async fn screenshot_png(&self) -> anyhow::Result<Vec<u8>> {
        self.frames.fetch_add(1, Ordering::Relaxed);
        Ok(self.png.clone())
    }
    async fn hierarchy_source_snapshot(&self) -> anyhow::Result<crate::HierarchySourceSnapshot> {
        let n = self.reads.fetch_add(1, Ordering::Relaxed) + 1;
        if self.loading_entry && self.frames.load(Ordering::Relaxed) < 4 {
            return Err(crate::driver::AccessibilityReadUnavailable {
                message: "accessibility queried before rendered sound rows settled".into(),
            }
            .into());
        }
        if self.unreadable_tab {
            if self.empty_tab_before_navigation && self.taps.load(Ordering::Relaxed) == 0 {
                tokio::time::sleep(Duration::from_secs(5)).await;
                return Ok(crate::HierarchySourceSnapshot {
                    generation: n as u64,
                    xml: "<hierarchy/>".into(),
                });
            }
            if self.taps.load(Ordering::Relaxed) == 0 || self.missing_tab_readback {
                tokio::time::sleep(Duration::from_secs(21)).await;
                return Err(crate::driver::AccessibilityReadUnavailable {
                    message: "sound sheet root unavailable before Hot navigation".into(),
                }
                .into());
            }
            return Ok(crate::HierarchySourceSnapshot {
                generation: n as u64,
                xml: sound_xml(false),
            });
        }
        if self.taps.load(Ordering::Relaxed) > 0
            && self.backs.load(Ordering::Relaxed) == 0
            && self.read_failure
        {
            return Err(crate::driver::AccessibilityReadUnavailable {
                message: "root unavailable after acknowledged sound tap".into(),
            }
            .into());
        }
        let xml = if self.backs.load(Ordering::Relaxed) > 0 {
            format!(
                r#"<hierarchy><node package="com.zhiliaoapp.musically" resource-id="com.zhiliaoapp.musically:id/tv_top_text" text="{}" bounds="[350,100][720,216]" enabled="true" displayed="true"/></hierarchy>"#,
                self.title
            )
        } else {
            sound_xml(self.selected_marker)
        };
        Ok(crate::HierarchySourceSnapshot {
            generation: n as u64,
            xml,
        })
    }
    async fn locate_all_described(&self, _: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
        Ok(if self.backs.load(Ordering::Relaxed) > 0 {
            vec![ElementBox {
                x: 350.,
                y: 100.,
                width: 370.,
                height: 116.,
                description: Some(self.title.into()),
                enabled: true,
                clickable: false,
            }]
        } else {
            vec![]
        })
    }
    async fn locate_all(&self, _: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
        Ok(vec![ElementBox {
            x: 350.,
            y: 100.,
            width: 370.,
            height: 116.,
            description: Some("Add sound".into()),
            enabled: true,
            clickable: true,
        }])
    }
}

struct LoadingOcr {
    calls: AtomicUsize,
}
#[async_trait::async_trait]
impl GuiReasoner for LoadingOcr {
    async fn resolve(&self, _: GuiRequest) -> anyhow::Result<GuiResponse> {
        unreachable!()
    }
    async fn ocr(&self, r: OcrRequest) -> anyhow::Result<OcrResponse> {
        let mut response = Ocr {
            bad_binding: false,
            missing_tab: false,
            stop: None,
        }
        .ocr(r)
        .await?;
        if self.calls.fetch_add(1, Ordering::Relaxed) >= 2 {
            for (i, text) in ["One", "Artist One", "Two", "Artist Two"]
                .iter()
                .enumerate()
            {
                response.lines.push(OcrLine {
                    text: (*text).into(),
                    confidence: 0.96,
                    bounds: OcrRect {
                        x: 270,
                        y: 1380 + i as u32 * 70,
                        width: 300,
                        height: 30,
                    },
                });
            }
            response.text = response
                .lines
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
        }
        Ok(response)
    }
}

struct TransientMissingTabOcr {
    calls: AtomicUsize,
}

struct TransientSheetOcr {
    calls: AtomicUsize,
}
#[async_trait::async_trait]
impl GuiReasoner for TransientSheetOcr {
    async fn resolve(&self, _: GuiRequest) -> anyhow::Result<GuiResponse> {
        unreachable!()
    }
    async fn ocr(&self, r: OcrRequest) -> anyhow::Result<OcrResponse> {
        let tabs = [(47, 63), (178, 132), (381, 162), (613, 126)];
        let mut lines: Vec<_> = ["Hot", "For You", "Favorites", "Recent"]
            .into_iter()
            .zip(tabs)
            .map(|(text, (x, width))| OcrLine {
                text: text.into(),
                confidence: 0.96,
                bounds: OcrRect {
                    x,
                    y: 1129,
                    width,
                    height: 29,
                },
            })
            .collect();
        let rows: Vec<OcrLine> = serde_json::from_str(include_str!(
            "../../fixtures/tiktok-publish/musically-45.7.3-en/hot-visual-ocr.json"
        ))?;
        lines.extend(
            rows.into_iter()
                .filter(|line| r.region().contains(&line.bounds)),
        );
        if self.calls.fetch_add(1, Ordering::Relaxed) == 0 {
            lines.retain(|line| line.text != "Recent");
        }
        Ok(OcrResponse {
            protocol_version: r.protocol_version,
            request_id: r.request_id,
            observation_id: r.observation_id,
            session_epoch: r.session_epoch,
            generation: r.generation,
            screenshot_sha256: r.screenshot.sha256,
            status: "resolved".into(),
            text: lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            lines,
            engine: "fixture local OCR".into(),
            elapsed_ms: 1,
        })
    }
}
#[async_trait::async_trait]
impl GuiReasoner for TransientMissingTabOcr {
    async fn resolve(&self, _: GuiRequest) -> anyhow::Result<GuiResponse> {
        unreachable!()
    }
    async fn ocr(&self, r: OcrRequest) -> anyhow::Result<OcrResponse> {
        let mut response = Ocr {
            bad_binding: false,
            missing_tab: false,
            stop: None,
        }
        .ocr(r)
        .await?;
        if self.calls.fetch_add(1, Ordering::Relaxed) == 0 {
            response.lines.retain(|line| line.text != "Recent");
            response.text = response
                .lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
        }
        Ok(response)
    }
}

#[tokio::test(start_paused = true)]
async fn sound_entry_waits_for_rendered_rows_before_first_accessibility_query() {
    let mut s = Session::new();
    s.loading_entry = true;
    s.read_failure = false;
    s.ocr = Arc::new(LoadingOcr {
        calls: AtomicUsize::new(0),
    });
    selection_recovery::wait_for_rows(&s, plan()).await.unwrap();
    assert_eq!(s.reads.load(Ordering::Relaxed), 0);
    assert_eq!(s.taps.load(Ordering::Relaxed), 0);
    assert_eq!(s.frames.load(Ordering::Relaxed), 4);
}

#[tokio::test(start_paused = true)]
async fn sheet_proof_retries_one_transient_missing_tab_without_input() {
    let mut session = Session::new();
    session.ocr = Arc::new(TransientMissingTabOcr {
        calls: AtomicUsize::new(0),
    });

    selection_recovery::prove_sheet(&session, plan())
        .await
        .unwrap();

    assert_eq!(session.frames.load(Ordering::Relaxed), 3);
    assert_eq!(session.taps.load(Ordering::Relaxed), 0);
    assert_eq!(session.backs.load(Ordering::Relaxed), 0);
}

#[tokio::test(start_paused = true)]
async fn sheet_resume_does_not_treat_one_missing_tab_frame_as_editor() {
    let mut session = Session::new();
    session.read_failure = false;
    session.png =
        include_bytes!("../../fixtures/tiktok-publish/musically-45.7.3-en/hot-visual.png").to_vec();
    session.ocr = Arc::new(TransientSheetOcr {
        calls: AtomicUsize::new(0),
    });

    resume_open_sounds(&session, plan(), 5).await.unwrap();

    assert!(session.frames.load(Ordering::Relaxed) >= 3);
    assert_eq!(session.taps.load(Ordering::Relaxed), 0);
    assert_eq!(session.backs.load(Ordering::Relaxed), 0);
}

#[tokio::test(start_paused = true)]
async fn sheet_resume_fails_closed_when_only_editor_xml_is_visible() {
    let mut session = Session::new();
    session.read_failure = false;
    session.backs.store(1, Ordering::Relaxed);
    session.ocr = Arc::new(Ocr {
        bad_binding: false,
        missing_tab: true,
        stop: None,
    });

    assert!(resume_open_sounds(&session, plan(), 5).await.is_err());

    assert!(session.frames.load(Ordering::Relaxed) > 2);
    assert_eq!(session.taps.load(Ordering::Relaxed), 0);
    assert_eq!(session.backs.load(Ordering::Relaxed), 1);
    assert_eq!(session.reads.load(Ordering::Relaxed), 0);
}
fn plan() -> SoundPickerPlan {
    SoundPickerPlan::resolve("com.zhiliaoapp.musically", "en", "45.7.3").unwrap()
}

#[tokio::test(start_paused = true)]
async fn unreadable_tab_uses_two_bound_frames_then_requires_xml_readback() {
    let mut s = Session::new();
    s.unreadable_tab = true;
    snapshot::select_section_tab(&s, plan()).await.unwrap();
    assert_eq!(s.taps.load(Ordering::Relaxed), 1);
    assert_eq!(s.frames.load(Ordering::Relaxed), 2);
    assert!(s.reads.load(Ordering::Relaxed) >= 2);
    assert_eq!(s.backs.load(Ordering::Relaxed), 0);
}

#[tokio::test(start_paused = true)]
async fn empty_sound_root_uses_fresh_image_navigation_before_polling_again() {
    let mut s = Session::new();
    s.unreadable_tab = true;
    s.empty_tab_before_navigation = true;
    snapshot::select_section_tab(&s, plan()).await.unwrap();
    assert_eq!(s.taps.load(Ordering::Relaxed), 1);
    assert_eq!(s.frames.load(Ordering::Relaxed), 2);
    assert_eq!(s.reads.load(Ordering::Relaxed), 2);
}

#[tokio::test(start_paused = true)]
async fn tab_recovery_never_retries_or_accepts_missing_xml_confirmation() {
    let mut s = Session::new();
    s.unreadable_tab = true;
    s.missing_tab_readback = true;
    assert!(snapshot::select_section_tab(&s, plan()).await.is_err());
    assert_eq!(s.taps.load(Ordering::Relaxed), 1);
    assert_eq!(s.frames.load(Ordering::Relaxed), 2);
}

#[tokio::test(start_paused = true)]
async fn tab_recovery_refuses_unbound_or_missing_tabs_and_changed_app() {
    for mode in 0..3 {
        let mut s = Session::new();
        s.unreadable_tab = true;
        s.changed_app = mode == 2;
        s.ocr = Arc::new(Ocr {
            bad_binding: mode == 0,
            missing_tab: mode == 1,
            stop: None,
        });
        assert!(snapshot::select_section_tab(&s, plan()).await.is_err());
        assert_eq!(s.taps.load(Ordering::Relaxed), 0);
    }
}

#[tokio::test(start_paused = true)]
async fn tab_recovery_stop_during_ocr_prevents_navigation() {
    let stop = Arc::new(AtomicBool::new(false));
    let mut s = Session::new();
    s.unreadable_tab = true;
    s.ocr = Arc::new(Ocr {
        bad_binding: false,
        missing_tab: false,
        stop: Some(stop.clone()),
    });
    assert!(
        with_sound_budget(&stop, snapshot::select_section_tab(&s, plan()))
            .await
            .is_err()
    );
    assert_eq!(s.taps.load(Ordering::Relaxed), 0);
}

#[tokio::test(start_paused = true)]
async fn lost_sound_selection_read_recovers_editor_without_selecting_again() {
    let s = Session::new();
    let p = observe_sound_pool(&s, plan(), 5).await.unwrap();
    choose_and_confirm_sound(&s, plan(), &p, 0).await.unwrap();
    assert_eq!(s.taps.load(Ordering::Relaxed), 1);
    assert_eq!(s.backs.load(Ordering::Relaxed), 1);
    assert_eq!(s.frames.load(Ordering::Relaxed), 2);
}
#[tokio::test(start_paused = true)]
async fn recovery_refuses_stale_ocr_missing_tabs_and_changed_app() {
    for mode in 0..3 {
        let mut s = Session::new();
        s.changed_app = mode == 2;
        s.ocr = Arc::new(Ocr {
            bad_binding: mode == 0,
            missing_tab: mode == 1,
            stop: None,
        });
        let p = observe_sound_pool(&s, plan(), 5).await.unwrap();
        assert!(choose_and_confirm_sound(&s, plan(), &p, 0).await.is_err());
        assert_eq!(s.taps.load(Ordering::Relaxed), 1);
        assert_eq!(s.backs.load(Ordering::Relaxed), 0);
    }
}
#[tokio::test(start_paused = true)]
async fn recovery_never_accepts_the_wrong_editor_title() {
    let mut s = Session::new();
    s.title = "Wrong";
    let p = observe_sound_pool(&s, plan(), 5).await.unwrap();
    assert!(choose_and_confirm_sound(&s, plan(), &p, 0).await.is_err());
    assert_eq!(s.taps.load(Ordering::Relaxed), 1);
    assert_eq!(s.backs.load(Ordering::Relaxed), 1);
}
#[tokio::test(start_paused = true)]
async fn recovery_stop_during_ocr_prevents_back() {
    let stop = Arc::new(AtomicBool::new(false));
    let mut s = Session::new();
    s.ocr = Arc::new(Ocr {
        bad_binding: false,
        missing_tab: false,
        stop: Some(stop.clone()),
    });
    let p = observe_sound_pool(&s, plan(), 5).await.unwrap();
    let e = with_sound_budget(&stop, choose_and_confirm_sound(&s, plan(), &p, 0))
        .await
        .unwrap_err();
    assert!(e
        .chain()
        .any(|c| c.downcast_ref::<SoundStopped>().is_some()));
    assert_eq!(s.backs.load(Ordering::Relaxed), 0);
}
