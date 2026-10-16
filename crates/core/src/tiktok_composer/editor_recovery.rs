//! A failed element/rect read grants observation, never another picker tap.
use super::*;
use crate::ui_automation::{OcrImage, OcrRect, OcrRequest};
use base64::Engine;
use sha2::{Digest, Sha256};

/// How long the first editor frame waits after the picker Next ACK. r134 (16/10/2026, box
/// 21-50, SM-G955F, Global 45.7.3/en): 19 of 19 frames taken 0.1-0.4 s after the ACK still
/// showed the picker, and each paid all three OCR regions with no chance of proof; 19 of 19
/// frames taken 0.7 s or later showed the editor. It delays one pixel read, never past the
/// wait's deadline, and Stop ends it.
const EDITOR_FIRST_FRAME_SETTLE: Duration = Duration::from_millis(1000);

/// How far an OCR box (Next, Your Story) may move between the two frames that prove the
/// editor. ce11171beb405c2701 (box 21-50, SM-G955F/N, Global 45.7.3/en, r131 attempt
/// 83e1064d) draws Next with an outline, and its box moved by exactly 1 px in x, y, width or
/// height between every pair of editor frames, so two equal boxes never came. OCR boxes are
/// never a tap target.
const OCR_NEXT_JITTER_PX: u32 = 1;

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
    let mut settle =
        StageDiagnosticOperation::start("editorFirstFrameSettle", &epoch, Some(deadline));
    sleep(
        EDITOR_FIRST_FRAME_SETTLE.min(deadline.saturating_duration_since(Instant::now())),
        stop,
    )
    .await;
    settle.finish(if stop.load(Ordering::Relaxed) {
        "stopped"
    } else {
        "settled"
    });
    drop(settle);
    let mut previous: Option<(OcrRect, Vec<OcrRect>)> = None;
    let mut generation = 0;
    while Instant::now() < deadline {
        if stop.load(Ordering::Relaxed) {
            return Ok(false);
        }
        generation += 1;
        // No region read starts once the window has closed: a frame whose screencap ends in
        // the read grace is unread, not judged on part of its regions. Past the deadline a
        // request carries no budget and the service refuses it (r134 ce041714a3159a0801,
        // `gui_ocr_request_invalid` at +74 s on an editor frame).
        let window_left = || match deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(15000) as u64
        {
            0 => Err(crate::tiktok_sound::OcrReadUnavailable(anyhow::anyhow!(
                "đã hết cửa sổ chờ màn chỉnh sửa trước lần đọc OCR của khung hình"
            ))),
            remaining_ms => Ok(remaining_ms),
        };
        let mut frame = StageDiagnosticOperation::start("editorOcrFrame", &epoch, Some(deadline));
        frame.snapshot(generation);
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
                remaining_ms: window_left()?,
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
                window_left()?;
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
                window_left()?;
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
        let response = match read_before_deadline(work, deadline, stop).await {
            Ok(ReadWaitResult::Ready(response)) => response,
            Ok(ReadWaitResult::Cancelled) => {
                frame.finish("cancelled");
                return Ok(false);
            }
            Ok(ReadWaitResult::DeadlineExceeded) => {
                frame.finish("deadline");
                return Ok(false);
            }
            Err(error) => {
                frame.finish("error");
                return Err(error);
            }
        };
        if stop.load(Ordering::Relaxed) {
            frame.finish("stopped");
            return Ok(false);
        }
        let next: Vec<_> = response
            .lines
            .iter()
            .filter(|l| l.text == "Next" && l.bounds.x > 540 && l.bounds.y > 1900)
            .collect();
        let stories: Vec<_> = response
            .lines
            .iter()
            .filter(|l| l.text == "Your Story" && l.bounds.x < 540 && l.bounds.y > 1900)
            .map(|l| l.bounds)
            .collect();
        let near = |a: &OcrRect, b: &OcrRect| {
            a.x.abs_diff(b.x) <= OCR_NEXT_JITTER_PX
                && a.y.abs_diff(b.y) <= OCR_NEXT_JITTER_PX
                && a.width.abs_diff(b.width) <= OCR_NEXT_JITTER_PX
                && a.height.abs_diff(b.height) <= OCR_NEXT_JITTER_PX
        };
        match next.as_slice() {
            [next] if !stories.is_empty() => {
                if let Some(event) = &mut frame.event {
                    let b = next.bounds;
                    event.bounds = Some([b.x, b.y, b.width, b.height].map(f64::from));
                }
                if previous.as_ref().is_some_and(|(old_next, old_stories)| {
                    near(old_next, &next.bounds)
                        && stories
                            .iter()
                            .any(|story| old_stories.iter().any(|old| near(old, story)))
                }) {
                    frame.finish("proved");
                    return Ok(true);
                }
                frame.finish("pair");
                previous = Some((next.bounds, stories));
            }
            _ => {
                frame.finish("noPair");
                previous = None;
            }
        }
        drop(frame);
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
        // Evidence for the editor XML proof: generation, the matched marker's rect and
        // whether a Loading label was visible (an outcome ending in `Loading`).
        let mut read = StageDiagnosticOperation::start("editorXmlRead", &epoch, Some(deadline));
        read.query(marker);
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
                read.finish("transientError");
                drop(read);
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
            Err(error) => {
                read.finish("error");
                return Err(error);
            }
            Ok(ReadWaitResult::Cancelled) => {
                read.finish("cancelled");
                return Ok(false);
            }
            Ok(ReadWaitResult::DeadlineExceeded) => {
                read.finish("deadline");
                return Ok(false);
            }
        };
        // A snapshot the read grace finished past `deadline` is judged once, here: no read
        // starts after the window (`begin_read`), and the proof rule below is unchanged. r130-
        // r133 (SM-G955F/N, Global 45.7.3/en): 8 editor waits had a second editor read finish
        // 3.3-19.0 s into the grace, and this check threw each away unjudged.
        if stop.load(Ordering::Relaxed) {
            read.finish("stopped");
            return Ok(false);
        }
        read.snapshot(snapshot.generation);
        let current_epoch = session.gui_session_epoch();
        if epoch != current_epoch {
            read.finish("sessionChanged");
            return Err(anyhow::anyhow!("editor recovery session replaced")
                .context(crate::publish_recovery::RecoveryFailure::new(
                    "editor_observation_invalidated",
                    crate::publish_recovery::FailureKind::Retryable,
                    format!("Editor read generation {} changed session; previous epoch={epoch}, current epoch={current_epoch}; discard prior targets and prepare afresh", snapshot.generation),
                )));
        }
        if snapshot.generation == 0 {
            read.finish("generationZero");
            drop(read);
            previous = None;
            sleep(
                POLL.min(deadline.saturating_duration_since(Instant::now())),
                stop,
            )
            .await;
            continue;
        }
        read.finish("error");
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
        let single = match matches.as_slice() {
            [index] => tree.nodes[*index].rect(),
            _ => None,
        };
        if let Some(rect) = &single {
            read.target(rect, None);
        }
        read.finish(match (matches.len(), loading) {
            (0, false) => "noMarker",
            (0, true) => "noMarkerLoading",
            (1, false) => "marker",
            (1, true) => "markerLoading",
            (_, false) => "markers",
            (_, true) => "markersLoading",
        });
        let target = single.filter(|rect| rect.enabled && !loading);
        if let Some(target) = target {
            if previous
                .as_ref()
                .is_some_and(|(old_epoch, generation, rect)| {
                    old_epoch == &epoch && tree.generation > *generation && rect == &target
                })
            {
                let proved = !stop.load(Ordering::Relaxed);
                read.finish(if proved { "proved" } else { "stopped" });
                return Ok(proved);
            }
            previous = Some((epoch.clone(), tree.generation, target));
        } else {
            previous = None;
        }
        drop(read);
        sleep(
            POLL.min(deadline.saturating_duration_since(Instant::now())),
            stop,
        )
        .await;
    }
    Ok(false)
}
