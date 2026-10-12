//! Measured native sound-sheet pixels, never a blind fallback.
use super::*;
use crate::ui_automation::{OcrImage, OcrLine, OcrRect, OcrRequest};
use base64::Engine;
use sha2::{Digest, Sha256};
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

/// #47 (Global 45.7.3/en, box3150g, 2026-10-10 01:00:19-57): the Hot contact tap at +8.7 s first
/// showed as selected in the frame completed at +21.6 s; the frame 4 s after the tap still showed
/// the old tab. A second Hot tap waits out this window on fresh frames first.
const HOT_TAP_SETTLE: Duration = Duration::from_secs(15);
/// Row contact taps per sound stage, across `wait_selected_row` and the composer's recovery.
/// Phone 44 (ce031713d903001b01, box3150k, 2026-10-10 13:15:13): the first row tap, like its
/// first Hot tap at 13:14:24, left the sheet unchanged; the second selected the row. The second
/// tap needs HOT_TAP_SETTLE and two fresh frames proving no row selected.
const ROW_TAP_CAP: u32 = 2;

// Scoped only to the existing pool observation. No extra reads or effect permission;
// production tracing retains the binding even without opt-in stage diagnostics.
struct VisualSoundDiagnostics {
    scope: Option<crate::ui_automation::GuiScope>,
    epoch: String,
    started: Instant,
    deadline: Instant,
    generation: u64,
    request_id: Option<String>,
    observation_id: Option<String>,
    vietnamese_request_id: Option<String>,
    vietnamese_observation_id: Option<String>,
    selected_tab: Option<usize>,
    tab_bounds: Vec<[u32; 4]>,
    saw_hot: bool,
    same_unselected_tabs: Option<bool>,
    navigation_attempts: u32,
    rejection: Option<&'static str>,
    rejection_events: u8,
}
tokio::task_local! {
    static VISUAL_SOUND_DIAGNOSTICS: std::cell::RefCell<VisualSoundDiagnostics>;
}
impl VisualSoundDiagnostics {
    fn emit(&self, phase: &'static str, acknowledged: Option<bool>) {
        let now = Instant::now();
        let remaining_effective_budget_ms = SOUND_BUDGET.try_with(|budget| budget.deadline
            .saturating_duration_since(now).as_millis().min(u64::MAX as u128) as u64).ok();
        macro_rules! emit {
            ($level:expr) => {
                tracing::event!($level,
                    serial=?self.scope.as_ref().map(|scope| scope.device_id.as_str()),
                    runId=?self.scope.as_ref().map(|scope| scope.run_id.as_str()),
                    assignmentId=?self.scope.as_ref().and_then(|scope| scope.assignment_id.as_deref()),
                    session=%self.epoch, captureGeneration=self.generation,
                    requestId=?self.request_id, observationId=?self.observation_id,
                    vietnameseRequestId=?self.vietnamese_request_id,
                    vietnameseObservationId=?self.vietnamese_observation_id,
                    selectedTabIndex=?self.selected_tab, tabBounds=?self.tab_bounds,
                    sawHot=self.saw_hot, sameUnselectedTabs=?self.same_unselected_tabs,
                    navigationAttempts=self.navigation_attempts, rejection=?self.rejection,
                    rejectionEvents=self.rejection_events, phase, acknowledged=?acknowledged,
                    elapsedMs=now.saturating_duration_since(self.started).as_millis().min(u64::MAX as u128) as u64,
                    remainingPhaseMs=self.deadline.saturating_duration_since(now).as_millis().min(u64::MAX as u128) as u64,
                    remainingEffectiveBudgetMs=?remaining_effective_budget_ms,
                    "visual sound observation diagnostic"
                );
            };
        }
        if matches!(phase, "rejectionChanged" | "timeout" | "observationError") {
            emit!(tracing::Level::WARN);
        } else {
            emit!(tracing::Level::INFO);
        }
    }

    fn reject(&mut self, reason: &'static str) {
        self.rejection = Some(reason);
        // A flickering sheet cannot turn the bounded polling loop into log spam.
        // Final timeout still retains the latest rejection after this cap.
        if self.rejection_events < 16 {
            self.rejection_events += 1;
            self.emit("rejectionChanged", None);
        }
    }
}

fn visual_diagnostic(update: impl FnOnce(&mut VisualSoundDiagnostics)) {
    let _ = VISUAL_SOUND_DIAGNOSTICS.try_with(|state| update(&mut state.borrow_mut()));
}

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
    visual_diagnostic(|state| {
        state.navigation_attempts += 1;
        state.emit("nativeHotDispatch", None);
    });
    let result = session.tap_image(point.x, point.y, 1080., 2220.).await;
    visual_diagnostic(|state| state.emit("nativeHotAck", Some(result.is_ok())));
    result?;
    check_wait()
}

/// Which sheet control a contact tap is for; only the log and the ReadRecovery note differ.
#[derive(Clone, Copy)]
enum SheetContact {
    Hot,
    Row(usize),
}

