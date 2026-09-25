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
    select_on_tap: bool,
    changed_app: bool,
    unreadable_tab: bool,
    missing_tab_readback: bool,
    empty_tab_before_navigation: bool,
    ocr: crate::ui_automation::SharedReasoner,
    loading_entry: bool,
    png: Vec<u8>,
    epoch_changed: Arc<AtomicBool>,
    xml_override: Option<String>,
    fixed_xml_generation: bool,
    incomplete_direct_reads: bool,
    screenshot_unavailable: bool,
    editor_before_sound: bool,
    hot_requires_tap: bool,
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
            select_on_tap: false,
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
            epoch_changed: Arc::new(AtomicBool::new(false)),
            xml_override: None,
            fixed_xml_generation: false,
            incomplete_direct_reads: false,
            screenshot_unavailable: false,
            editor_before_sound: false,
            hot_requires_tap: false,
        }
    }
}
fn sound_xml(selected: bool, hot_selected: bool) -> String {
    let node = |id: &str, text: &str, bounds: &str, selected: bool| {
        format!(
            r#"<node package="com.zhiliaoapp.musically" resource-id="com.zhiliaoapp.musically{id}" text="{text}" bounds="{bounds}" displayed="true" enabled="true" clickable="true" selected="{selected}"/>"#
        )
    };
    format!(
        "<hierarchy>{}{}{}{}{}{}{}</hierarchy>",
        node(":id/wrv", "Hot", "[40,1100][140,1150]", hot_selected),
        node(":id/wrv", "For You", "[175,1100][313,1150]", !hot_selected),
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
        if self.epoch_changed.load(Ordering::Relaxed) {
            "replacement-session".into()
        } else {
            "owned-session".into()
        }
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
        if self.screenshot_unavailable {
            return Err(crate::driver::ScreenshotReadUnavailable { bytes: 0 }.into());
        }
        Ok(self.png.clone())
    }
    async fn hierarchy_source_snapshot(&self) -> anyhow::Result<crate::HierarchySourceSnapshot> {
        let n = self.reads.fetch_add(1, Ordering::Relaxed) + 1;
        if self.incomplete_direct_reads
            && ((!self.editor_before_sound && n <= 2)
                || (self.editor_before_sound && self.taps.load(Ordering::Relaxed) > 0 && n <= 5))
        {
            return Ok(crate::HierarchySourceSnapshot {
                generation: n as u64,
                xml: "<hierarchy/>".into(),
            });
        }
        if self.editor_before_sound && self.taps.load(Ordering::Relaxed) == 0 {
            return Ok(crate::HierarchySourceSnapshot {
                generation: n as u64,
                xml: "<hierarchy><node package=\"com.zhiliaoapp.musically\" resource-id=\"com.zhiliaoapp.musically:id/dou\" bounds=\"[350,100][720,216]\" enabled=\"true\" clickable=\"true\" displayed=\"true\"/></hierarchy>".into(),
            });
        }
        if let Some(xml) = &self.xml_override {
            return Ok(crate::HierarchySourceSnapshot {
                generation: if self.fixed_xml_generation {
                    1
                } else {
                    n as u64
                },
                xml: xml.clone(),
            });
        }
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
                xml: sound_xml(false, true),
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
            sound_xml(
                self.selected_marker
                    || (self.select_on_tap && self.taps.load(Ordering::Relaxed) > 0),
                !self.hot_requires_tap || self.taps.load(Ordering::Relaxed) >= 2,
            )
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

#[tokio::test(start_paused = true)]
async fn secure_capture_failure_uses_measured_entry_and_xml_without_replaying_open() {
    let mut session = Session::new();
    session.screenshot_unavailable = true;
    session.read_failure = false;
    session.editor_before_sound = true;

    let pool = open_and_observe_sounds(&session, plan(), 5).await.unwrap();

    assert_eq!(pool.candidates.len(), 1);
    assert_eq!(pool.candidates[0].title, "One");
    assert_eq!(session.frames.load(Ordering::Relaxed), 1);
    assert_eq!(session.taps.load(Ordering::Relaxed), 1);
    assert_eq!(session.reads.load(Ordering::Relaxed), 7);
}

#[tokio::test(start_paused = true)]
async fn secure_capture_fallback_refuses_changed_app_before_sound_open() {
    let mut session = Session::new();
    session.screenshot_unavailable = true;
    session.editor_before_sound = true;
    session.changed_app = true;
    let error = open_and_observe_sounds(&session, plan(), 5)
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("sound app/session changed"));
    assert_eq!(session.taps.load(Ordering::Relaxed), 0);
}

#[tokio::test(start_paused = true)]
async fn secure_capture_waits_for_loading_hot_xml_without_image_or_second_open() {
    let mut session = Session::new();
    session.screenshot_unavailable = true;
    session.editor_before_sound = true;
    session.incomplete_direct_reads = true;
    session.read_failure = false;
    let pool = open_and_observe_sounds(&session, plan(), 5).await.unwrap();
    assert_eq!(pool.candidates[0].title, "One");
    assert_eq!(session.taps.load(Ordering::Relaxed), 1);
    assert_eq!(session.frames.load(Ordering::Relaxed), 1);
}

#[tokio::test(start_paused = true)]
async fn secure_capture_selects_for_you_to_hot_once_then_reproves_rows() {
    let mut session = Session::new();
    session.screenshot_unavailable = true;
    session.editor_before_sound = true;
    session.hot_requires_tap = true;
    session.read_failure = false;
    let pool = open_and_observe_sounds(&session, plan(), 5).await.unwrap();
    assert_eq!(pool.candidates[0].title, "One");
    assert_eq!(session.taps.load(Ordering::Relaxed), 2);
    assert_eq!(session.frames.load(Ordering::Relaxed), 1);
}

struct Ocr429;
#[async_trait::async_trait]
impl GuiReasoner for Ocr429 {
    async fn resolve(&self, _: GuiRequest) -> anyhow::Result<GuiResponse> {
        unreachable!()
    }

    async fn ocr(&self, _: OcrRequest) -> anyhow::Result<OcrResponse> {
        anyhow::bail!("gui_ocr_http_429")
    }
}

#[tokio::test(start_paused = true)]
async fn measured_hot_xml_precedes_unavailable_ocr_and_keeps_duplicate_rows_unselectable() {
    let mut session = Session::new();
    session.xml_override = Some(
        include_str!(
            "../../fixtures/tiktok-publish/musically-45.7.3-en/hot-machine10-redacted.txt"
        )
        .into(),
    );
    session.ocr = Arc::new(Ocr429);
    let pool = resume_open_sounds(&session, plan(), 5).await.unwrap();

    assert_eq!(pool.candidates.len(), 1);
    assert_eq!(pool.candidates[0].title, "Song One");
    assert!(!pool.visual);
    assert_eq!(session.frames.load(Ordering::Relaxed), 0);
    assert_eq!(session.reads.load(Ordering::Relaxed), 2);
    assert_eq!(session.taps.load(Ordering::Relaxed), 0);
}

#[tokio::test(start_paused = true)]
async fn direct_sound_xml_refuses_stale_or_unselected_hot_before_ocr_fallback() {
    let measured = include_str!(
        "../../fixtures/tiktok-publish/musically-45.7.3-en/hot-machine10-redacted.txt"
    );
    for (xml, fixed_generation) in [
        (measured.to_string(), true),
        (
            measured.replace(
                "text=\"Hot\" selected=\"true\"",
                "text=\"Hot\" selected=\"false\"",
            ),
            false,
        ),
    ] {
        let mut session = Session::new();
        session.xml_override = Some(xml);
        session.fixed_xml_generation = fixed_generation;
        session.ocr = Arc::new(Ocr429);
        let error = resume_open_sounds(&session, plan(), 5).await.unwrap_err();
        assert!(format!("{error:#}").contains("gui_ocr_http_429"));
        assert_eq!(session.taps.load(Ordering::Relaxed), 0);
        assert_eq!(session.backs.load(Ordering::Relaxed), 0);
    }
}

#[tokio::test(start_paused = true)]
async fn direct_sound_xml_rejects_changed_foreground_and_other_builds() {
    let mut session = Session::new();
    session.xml_override = Some(
        include_str!(
            "../../fixtures/tiktok-publish/musically-45.7.3-en/hot-machine10-redacted.txt"
        )
        .into(),
    );
    session.changed_app = true;
    let error = resume_open_sounds(&session, plan(), 5).await.unwrap_err();
    assert!(format!("{error:#}").contains("sound app changed"));
    assert_eq!(session.taps.load(Ordering::Relaxed), 0);
    let trill = SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2").unwrap();
    assert!(!selection_recovery::measured(trill));
    let global_other =
        SoundPickerPlan::resolve("com.zhiliaoapp.musically", "en", "46.2.1").unwrap();
    assert!(!selection_recovery::measured(global_other));
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
struct TabsWithoutRowsOcr;
struct SlowOcr;
#[async_trait::async_trait]
impl GuiReasoner for SlowOcr {
    async fn resolve(&self, _: GuiRequest) -> anyhow::Result<GuiResponse> {
        unreachable!()
    }

    async fn ocr(&self, _: OcrRequest) -> anyhow::Result<OcrResponse> {
        tokio::time::sleep(Duration::from_secs(31)).await;
        unreachable!()
    }
}
struct SwitchEpochOnSheetProofOcr {
    changed: Arc<AtomicBool>,
    proofs: AtomicUsize,
}
#[async_trait::async_trait]
impl GuiReasoner for SwitchEpochOnSheetProofOcr {
    async fn resolve(&self, _: GuiRequest) -> anyhow::Result<GuiResponse> {
        unreachable!()
    }
    async fn ocr(&self, request: OcrRequest) -> anyhow::Result<OcrResponse> {
        let sheet_proof = request.roi.is_some_and(|region| region.height == 555);
        let response = TabsWithoutRowsOcr.ocr(request).await?;
        if sheet_proof && self.proofs.fetch_add(1, Ordering::Relaxed) == 1 {
            self.changed.store(true, Ordering::Relaxed);
        }
        Ok(response)
    }
}
#[async_trait::async_trait]
impl GuiReasoner for TabsWithoutRowsOcr {
    async fn resolve(&self, _: GuiRequest) -> anyhow::Result<GuiResponse> {
        unreachable!()
    }
    async fn ocr(&self, r: OcrRequest) -> anyhow::Result<OcrResponse> {
        let lines: Vec<_> = ["Hot", "For You", "Favorites", "Recent"]
            .into_iter()
            .zip([(47, 63), (178, 132), (381, 162), (613, 126)])
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

    assert_eq!(session.frames.load(Ordering::Relaxed), 0);
    assert!(session.reads.load(Ordering::Relaxed) >= 2);
    assert_eq!(session.taps.load(Ordering::Relaxed), 0);
    assert_eq!(session.backs.load(Ordering::Relaxed), 0);
}

#[tokio::test(start_paused = true)]
async fn loaded_sheet_uses_proven_hierarchy_rows_when_ocr_cannot_read_them() {
    let mut session = Session::new();
    session.read_failure = false;
    session.png =
        include_bytes!("../../fixtures/tiktok-publish/musically-45.7.3-en/hot-visual.png").to_vec();
    session.ocr = Arc::new(TabsWithoutRowsOcr);
    let stop = AtomicBool::new(false);

    let pool = tokio::time::timeout(
        Duration::from_secs(70),
        with_sound_budget(&stop, resume_open_sounds(&session, plan(), 5)),
    )
    .await
    .expect("a rendered sheet must not wait through the entire sound budget")
    .unwrap();

    assert_eq!(pool.candidates.len(), 1);
    assert_eq!(pool.candidates[0].title, "One");
    assert!(!pool.visual);
    assert_eq!(session.taps.load(Ordering::Relaxed), 0);
    assert_eq!(session.backs.load(Ordering::Relaxed), 0);
    assert!(session.reads.load(Ordering::Relaxed) >= 2);
}

#[tokio::test(start_paused = true)]
async fn replaced_session_during_sound_proof_retries_from_fresh_sheet_without_tapping() {
    let mut session = Session::new();
    session.read_failure = false;
    session.incomplete_direct_reads = true;
    session.png =
        include_bytes!("../../fixtures/tiktok-publish/musically-45.7.3-en/hot-visual.png").to_vec();
    session.ocr = Arc::new(SwitchEpochOnSheetProofOcr {
        changed: session.epoch_changed.clone(),
        proofs: AtomicUsize::new(0),
    });

    let first = resume_open_sounds(&session, plan(), 5).await.unwrap_err();
    let failure = crate::publish_recovery::describe(&first);
    assert_eq!(failure.code, "sound_session_replaced");
    assert_eq!(
        failure.kind,
        crate::publish_recovery::FailureKind::Retryable
    );
    assert_eq!(session.taps.load(Ordering::Relaxed), 0);

    let pool = resume_open_sounds(&session, plan(), 5).await.unwrap();
    assert_eq!(pool.candidates[0].title, "One");
    assert_eq!(session.taps.load(Ordering::Relaxed), 0);
    assert_eq!(session.backs.load(Ordering::Relaxed), 0);
}

#[tokio::test(start_paused = true)]
async fn other_foreground_app_during_sound_observation_stays_terminal() {
    let mut session = Session::new();
    session.changed_app = true;
    let error = resume_open_sounds(&session, plan(), 5).await.unwrap_err();
    let failure = crate::publish_recovery::describe(&error);
    assert_eq!(failure.kind, crate::publish_recovery::FailureKind::Terminal);
    assert_eq!(session.taps.load(Ordering::Relaxed), 0);
    assert_eq!(session.backs.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn stop_during_sound_observation_is_not_masked_by_foreground_check() {
    let mut session = Session::new();
    session.changed_app = true;
    let error = checked_measured_sound_observation::<()>(
        &session,
        plan(),
        "owned-session",
        Err(SoundStopped.into()),
    )
    .await
    .unwrap_err();
    assert!(error.is::<SoundStopped>());
    assert_eq!(session.taps.load(Ordering::Relaxed), 0);
}

#[tokio::test(start_paused = true)]
async fn hierarchy_fallback_selects_once_and_confirms_exact_editor_sound() {
    let mut session = Session::new();
    session.read_failure = false;
    session.select_on_tap = true;
    session.png =
        include_bytes!("../../fixtures/tiktok-publish/musically-45.7.3-en/hot-visual.png").to_vec();
    session.ocr = Arc::new(TabsWithoutRowsOcr);
    let stop = AtomicBool::new(false);

    tokio::time::timeout(
        Duration::from_secs(120),
        with_sound_budget(&stop, async {
            let pool = resume_open_sounds(&session, plan(), 5).await?;
            anyhow::ensure!(!pool.visual, "expected measured hierarchy fallback");
            choose_and_confirm_sound(&session, plan(), &pool, 0).await
        }),
    )
    .await
    .expect("selection and readback must fit the sound budget")
    .unwrap();

    assert_eq!(session.taps.load(Ordering::Relaxed), 1);
    assert_eq!(session.backs.load(Ordering::Relaxed), 1);
}

#[tokio::test(start_paused = true)]
async fn unreadable_hierarchy_after_ocr_timeout_returns_retryable_without_a_tap() {
    let mut session = Session::new();
    session.read_failure = false;
    session.unreadable_tab = true;
    session.png =
        include_bytes!("../../fixtures/tiktok-publish/musically-45.7.3-en/hot-visual.png").to_vec();
    session.ocr = Arc::new(TabsWithoutRowsOcr);
    let stop = AtomicBool::new(false);

    let error = tokio::time::timeout(
        Duration::from_secs(120),
        with_sound_budget(&stop, resume_open_sounds(&session, plan(), 5)),
    )
    .await
    .expect("unreadable hierarchy must not hang until the full 180-second budget")
    .unwrap_err();

    assert_eq!(
        crate::publish_recovery::describe(&error).code,
        "sound_hierarchy_unavailable"
    );
    assert_eq!(session.taps.load(Ordering::Relaxed), 0);
    assert_eq!(session.backs.load(Ordering::Relaxed), 0);
}

#[tokio::test(start_paused = true)]
async fn sound_recovery_deadline_is_retryable_before_any_tap() {
    let mut session = Session::new();
    session.read_failure = false;
    session.ocr = Arc::new(SlowOcr);
    let error = selection_recovery::prove_sheet(&session, plan())
        .await
        .unwrap_err();

    let failure = crate::publish_recovery::describe(&error);
    assert_eq!(failure.code, "sound_load_timeout");
    assert_eq!(
        failure.kind,
        crate::publish_recovery::FailureKind::Retryable
    );
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

    let error = resume_open_sounds(&session, plan(), 5).await.unwrap_err();
    assert_eq!(
        crate::publish_recovery::describe(&error).code,
        "sound_tab_unavailable"
    );

    assert!(session.frames.load(Ordering::Relaxed) > 2);
    assert_eq!(session.taps.load(Ordering::Relaxed), 0);
    assert_eq!(session.backs.load(Ordering::Relaxed), 1);
    assert_eq!(session.reads.load(Ordering::Relaxed), 1);
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
