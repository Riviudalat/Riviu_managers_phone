//! Measured native sound-sheet pixels, never a blind fallback.
use super::*;
use crate::ui_automation::{OcrImage, OcrLine, OcrRect, OcrRequest};
use base64::Engine;
use sha2::{Digest, Sha256};

async fn tap_native_image(session: &dyn UiSession, point: crate::TapPoint) -> anyhow::Result<()> {
    check_wait()?;
    session.tap_image(point.x, point.y, 1080., 2220.).await?;
    check_wait()
}

fn loading_chip(img: &image::RgbImage) -> bool {
    if img.dimensions() != (1080, 2220) {
        return false;
    }
    let Ok(reference) = image::load_from_memory(include_bytes!(
        "../../fixtures/tiktok-publish/musically-45.7.3-en/loading-chip.png"
    )) else {
        return false;
    };
    let reference = reference.to_rgb8();
    let matched = reference
        .enumerate_pixels()
        .filter(|(x, y, p)| {
            let actual = img.get_pixel(466 + x, 132 + y);
            p.0.iter().zip(actual.0).all(|(a, b)| a.abs_diff(b) <= 25)
        })
        .count();
    matched as f64 / f64::from(reference.width() * reference.height()) >= 0.94
}

pub(super) async fn open_loading_entry(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
) -> anyhow::Result<bool> {
    if !selection_recovery::measured(plan) || session.gui_reasoner().is_none() {
        return Ok(false);
    }
    let epoch = session.gui_session_epoch();
    anyhow::ensure!(!epoch.is_empty(), "loading sound session missing");
    for _ in 0..2 {
        check_wait()?;
        anyhow::ensure!(
            session.gui_session_epoch() == epoch
                && read_sound(session.active_app_bundle()).await? == plan.package,
            "loading sound app/session changed"
        );
        let png = read_sound(session.screenshot_png()).await?;
        let img = image::load_from_memory(&png)?.to_rgb8();
        if !loading_chip(&img) {
            return Ok(false);
        }
    }
    // ce051715ab9fee2002, 20/09/2026: chip [368,100][711,216]. The
    // exact Loading glyph template above must match both new native frames.
    anyhow::ensure!(
        session.gui_session_epoch() == epoch,
        "loading sound stale frame"
    );
    tap_native_image(session, crate::TapPoint { x: 540., y: 158. }).await?;
    Ok(true)
}

fn red_fraction(img: &image::RgbImage, r: OcrRect) -> f64 {
    let (mut red, mut dark) = (0u32, 0u32);
    for y in r.y..(r.y + r.height).min(img.height()) {
        for x in r.x..(r.x + r.width).min(img.width()) {
            let [a, b, c] = img.get_pixel(x, y).0;
            if a > 170 && f64::from(a) > f64::from(b) * 1.5 && f64::from(a) > f64::from(c) * 1.2 {
                red += 1;
            }
            if a.max(b).max(c) < 180 {
                dark += 1;
            }
        }
    }
    f64::from(red) / f64::from((red + dark).max(1))
}

fn hot_selected(img: &image::RgbImage, tabs: &[OcrRect]) -> bool {
    if tabs.len() != 4 {
        return false;
    }
    let underlined = |r: &OcrRect| {
        (r.y + r.height + 15..r.y + r.height + 50)
            .filter(|y| *y < img.height())
            .any(|y| {
                (r.x..r.x + r.width)
                    .filter(|x| *x < img.width())
                    .filter(|x| img.get_pixel(*x, y).0.iter().all(|c| *c < 80))
                    .count() as f64
                    / f64::from(r.width.max(1))
                    > 0.8
            })
    };
    underlined(&tabs[0]) && tabs[1..].iter().all(|r| !underlined(r))
}

