//! Measured Send-to friends picker: musically 45.7.3/en, 2026-09-21.
//! Labels, parent rows and selected checkbox were observed through Riviu Inspector.
use crate::{ui_automation::tree::Tree, ElementBox, UiSession};
use anyhow::{ensure, Context};

pub fn supported(package: &str, version: &str, locale: &str) -> bool {
    package == "com.zhiliaoapp.musically" && version == "45.7.3" && locale.starts_with("en")
}
fn unique(
    tree: &Tree,
    package: &str,
    predicate: impl Fn(&crate::ui_automation::tree::Node) -> bool,
) -> anyhow::Result<ElementBox> {
    let found: Vec<_> = tree
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, n)| tree.ancestors_visible(*i) && n.visible(package) && predicate(n))
        .map(|(_, n)| n)
        .filter_map(|n| n.rect())
        .filter(|r| r.enabled && r.clickable)
        .collect();
    ensure!(found.len() == 1, "Share target missing or ambiguous");
    Ok(found[0].clone())
}
async fn tree(session: &dyn UiSession) -> anyhow::Result<Tree> {
    Tree::parse(session.hierarchy_source_snapshot().await?)
}
async fn tap(session: &dyn UiSession, b: ElementBox) -> anyhow::Result<()> {
    session.tap(b.centre()).await
}

#[derive(Debug, Clone)]
pub struct Friend {
    pub handle: String,
    pub row: usize,
    pub rect: ElementBox,
}
pub fn friends(tree: &Tree, package: &str) -> Vec<Friend> {
    tree.nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.visible(package) && n.attr("resource-id").ends_with(":id/epw"))
        .filter_map(|(row, n)| {
            let children: Vec<_> = tree
                .nodes
                .iter()
                .enumerate()
                .filter(|(i, _)| tree.inside(*i, row))
                .map(|(_, n)| n)
                .collect();
            if !children
                .iter()
                .any(|n| n.attr("text").trim() == "· Friends")
            {
                return None;
            }
            let handles: Vec<_> = children
                .iter()
                .filter(|n| n.attr("resource-id").ends_with(":id/fni"))
                .collect();
            if handles.len() != 1 {
                return None;
            }
            let handle = handles[0]
                .attr("text")
                .trim()
                .trim_start_matches('\u{200e}')
                .to_owned();
            if handle.is_empty() {
                return None;
            }
            let rect = n.rect()?;
            if !rect.enabled || !rect.clickable {
                return None;
            }
            Some(Friend { handle, row, rect })
        })
        .collect()
}

