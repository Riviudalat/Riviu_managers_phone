//! Recover navigation on the measured inline sheet using fresh local OCR.
//! XML still proves the selected tab/rows or the exact editor title afterwards.
//! This path cannot choose a sound or issue a public action.
use super::*;
use crate::ui_automation::{OcrImage, OcrRect, OcrRequest, OcrResponse};
use base64::Engine;
use sha2::{Digest, Sha256};

async fn bounded<T>(
    deadline: Instant,
    work: impl std::future::Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    read_sound(async {
        tokio::time::timeout_at(deadline, work).await.map_err(|_| {
            crate::publish_recovery::retryable_error(
                "sound_load_timeout",
                "TikTok sound recovery observation timed out; no Post was sent",
            )
        })?
    })
    .await
}

pub(super) fn measured(plan: SoundPickerPlan) -> bool {
    !plan.dynamic
        && MEASURED_SOUND_PICKERS.iter().any(|p| {
            p.version == "45.7.3" && p.language == "en" && p.plan.provenance == plan.provenance
        })
}

pub(super) fn tabs(response: &OcrResponse) -> anyhow::Result<Vec<OcrRect>> {
    let mut found = Vec::new();
    for label in ["Hot", "For You", "Favorites", "Recent"] {
        let matches: Vec<_> = response
            .lines
            .iter()
            .filter(|line| line.text == label && line.confidence >= 0.9)
            .collect();
        let [line] = matches.as_slice() else {
            anyhow::bail!("sound recovery tab missing or ambiguous");
        };
        found.push(line.bounds);
    }
    anyhow::ensure!(
        found.windows(2).all(|pair| {
            pair[0].x + pair[0].width < pair[1].x && pair[0].y.abs_diff(pair[1].y) <= 15
        }),
        "sound recovery tab geometry changed"
    );
    Ok(found)
}

pub(super) async fn prove_sheet(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
) -> anyhow::Result<crate::TapPoint> {
    observe_rendered_sheet(session, plan, false).await
}

/// Do not repeatedly invalidate Android's accessibility cache while the sheet
/// is still rendering placeholders. This only waits; XML owns every decision.
#[cfg(test)]
pub(super) async fn wait_for_rows(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
) -> anyhow::Result<()> {
    if measured(plan) && session.gui_reasoner().is_some() {
        observe_rendered_sheet(session, plan, true).await?;
    }
    Ok(())
}