fn pool_from_image(
    image: &image::RgbImage,
    lines: &[OcrLine],
    tabs: &[OcrRect],
    plan: SoundPickerPlan,
    maximum: usize,
) -> anyhow::Result<ObservedSoundPool> {
    anyhow::ensure!(
        selection_recovery::measured(plan) && image.dimensions() == (1080, 2220),
        "visual sound tuple unmeasured"
    );
    anyhow::ensure!(
        hot_selected(image, tabs),
        "visual sound Hot tab not selected"
    );
    let mut text: Vec<_> = lines
        .iter()
        .filter(|l| {
            l.confidence >= 0.9
                && l.bounds.x >= 250
                && l.bounds.y >= 1320
                && l.bounds.x + l.bounds.width < 1040
                && l.bounds.y + l.bounds.height <= 1900
                && l.bounds.width >= 120
        })
        .collect();
    text.sort_by_key(|l| l.bounds.y);
    let titles: Vec<_> = text
        .iter()
        .copied()
        .filter(|l| l.bounds.height >= 39 && l.bounds.y + l.bounds.height + 80 <= 1900)
        .collect();
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for title in titles.into_iter().take(maximum) {
        // A red title is already selected. Never toggle it or guess text hidden
        // by the inline equalizer/trim buttons. Other complete rows remain usable.
        if red_fraction(image, title.bounds) > 0.5 {
            continue;
        }
        anyhow::ensure!(
            !title.text.contains('…')
                && !title.text.ends_with("...")
                && title.text.chars().count() >= 3,
            "visual sound title clipped"
        );
        let bottom = title.bounds.y + title.bounds.height;
        let artists: Vec<_> = text
            .iter()
            .copied()
            .filter(|l| {
                l.bounds.y >= bottom + 4
                    && l.bounds.y <= bottom + 30
                    && (20..39).contains(&l.bounds.height)
                    && l.bounds.x.abs_diff(title.bounds.x) <= 16
            })
            .collect();
        let [artist] = artists.as_slice() else {
            continue;
        };
        let artist = artist
            .text
            .split(" · ")
            .next()
            .unwrap_or(&artist.text)
            .split(" - ")
            .next()
            .unwrap_or(&artist.text)
            .trim();
        anyhow::ensure!(
            !artist.is_empty()
                && !candidates
                    .iter()
                    .any(|c: &SoundCandidate| c.title == title.text),
            "visual sound identity ambiguous"
        );
        candidates.push(SoundCandidate {
            section: plan.canonical_section.into(),
            title: title.text.trim().into(),
            artist: artist.into(),
        });
        targets.push(ElementBox {
            x: f64::from(title.bounds.x),
            y: f64::from(title.bounds.y),
            width: f64::from(title.bounds.width),
            height: f64::from(title.bounds.height),
            description: Some(title.text.clone()),
            enabled: true,
            clickable: true,
        });
    }
    anyhow::ensure!(
        !candidates.is_empty(),
        "visual sound has no complete unselected row"
    );
    Ok(ObservedSoundPool {
        effective_plan: None,
        candidates,
        maximum_visible: maximum,
        targets,
        selected_index: None,
        visual: true,
    })
}

async fn capture(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    generation: u64,
) -> anyhow::Result<(image::RgbImage, Vec<OcrLine>, Vec<OcrRect>, String)> {
    check_wait()?;
    let epoch = session.gui_session_epoch();
    anyhow::ensure!(!epoch.is_empty(), "visual sound session missing");
    anyhow::ensure!(
        read_sound(session.active_app_bundle()).await? == plan.package,
        "visual sound app changed"
    );
    let png = read_sound(session.screenshot_png()).await?;
    let image = image::load_from_memory(&png)?.to_rgb8();
    anyhow::ensure!(
        image.dimensions() == (1080, 2220),
        "visual sound geometry unmeasured"
    );
    let request = OcrRequest {
        protocol_version: 1,
        request_id: uuid::Uuid::new_v4().to_string(),
        observation_id: uuid::Uuid::new_v4().to_string(),
        session_epoch: epoch.clone(),
        generation,
        remaining_ms: 15_000,
        screenshot: OcrImage {
            bytes_base64: base64::engine::general_purpose::STANDARD.encode(&png),
            sha256: format!("{:x}", Sha256::digest(&png)),
            width: 1080,
            height: 2220,
        },
        roi: Some(OcrRect {
            x: 0,
            y: 888,
            width: 1080,
            height: 1012,
        }),
        languages: vec!["vi".into(), "en".into()],
        min_confidence: 0.9,
    };
    let reasoner = session
        .gui_reasoner()
        .context("visual sound local OCR missing")?;
    let response = read_sound(reasoner.ocr(request.clone())).await?;
    response.validate_binding(&request)?;
    anyhow::ensure!(
        epoch == session.gui_session_epoch()
            && read_sound(session.active_app_bundle()).await? == plan.package,
        "visual sound session changed"
    );
    check_wait()?;
    // The first frame after opening the sheet can contain only the video.
    // This is a pending observation, never permission to tap or an adapter error.
    let tabs = selection_recovery::tabs(&response).unwrap_or_default();
    Ok((image, response.lines, tabs, epoch))
}

