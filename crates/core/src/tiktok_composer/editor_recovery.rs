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
    stop: &AtomicBool,
) -> anyhow::Result<bool> {
    let reasoner = session
        .gui_reasoner()
        .context("editor local OCR unavailable")?;
    let deadline = Instant::now() + Duration::from_secs(60);
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
            let mut response = reasoner.ocr(request.clone()).await?;
            response.validate_binding(&request)?;
            if !response.lines.iter().any(|line| line.text == "Your Story") {
                let story_request = OcrRequest {
                    request_id: uuid::Uuid::new_v4().to_string(),
                    roi: Some(OcrRect {
                        x: 210,
                        y: 1940,
                        width: 300,
                        height: 110,
                    }),
                    ..request
                };
                let story = reasoner.ocr(story_request.clone()).await?;
                story.validate_binding(&story_request)?;
                response.lines.extend(story.lines);
            }
            anyhow::ensure!(
                session.gui_session_epoch() == epoch,
                "editor render session replaced"
            );
            Ok::<_, anyhow::Error>(response)
        };
        let response = match tokio::time::timeout_at(deadline, work).await {
            Ok(r) => r?,
            Err(_) => return Ok(false),
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
        sleep(POLL, stop).await;
    }
    Ok(false)
}

// Two successful XML reads on the measured 45.7.3 editor took about 8 seconds
// each. Bound recovery independently; a persistent unreadable app still refuses.
const RECOVERY_WINDOW: Duration = Duration::from_secs(30);

pub(super) fn transient_read(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<crate::driver::AccessibilityReadUnavailable>()
            .is_some()
            || cause
                .downcast_ref::<reqwest::Error>()
                .is_some_and(reqwest::Error::is_timeout)
    })
}

pub(super) async fn observe(
    session: &dyn UiSession,
    package: &str,
    marker: ElementQuery<'_>,
    stop: &AtomicBool,
) -> anyhow::Result<bool> {
    let deadline = Instant::now() + RECOVERY_WINDOW;
    let mut previous: Option<(String, u64, ElementBox)> = None;
    while Instant::now() < deadline {
        if stop.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let epoch = session.gui_session_epoch();
        let snapshot =
            match tokio::time::timeout_at(deadline, session.hierarchy_source_snapshot()).await {
                Ok(Ok(snapshot)) => snapshot,
                Ok(Err(error)) if transient_read(&error) => {
                    previous = None;
                    sleep(POLL, stop).await;
                    continue;
                }
                Ok(Err(error)) => return Err(error),
                Err(_) => return Ok(false),
            };
        if stop.load(Ordering::Relaxed) || Instant::now() >= deadline {
            return Ok(false);
        }
        if epoch.is_empty() || epoch != session.gui_session_epoch() || snapshot.generation == 0 {
            previous = None;
            sleep(POLL, stop).await;
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
            previous = Some((epoch, tree.generation, target));
        } else {
            previous = None;
        }
        sleep(POLL, stop).await;
    }
    Ok(false)
}