/// The human contact tap (45-130 ms, the same gesture the XML route sends through
/// `sound_tap`) at a point measured on a fresh 1080x2220 frame. Machine29 (2026-10-01) and #47
/// (Global 45.7.3/en, 2026-10-10 00:11:01, box3150e): acknowledged 16 ms native taps on the
/// OCR-located Hot centre left For You selected, and #47's native row tap never turned its row
/// red in the 60 s that followed; the contact tap at the same centre did switch the tab. The
/// frame is 1080x2220 by construction, so the point reaches the same control only when the
/// screen proves the same size, on the same session and app, right before the gesture.
async fn tap_contact_image(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    epoch: &str,
    point: crate::TapPoint,
    contact: SheetContact,
) -> anyhow::Result<()> {
    check_wait()?;
    let screen = read_sound(session.window_size()).await?;
    anyhow::ensure!(
        screen == (1080.0, 2220.0),
        "visual sound contact coordinates unmeasured"
    );
    let package = read_sound(session.active_app_bundle()).await?;
    anyhow::ensure!(
        session.gui_session_epoch() == epoch && package == plan.package,
        "visual sound app/session changed before the contact tap"
    );
    check_wait()?;
    let row_tap = match contact {
        SheetContact::Hot => None,
        SheetContact::Row(_) => Some(claim_row_tap(ROW_TAP_CAP).ok_or_else(|| {
            crate::publish_recovery::retryable_error(
                "sound_row_tap_cap",
                "Đã bấm hàng nhạc đủ số lần cho phép trong lượt này mà chưa chứng minh được hàng đã chọn; không bấm thêm, chưa bấm Đăng",
            )
        })?),
    };
    let serial = session.gui_scope().map(|scope| scope.device_id);
    let (dispatch, ack, index) = match contact {
        SheetContact::Hot => ("contactHotDispatch", "contactHotAck", None),
        SheetContact::Row(index) => ("contactRowDispatch", "contactRowAck", Some(index)),
    };
    visual_diagnostic(|state| {
        if matches!(contact, SheetContact::Hot) {
            state.navigation_attempts += 1;
        }
        state.emit(dispatch, None);
    });
    // The row tap runs outside the observation's diagnostic scope; #47's row step logged
    // nothing for 60 s. This event is the dispatch evidence either way.
    let row_tap = row_tap.map(|count| format!("{count}/{ROW_TAP_CAP}"));
    tracing::info!(serial=?serial, session=%epoch, phase=dispatch, rowIndex=?index,
        rowTap=?row_tap, x=point.x, y=point.y, "visual sound contact tap");
    let result = session.tap(point).await;
    visual_diagnostic(|state| state.emit(ack, Some(result.is_ok())));
    tracing::info!(serial=?serial, session=%epoch, phase=ack, rowIndex=?index,
        rowTap=?row_tap, acknowledged=result.is_ok(), "visual sound contact tap");
    result?;
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

fn red_pixel([a, b, c]: [u8; 3]) -> bool {
    a > 170 && f64::from(a) > f64::from(b) * 1.5 && f64::from(a) > f64::from(c) * 1.2
}

fn red_fraction(img: &image::RgbImage, r: OcrRect) -> f64 {
    let (mut red, mut dark) = (0u32, 0u32);
    for y in r.y..(r.y + r.height).min(img.height()) {
        for x in r.x..(r.x + r.width).min(img.width()) {
            let pixel = img.get_pixel(x, y).0;
            if red_pixel(pixel) {
                red += 1;
            }
            if pixel.iter().all(|c| *c < 180) {
                dark += 1;
            }
        }
    }
    f64::from(red) / f64::from((red + dark).max(1))
}

/// Sheet list band. 45.7.3/en rows start at y≈1370 without the region chips and y≈1686 under
/// them (phone 44 ce031713d903001b01, 2026-10-10, `44-after-fail.png`); the list clips at ~1930.
const ROW_LIST: std::ops::Range<u32> = 1200..1935;
/// A selected row's thumbnail gains a 4 px red frame at x 110..234 (unselected art spans
/// ~115..230). Measured on `hot-visual-selected.png`, `hot-selected-third.png` and phone 44's
/// `44-after-fail.png`: both columns red for 107 px; no unselected thumbnail reaches them.
const THUMB_FRAME_COLUMNS: [u32; 2] = [112, 232];
const THUMB_FRAME_MIN_RUN: u32 = 90;
/// Title, equalizer and artist column; unselected titles are black on white.
const ROW_TEXT_COLUMNS: std::ops::Range<u32> = 250..1040;

/// What the list pixels alone say about selection, independent of OCR.
struct RowPixels {
    /// y-spans where both thumbnail frame columns are red for a full row height.
    framed: Vec<(u32, u32)>,
    /// Any red at the frame columns or on a text-column scanline (≥3 px) in the list.
    any_red: bool,
}

fn row_pixels(img: &image::RgbImage) -> RowPixels {
    let mut framed = Vec::new();
    let mut any_red = false;
    let mut run: Option<u32> = None;
    let bottom = ROW_LIST.end.min(img.height());
    for y in ROW_LIST.start..=bottom {
        let both = y < bottom
            && THUMB_FRAME_COLUMNS
                .iter()
                .all(|x| *x < img.width() && red_pixel(img.get_pixel(*x, y).0));
        if y < bottom {
            any_red |= THUMB_FRAME_COLUMNS.iter().any(|centre| {
                (centre - 1..=centre + 1)
                    .any(|x| x < img.width() && red_pixel(img.get_pixel(x, y).0))
            }) || ROW_TEXT_COLUMNS
                .clone()
                .filter(|x| *x < img.width() && red_pixel(img.get_pixel(*x, y).0))
                .nth(2)
                .is_some();
        }
        match (both, run) {
            (true, None) => run = Some(y),
            (false, Some(start)) => {
                if y - start >= THUMB_FRAME_MIN_RUN {
                    framed.push((start, y));
                }
                run = None;
            }
            _ => {}
        }
    }
    RowPixels { framed, any_red }
}

fn target_rect(target: &ElementBox) -> OcrRect {
    OcrRect {
        x: target.x as u32,
        y: target.y as u32,
        width: target.width as u32,
        height: target.height as u32,
    }
}

/// Pool targets whose vertical centre lies inside a framed row.
fn targets_in_span(targets: &[ElementBox], (top, bottom): (u32, u32)) -> Vec<usize> {
    targets
        .iter()
        .enumerate()
        .filter(|(_, target)| {
            let centre = target.y + target.height / 2.0;
            centre >= f64::from(top) && centre < f64::from(bottom)
        })
        .map(|(index, _)| index)
        .collect()
}

fn hot_selected(img: &image::RgbImage, tabs: &[OcrRect]) -> bool {
    selected_tab(img, tabs) == Some(0)
}

fn selected_tab(img: &image::RgbImage, tabs: &[OcrRect]) -> Option<usize> {
    if tabs.len() != 4 {
        return None;
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
    let mut selected = tabs.iter().enumerate().filter(|(_, rect)| underlined(rect));
    let index = selected.next()?.0;
    selected.next().is_none().then_some(index)
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
    let mut title_folds = std::collections::HashSet::new();
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
        anyhow::ensure!(
            title_folds.insert(fold_sound_title(&title.text)),
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
    // A selected row OCR could not bind (its red title is often clipped or missing) still shows
    // its red thumbnail frame. Such a frame is never read as "no row selected".
    match row_pixels(image).framed.as_slice() {
        [] => {}
        [span] => {
            let [row] = targets_in_span(&targets, *span)[..] else {
                anyhow::bail!("visual sound selected row unbound");
            };
            anyhow::ensure!(
                selected_index.is_none_or(|selected| selected == row),
                "visual sound selected row ambiguous"
            );
            selected_index = Some(row);
        }
        _ => anyhow::bail!("visual sound selected row ambiguous"),
    }
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
        "visual sound selected row unbound" => "selected_row_unbound",
        "visual sound has no complete unselected row" => "no_complete_row",
        _ => "other",
    }
}

fn fold_sound_title(value: &str) -> String {
    value
        .nfd()
        .filter(|character| !is_combining_mark(*character))
        .map(|character| match character {
            'đ' => 'd',
            'Đ' => 'D',
            other => other,
        })
        .collect()
}

fn accent_count(value: &str) -> usize {
    value
        .nfd()
        .filter(|character| is_combining_mark(*character) || matches!(character, 'đ' | 'Đ'))
        .count()
}

fn enrich_title_lines(lines: &mut [OcrLine], vietnamese: &[OcrLine]) {
    for line in lines.iter_mut().filter(|line| {
        line.confidence >= 0.9
            && line.bounds.x >= 250
            && (1320..1900).contains(&line.bounds.y)
            && line.bounds.width >= 80
            && line.bounds.height >= 28
    }) {
        let original_fold = fold_sound_title(&line.text);
        let original_accents = accent_count(&line.text);
        let matching = vietnamese
            .iter()
            .filter(|candidate| {
                candidate.confidence >= 0.9
                    && line.bounds.x.abs_diff(candidate.bounds.x) <= 4
                    && line.bounds.y.abs_diff(candidate.bounds.y) <= 4
                    && line.bounds.width.abs_diff(candidate.bounds.width) <= 4
                    && line.bounds.height.abs_diff(candidate.bounds.height) <= 4
                    && fold_sound_title(&candidate.text) == original_fold
                    && accent_count(&candidate.text) > original_accents
            })
            .collect::<Vec<_>>();
        if let [candidate] = matching.as_slice() {
            line.text = candidate.text.nfc().collect();
        }
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
    // #47 (Global 45.7.3/en, 2026-10-09): after the hand-off this XML popup check waited
    // ~10.5 s per frame for the same unreadable root, and its failure ended the visual route
    // before any frame. Past the cap the frame alone decides: a covering popup hides the
    // measured tabs and rows, so nothing is tapped; no popup control is pressed without XML.
    if visual_handoff_due(session, plan) {
        note_popup_check_skipped("ảnh bảng nhạc");
    } else {
        match clear_sound_popup(session, phase_deadline(SOUND_WINDOW), Some(plan.package)).await {
            Ok(()) => {}
            Err(error) if unreadable_root(&error) && visual_handoff_due(session, plan) => {
                note_popup_check_skipped("ảnh bảng nhạc");
            }
            Err(error) => return Err(error),
        }
    }
    // The selected tab and tab bounds stay those of the last frame OCR answered until this
    // frame's OCR replaces them. Run 72401bfd (other PC, 2026-10-12): both timeouts fired while
    // a frame was being captured, so clearing them here logged `tabBounds=[]` and hid what the
    // sheet showed after the second Hot tap.
    visual_diagnostic(|state| {
        state.generation = generation;
        state.request_id = None;
        state.observation_id = None;
        state.vietnamese_request_id = None;
        state.vietnamese_observation_id = None;
        state.same_unselected_tabs = None;
        if generation <= 2 {
            state.emit("captureStart", None);
        }
    });
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
    visual_diagnostic(|state| {
        state.epoch = epoch.clone();
        state.request_id = Some(request.request_id.clone());
        state.observation_id = Some(request.observation_id.clone());
        if generation <= 2 {
            state.emit("ocrRequest", None);
        }
    });
    let reasoner = session
        .gui_reasoner()
        .context("visual sound local OCR missing")?;
    let response = read_sound(async {
        reasoner
            .ocr(request.clone())
            .await
            .map_err(|error| anyhow::Error::new(OcrReadUnavailable(error)))
    })
    .await?;
    response.validate_binding(&request)?;
    let tabs = selection_recovery::tabs(&response).unwrap_or_default();
    visual_diagnostic(|state| {
        state.selected_tab = selected_tab(&image, &tabs);
        state.tab_bounds = tabs.iter().take(4)
            .map(|rect| [rect.x, rect.y, rect.width, rect.height]).collect();
    });
    let mut lines = response.lines;
    if tabs.len() == 4
        && !network_unavailable(&lines, &tabs)
        && lines
            .iter()
            .any(|line| line.bounds.x >= 250 && (1320..1900).contains(&line.bounds.y))
    {
        let mut vietnamese_request = request.clone();
        vietnamese_request.request_id = uuid::Uuid::new_v4().to_string();
        vietnamese_request.observation_id = uuid::Uuid::new_v4().to_string();
        vietnamese_request.languages = vec!["vi".into()];
        visual_diagnostic(|state| {
            state.vietnamese_request_id = Some(vietnamese_request.request_id.clone());
            state.vietnamese_observation_id = Some(vietnamese_request.observation_id.clone());
        });
        if let Ok(vietnamese) = read_sound(reasoner.ocr(vietnamese_request.clone())).await {
            if vietnamese.validate_binding(&vietnamese_request).is_ok() {
                enrich_title_lines(&mut lines, &vietnamese.lines);
            }
        }
    }
    anyhow::ensure!(
        epoch == session.gui_session_epoch()
            && read_sound(session.active_app_bundle()).await? == plan.package,
        "visual sound session changed"
    );
    check_wait()?;
    // The first frame after opening the sheet can contain only the video.
    // This is a pending observation, never permission to tap or an adapter error.
    Ok((image, lines, tabs, epoch))
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
/// Past the blind-root cap that read cannot answer, so until Hot is proven the frames get the
/// enclosing sound budget, which stays finite. #44 (box3150j, 2026-10-10 08:32:08): the 60 s cap timed out
/// with 23.7 s of that budget left while a Hot tap was still settling (see HOT_TAP_SETTLE).
pub(super) async fn observe_initial(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
) -> anyhow::Result<ObservedSoundPool> {
    let saw_hot = AtomicBool::new(false);
    let window = if visual_handoff_due(session, plan) {
        SOUND_WINDOW
    } else {
        Duration::from_secs(60)
    };
    let deadline = phase_deadline(window).min(Instant::now() + window);
    let result = with_stage_budget(
        window,
        observe_inner(session, plan, maximum, true, &saw_hot, window),
    )
    .await;
    check_wait()?;
    match result {
        Err(_) if Instant::now() >= deadline => visual_pool_timeout(&saw_hot),
        result => result,
    }
}

fn visual_pool_timeout(saw_hot: &AtomicBool) -> anyhow::Result<ObservedSoundPool> {
    if saw_hot.load(Ordering::Relaxed) {
        Err(VisualSoundPoolUnavailable.into())
    } else {
        note_hot_unconfirmed();
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
    observe_inner(
        session,
        plan,
        maximum,
        navigate,
        &AtomicBool::new(false),
        Duration::from_secs(60),
    )
    .await
}

async fn observe_inner(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
    navigate: bool,
    saw_hot: &AtomicBool,
    window: Duration,
) -> anyhow::Result<ObservedSoundPool> {
    let started = Instant::now();
    let deadline = phase_deadline(window);
    // Once Hot is proven, rows that do not bind within 60 s of the stage start will not bind by
    // sight; the rest of a longer window stays for the caller's hierarchy read.
    let rows_deadline = phase_deadline(Duration::from_secs(60));
    let diagnostics = VisualSoundDiagnostics {
        scope: session.gui_scope(), epoch: session.gui_session_epoch(), started, deadline,
        generation: 0, request_id: None, observation_id: None,
        vietnamese_request_id: None, vietnamese_observation_id: None,
        selected_tab: None, tab_bounds: Vec::new(), navigation_attempts: 0,
        saw_hot: saw_hot.load(Ordering::Relaxed), same_unselected_tabs: None,
        rejection: None, rejection_events: 0,
    };
    VISUAL_SOUND_DIAGNOSTICS.scope(std::cell::RefCell::new(diagnostics), async {
        let result = observe_loop(
            session,
            plan,
            maximum,
            navigate,
            saw_hot,
            deadline,
            rows_deadline,
        )
        .await;
        if result.is_err() {
            let expired = Instant::now() >= deadline
                || (saw_hot.load(Ordering::Relaxed) && Instant::now() >= rows_deadline);
            visual_diagnostic(|state| state.emit(
                if expired { "timeout" } else { "observationError" }, None));
        }
        result
    }).await
}

async fn observe_loop(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
    navigate: bool,
    saw_hot: &AtomicBool,
    deadline: Instant,
    rows_deadline: Instant,
) -> anyhow::Result<ObservedSoundPool> {
    let mut prior: Option<(String, ObservedSoundPool)> = None;
    let mut navigation_attempts = 0;
    let mut last_hot_tap: Option<Instant> = None;
    let mut unselected_tabs: Option<(String, usize, Vec<OcrRect>)> = None;
    let mut generation = 0;
    let mut last_rejection = None;
    loop {
        check_wait()?;
        if Instant::now() >= deadline
            || (saw_hot.load(Ordering::Relaxed) && Instant::now() >= rows_deadline)
        {
            return visual_pool_timeout(saw_hot);
        }
        generation += 1;
        let (img, lines, tabs, epoch) = capture(session, plan, generation).await?;
        if tabs.len() != 4 {
            visual_diagnostic(|state| {
                if generation <= 2 {
                    state.emit("captureComplete", None);
                }
            });
            if last_rejection != Some("tabs_unreadable") {
                visual_diagnostic(|state| state.reject("tabs_unreadable"));
                last_rejection = Some("tabs_unreadable");
            }
            prior = None;
            tokio::time::sleep(POLL).await;
            continue;
        }
        if hot_selected(&img, &tabs) {
            saw_hot.store(true, Ordering::Relaxed);
        }
        let selected = selected_tab(&img, &tabs);
        let same_unselected_tabs =
            unselected_tabs
                .as_ref()
                .is_some_and(|(before_epoch, before_selected, before)| {
                    before_epoch == &epoch && Some(*before_selected) == selected && before == &tabs
                });
        visual_diagnostic(|state| {
            state.saw_hot = saw_hot.load(Ordering::Relaxed);
            state.same_unselected_tabs = Some(same_unselected_tabs);
            if generation <= 2 {
                state.emit("captureComplete", None);
            }
        });
        unselected_tabs = selected
            .filter(|index| *index != 0)
            .map(|index| (epoch.clone(), index, tabs.clone()));
        // A navigation ACK is not arrival: machine25 stayed on For You after
        // its first Hot tap. One retry requires two fresh unchanged tab reads and
        // HOT_TAP_SETTLE since the last tap; never retry after Hot has appeared.
        // Both attempts are contact taps at the OCR-located Hot centre: the 16 ms
        // native tap was acknowledged and did nothing on machine29 and #47
        // (see `tap_contact_image`); the retry still needs fresh Hot proof.
        if navigate
            && !saw_hot.load(Ordering::Relaxed)
            && !hot_selected(&img, &tabs)
            && navigation_attempts < 2
            && (navigation_attempts == 0
                || (same_unselected_tabs
                    && last_hot_tap.is_some_and(|at: Instant| at.elapsed() >= HOT_TAP_SETTLE)))
        {
            visual_diagnostic(|state| state.emit("navigationProofStart", None));
            // #44 (box3150j, 2026-10-10 08:31:14-25): a second two-frame OCR proof here cost
            // ~11 s (two ~5.5 s screenshot+OCR passes) between this frame and the tap. This
            // frame is already fresh, on this epoch and app, with four unique tab labels in
            // the measured geometry (`selection_recovery::tabs`), so its Hot box is the target.
            let hot = tabs[0];
            let point = crate::TapPoint {
                x: f64::from(hot.x) + f64::from(hot.width) / 2.0,
                y: f64::from(hot.y) + f64::from(hot.height) / 2.0,
            };
            anyhow::ensure!(Instant::now() < deadline, "Hot navigation deadline expired");
            last_hot_tap = Some(Instant::now());
            let tapped = tap_contact_image(session, plan, &epoch, point, SheetContact::Hot).await;
            note_tab_tap();
            tapped?;
            navigation_attempts += 1;
            crate::publish_recovery::note_read(
                "sound",
                "visualHotNavigation",
                navigation_attempts,
                "awaitingProof",
                Some(&format!(
                    "Đã bấm tab Hot theo vị trí OCR (lần {navigation_attempts}, cách bấm contact); chờ ảnh mới chứng minh Hot đã chọn, chưa chọn nhạc"
                )),
            );
            unselected_tabs = None;
            prior = None;
            continue;
        }
        match pool_from_image(&img, &lines, &tabs, plan, maximum) {
            Ok(pool) => {
                if last_rejection.is_some() {
                    visual_diagnostic(|state| {
                        state.rejection = None;
                        if state.rejection_events < 16 {
                            state.rejection_events += 1;
                            state.emit("rejectionCleared", None);
                        }
                    });
                }
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
                    visual_diagnostic(|state| state.reject(reason));
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
    let row_started = Instant::now();
    select_row_and_close(session, plan, pool, index, &mut false).await?;
    let row_ms = row_started.elapsed().as_millis() as u64;
    let candidate = pool
        .candidates
        .get(index)
        .context("visual sound index out of range")?;
    let editor_started = Instant::now();
    selection_recovery::confirm_editor_or_chip(session, plan, candidate).await?;
    let editor_ms = editor_started.elapsed().as_millis() as u64;
    crate::publish_recovery::note_read(
        "sound",
        "visualSelection",
        1,
        "confirmed",
        Some(&format!(
            "selectionRoute: visual; bấm một hàng theo OCR, chờ hàng chuyển hồng và đóng bảng mất {row_ms} ms; xác nhận tên nhạc trên màn chỉnh sửa mất {editor_ms} ms (số lần không đọc được cây giao diện: {})",
            unreadable_roots()
        )),
    );
    Ok(())
}

/// The XML pool froze `frozen.candidates[index]`, then the sheet root stopped answering.
/// Find that exact title/artist once among stable visual rows and select it there. A row
/// already shown selected is never tapped again: a second tap deselects the sound.
pub(super) async fn hand_off(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    frozen: &ObservedSoundPool,
    index: usize,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        selection_recovery::measured(plan) && plan.section_label == "Hot",
        "visual sound hand-off tuple unmeasured"
    );
    let candidate = frozen
        .candidates
        .get(index)
        .context("sound selection index is outside the observed pool")?;
    let (_, _, tabs, _) = capture(session, plan, 1).await?;
    if tabs.len() != 4 {
        // No sheet in this frame, so nothing here may be selected. An earlier pick can
        // only be proved on the editor; a sheet that is still open fails that proof.
        return selection_recovery::confirm_editor_or_chip(session, plan, candidate).await;
    }
    let fresh = observe(session, plan, frozen.maximum_visible, false).await?;
    let matches: Vec<_> = fresh
        .candidates
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            same_editor_sound_title(&row.title, &candidate.title)
                && same_editor_sound_title(&row.artist, &candidate.artist)
        })
        .map(|(row, _)| row)
        .collect();
    let [visual_index] = matches.as_slice() else {
        return Err(crate::publish_recovery::retryable_error(
            "sound_visual_identity_unmatched",
            "Ảnh màn hình không cho thấy đúng một bài nhạc trùng tên và nghệ sĩ đã chốt; chưa bấm chọn, chưa bấm Đăng",
        ));
    };
    select_row_and_close(session, plan, &fresh, *visual_index, &mut false).await?;
    selection_recovery::confirm_editor_or_chip(session, plan, candidate).await
}

pub(super) fn provisional_title_matches(observed: &str, expected: &str) -> bool {
    !observed.trim().is_empty()
        && !expected.trim().is_empty()
        && fold_sound_title(observed.trim()) == fold_sound_title(expected.trim())
}

/// Only an unbound TrendingAny row may get its authoritative spelling from
/// the editor. The visual row/artist and selected state must already be proved.
pub(super) async fn choose_provisional(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    pool: &ObservedSoundPool,
    index: usize,
    recovering: bool,
    row_proved: &mut bool,
) -> anyhow::Result<String> {
    anyhow::ensure!(
        selection_recovery::measured(plan) && pool.visual,
        "provisional sound tuple unmeasured"
    );
    let epoch = session.gui_session_epoch();
    let expected = &pool
        .candidates
        .get(index)
        .context("provisional sound row missing")?
        .title;
    anyhow::ensure!(
        pool.candidates
            .iter()
            .filter(|candidate| provisional_title_matches(&candidate.title, expected))
            .count()
            == 1,
        "provisional sound identity ambiguous"
    );
    if recovering {
        let (image, lines, tabs, current) = capture(session, plan, 1).await?;
        anyhow::ensure!(current == epoch, "provisional sound session changed");
        if tabs.len() != 4 && !*row_proved {
            // Machine29, 2026-09-30T21:00:10Z: selected Hot row was visible,
            // but OCR missed a tab. Missing sheet proof is not editor proof.
            // Keep the existing proposal and let its bounded owner retry only
            // observations; never send the uncertain row tap again.
            return Err(crate::publish_recovery::retryable_error(
                "sound_selection_observation_unknown",
                "Chưa đọc đủ màn nhạc đã chọn; chờ bằng chứng mới, không bấm chọn lại",
            ));
        }
        if tabs.len() == 4 {
            let fresh = pool_from_image(&image, &lines, &tabs, plan, pool.maximum_visible)?;
            reproof_target(pool, &fresh, index)?;
            let pixels = row_pixels(&image);
            let proof = row_proof(&image, &tabs, pool, index, &pixels);
            log_row_proof(
                session,
                RowProofFrame {
                    phase: "rowRecoveryProof",
                    generation: 1,
                    image: &image,
                    tabs: &tabs,
                    pool,
                    index,
                    pixels: &pixels,
                    proof,
                    started: Instant::now(),
                    deadline: phase_deadline(Duration::from_secs(60)),
                },
            );
            anyhow::ensure!(
                proof == RowProof::Selected,
                "provisional sound selection remains unknown; refusing another row tap"
            );
            wait_selected_row(session, plan, pool, index, None).await?;
            *row_proved = true;
            check_wait()?;
            session.back().await?;
        }
    } else {
        select_row_and_close(session, plan, pool, index, row_proved).await?;
    }
    anyhow::ensure!(
        *row_proved && session.gui_session_epoch() == epoch,
        "provisional sound row proof missing or session changed"
    );
    selection_recovery::confirm_provisional_editor(session, plan, expected, &epoch).await
}

async fn select_row_and_close(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    pool: &ObservedSoundPool,
    index: usize,
    row_proved: &mut bool,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        index < pool.candidates.len(),
        "visual sound index out of range"
    );
    let epoch = session.gui_session_epoch();
    let fresh = observe(session, plan, pool.maximum_visible, false).await?;
    anyhow::ensure!(
        session.gui_session_epoch() == epoch,
        "visual sound session changed before the row tap"
    );
    let mut tapped = None;
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
            tap_contact_image(
                session,
                plan,
                &epoch,
                target.centre(),
                SheetContact::Row(index),
            )
            .await?;
            tapped = Some(Instant::now());
        } else {
            reproof_target(pool, &fresh, index)?;
        }
    } else {
        let target = reproof_target(pool, &fresh, index)?;
        tap_contact_image(
            session,
            plan,
            &epoch,
            target.centre(),
            SheetContact::Row(index),
        )
        .await?;
        tapped = Some(Instant::now());
    }
    wait_selected_row(session, plan, &fresh, index, tapped).await?;
    *row_proved = true;
    // One reversible selection only. The fresh sheet frame that ended `wait_selected_row`
    // authorizes Back; exact title on two XML editor snapshots is the final verification.
    check_wait()?;
    session.back().await?;
    Ok(())
}

pub(super) async fn recover(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    pool: &ObservedSoundPool,
    index: usize,
) -> anyhow::Result<()> {
    let candidate = pool.candidates.get(index).context("sound index missing")?;
    let started = Instant::now();
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
        return selection_recovery::confirm_editor_or_chip(session, plan, candidate).await;
    }
    let pixels = row_pixels(&image);
    let proof = row_proof(&image, &tabs, pool, index, &pixels);
    log_row_proof(
        session,
        RowProofFrame {
            phase: "rowRecoveryProof",
            generation: 1,
            image: &image,
            tabs: &tabs,
            pool,
            index,
            pixels: &pixels,
            proof,
            started,
            deadline: phase_deadline(Duration::from_secs(60)),
        },
    );
    match proof {
        // Only a frame that positively shows no red row lets `choose` re-observe and tap; the
        // stage-wide ROW_TAP_CAP still bounds that tap.
        RowProof::NoRowRed if row_taps() < ROW_TAP_CAP => choose(session, plan, pool, index).await,
        RowProof::NoRowRed => Err(crate::publish_recovery::retryable_error(
            "sound_row_tap_cap",
            "Đã bấm hàng nhạc đủ số lần cho phép trong lượt này mà chưa chứng minh được hàng đã chọn; không bấm thêm, chưa bấm Đăng",
        )),
        RowProof::WrongRowRed | RowProof::ManyRowsRed => {
            Err(crate::publish_recovery::retryable_error(
                "sound_selection_wrong_row",
                "Ảnh màn hình cho thấy hàng nhạc khác (hoặc nhiều hàng) đang được chọn; không bấm thêm, chưa bấm Đăng",
            ))
        }
        // Our row, or a frame that cannot be judged: observe only. Nothing is tapped here; a
        // proven selection continues with Back and the editor proof.
        RowProof::Selected
        | RowProof::TabsUnreadable
        | RowProof::HotNotUnderlined
        | RowProof::Unjudgeable => {
            wait_selected_row(session, plan, pool, index, None).await?;
            check_wait()?;
            session.back().await?;
            selection_recovery::confirm_editor_or_chip(session, plan, candidate).await
        }
    }
}

/// Returns on the second fresh frame in a row that shows the measured sheet (four tabs, Hot
/// underlined) with only `index` red. That last frame is the sheet proof for the Back that
/// follows: #44 (box3150j, 2026-10-10) spent ~11 s on a further two-frame OCR proof here.
/// With `last_tap` set (this caller just tapped the row), one more contact tap is allowed once
/// HOT_TAP_SETTLE has passed since it and two consecutive fresh frames positively prove that no
/// row is red while OCR re-binds the same row within 4 px; ROW_TAP_CAP bounds the stage.
async fn wait_selected_row(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    pool: &ObservedSoundPool,
    index: usize,
    mut last_tap: Option<Instant>,
) -> anyhow::Result<()> {
    let started = Instant::now();
    let deadline = phase_deadline(Duration::from_secs(60)).min(started + Duration::from_secs(60));
    let epoch = session.gui_session_epoch();
    let mut stable = 0;
    let mut unselected = 0;
    let mut generation = 0;
    loop {
        check_wait()?;
        anyhow::ensure!(
            Instant::now() < deadline,
            "TikTok chưa xác nhận tải nhạc đã chọn; chưa bấm Đăng"
        );
        generation += 1;
        let frame_started = Instant::now();
        let (image, lines, tabs, current) = capture(session, plan, generation).await?;
        anyhow::ensure!(current == epoch, "sound selection session changed");
        let pixels = row_pixels(&image);
        let proof = row_proof(&image, &tabs, pool, index, &pixels);
        log_row_proof(
            session,
            RowProofFrame {
                phase: "rowProof",
                generation,
                image: &image,
                tabs: &tabs,
                pool,
                index,
                pixels: &pixels,
                proof,
                started,
                deadline,
            },
        );
        match proof {
            RowProof::Selected => {
                unselected = 0;
                stable += 1;
                if stable == 2 {
                    anyhow::ensure!(
                        session.gui_session_epoch() == epoch,
                        "sound selection session changed"
                    );
                    return Ok(());
                }
            }
            RowProof::NoRowRed => {
                stable = 0;
                let rebound = pool_from_image(&image, &lines, &tabs, plan, pool.maximum_visible)
                    .ok()
                    .filter(|fresh| fresh.selected_index.is_none())
                    .and_then(|fresh| {
                        let target = reproof_target(pool, &fresh, index).ok()?.clone();
                        let before = pool.targets.get(index)?;
                        ((target.x - before.x).abs() <= 4.0
                            && (target.y - before.y).abs() <= 4.0
                            && (target.width - before.width).abs() <= 4.0
                            && (target.height - before.height).abs() <= 4.0)
                            .then_some(target)
                    });
                unselected = if rebound.is_some() { unselected + 1 } else { 0 };
                if let (Some(target), Some(tapped)) = (rebound, last_tap) {
                    if unselected >= 2
                        && frame_started.saturating_duration_since(tapped) >= HOT_TAP_SETTLE
                        && row_taps() < ROW_TAP_CAP
                        && session.gui_session_epoch() == epoch
                    {
                        tap_contact_image(
                            session,
                            plan,
                            &epoch,
                            target.centre(),
                            SheetContact::Row(index),
                        )
                        .await?;
                        last_tap = Some(Instant::now());
                        unselected = 0;
                        crate::publish_recovery::note_read(
                            "sound",
                            "visualRowRetap",
                            row_taps(),
                            "awaitingProof",
                            Some("Hai ảnh mới sau khi chờ ổn định đều cho thấy chưa hàng nào được chọn; bấm lại đúng hàng đó một lần, chờ ảnh chứng minh"),
                        );
                    }
                }
            }
            RowProof::TabsUnreadable
            | RowProof::HotNotUnderlined
            | RowProof::ManyRowsRed
            | RowProof::WrongRowRed
            | RowProof::Unjudgeable => {
                stable = 0;
                unselected = 0;
            }
        }
        tokio::time::sleep(POLL).await;
    }
}

/// What one fresh sheet frame proves about the row we tapped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RowProof {
    /// Exactly one red row, and it is `index`.
    Selected,
    /// Four tabs, Hot underlined and no red at all in the list: positively nothing selected.
    NoRowRed,
    TabsUnreadable,
    HotNotUnderlined,
    ManyRowsRed,
    WrongRowRed,
    /// Red in the list that is not one framed or title-red pool row.
    Unjudgeable,
}

