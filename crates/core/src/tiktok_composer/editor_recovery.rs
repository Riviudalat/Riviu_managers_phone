//! A failed element/rect read grants observation, never another picker tap.
use super::*;
use crate::ui_automation::{OcrImage, OcrRect, OcrRequest};
use base64::Engine;
use sha2::{Digest, Sha256};

/// Let the measured editor finish rendering before invalidating the Android
/// accessibility cache. Two independent frames prove this measured editor;
/// loading the suggested sound is a separate state proved by the sound adapter.
pub(super) async fn wait_rendered(
    session: &dyn UiSession,
    deadline: Instant,
    stop: &AtomicBool,
) -> anyhow::Result<bool> {
    use crate::ui_automation::runtime::{read_before_deadline, ReadWaitResult};
    let reasoner = session
        .gui_reasoner()
        .context("editor local OCR unavailable")?;
    let epoch = session.gui_session_epoch();
    anyhow::ensure!(!epoch.is_empty(), "editor render session missing");
    let mut previous = None;
    let mut generation = 0;
    while Instant::now() < deadline {
        if stop.load(Ordering::Relaxed) {
            return Ok(false);
        }
        generation += 1;
        let work = async {
            anyhow::ensure!(
                session.gui_session_epoch() == epoch
                    && session.active_app_bundle().await? == "com.zhiliaoapp.musically",
                "editor render app/session changed"
            );
            let png = session.screenshot_png().await?;
            let dimensions = image::ImageReader::new(std::io::Cursor::new(&png))
                .with_guessed_format()?
                .into_dimensions()?;
            anyhow::ensure!(
                dimensions == (1080, 2220),
                "editor render geometry unmeasured"
            );
            let request = OcrRequest {
                protocol_version: 1,
                request_id: uuid::Uuid::new_v4().to_string(),
                observation_id: uuid::Uuid::new_v4().to_string(),
                session_epoch: epoch.clone(),
                generation,
                remaining_ms: deadline
                    .saturating_duration_since(Instant::now())
                    .as_millis()
                    .min(15000) as u64,
                screenshot: OcrImage {
                    bytes_base64: base64::engine::general_purpose::STANDARD.encode(&png),
                    sha256: format!("{:x}", Sha256::digest(&png)),
                    width: 1080,
                    height: 2220,
                },
                // Both native photo and video editors have these measured
                // bottom controls. Whole-screen OCR missed them on a black
                // photo preview; this region returned both at >95% confidence.
                roi: Some(OcrRect {
                    x: 0,
                    y: 1880,
                    width: 1080,
                    height: 214,
                }),
                languages: vec!["en".into()],
                min_confidence: 0.9,
            };
            let mut response = reasoner
                .ocr(request.clone())
                .await
                .map_err(crate::tiktok_sound::OcrReadUnavailable)?;
            response.validate_binding(&request)?;
            // Global45.7.3/en machine29, 2026-09-29T21:39:16Z, 1080x2220:
            // gesture navigation puts Next at y2078 (bottom2109), below the
            // original crop. Whole union crop missed it; this measured region
            // reads Next at confidence0.9587. Only observe the same frame.
            if !response.lines.iter().any(|line| line.text == "Next") {
                let lower_request = OcrRequest {
                    request_id: uuid::Uuid::new_v4().to_string(),
                    roi: Some(OcrRect {
                        x: 0,
                        y: 1980,
                        width: 1080,
                        height: 214,
                    }),
                    ..request.clone()
                };
                let lower = reasoner
                    .ocr(lower_request.clone())
                    .await
                    .map_err(crate::tiktok_sound::OcrReadUnavailable)?;
                lower.validate_binding(&lower_request)?;
                response.lines.extend(lower.lines);
            }
            if !response.lines.iter().any(|line| line.text == "Your Story") {
                let story_request = OcrRequest {
                    request_id: uuid::Uuid::new_v4().to_string(),
                    roi: Some(OcrRect {
                        x: 210,
                        y: 1940,
                        width: 300,
                        // Includes the measured button-navigation and gesture-
                        // navigation labels; same-frame OCR still requires exact text.
                        height: 200,
                    }),
                    ..request
                };
                let story = reasoner
                    .ocr(story_request.clone())
                    .await
                    .map_err(crate::tiktok_sound::OcrReadUnavailable)?;
                story.validate_binding(&story_request)?;
                response.lines.extend(story.lines);
            }
            anyhow::ensure!(
                session.gui_session_epoch() == epoch,
                "editor render session replaced"
            );
            Ok::<_, anyhow::Error>(response)
        };
        let response = match read_before_deadline(work, deadline, stop).await? {
            ReadWaitResult::Ready(response) => response,
            ReadWaitResult::Cancelled | ReadWaitResult::DeadlineExceeded => return Ok(false),
        };
        if stop.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let next: Vec<_> = response
            .lines
            .iter()
            .filter(|l| l.text == "Next" && l.bounds.x > 540 && l.bounds.y > 1900)
            .collect();
        let story = response
            .lines
            .iter()
            .any(|l| l.text == "Your Story" && l.bounds.x < 540 && l.bounds.y > 1900);
        if let [next] = next.as_slice() {
            if story {
                if previous == Some(next.bounds) {
                    return Ok(true);
                }
                previous = Some(next.bounds);
            } else {
                previous = None;
            }
        } else {
            previous = None;
        }
        sleep(
            POLL.min(deadline.saturating_duration_since(Instant::now())),
            stop,
        )
        .await;
    }
    Ok(false)
}

