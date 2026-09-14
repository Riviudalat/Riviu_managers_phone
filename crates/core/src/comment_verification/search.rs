//! Navigation and observations only. This module cannot type or send a comment.
use crate::tiktok_labels::{TikTokControl, TikTokControls};
use crate::ui_automation::tree::Tree;
use crate::{CommentLocatorIdentity, ElementBox, ElementQuery, UiSession};
use anyhow::Context;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;

#[derive(Debug, Clone)]
pub struct FoundComment {
    pub hidden: bool,
    pub body: ElementBox,
    pub author: ElementBox,
    pub reply: Option<ElementBox>,
    pub identity: CommentLocatorIdentity,
    pub snapshot: String,
}

/// Trill 38.3.2 adds U+200B at a visual wrap after "Đà Lạt". Ignore only
/// that layout separator; retain accents, spaces, punctuation and emoji joiners.
pub(crate) fn rendered_text_matches(observed: &str, expected: &str) -> bool {
    observed
        .chars()
        .filter(|c| *c != '\u{200b}')
        .eq(expected.chars().filter(|c| *c != '\u{200b}'))
}

pub(crate) fn comment_bodies(tree: &Tree, package: &str, text: &str) -> Vec<usize> {
    tree.nodes
        .iter()
        .enumerate()
        .filter(|(i, n)| {
            n.visible(package)
                && tree.ancestors_visible(*i)
                && n.rect().is_some()
                && rendered_text_matches(n.attr("text"), text)
        })
        .map(|(i, _)| i)
        .collect()
}

pub fn parent_matches(
    found: &FoundComment,
    package: &str,
    parent: &CommentLocatorIdentity,
) -> anyhow::Result<bool> {
    let tree = Tree::parse(crate::HierarchySourceSnapshot {
        generation: 1,
        xml: found.snapshot.clone(),
    })?;
    let Some(p) = row(&tree, package, &parent.text, Some(&parent.author_label))? else {
        return Ok(false);
    };
    if p.body.y >= found.body.y {
        return Ok(false);
    }
    // A reply to another reply displays its addressee in the same author row.
    if found.body.x <= p.body.x {
        return Ok(tree.nodes.iter().enumerate().any(|(i, n)| {
            n.visible(package)
                && tree.ancestors_visible(i)
                && n.attr("resource-id").ends_with(":id/seu")
                && n.attr("text") == parent.author_label
                && n.rect()
                    .is_some_and(|r| r.y < found.body.y && r.y >= found.author.y)
        }));
    }
    let body_id = tree
        .nodes
        .iter()
        .find(|n| rendered_text_matches(n.attr("text"), &parent.text))
        .map(|n| n.attr("resource-id"))
        .unwrap_or_default();
    if body_id.is_empty() {
        return Ok(false);
    }
    Ok(!tree.nodes.iter().enumerate().any(|(i, n)| {
        n.visible(package)
            && tree.ancestors_visible(i)
            && n.attr("resource-id") == body_id
            && n.rect()
                .is_some_and(|r| r.y > p.body.y && r.y < found.body.y && r.x <= p.body.x)
    }))
}

pub fn row(
    tree: &Tree,
    package: &str,
    text: &str,
    author: Option<&str>,
) -> anyhow::Result<Option<FoundComment>> {
    let bodies = comment_bodies(tree, package, text);
    anyhow::ensure!(
        bodies.len() <= 1,
        "comment_ambiguous: nhiều câu trùng nội dung"
    );
    let Some(&i) = bodies.first() else {
        return Ok(None);
    };
    let body = tree.nodes[i].rect().context("comment_bounds_missing")?;
    let mut current = tree.nodes[i].parent;
    while let Some(index) = current {
        let authors: Vec<_> = tree
            .matching(package, ElementQuery::ClassName("android.widget.Button"))
            .into_iter()
            .filter(|j| tree.inside(*j, index))
            .filter_map(|j| tree.nodes[j].rect())
            .filter(|r| {
                let label = r.description.as_deref().unwrap_or_default();
                !label.is_empty()
                    && !matches!(label, "Reply" | "Trả lời")
                    && r.y + r.height <= body.y + crate::interaction_hierarchy::ABOVE_SLACK
                    && (r.x - body.x).abs() <= crate::interaction_hierarchy::AUTHOR_LEFT_SLACK
                    && r.y >= body.y - crate::interaction_hierarchy::AUTHOR_REACH
            })
            .collect();
        let replies: Vec<_> = tree
            .nodes
            .iter()
            .enumerate()
            .filter(|(j, n)| {
                tree.inside(*j, index)
                    && n.visible(package)
                    && tree.ancestors_visible(*j)
                    && matches!(n.attr("text"), "Reply" | "Trả lời")
            })
            .filter_map(|(_, n)| n.rect())
            .collect();
        if replies.len() > 1 {
            return Ok(None);
        }
        if authors.len() == 1 {
            let a = &authors[0];
            let label = a.description.clone().unwrap_or_default();
            if author.is_some_and(|expected| expected != label) {
                return Ok(None);
            }
            return Ok(Some(FoundComment {
                hidden: false,
                body,
                author: a.clone(),
                reply: replies.first().cloned(),
                identity: CommentLocatorIdentity {
                    author_label: label,
                    text: text.into(),
                    locator_version: "android-snapshot-v2".into(),
                    frame_sha256: String::new(),
                },
                snapshot: String::new(),
            }));
        }
        current = tree.nodes[index].parent;
    }
    Ok(None)
}