impl RowProof {
    fn rejection(self) -> Option<&'static str> {
        match self {
            Self::Selected => None,
            Self::NoRowRed => Some("zero_red_rows"),
            Self::TabsUnreadable => Some("tabs_not_4"),
            Self::HotNotUnderlined => Some("hot_not_underlined"),
            Self::ManyRowsRed => Some("many_red_rows"),
            Self::WrongRowRed => Some("wrong_row_red"),
            Self::Unjudgeable => Some("red_unjudgeable"),
        }
    }
}

/// Selection inserts an equalizer and trim controls and shifts the red title right, clipping
/// even a title previously read in full; on measured 45.7.3 the red title can disappear from
/// OCR entirely. The selected row is found by its own pixels in this frame (red thumbnail
/// frame) and mapped to the pool by the vertical centre of the target we tapped; a red title at
/// the pre-tap bounds is accepted alone only when no frame is drawn. A unique red row only
/// acknowledges our pick: the exact full title on two editor XML snapshots stays mandatory.
fn row_proof(
    image: &image::RgbImage,
    tabs: &[OcrRect],
    pool: &ObservedSoundPool,
    index: usize,
    pixels: &RowPixels,
) -> RowProof {
    if tabs.len() != 4 {
        return RowProof::TabsUnreadable;
    }
    if !hot_selected(image, tabs) {
        return RowProof::HotNotUnderlined;
    }
    if index >= pool.targets.len() {
        return RowProof::Unjudgeable;
    }
    let red_titles: Vec<_> = pool
        .targets
        .iter()
        .enumerate()
        .filter(|(_, target)| red_fraction(image, target_rect(target)) > 0.5)
        .map(|(row, _)| row)
        .collect();
    if pixels.framed.len() > 1 || red_titles.len() > 1 {
        return RowProof::ManyRowsRed;
    }
    if let [span] = pixels.framed[..] {
        return match targets_in_span(&pool.targets, span)[..] {
            [row] if row == index && red_titles.iter().all(|title| *title == index) => {
                RowProof::Selected
            }
            [row] if row == index => RowProof::ManyRowsRed,
            [_] => RowProof::WrongRowRed,
            _ => RowProof::Unjudgeable,
        };
    }
    match red_titles[..] {
        [row] if row == index => RowProof::Selected,
        [_] => RowProof::WrongRowRed,
        _ if !pixels.any_red => RowProof::NoRowRed,
        _ => RowProof::Unjudgeable,
    }
}

