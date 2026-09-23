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
        tokio::time::timeout_at(deadline, work)
            .await
            .context("sound recovery deadline")?
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
    let deadline =
        phase_deadline(Duration::from_secs(30)).min(Instant::now() + Duration::from_secs(30));
    let mut previous: Option<(String, u64)> = None;
    while Instant::now() < deadline {
        check_wait()?;
        let epoch = session.gui_session_epoch();
        let source = match bounded(deadline, session.hierarchy_source_snapshot()).await {
            Ok(source) => source,
            Err(error) if transient_sound_read(&error) => {
                previous = None;
                tokio::time::sleep(POLL).await;
                continue;
            }
            Err(error) => return Err(error),
        };
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
        let valid = matches!(title.as_slice(),[index]if tree.nodes[*index].attr("text").trim()==expected.trim());
        let sheet = plan.snapshot_layout().is_some_and(|layout| {
            !tree
                .matching(plan.package, ElementQuery::ResourceIdSuffix(layout.tab_id))
                .is_empty()
        });
        if valid && !sheet {
            if previous
                .as_ref()
                .is_some_and(|(p, g)| p == &epoch && tree.generation > *g)
            {
                anyhow::ensure!(
                    read_sound(session.active_app_bundle()).await? == plan.package,
                    "sound recovery app changed"
                );
                check_wait()?;
                return Ok(());
            }
            previous = Some((epoch, tree.generation));
        } else {
            previous = None;
        }
        tokio::time::sleep(POLL).await;
    }
    anyhow::bail!("selected sound was not confirmed on two fresh editor snapshots")
}
