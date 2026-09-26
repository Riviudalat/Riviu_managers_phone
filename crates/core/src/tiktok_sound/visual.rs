//! Measured native sound-sheet pixels, never a blind fallback.
use super::*;
use crate::ui_automation::{OcrImage, OcrLine, OcrRect, OcrRequest};
use base64::Engine;
use sha2::{Digest, Sha256};

async fn tap_native_image(session: &dyn UiSession, point: crate::TapPoint) -> anyhow::Result<()> {
    tap_native_image_armed(session, point, &mut || {}).await
}
async fn tap_native_image_armed(
    session: &dyn UiSession,
    point: crate::TapPoint,
    before_tap: &mut (dyn FnMut() + Send),
) -> anyhow::Result<()> {
    check_wait()?;
    before_tap();
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
    before_open: &mut (dyn FnMut() + Send),
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
    tap_native_image_armed(session, crate::TapPoint { x: 540., y: 158. }, before_open).await?;
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
                && l.bounds.width >= 80
        })
        .collect();
    text.sort_by_key(|l| l.bounds.y);
    let titles: Vec<_> = text
        .iter()
        .copied()
        // Tesseract 5.5.2 reports the same 45.7.3 title font as 28-45 px
        // depending on glyphs and locale. Row geometry + a paired artist line
        // distinguishes titles; a 39 px floor discarded every current US row.
        .filter(|l| l.bounds.height >= 28 && l.bounds.y + l.bounds.height + 80 <= 1900)
        .collect();
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    let mut selected_index = None;
    for title in titles.into_iter().take(maximum) {
        let selected = red_fraction(image, title.bounds) > 0.5;
        if !complete_sound_title(&title.text) || title.text.chars().count() < 3 {
            continue;
        }
        let bottom = title.bounds.y + title.bounds.height;
        let artists: Vec<_> = text
            .iter()
            .copied()
            .filter(|l| {
                l.bounds.y >= bottom + 4
                        && l.bounds.y <= bottom + 30
                        && (20..39).contains(&l.bounds.height)
                        // The inline equalizer shifts the measured selected title
                        // 51 px right. Its artist stays on the same row; unselected
                        // rows retain the tighter pairing used for tap targets.
                        && l.bounds.x.abs_diff(title.bounds.x)
                            <= if selected { 64 } else { 16 }
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
        if selected {
            anyhow::ensure!(
                selected_index.replace(candidates.len() - 1).is_none(),
                "visual sound selected row ambiguous"
            );
        }
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
        selected_index,
        visual: true,
    })
}

fn visual_rejection_code(error: &anyhow::Error) -> &'static str {
    match error.to_string().as_str() {
        "visual sound tuple unmeasured" => "tuple_unmeasured",
        "visual sound Hot tab not selected" => "hot_not_selected",
        "visual sound title clipped" => "title_clipped",
        "visual sound identity ambiguous" => "identity_ambiguous",
        "visual sound selected row ambiguous" => "selected_row_ambiguous",
        "visual sound has no complete unselected row" => "no_complete_row",
        _ => "other",
    }
}

async fn capture(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    generation: u64,
) -> anyhow::Result<(image::RgbImage, Vec<OcrLine>, Vec<OcrRect>, String)> {
    let result = capture_sheet(session, plan, generation).await?;
    if network_unavailable(&result.1, &result.2) {
        return Err(SoundNetworkUnavailable.into());
    }
    Ok(result)
}