pub(super) async fn observe(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
    navigate: bool,
) -> anyhow::Result<ObservedSoundPool> {
    let deadline = phase_deadline(Duration::from_secs(60));
    let mut prior: Option<(String, ObservedSoundPool)> = None;
    let mut navigated = !navigate;
    let mut generation = 0;
    loop {
        check_wait()?;
        anyhow::ensure!(
            Instant::now() < deadline,
            "visual sound pool did not stabilize"
        );
        generation += 1;
        let (img, lines, tabs, epoch) = capture(session, plan, generation).await?;
        if tabs.len() != 4 {
            prior = None;
            tokio::time::sleep(POLL).await;
            continue;
        }
        if !hot_selected(&img, &tabs) && !navigated {
            let point = selection_recovery::prove_sheet(session, plan).await?;
            tap_native_image(session, point).await?;
            navigated = true;
            prior = None;
            continue;
        }
        match pool_from_image(&img, &lines, &tabs, plan, maximum) {
            Ok(pool) => {
                if prior
                    .as_ref()
                    .is_some_and(|(old, p)| old == &epoch && p.stable_with(&pool))
                {
                    return Ok(pool);
                }
                prior = Some((epoch, pool));
            }
            Err(error) => {
                tracing::debug!(%error, "visual sound rows are not yet complete");
                prior = None;
            }
        }
        tokio::time::sleep(POLL).await;
    }
}