async fn sent_toast(
    session: &dyn UiSession,
    before: &[u8],
    after: Vec<u8>,
    name: &str,
) -> anyhow::Result<bool> {
    use crate::ui_automation::{OcrImage, OcrRect, OcrRequest};
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let Some(reasoner) = session.gui_reasoner() else {
        return Ok(false);
    };
    if before == after {
        return Ok(false);
    }
    let image = image::load_from_memory(&after)?;
    // SM-G955F ce031713cd92d90701 45.7.3/en, 21/09: Sent-to toast at
    // native y1780..1900 over the lower caption. OCR is readback only, never tap.
    if image.width() != 1080 || image.height() != 2220 {
        return Ok(false);
    }
    let mut request = OcrRequest {
        protocol_version: 1,
        request_id: uuid::Uuid::new_v4().to_string(),
        observation_id: uuid::Uuid::new_v4().to_string(),
        session_epoch: session.gui_session_epoch(),
        generation: 1,
        remaining_ms: 8000,
        screenshot: OcrImage {
            sha256: format!("{:x}", Sha256::digest(&after)),
            bytes_base64: base64::engine::general_purpose::STANDARD.encode(after),
            width: 1080,
            height: 2220,
        },
        roi: Some(OcrRect {
            x: 20,
            y: 1770,
            width: 1030,
            height: 150,
        }),
        languages: vec!["en".into(), "vi".into()],
        min_confidence: 0.85,
    };
    let wanted = format!("sent to {}", name.trim_start_matches('\u{200e}')).to_lowercase();
    for attempt in 0..2 {
        if attempt == 1 {
            // ce021712aaf9533405, 22/09: the wide band overlaps the comment
            // field and truncates a long recipient name. This measured text-line
            // crop reads the full name from the SAME captured post-Send frame.
            request.request_id = uuid::Uuid::new_v4().to_string();
            request.roi = Some(OcrRect {
                x: 125,
                y: 1800,
                width: 700,
                height: 65,
            });
            request.languages = vec!["vi".into(), "en".into()];
        }
        let result = reasoner.ocr(request.clone()).await?;
        result.validate_binding(&request)?;
        if result.lines.iter().any(|l| {
            l.confidence >= 0.85
                && l.text
                    .trim()
                    .trim_start_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase()
                    == wanted
        }) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Returns after selection, before Send. An empty list is never replaced by suggestions.
pub async fn select_friend(
    session: &dyn UiSession,
    package: &str,
    seed: u64,
    key: &str,
) -> anyhow::Result<Option<String>> {
    ensure!(
        supported(
            package,
            &session
                .app_version(package)
                .await
                .context("Unknown TikTok version")?,
            &session
                .ui_language()
                .await
                .context("Unknown TikTok language")?
        ),
        "Share friends chưa đo trên phiên bản này"
    );
    let current = tree(session).await?;
    let share = unique(&current, package, |n| {
        n.attr("content-desc").starts_with("Share video")
    })?;
    tap(session, share).await?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let t = tree(session).await?;
        if t.nodes.iter().any(|n| n.attr("text") == "Send to") {
            tap(
                session,
                unique(&t, package, |n| n.attr("content-desc") == "Search")?,
            )
            .await?;
            break;
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "Share sheet did not open"
        );
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut last = Vec::new();
    loop {
        let t = tree(session).await?;
        let (width, height) = session.window_size().await?;
        let list: Vec<_> = friends(&t, package)
            .into_iter()
            .filter(|f| f.rect.x + f.rect.width <= width && f.rect.y + f.rect.height <= height)
            .collect();
        let ids: Vec<_> = list.iter().map(|f| f.handle.clone()).collect();
        if !ids.is_empty() && ids == last {
            let selected = &list[(crate::seeding::stable(seed, key) % list.len() as u64) as usize];
            ensure!(
                list.iter().filter(|f| f.handle == selected.handle).count() == 1,
                "Ambiguous friend handle"
            );
            tap(session, selected.rect.clone()).await?;
            return Ok(Some(selected.handle.clone()));
        }
        if tokio::time::Instant::now() >= deadline {
            ensure!(
                t.nodes.iter().any(|n| n.attr("text") == "Send to"),
                "Friends picker lost"
            );
            if ids.is_empty()
                && last.is_empty()
                && t.nodes
                    .iter()
                    .any(|n| matches!(n.attr("text"), "No friends yet" | "No friends"))
            {
                session.back().await?;
                return Ok(None);
            }
            anyhow::bail!("Friends list did not stabilize");
        }
        last = ids;
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    }
}

pub(crate) async fn send_selected(
    session: &dyn UiSession,
    package: &str,
    handle: &str,
    gate: &mut crate::interaction_target::ActionEffectGate<'_>,
) -> Result<bool, crate::ActionFailure> {
    let ready_until = tokio::time::Instant::now() + std::time::Duration::from_secs(8);
    let (send, name) = loop {
        let t = tree(session).await.map_err(crate::ActionFailure::before)?;
        let list = friends(&t, package);
        let friend = list
            .iter()
            .find(|f| f.handle == handle)
            .context("Selected friend disappeared")
            .map_err(crate::ActionFailure::before)?;
        let checked: Vec<_> = t
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.attr("checked") == "true")
            .collect();
        if checked.len() != 1 || !t.inside(checked[0].0, friend.row) {
            return Err(crate::ActionFailure::before(anyhow::anyhow!(
                "Share recipient selection changed"
            )));
        }
        let send = unique(&t, package, |n| {
            n.attr("text") == "Send" && n.attr("resource-id").ends_with(":id/tv_send")
        });
        match send {
            Ok(send) => {
                let names: Vec<_> = t
                    .nodes
                    .iter()
                    .enumerate()
                    .filter(|(i, n)| {
                        t.inside(*i, friend.row) && n.attr("resource-id").ends_with(":id/ouz")
                    })
                    .map(|(_, n)| n.attr("text").to_owned())
                    .collect();
                if names.len() != 1 {
                    return Err(crate::ActionFailure::before(anyhow::anyhow!(
                        "Recipient display name ambiguous"
                    )));
                }
                break (send, names[0].clone());
            }
            Err(error) if tokio::time::Instant::now() >= ready_until => {
                return Err(crate::ActionFailure::before(error))
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(250)).await,
        }
    };
    let before = session
        .screenshot_png()
        .await
        .map_err(crate::ActionFailure::before)?;
    gate.cross()?;
    tap(session, send)
        .await
        .map_err(crate::ActionFailure::after)?;
    if let Ok(after) = session.screenshot_png().await {
        if sent_toast(session, &before, after, &name)
            .await
            .unwrap_or(false)
        {
            return Ok(true);
        }
    }
    let until = tokio::time::Instant::now() + std::time::Duration::from_secs(8);
    while tokio::time::Instant::now() < until {
        let t = tree(session).await.map_err(crate::ActionFailure::after)?;
        if t.nodes
            .iter()
            .any(|n| n.visible(package) && matches!(n.attr("text"), "Sent" | "Shared successfully"))
        {
            return Ok(true);
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    Ok(false)
}