async fn observe_rendered_sheet(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    require_rows: bool,
) -> anyhow::Result<crate::TapPoint> {
    let reasoner = session
        .gui_reasoner()
        .context("sound recovery local OCR unavailable")?;
    let window = Duration::from_secs(if require_rows { 60 } else { 30 });
    let deadline = phase_deadline(window).min(Instant::now() + window);
    let epoch = session.gui_session_epoch();
    anyhow::ensure!(!epoch.is_empty(), "sound recovery session missing");
    let mut previous: Option<Vec<OcrRect>> = None;
    let mut generation = 0;
    loop {
        generation += 1;
        check_wait()?;
        anyhow::ensure!(
            session.gui_session_epoch() == epoch,
            "sound recovery session replaced"
        );
        anyhow::ensure!(
            read_sound(session.active_app_bundle()).await? == plan.package,
            "sound recovery app changed"
        );
        let png = bounded(deadline, session.screenshot_png()).await?;
        anyhow::ensure!(
            png.len() <= 8 * 1024 * 1024,
            "sound recovery frame too large"
        );
        let (width, height) = image::ImageReader::new(std::io::Cursor::new(&png))
            .with_guessed_format()?
            .into_dimensions()?;
        // Only the real Samsung 45.7.3/en geometry measured in this recovery.
        anyhow::ensure!(
            (width, height) == (1080, 2220),
            "sound recovery geometry unmeasured"
        );
        let remaining_ms = deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(15_000) as u64;
        anyhow::ensure!(remaining_ms > 0, "sound recovery observation expired");
        let request = OcrRequest {
            protocol_version: 1,
            request_id: uuid::Uuid::new_v4().to_string(),
            observation_id: uuid::Uuid::new_v4().to_string(),
            session_epoch: epoch.clone(),
            generation,
            remaining_ms,
            screenshot: OcrImage {
                sha256: format!("{:x}", Sha256::digest(&png)),
                bytes_base64: base64::engine::general_purpose::STANDARD.encode(png),
                width,
                height,
            },
            roi: Some(OcrRect {
                x: 0,
                y: height * 2 / 5,
                width,
                height: if require_rows {
                    height * 9 / 20
                } else {
                    height / 4
                },
            }),
            languages: if require_rows {
                vec!["vi".into(), "en".into()]
            } else {
                vec!["en".into()]
            },
            min_confidence: 0.9,
        };
        let response = bounded(deadline, reasoner.ocr(request.clone())).await?;
        response.validate_binding(&request)?;
        let observed = tabs(&response);
        let rows_ready = !require_rows
            || response
                .lines
                .iter()
                .filter(|line| {
                    line.bounds.y >= 1300
                        && line.bounds.x >= 200
                        && line.bounds.width >= 120
                        && line.bounds.height >= 20
                        && line.confidence >= 0.9
                })
                .count()
                >= 4;
        if observed.is_err() || !rows_ready {
            previous = None;
            if Instant::now() + POLL >= deadline {
                if let Err(error) = observed {
                    return Err(error);
                }
                anyhow::bail!("sound rows did not finish rendering");
            }
            tokio::time::sleep(POLL).await;
            continue;
        }
        let observed = observed?;
        check_wait()?;
        anyhow::ensure!(
            Instant::now() < deadline && session.gui_session_epoch() == epoch,
            "sound recovery stale observation"
        );
        if let Some(before) = &previous {
            anyhow::ensure!(
                before
                    .iter()
                    .zip(&observed)
                    .all(|(a, b)| a.x.abs_diff(b.x) <= 4
                        && a.y.abs_diff(b.y) <= 4
                        && a.width.abs_diff(b.width) <= 4
                        && a.height.abs_diff(b.height) <= 4),
                "sound recovery sheet moved"
            );
            previous = Some(observed);
            break;
        }
        previous = Some(observed);
        tokio::time::sleep(POLL).await;
    }
    anyhow::ensure!(
        read_sound(session.active_app_bundle()).await? == plan.package
            && session.gui_session_epoch() == epoch,
        "sound recovery app or session changed"
    );
    check_wait()?;
    let hot = previous.context("sound recovery observations missing")?[0];
    Ok(crate::TapPoint {
        x: f64::from(hot.x) + f64::from(hot.width) / 2.0,
        y: f64::from(hot.y) + f64::from(hot.height) / 2.0,
    })
}

pub(super) async fn confirm_editor(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    expected: &str,
) -> anyhow::Result<()> {
    confirm_editor_observed(session, plan, expected, None, false)
        .await
        .map(|_| ())
}

pub(super) async fn confirm_editor_in_epoch(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    expected: &str,
    epoch: &str,
) -> anyhow::Result<()> {
    confirm_editor_observed(session, plan, expected, Some(epoch), false)
        .await
        .map(|_| ())
}

pub(super) async fn confirm_provisional_editor(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    expected: &str,
    epoch: &str,
) -> anyhow::Result<String> {
    anyhow::ensure!(measured(plan), "provisional sound tuple unmeasured");
    confirm_editor_observed(session, plan, expected, Some(epoch), true).await
}