/// Explicit reveal label and its own enabled hitbox, excluding multi-action containers.
pub fn hidden_control(tree: &Tree, package: &str) -> anyhow::Result<Option<ElementBox>> {
    let labels: Vec<_> = tree
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, n)| {
            n.visible(package)
                && tree.ancestors_visible(*i)
                && matches!(
                    n.attr("text"),
                    "View folded comments" | "Community-flagged comments" | "Xem bình luận bị ẩn"
                )
                && n.rect().is_some()
        })
        .map(|(i, _)| i)
        .collect();
    anyhow::ensure!(labels.len() <= 1, "hidden_comments_ambiguous");
    let Some(&label) = labels.first() else {
        return Ok(None);
    };
    let mut at = Some(label);
    for _ in 0..4 {
        let Some(i) = at else { break };
        let n = &tree.nodes[i];
        if !n.visible(package) || !tree.ancestors_visible(i) || n.attr("enabled") != "true" {
            break;
        }
        if n.attr("clickable") == "true" {
            let competitors = tree.nodes.iter().enumerate().any(|(j, other)| {
                j != i
                    && tree.inside(j, i)
                    && other.attr("clickable") == "true"
                    && other.rect().is_some()
            });
            anyhow::ensure!(!competitors, "hidden_comments_ambiguous_hitbox");
            return Ok(n.rect());
        }
        at = n.parent;
    }
    anyhow::bail!("hidden_comments_unsupported: chưa xác định vùng mở")
}

pub async fn find(
    session: &dyn UiSession,
    labels: TikTokControls,
    text: &str,
    author: Option<&str>,
    root: Option<&CommentLocatorIdentity>,
    stop: &AtomicBool,
) -> anyhow::Result<FoundComment> {
    find_with_options(session, labels, text, author, root, stop, false).await
}

pub async fn find_for_reply(
    session: &dyn UiSession,
    labels: TikTokControls,
    parent: &CommentLocatorIdentity,
    root: Option<&CommentLocatorIdentity>,
    stop: &AtomicBool,
) -> anyhow::Result<FoundComment> {
    find_with_options(
        session,
        labels,
        &parent.text,
        Some(&parent.author_label),
        root,
        stop,
        true,
    )
    .await
}