pub(super) async fn choose(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    pool: &ObservedSoundPool,
    index: usize,
) -> anyhow::Result<()> {
    let candidate = pool
        .candidates
        .get(index)
        .context("visual sound index out of range")?;
    let fresh = observe(session, plan, pool.maximum_visible, false).await?;
    let target = reproof_target(pool, &fresh, index)?;
    tap_native_image(session, target.centre()).await?;
    // One reversible selection only. Fresh sheet proof authorizes Back; exact
    // title on two XML editor snapshots is the final independent verification.
    selection_recovery::prove_sheet(session, plan).await?;
    check_wait()?;
    session.back().await?;
    selection_recovery::confirm_editor(session, plan, &candidate.title).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_automation::{GuiReasoner, GuiRequest, GuiResponse, OcrResponse};
    use std::sync::atomic::AtomicUsize;
    fn fixture() -> (image::RgbImage, Vec<OcrLine>, Vec<OcrRect>, SoundPickerPlan) {
        let img = image::load_from_memory(include_bytes!(
            "../../fixtures/tiktok-publish/musically-45.7.3-en/hot-visual.png"
        ))
        .unwrap()
        .to_rgb8();
        let lines = serde_json::from_str(include_str!(
            "../../fixtures/tiktok-publish/musically-45.7.3-en/hot-visual-ocr.json"
        ))
        .unwrap();
        let tabs = [(47, 63), (178, 132), (381, 162), (613, 126)]
            .into_iter()
            .map(|(x, width)| OcrRect {
                x,
                y: 1129,
                width,
                height: 29,
            })
            .collect();
        (
            img,
            lines,
            tabs,
            SoundPickerPlan::resolve("com.zhiliaoapp.musically", "en", "45.7.3").unwrap(),
        )
    }
    #[test]
    fn measured_pixels_bind_hot_titles_and_artists_without_xml() {
        let (img, lines, tabs, plan) = fixture();
        let pool = pool_from_image(&img, &lines, &tabs, plan, 5).unwrap();
        assert_eq!(pool.candidates.len(), 3);
        assert_eq!(pool.candidates[0].title, "Thương Nhau Đến Thế");
        assert_eq!(pool.selected_index, None);
        assert!(pool.targets.iter().all(|t| t.x >= 250. && t.y >= 1300.));
    }
    #[test]
    fn loading_chip_template_refuses_other_pixels_and_geometry() {
        let mut img = image::RgbImage::new(1080, 2220);
        assert!(!loading_chip(&img));
        let r = image::load_from_memory(include_bytes!(
            "../../fixtures/tiktok-publish/musically-45.7.3-en/loading-chip.png"
        ))
        .unwrap()
        .to_rgb8();
        image::imageops::replace(&mut img, &r, 466, 132);
        assert!(loading_chip(&img));
        assert!(!loading_chip(&image::RgbImage::new(720, 1480)));
    }
    #[test]
    fn wrong_tab_incomplete_or_ambiguous_rows_never_authorize_selection() {
        let (mut img, lines, tabs, plan) = fixture();
        for y in 1170..1210 {
            for x in 0..160 {
                img.put_pixel(x, y, image::Rgb([255, 255, 255]));
            }
        }
        assert!(pool_from_image(&img, &lines, &tabs, plan, 5).is_err());
        let (img, mut lines, tabs, plan) = fixture();
        lines.remove(1);
        let pool = pool_from_image(&img, &lines, &tabs, plan, 5).unwrap();
        assert_eq!(pool.candidates.len(), 2);
        assert!(pool
            .candidates
            .iter()
            .all(|c| c.title != "Thương Nhau Đến Thế"));
        lines.retain(|l| l.bounds.height >= 39);
        assert!(pool_from_image(&img, &lines, &tabs, plan, 5).is_err());
    }

    #[test]
    fn selected_red_row_is_skipped_without_toggling_and_bottom_clipped_row_is_excluded() {
        let (_, mut lines, tabs, plan) = fixture();
        let img = image::load_from_memory(include_bytes!(
            "../../fixtures/tiktok-publish/musically-45.7.3-en/hot-visual-selected.png"
        ))
        .unwrap()
        .to_rgb8();
        lines.push(OcrLine {
            text: "Cut off".into(),
            confidence: 0.96,
            bounds: OcrRect {
                x: 268,
                y: 1858,
                width: 400,
                height: 42,
            },
        });
        let pool = pool_from_image(&img, &lines, &tabs, plan, 5).unwrap();
        assert_eq!(pool.candidates.len(), 2);
        assert_eq!(pool.candidates[0].title, "Ăm Chả Húi (Short Mix)");
    }

    struct Ocr;
    #[async_trait::async_trait]
    impl GuiReasoner for Ocr {
        async fn resolve(&self, _: GuiRequest) -> anyhow::Result<GuiResponse> {
            unreachable!()
        }
        async fn ocr(&self, r: OcrRequest) -> anyhow::Result<OcrResponse> {
            let (_, rows, tabs, _) = fixture();
            let mut lines: Vec<_> = ["Hot", "For You", "Favorites", "Recent"]
                .into_iter()
                .zip(tabs)
                .map(|(text, bounds)| OcrLine {
                    text: text.into(),
                    bounds,
                    confidence: 0.96,
                })
                .collect();
            lines.extend(rows.into_iter().filter(|l| r.region().contains(&l.bounds)));
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
    struct Session {
        taps: AtomicUsize,
        backs: AtomicUsize,
        reads: AtomicUsize,
        wrong_title: bool,
    }
    #[async_trait::async_trait]
    impl UiSession for Session {
        fn stream_url(&self) -> Option<String> {
            None
        }
        fn gui_session_epoch(&self) -> String {
            "visual-session".into()
        }
        fn gui_reasoner(&self) -> Option<crate::ui_automation::SharedReasoner> {
            Some(Arc::new(Ocr))
        }
        async fn active_app_bundle(&self) -> anyhow::Result<String> {
            Ok("com.zhiliaoapp.musically".into())
        }
        async fn screenshot_png(&self) -> anyhow::Result<Vec<u8>> {
            Ok(
                include_bytes!("../../fixtures/tiktok-publish/musically-45.7.3-en/hot-visual.png")
                    .to_vec(),
            )
        }
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
        async fn hierarchy_source_snapshot(
            &self,
        ) -> anyhow::Result<crate::HierarchySourceSnapshot> {
            assert_eq!(
                self.backs.load(Ordering::Relaxed),
                1,
                "no XML query on unreadable sheet"
            );
            let n = self.reads.fetch_add(1, Ordering::Relaxed) + 1;
            let title = if self.wrong_title {
                "Wrong"
            } else {
                "Thương Nhau Đến Thế"
            };
            Ok(crate::HierarchySourceSnapshot {
                generation: n as u64,
                xml: format!(
                    r#"<hierarchy><node package="com.zhiliaoapp.musically" resource-id="com.zhiliaoapp.musically:id/tv_top_text" text="{title}" bounds="[300,100][780,216]" displayed="true" enabled="true"/></hierarchy>"#
                ),
            })
        }
    }
    #[tokio::test(start_paused = true)]
    async fn visual_selection_requires_exact_editor_title_without_replaying_the_pick() {
        for wrong_title in [false, true] {
            let s = Session {
                taps: AtomicUsize::new(0),
                backs: AtomicUsize::new(0),
                reads: AtomicUsize::new(0),
                wrong_title,
            };
            let plan = fixture().3;
            let pool = observe(&s, plan, 5, false).await.unwrap();
            let result = choose(&s, plan, &pool, 0).await;
            assert_eq!(result.is_ok(), !wrong_title);
            assert_eq!(s.taps.load(Ordering::Relaxed), 1);
            assert_eq!(s.backs.load(Ordering::Relaxed), 1);
            assert!(s.reads.load(Ordering::Relaxed) >= 2);
        }
    }
}