async fn confirm_editor_observed(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    expected: &str,
    pinned_epoch: Option<&str>,
    provisional: bool,
) -> anyhow::Result<String> {
    let deadline =
        phase_deadline(Duration::from_secs(30)).min(Instant::now() + Duration::from_secs(30));
    let mut previous: Option<(String, u64, String)> = None;
    let mut read_any = false;
    while Instant::now() < deadline {
        check_wait()?;
        let epoch = session.gui_session_epoch();
        if let Some(pinned) = pinned_epoch {
            anyhow::ensure!(
                epoch == pinned && read_sound(session.active_app_bundle()).await? == plan.package,
                "sound recovery app/session changed again"
            );
        }
        let source = match bounded(deadline, session.hierarchy_source_snapshot()).await {
            Ok(source) => {
                note_readable_root();
                source
            }
            Err(error) if transient_sound_read(&error) => {
                if unreadable_root(&error) {
                    note_unreadable_root();
                }
                previous = None;
                tokio::time::sleep(POLL).await;
                continue;
            }
            Err(error) => return Err(error),
        };
        read_any = true;
        check_wait()?;
        if epoch.is_empty() || session.gui_session_epoch() != epoch || source.generation == 0 {
            previous = None;
            continue;
        }
        let tree = crate::ui_automation::tree::Tree::parse(source)?;
        let title = tree.matching(
            plan.package,
            ElementQuery::ResourceIdSuffix(plan.current_title_id),
        );
        let observed_title = match title.as_slice() {
            [index] => tree.nodes[*index].attr("text"),
            _ => "",
        };
        let valid = if provisional {
            visual::provisional_title_matches(observed_title, expected)
        } else {
            same_editor_sound_title(observed_title, expected)
        };
        let sheet = plan.snapshot_layout().is_some_and(|layout| {
            !tree
                .matching(plan.package, ElementQuery::ResourceIdSuffix(layout.tab_id))
                .is_empty()
        }) || (recent::measured(plan)
            && [":id/q_g", plan.row_id].iter().any(|id| {
                !tree
                    .matching(plan.package, ElementQuery::ResourceIdSuffix(id))
                    .is_empty()
            }));
        let editor_next = !recent::measured(plan)
            || matches!(tree.matching(plan.package,ElementQuery::ResourceIdSuffix(":id/kl_")).as_slice(),[i] if tree.nodes[*i].attr("text")=="Next");
        if valid && !sheet && editor_next {
            if previous.as_ref().is_some_and(|(p, g, prior_title)| {
                p == &epoch
                    && tree.generation > *g
                    && same_editor_sound_title(prior_title, observed_title)
            }) {
                anyhow::ensure!(
                    read_sound(session.active_app_bundle()).await? == plan.package,
                    "sound recovery app changed"
                );
                check_wait()?;
                anyhow::ensure!(
                    session.gui_session_epoch() == epoch,
                    "sound recovery session changed after editor proof"
                );
                return Ok(observed_title.nfc().collect());
            }
            previous = Some((epoch, tree.generation, observed_title.into()));
        } else {
            previous = None;
        }
        tokio::time::sleep(POLL).await;
    }
    let unconfirmed =
        anyhow::anyhow!("selected sound was not confirmed on two fresh editor snapshots");
    Err(if read_any {
        unconfirmed
    } else {
        unconfirmed.context(EditorRootUnreadable)
    })
}

/// XML editor proof, or, only when no editor read returned a tree at all, two fresh frames
/// whose chip OCR equals the frozen title. That stands in for the editor chip here only:
/// the final sound reproof before Post still reads the chip from XML.
pub(super) async fn confirm_editor_or_chip(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    candidate: &SoundCandidate,
) -> anyhow::Result<()> {
    match confirm_editor(session, plan, &candidate.title).await {
        Ok(()) => {
            // #44/#47: the editor reads fine on XML while the sheet is blind. This proof,
            // not any OCR row or tab, is what marks the selection confirmed.
            crate::publish_recovery::note_read(
                "sound",
                "editorProof",
                unreadable_roots(),
                "xml",
                Some(&format!(
                    "editorProof: true; hai snapshot XML mới của màn chỉnh sửa cùng tên nhạc '{}' của '{}' sau khi chọn bằng ảnh màn hình",
                    candidate.title, candidate.artist
                )),
            );
            Ok(())
        }
        Err(error) if error.is::<EditorRootUnreadable>() && session.gui_reasoner().is_some() => {
            prove_editor_chip(session, plan, &candidate.title)
                .await
                .with_context(|| format!("{error:#}"))?;
            crate::publish_recovery::note_read(
                "sound",
                "editorProof",
                unreadable_roots(),
                "ocrChip",
                Some(&format!(
                    "Màn chỉnh sửa không đọc được; ảnh màn hình cho thấy nhạc '{}' của '{}'. Tên nhạc vẫn được kiểm lại trước khi Đăng",
                    candidate.title, candidate.artist
                )),
            );
            Ok(())
        }
        result => result,
    }
}