// Two successful XML reads on the measured 45.7.3 editor took about 8 seconds
// each. This is one total phase budget including the initial element read.
pub(super) const RECOVERY_WINDOW: Duration = Duration::from_secs(30);

pub(super) fn transient_read(error: &anyhow::Error) -> bool {
    matches!(
        crate::driver::classify_read_failure(error),
        crate::driver::ReadFailureKind::Transient | crate::driver::ReadFailureKind::Unavailable
    )
}

pub(super) async fn observe(
    session: &dyn UiSession,
    package: &str,
    marker: ElementQuery<'_>,
    deadline: Instant,
    stop: &AtomicBool,
) -> anyhow::Result<bool> {
    use crate::ui_automation::runtime::{read_before_deadline, ReadWaitResult};
    let epoch = session.gui_session_epoch();
    let mut previous: Option<(String, u64, ElementBox)> = None;
    while Instant::now() < deadline {
        if stop.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let snapshot = match read_before_deadline(
            async {
                anyhow::ensure!(
                    !epoch.is_empty()
                        && session.gui_session_epoch() == epoch
                        && session.active_app_bundle().await? == package,
                    "editor recovery app/session changed"
                );
                session.hierarchy_source_snapshot().await
            },
            deadline,
            stop,
        )
        .await
        {
            Ok(ReadWaitResult::Ready(snapshot)) => snapshot,
            Err(error) if transient_read(&error) => {
                if stop.load(Ordering::Relaxed) || Instant::now() >= deadline {
                    return Ok(false);
                }
                let current_epoch = session.gui_session_epoch();
                if current_epoch != epoch {
                    // The failed legacy read may have recreated the driver session.
                    // Preserve that unavailable read, never reuse the prior target or
                    // claim the driver's stronger fresh-foreground repair proof.
                    let message = format!("Editor observation invalidated by an unavailable read; previous epoch={epoch}, current epoch={current_epoch}: {error:#}");
                    return Err(error.context(crate::publish_recovery::RecoveryFailure::new(
                        "editor_observation_invalidated",
                        crate::publish_recovery::FailureKind::Retryable,
                        message,
                    )));
                }
                previous = None;
                sleep(
                    POLL.min(deadline.saturating_duration_since(Instant::now())),
                    stop,
                )
                .await;
                continue;
            }
            Err(error) => return Err(error),
            Ok(ReadWaitResult::Cancelled | ReadWaitResult::DeadlineExceeded) => return Ok(false),
        };
        if stop.load(Ordering::Relaxed) || Instant::now() >= deadline {
            return Ok(false);
        }
        let current_epoch = session.gui_session_epoch();
        if epoch != current_epoch {
            return Err(anyhow::anyhow!("editor recovery session replaced")
                .context(crate::publish_recovery::RecoveryFailure::new(
                    "editor_observation_invalidated",
                    crate::publish_recovery::FailureKind::Retryable,
                    format!("Editor read generation {} changed session; previous epoch={epoch}, current epoch={current_epoch}; discard prior targets and prepare afresh", snapshot.generation),
                )));
        }
        if snapshot.generation == 0 {
            previous = None;
            sleep(
                POLL.min(deadline.saturating_duration_since(Instant::now())),
                stop,
            )
            .await;
            continue;
        }
        let tree = crate::ui_automation::tree::Tree::parse(snapshot)?;
        let loading = tree.nodes.iter().enumerate().any(|(index, node)| {
            node.visible(package)
                && tree.ancestors_visible(index)
                && [node.attr("text"), node.attr("content-desc")]
                    .iter()
                    .any(|label| matches!(*label, "Loading" | "Loading..." | "Loading…"))
        });
        let matches = if let ElementQuery::Semantic(role) = marker {
            crate::app_automation::tiktok_roles::indices(&tree, package, role)
        } else {
            tree.matching(package, marker)
        };
        let target = match matches.as_slice() {
            [index] if !loading => tree.nodes[*index].rect().filter(|rect| rect.enabled),
            _ => None,
        };
        if let Some(target) = target {
            if previous
                .as_ref()
                .is_some_and(|(old_epoch, generation, rect)| {
                    old_epoch == &epoch && tree.generation > *generation && rect == &target
                })
            {
                return Ok(!stop.load(Ordering::Relaxed));
            }
            previous = Some((epoch.clone(), tree.generation, target));
        } else {
            previous = None;
        }
        sleep(
            POLL.min(deadline.saturating_duration_since(Instant::now())),
            stop,
        )
        .await;
    }
    Ok(false)
}