async fn find_with_options(
    session: &dyn UiSession,
    labels: TikTokControls,
    text: &str,
    author: Option<&str>,
    root: Option<&CommentLocatorIdentity>,
    stop: &AtomicBool,
    require_reply: bool,
) -> anyhow::Result<FoundComment> {
    let end = tokio::time::Instant::now() + Duration::from_secs(45);
    let future = async {
        let mut seen = HashSet::new();
        let mut opened = HashSet::new();
        let mut scrolls = 0;
        let mut missing_drawer = 0;
        loop {
            anyhow::ensure!(!stop.load(Ordering::Relaxed), "comment_search_cancelled");
            anyhow::ensure!(
                session
                    .gui_scope()
                    .and_then(|s| s.deadline_ms)
                    .is_none_or(|d| chrono::Utc::now().timestamp_millis() < d),
                "comment_search_deadline"
            );
            let snapshot = session
                .hierarchy_source_snapshot()
                .await
                .context("comment_read_failed")?;
            let xml = snapshot.xml.clone();
            let tree = Tree::parse(snapshot)?;
            if let Some(mut found) = row(&tree, labels.package(), text, author)?
                .filter(|found| !require_reply || found.reply.is_some())
            {
                found.hidden = opened.contains("hidden-comments");
                found.identity.frame_sha256 = format!("{:x}", Sha256::digest(xml.as_bytes()));
                found.snapshot = xml;
                return Ok(found);
            }
            let control = if let Some(root) = root {
                crate::interaction_hierarchy::replies::expand_target(&tree, labels.package(), root)?
            } else {
                None
            };
            let key = if control.is_some() {
                "root-replies"
            } else {
                "hidden-comments"
            };
            let control = control.or(hidden_control(&tree, labels.package())?);
            if let Some(control) = control {
                if opened.insert(key) {
                    session.tap(control.centre()).await?;
                    tokio::time::sleep(Duration::from_millis(700)).await;
                    continue;
                }
            }
            let inputs = tree.matching(
                labels.package(),
                ElementQuery::ClassName("android.widget.EditText"),
            );
            let field = inputs
                .iter()
                .filter_map(|i| tree.nodes[*i].rect())
                .max_by(|a, b| a.y.total_cmp(&b.y));
            let Some(field) = field else {
                missing_drawer += 1;
                anyhow::ensure!(missing_drawer <= 4, "comment_drawer_closed");
                tokio::time::sleep(Duration::from_millis(400)).await;
                continue;
            };
            missing_drawer = 0;
            let signature: Vec<_> = tree
                .nodes
                .iter()
                .filter(|n| n.visible(labels.package()) && !n.attr("text").is_empty())
                .map(|n| (n.attr("text").to_owned(), n.attr("bounds").to_owned()))
                .collect();
            anyhow::ensure!(
                scrolls < 10 && seen.insert(signature),
                "comment_not_visible: chưa thấy bình luận trong danh sách đã đọc"
            );
            // Measured drawer region: use the same body/field-derived scroll as the sending path.
            crate::interaction_hierarchy::scroll_comment_list(
                session,
                session.window_size().await?,
                &field,
            )
            .await?;
            scrolls += 1;
            tokio::time::sleep(Duration::from_millis(700)).await;
        }
    };
    tokio::select! {
        result=tokio::time::timeout_at(end,future)=>result.context("comment_search_timeout")?,
        _=async {while !stop.load(Ordering::Relaxed){tokio::time::sleep(Duration::from_millis(50)).await}}=>anyhow::bail!("comment_search_cancelled")
    }
}

pub async fn open_drawer(session: &dyn UiSession, labels: TikTokControls) -> anyhow::Result<()> {
    let label = labels
        .label(TikTokControl::Comments)
        .context("comment_control_unsupported")?;
    let button = session
        .locate(label.to_query())
        .await?
        .context("comment_control_missing")?;
    session.tap(button.centre()).await?;
    tokio::time::sleep(Duration::from_millis(700)).await;
    Ok(())
}

/// A known username is verified by opening the matched row's own profile.
pub async fn verify_author(
    session: &dyn UiSession,
    labels: TikTokControls,
    found: &FoundComment,
    account: &str,
) -> anyhow::Result<()> {
    session.tap(found.author.centre()).await?;
    let result = async {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let tree = Tree::parse(session.hierarchy_source_snapshot().await?)?;
            let handles: Vec<_> = tree
                .nodes
                .iter()
                .enumerate()
                .filter(|(i, n)| {
                    n.visible(labels.package())
                        && tree.ancestors_visible(*i)
                        && n.attr("text").starts_with('@')
                        && matches!(
                            n.attr("class"),
                            "android.widget.Button" | "android.widget.TextView"
                        )
                })
                .map(|(_, n)| n.attr("text").trim_start_matches('@').to_owned())
                .collect();
            let profile = tree.nodes.iter().enumerate().any(|(i, n)| {
                n.visible(labels.package())
                    && tree.ancestors_visible(i)
                    && [n.attr("text"), n.attr("content-desc")].iter().any(|s| {
                        matches!(
                            *s,
                            "Followers"
                                | "Following"
                                | "Người theo dõi"
                                | "Đang Follow"
                                | "Profile menu"
                        )
                    })
            });
            if profile
                && handles
                    .iter()
                    .any(|h| h.eq_ignore_ascii_case(account.trim_start_matches('@')))
                && handles
                    .iter()
                    .all(|h| h.eq_ignore_ascii_case(account.trim_start_matches('@')))
            {
                return Ok(());
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "comment_author_mismatch: chưa xác nhận username tác giả"
            );
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
    }
    .await;
    let back = session.back().await;
    result?;
    back?;
    tokio::time::sleep(Duration::from_millis(700)).await;
    Ok(())
}