// Editor chip `tv_top_text` on Global 45.7.3/en at 1080x2220: [362,100][698,216] on Riviu #44
// (SM-G955F ce031713d903001b01, real-20261009-editor.xml) and [398,100][662,216] on
// ce031713b0c610ab0c (15-readback.xml). The title is centred in that band.
const CHIP_LEFT: u32 = 100;
const CHIP_RIGHT: u32 = 980;
const CHIP_TOP: u32 = 90;
const CHIP_BOTTOM: u32 = 230;
// Sheet tabs read at y=1129 on hot-visual.png (same device class); prove_sheet scans 888..1443.
const SHEET_TABS_TOP: u32 = 888;
const OCR_BOTTOM: u32 = 1443;

async fn prove_editor_chip(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    expected: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(measured(plan), "sound editor chip OCR unmeasured");
    let reasoner = session
        .gui_reasoner()
        .context("sound editor chip local OCR unavailable")?;
    let window = Duration::from_secs(30);
    let deadline = phase_deadline(window).min(Instant::now() + window);
    let epoch = session.gui_session_epoch();
    anyhow::ensure!(!epoch.is_empty(), "sound editor chip session missing");
    let mut matched = 0;
    let mut generation = 0;
    while Instant::now() < deadline {
        generation += 1;
        check_wait()?;
        anyhow::ensure!(
            session.gui_session_epoch() == epoch
                && read_sound(session.active_app_bundle()).await? == plan.package,
            "sound editor chip app/session changed"
        );
        let png = bounded(deadline, session.screenshot_png()).await?;
        anyhow::ensure!(
            png.len() <= 8 * 1024 * 1024,
            "sound editor chip frame too large"
        );
        let (width, height) = image::ImageReader::new(std::io::Cursor::new(&png))
            .with_guessed_format()?
            .into_dimensions()?;
        anyhow::ensure!(
            (width, height) == (1080, 2220),
            "sound editor chip geometry unmeasured"
        );
        let remaining_ms = deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(15_000) as u64;
        anyhow::ensure!(remaining_ms > 0, "sound editor chip observation expired");
        let request = OcrRequest {
            protocol_version: 1,
            request_id: uuid::Uuid::new_v4().to_string(),
            observation_id: uuid::Uuid::new_v4().to_string(),
            session_epoch: epoch.clone(),
            generation,
            remaining_ms,
            screenshot: OcrImage {
                sha256: format!("{:x}", Sha256::digest(&png)),
                bytes_base64: base64::engine::general_purpose::STANDARD.encode(png),
                width,
                height,
            },
            roi: Some(OcrRect {
                x: 0,
                y: 0,
                width,
                height: OCR_BOTTOM,
            }),
            languages: vec!["vi".into(), "en".into()],
            min_confidence: 0.9,
        };
        let response = bounded(deadline, reasoner.ocr(request.clone())).await?;
        response.validate_binding(&request)?;
        // Any tab label in the sheet band means the picker may still cover the editor.
        let sheet = response.lines.iter().any(|line| {
            line.bounds.y >= SHEET_TABS_TOP
                && ["Hot", "For You", "Favorites", "Recent"].contains(&line.text.trim())
        });
        let chip: Vec<_> = response
            .lines
            .iter()
            .filter(|line| {
                line.bounds.x >= CHIP_LEFT
                    && line.bounds.x + line.bounds.width <= CHIP_RIGHT
                    && line.bounds.y >= CHIP_TOP
                    && line.bounds.y + line.bounds.height <= CHIP_BOTTOM
            })
            .collect();
        let shown = !sheet
            && matches!(chip.as_slice(), [line]
                if line.confidence >= 0.9 && same_editor_sound_title(&line.text, expected));
        check_wait()?;
        anyhow::ensure!(
            session.gui_session_epoch() == epoch,
            "sound editor chip session changed"
        );
        matched = if shown { matched + 1 } else { 0 };
        if matched == 2 {
            anyhow::ensure!(
                read_sound(session.active_app_bundle()).await? == plan.package
                    && session.gui_session_epoch() == epoch,
                "sound editor chip app/session changed"
            );
            check_wait()?;
            return Ok(());
        }
        tokio::time::sleep(POLL).await;
    }
    Err(crate::publish_recovery::retryable_error(
        "sound_editor_chip_unconfirmed",
        "Ảnh màn hình chưa cho thấy đúng tên nhạc đã chọn trên màn chỉnh sửa; chưa bấm Đăng",
    ))
}