struct RowProofFrame<'a> {
    phase: &'static str,
    generation: u64,
    image: &'a image::RgbImage,
    tabs: &'a [OcrRect],
    pool: &'a ObservedSoundPool,
    index: usize,
    pixels: &'a RowPixels,
    proof: RowProof,
    started: Instant,
    deadline: Instant,
}

/// The row proof runs outside `observe_loop`'s diagnostic scope; phone 44 (box3150k,
/// 2026-10-10 13:15:13-13:16:18) logged nothing for 65 s. One line per frame, coordinates and
/// numbers only: no OCR text.
fn log_row_proof(session: &dyn UiSession, frame: RowProofFrame<'_>) {
    let now = Instant::now();
    let millis = |duration: Duration| duration.as_millis().min(u64::MAX as u128) as u64;
    let remaining_effective_budget_ms = SOUND_BUDGET
        .try_with(|budget| millis(budget.deadline.saturating_duration_since(now)))
        .ok();
    let targets: Vec<[u32; 4]> = frame
        .pool
        .targets
        .iter()
        .map(|target| {
            let rect = target_rect(target);
            [rect.x, rect.y, rect.width, rect.height]
        })
        .collect();
    // Per-mille of red among red-or-dark pixels in each pre-tap target box.
    let red_permille: Vec<u32> = frame
        .pool
        .targets
        .iter()
        .map(|target| (red_fraction(frame.image, target_rect(target)) * 1000.0).round() as u32)
        .collect();
    let tab_bounds: Vec<[u32; 4]> = frame
        .tabs
        .iter()
        .take(4)
        .map(|rect| [rect.x, rect.y, rect.width, rect.height])
        .collect();
    tracing::info!(
        serial=?session.gui_scope().map(|scope| scope.device_id),
        session=%session.gui_session_epoch(), phase=frame.phase,
        captureGeneration=frame.generation, rowIndex=frame.index,
        tabCount=frame.tabs.len(), tabBounds=?tab_bounds,
        selectedTabIndex=?selected_tab(frame.image, frame.tabs),
        hotSelected=hot_selected(frame.image, frame.tabs),
        targets=?targets, redPermille=?red_permille,
        framedRows=?frame.pixels.framed, anyRed=frame.pixels.any_red,
        proof=?frame.proof, rejection=?frame.proof.rejection(),
        rowTaps=row_taps(), rowTapCap=ROW_TAP_CAP,
        elapsedMs=millis(now.saturating_duration_since(frame.started)),
        remainingPhaseMs=millis(frame.deadline.saturating_duration_since(now)),
        remainingEffectiveBudgetMs=?remaining_effective_budget_ms,
        "visual sound observation diagnostic"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_automation::{GuiReasoner, GuiRequest, GuiResponse, OcrResponse};
    use std::sync::atomic::AtomicUsize;

    fn selected_row_ready(
        image: &image::RgbImage,
        tabs: &[OcrRect],
        pool: &ObservedSoundPool,
        index: usize,
    ) -> bool {
        row_proof(image, tabs, pool, index, &row_pixels(image)) == RowProof::Selected
    }
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
        let mut ambiguous = lines;
        ambiguous[2].text = "Thuong Nhau Den The".into();
        assert!(pool_from_image(&img, &ambiguous, &tabs, plan, 5).is_err());
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

    struct Ocr {
        accent_loss: bool,
        canonical_probe: bool,
        missing_tab: bool,
    }
    #[async_trait::async_trait]
    impl GuiReasoner for Ocr {
        async fn resolve(&self, _: GuiRequest) -> anyhow::Result<GuiResponse> {
            unreachable!()
        }
        async fn ocr(&self, r: OcrRequest) -> anyhow::Result<OcrResponse> {
            let (_, mut rows, tabs, _) = fixture();
            if self.canonical_probe || (self.accent_loss && r.languages == ["vi", "en"]) {
                rows[0].text = "Thuong Nhau Den The".into();
            }
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
            if self.missing_tab {
                lines.retain(|line| line.text != "Recent");
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
    struct Session {
        taps: AtomicUsize,
        backs: AtomicUsize,
        reads: AtomicUsize,
        wrong_title: bool,
        selection_reads: AtomicUsize,
        loading_frames: usize,
        initially_selected: bool,
        accent_loss: bool,
        canonical_probe: bool,
        lose_selection_ack: bool,
        next_capture_missing_tab: AtomicBool,
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
            Some(Arc::new(Ocr {
                accent_loss: self.accent_loss,
                canonical_probe: self.canonical_probe,
                missing_tab: self.next_capture_missing_tab.swap(false, Ordering::Relaxed),
            }))
        }
        async fn active_app_bundle(&self) -> anyhow::Result<String> {
            Ok("com.zhiliaoapp.musically".into())
        }
        // The fixture frames are 1080x2220; the contact tap proves the screen matches.
        async fn window_size(&self) -> anyhow::Result<(f64, f64)> {
            Ok((1080.0, 2220.0))
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
            anyhow::ensure!(!self.lose_selection_ack, "fixture lost sound selection ACK");
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
        for (wrong_title, accent_loss, expected_success) in [
            (false, false, true),
            (true, false, false),
            (false, true, true),
        ] {
            let s = Session {
                taps: AtomicUsize::new(0),
                backs: AtomicUsize::new(0),
                reads: AtomicUsize::new(0),
                wrong_title,
                selection_reads: AtomicUsize::new(0),
                loading_frames: 0,
                initially_selected: false,
                accent_loss,
                canonical_probe: false,
                lose_selection_ack: false,
                next_capture_missing_tab: AtomicBool::new(false),
            };
            let plan = fixture().3;
            let pool = observe(&s, plan, 5, false).await.unwrap();
            let result = choose(&s, plan, &pool, 0).await;
            assert_eq!(result.is_ok(), expected_success);
            assert_eq!(s.taps.load(Ordering::Relaxed), 1);
            assert_eq!(s.backs.load(Ordering::Relaxed), 1);
            assert!(s.reads.load(Ordering::Relaxed) >= 2);
        }
    }
    #[tokio::test(start_paused = true)]
    async fn provisional_visual_choice_reads_canonical_editor_spelling_without_second_pick() {
        for (wrong_title, lost_ack, missing_recovery_tab) in [
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (false, true, true),
        ] {
            let session = Session {
                taps: AtomicUsize::new(0),
                backs: AtomicUsize::new(0),
                reads: AtomicUsize::new(0),
                wrong_title,
                selection_reads: AtomicUsize::new(0),
                loading_frames: 0,
                initially_selected: false,
                accent_loss: true,
                canonical_probe: true,
                lose_selection_ack: lost_ack,
                next_capture_missing_tab: AtomicBool::new(false),
            };
            let plan = fixture().3;
            let pool = observe(&session, plan, 5, false).await.unwrap();
            assert_eq!(pool.candidates[0].title, "Thuong Nhau Den The");
            let mut row_proved = false;
            let first = choose_provisional(&session, plan, &pool, 0, false, &mut row_proved).await;
            let result = if lost_ack {
                assert!(first.is_err());
                assert!(!row_proved);
                if missing_recovery_tab {
                    session
                        .next_capture_missing_tab
                        .store(true, Ordering::Relaxed);
                    let unknown =
                        choose_provisional(&session, plan, &pool, 0, true, &mut row_proved)
                            .await
                            .expect_err(
                                "one missing OCR tab cannot prove the editor or selected row",
                            );
                    let failure = crate::publish_recovery::describe(&unknown);
                    assert_eq!(
                        failure.kind,
                        crate::publish_recovery::FailureKind::Retryable,
                        "partial recovery must retain observation-only retry: {unknown:#}"
                    );
                    assert_eq!(failure.code, "sound_selection_observation_unknown");
                    assert!(!row_proved);
                    assert_eq!(session.taps.load(Ordering::Relaxed), 1);
                    assert_eq!(session.backs.load(Ordering::Relaxed), 0);
                    assert_eq!(session.reads.load(Ordering::Relaxed), 0);
                }
                choose_provisional(&session, plan, &pool, 0, true, &mut row_proved).await
            } else {
                first
            };
            if wrong_title {
                assert!(result.is_err(), "different words cannot be canonicalized");
            } else {
                assert_eq!(result.unwrap(), "Thương Nhau Đến Thế");
            }
            assert!(row_proved);
            assert_eq!(session.taps.load(Ordering::Relaxed), 1);
            assert_eq!(session.backs.load(Ordering::Relaxed), 1);
            assert!(session.reads.load(Ordering::Relaxed) >= 2);
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
            accent_loss: false,
            canonical_probe: false,
            lose_selection_ack: false,
            next_capture_missing_tab: AtomicBool::new(false),
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
            accent_loss: false,
            canonical_probe: false,
            lose_selection_ack: false,
            next_capture_missing_tab: AtomicBool::new(false),
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
            accent_loss: false,
            canonical_probe: false,
            lose_selection_ack: false,
            next_capture_missing_tab: AtomicBool::new(false),
        };
        let plan = fixture().3;
        let pool = observe(&session, plan, 5, false).await.unwrap();
        assert_eq!(pool.selected_index, Some(0));

        choose(&session, plan, &pool, 0).await.unwrap();

        assert_eq!(session.taps.load(Ordering::Relaxed), 0);
        assert_eq!(session.backs.load(Ordering::Relaxed), 1);
    }
}