async fn capture_sheet(
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

pub(super) async fn retry_network(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
) -> anyhow::Result<bool> {
    let first = capture_sheet(session, plan, 1).await?;
    if !network_unavailable(&first.1, &first.2) {
        return Ok(false);
    }
    let second = capture_sheet(session, plan, 2).await?;
    if first.3 != second.3 || !network_unavailable(&second.1, &second.2) {
        return Ok(false);
    }
    let rows: Vec<_> = second
        .1
        .iter()
        .filter(|line| {
            line.confidence >= 0.9
                && (1300..1900).contains(&line.bounds.y)
                && line
                    .text
                    .contains("Your network is unstable. Please try again.")
        })
        .collect();
    let [line] = rows.as_slice() else {
        anyhow::bail!("music retry target ambiguous")
    };
    // 45.7.3/en, ce031713b0c610ab0c: the measured refresh row includes the
    // circular retry icon and error text. Two fresh sheet observations gate this tap.
    tap_native_image(
        session,
        crate::TapPoint {
            x: f64::from(line.bounds.x + line.bounds.width / 2),
            y: f64::from(line.bounds.y + line.bounds.height / 2),
        },
    )
    .await?;
    let deadline = phase_deadline(Duration::from_secs(15));
    loop {
        check_wait()?;
        let (_, lines, tabs, _) = capture_sheet(session, plan, 3).await?;
        if !network_unavailable(&lines, &tabs) {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        tokio::time::sleep(POLL).await;
    }
}

fn network_unavailable(lines: &[OcrLine], tabs: &[OcrRect]) -> bool {
    tabs.len() == 4
        && lines.iter().any(|line| {
            line.confidence >= 0.9
                && (1300..1900).contains(&line.bounds.y)
                && line
                    .text
                    .contains("Your network is unstable. Please try again.")
        })
}

#[derive(Debug, thiserror::Error)]
#[error("visual sound pool did not stabilize")]
pub(super) struct VisualSoundPoolUnavailable;

/// Reserve time for a hierarchy read when OCR sees the sheet but cannot bind
/// its rows. Later selection/readback still uses the full shared sound budget.
pub(super) async fn observe_initial(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
) -> anyhow::Result<ObservedSoundPool> {
    let saw_hot = AtomicBool::new(false);
    let result = tokio::time::timeout(
        Duration::from_secs(60),
        observe_inner(session, plan, maximum, true, &saw_hot),
    )
    .await;
    check_wait()?;
    match result {
        Ok(result) => result,
        Err(_) => visual_pool_timeout(&saw_hot),
    }
}

fn visual_pool_timeout(saw_hot: &AtomicBool) -> anyhow::Result<ObservedSoundPool> {
    if saw_hot.load(Ordering::Relaxed) {
        Err(VisualSoundPoolUnavailable.into())
    } else {
        Err(crate::publish_recovery::retryable_error(
            "sound_tab_unavailable",
            "TikTok chưa xác nhận tab Hot của bảng nhạc; chưa chọn nhạc hoặc bấm Đăng",
        ))
    }
}

pub(super) async fn observe(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
    navigate: bool,
) -> anyhow::Result<ObservedSoundPool> {
    observe_inner(session, plan, maximum, navigate, &AtomicBool::new(false)).await
}

async fn observe_inner(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
    navigate: bool,
    saw_hot: &AtomicBool,
) -> anyhow::Result<ObservedSoundPool> {
    let deadline = phase_deadline(Duration::from_secs(60));
    let mut prior: Option<(String, ObservedSoundPool)> = None;
    let mut navigated = !navigate;
    let mut generation = 0;
    let mut last_rejection = None;
    loop {
        check_wait()?;
        if Instant::now() >= deadline {
            return visual_pool_timeout(saw_hot);
        }
        generation += 1;
        let (img, lines, tabs, epoch) = capture(session, plan, generation).await?;
        if tabs.len() != 4 {
            if last_rejection != Some("tabs_unreadable") {
                tracing::warn!(
                    reason = "tabs_unreadable",
                    "visual sound row proof is incomplete"
                );
                last_rejection = Some("tabs_unreadable");
            }
            prior = None;
            tokio::time::sleep(POLL).await;
            continue;
        }
        if hot_selected(&img, &tabs) {
            saw_hot.store(true, Ordering::Relaxed);
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
                last_rejection = None;
                if prior
                    .as_ref()
                    .is_some_and(|(old, p)| old == &epoch && p.stable_with(&pool))
                {
                    return Ok(pool);
                }
                prior = Some((epoch, pool));
            }
            Err(error) => {
                let reason = visual_rejection_code(&error);
                if last_rejection != Some(reason) {
                    tracing::warn!(reason, "visual sound row proof is incomplete");
                    last_rejection = Some(reason);
                }
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
    if let Some(selected) = fresh.selected_index {
        if selected != index {
            anyhow::ensure!(
                pool.selected_index == Some(selected)
                    && pool.candidates.get(selected) == fresh.candidates.get(selected),
                "a different sound row became selected after observation"
            );
            // A preselection already present in the original stable pool is
            // TikTok's initial choice. The durable target may replace it once.
            let target = reproof_target(pool, &fresh, index)?;
            tap_native_image(session, target.centre()).await?;
        } else {
            reproof_target(pool, &fresh, index)?;
        }
    } else {
        let target = reproof_target(pool, &fresh, index)?;
        tap_native_image(session, target.centre()).await?;
    }
    wait_selected_row(session, plan, &fresh, index).await?;
    // One reversible selection only. Fresh sheet proof authorizes Back; exact
    // title on two XML editor snapshots is the final independent verification.
    selection_recovery::prove_sheet(session, plan).await?;
    check_wait()?;
    session.back().await?;
    selection_recovery::confirm_editor(session, plan, &candidate.title).await
}

pub(super) async fn recover(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    pool: &ObservedSoundPool,
    index: usize,
) -> anyhow::Result<()> {
    let candidate = pool.candidates.get(index).context("sound index missing")?;
    let (image, _, tabs, _) = capture(session, plan, 1).await?;
    if tabs.len() != 4 {
        let source = read_sound(session.hierarchy_source_snapshot()).await?;
        let tree = crate::ui_automation::tree::Tree::parse(source)?;
        if tree
            .matching(
                plan.package,
                ElementQuery::ResourceIdSuffix(plan.current_title_id),
            )
            .is_empty()
        {
            anyhow::bail!("composer state lost; rebuild approved media before retry");
        }
        return selection_recovery::confirm_editor(session, plan, &candidate.title).await;
    }
    if selected_row_ready(&image, &tabs, pool, index) {
        wait_selected_row(session, plan, pool, index).await?;
        selection_recovery::prove_sheet(session, plan).await?;
        check_wait()?;
        session.back().await?;
        return selection_recovery::confirm_editor(session, plan, &candidate.title).await;
    }
    // choose reobserves two stable rows and either proves the same selected identity
    // or performs one selection from a fully unselected pool.
    choose(session, plan, pool, index).await
}

async fn wait_selected_row(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    pool: &ObservedSoundPool,
    index: usize,
) -> anyhow::Result<()> {
    let deadline =
        phase_deadline(Duration::from_secs(60)).min(Instant::now() + Duration::from_secs(60));
    let epoch = session.gui_session_epoch();
    let mut stable = 0;
    let mut generation = 0;
    loop {
        check_wait()?;
        anyhow::ensure!(
            Instant::now() < deadline,
            "TikTok chưa xác nhận tải nhạc đã chọn; chưa bấm Đăng"
        );
        generation += 1;
        let (image, _, tabs, current) = capture(session, plan, generation).await?;
        anyhow::ensure!(current == epoch, "sound selection session changed");
        if selected_row_ready(&image, &tabs, pool, index) {
            stable += 1;
            if stable == 2 {
                return Ok(());
            }
        } else {
            stable = 0;
        }
        tokio::time::sleep(POLL).await;
    }
}

fn selected_row_ready(
    image: &image::RgbImage,
    tabs: &[OcrRect],
    pool: &ObservedSoundPool,
    index: usize,
) -> bool {
    if tabs.len() != 4 || !hot_selected(image, tabs) || index >= pool.targets.len() {
        return false;
    }
    // Selection inserts an equalizer and trim controls, clipping even a title
    // previously read in full. On measured 45.7.3 the red title can disappear
    // from OCR entirely. A unique red row only acknowledges our single pick;
    // choose() still requires the exact full title on two editor XML snapshots.
    let selected: Vec<_> = pool
        .targets
        .iter()
        .enumerate()
        .filter_map(|(i, target)| {
            let bounds = OcrRect {
                x: target.x as u32,
                y: target.y as u32,
                width: target.width as u32,
                height: target.height as u32,
            };
            (red_fraction(image, bounds) > 0.5).then_some(i)
        })
        .collect();
    selected == [index]
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
    fn clipped_ocr_title_without_ellipsis_does_not_hide_complete_alternative_rows() {
        for title in ["Song (Afternoon Ra", "Song...", "Song\u{2026}"] {
            let (img, mut lines, tabs, plan) = fixture();
            lines[0].text = title.into();
            let pool = pool_from_image(&img, &lines, &tabs, plan, 5).unwrap();
            assert_eq!(pool.candidates.len(), 2);
            assert!(pool
                .candidates
                .iter()
                .all(|candidate| candidate.title != title));
            assert_eq!(pool.maximum_visible, 5);
        }
    }

    #[test]
    fn visual_rejection_logging_never_includes_ocr_content() {
        assert_eq!(
            visual_rejection_code(&anyhow::anyhow!(
                "visual sound has no complete unselected row"
            )),
            "no_complete_row"
        );
        assert_eq!(
            visual_rejection_code(&anyhow::anyhow!("private sound title: account name")),
            "other"
        );
    }

    #[test]
    fn live_us_sheet_accepts_complete_31px_titles_and_excludes_the_cut_row() {
        let img = image::load_from_memory(include_bytes!(
            "../../fixtures/tiktok-publish/musically-45.7.3-en/hot-us-visual.png"
        ))
        .unwrap()
        .to_rgb8();
        let lines: Vec<OcrLine> = serde_json::from_str(include_str!(
            "../../fixtures/tiktok-publish/musically-45.7.3-en/hot-us-visual-ocr.json"
        ))
        .unwrap();
        let tabs = [(46, 64), (179, 131), (381, 162), (613, 126)]
            .into_iter()
            .map(|(x, width)| OcrRect {
                x,
                y: 1128,
                width,
                height: 29,
            })
            .collect::<Vec<_>>();
        let plan = SoundPickerPlan::resolve("com.zhiliaoapp.musically", "en", "45.7.3").unwrap();
        let pool = pool_from_image(&img, &lines, &tabs, plan, 5).unwrap();
        assert_eq!(
            pool.candidates
                .iter()
                .map(|candidate| candidate.title.as_str())
                .collect::<Vec<_>>(),
            ["Vibin", "GGEZ", "Nicole Kidman"]
        );
        assert!(pool
            .candidates
            .iter()
            .all(|candidate| candidate.title != "nothing"));
    }

    #[test]
    fn clipped_selected_title_can_acknowledge_only_the_chosen_row() {
        let (_, _, tabs, plan) = fixture();
        let before = image::load_from_memory(include_bytes!(
            "../../fixtures/tiktok-publish/musically-45.7.3-en/hot-before-third.png"
        ))
        .unwrap()
        .to_rgb8();
        let selected = image::load_from_memory(include_bytes!(
            "../../fixtures/tiktok-publish/musically-45.7.3-en/hot-selected-third.png"
        ))
        .unwrap()
        .to_rgb8();
        let lines: Vec<OcrLine> = serde_json::from_str(include_str!(
            "../../fixtures/tiktok-publish/musically-45.7.3-en/hot-before-third-ocr.json"
        ))
        .unwrap();
        let after_lines: Vec<OcrLine> = serde_json::from_str(include_str!(
            "../../fixtures/tiktok-publish/musically-45.7.3-en/hot-selected-third-ocr.json"
        ))
        .unwrap();
        let pool = pool_from_image(&before, &lines, &tabs, plan, 5).unwrap();
        assert_eq!(pool.candidates[2].title, "Thương Phận Hồng Nhan - remix");
        assert!(!after_lines
            .iter()
            .any(|l| l.text == pool.candidates[2].title));
        assert!(!selected_row_ready(&before, &tabs, &pool, 2));
        assert!(selected_row_ready(&selected, &tabs, &pool, 2));
        assert!(!selected_row_ready(&selected, &tabs, &pool, 0));
        assert!(!selected_row_ready(&selected, &[], &pool, 2));
        let mut ambiguous = pool.clone();
        ambiguous.targets.push(pool.targets[2].clone());
        assert!(!selected_row_ready(&selected, &tabs, &ambiguous, 2));
    }

    #[test]
    fn music_network_error_requires_the_sheet_and_measured_message_region() {
        let (_, _, tabs, _) = fixture();
        let mut line = OcrLine {
            text: "€ Your network is unstable. Please try again.".into(),
            bounds: OcrRect {
                x: 199,
                y: 1622,
                width: 681,
                height: 37,
            },
            confidence: 0.9447,
        };
        assert!(network_unavailable(&[line.clone()], &tabs));
        assert!(!network_unavailable(&[line.clone()], &[]));
        line.bounds.y = 800;
        assert!(!network_unavailable(&[line], &tabs));
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
    fn selected_red_row_is_retained_as_state_and_bottom_clipped_row_is_excluded() {
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
        assert_eq!(pool.candidates.len(), 3);
        assert_eq!(pool.selected_index, Some(0));
        assert_eq!(pool.candidates[0].title, "Thương Nhau Đến Thế");
    }

    struct Ocr;
    #[async_trait::async_trait]
    impl GuiReasoner for Ocr {
        async fn resolve(&self, _: GuiRequest) -> anyhow::Result<GuiResponse> {
            unreachable!()
        }
        async fn ocr(&self, r: OcrRequest) -> anyhow::Result<OcrResponse> {
            let (_, mut rows, tabs, _) = fixture();
            let selected_hash = format!(
                "{:x}",
                Sha256::digest(include_bytes!(
                    "../../fixtures/tiktok-publish/musically-45.7.3-en/hot-visual-selected.png"
                ))
            );
            if r.screenshot.sha256 == selected_hash {
                rows[0].bounds.x = 321;
            }
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
        selection_reads: AtomicUsize,
        loading_frames: usize,
        initially_selected: bool,
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
            if (self.initially_selected || self.taps.load(Ordering::Relaxed) > 0)
                && self.selection_reads.fetch_add(1, Ordering::Relaxed) >= self.loading_frames
            {
                return Ok(include_bytes!(
                    "../../fixtures/tiktok-publish/musically-45.7.3-en/hot-visual-selected.png"
                )
                .to_vec());
            }
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
            anyhow::ensure!(
                self.selection_reads.load(Ordering::Relaxed) > self.loading_frames,
                "Back interrupted the pending sound download"
            );
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
                selection_reads: AtomicUsize::new(0),
                loading_frames: 0,
                initially_selected: false,
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
    #[tokio::test(start_paused = true)]
    async fn slow_sound_download_is_observed_before_leaving_sheet_without_second_pick() {
        let session = Session {
            taps: AtomicUsize::new(0),
            backs: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
            wrong_title: false,
            selection_reads: AtomicUsize::new(0),
            loading_frames: 3,
            initially_selected: false,
        };
        let plan = fixture().3;
        let pool = observe(&session, plan, 5, false).await.unwrap();
        choose(&session, plan, &pool, 0).await.unwrap();
        assert_eq!(session.taps.load(Ordering::Relaxed), 1);
        assert_eq!(session.backs.load(Ordering::Relaxed), 1);
        assert!(session.selection_reads.load(Ordering::Relaxed) >= 5);
    }

    #[tokio::test(start_paused = true)]
    async fn recovery_of_lost_selection_ack_observes_red_row_without_second_pick() {
        let session = Session {
            taps: AtomicUsize::new(0),
            backs: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
            wrong_title: false,
            selection_reads: AtomicUsize::new(0),
            loading_frames: 0,
            initially_selected: false,
        };
        let plan = fixture().3;
        let pool = observe(&session, plan, 5, false).await.unwrap();
        session.tap(pool.targets[0].centre()).await.unwrap();
        recover(&session, plan, &pool, 0).await.unwrap();
        assert_eq!(session.taps.load(Ordering::Relaxed), 1);
        assert_eq!(session.backs.load(Ordering::Relaxed), 1);
        assert!(session.reads.load(Ordering::Relaxed) >= 2);
    }

    #[tokio::test(start_paused = true)]
    async fn an_already_selected_visual_row_is_confirmed_without_toggling_it() {
        let session = Session {
            taps: AtomicUsize::new(0),
            backs: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
            wrong_title: false,
            selection_reads: AtomicUsize::new(0),
            loading_frames: 0,
            initially_selected: true,
        };
        let plan = fixture().3;
        let pool = observe(&session, plan, 5, false).await.unwrap();
        assert_eq!(pool.selected_index, Some(0));

        choose(&session, plan, &pool, 0).await.unwrap();

        assert_eq!(session.taps.load(Ordering::Relaxed), 0);
        assert_eq!(session.backs.load(Ordering::Relaxed), 1);
    }
}
