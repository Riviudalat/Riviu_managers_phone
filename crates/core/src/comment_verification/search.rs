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
    /// The row's own like control and its state — see [`row_like_control`]. `None` means this
    /// build cannot identify a comment heart at all, which is a refusal, not a "not liked".
    pub like: Option<RowLike>,
    pub identity: CommentLocatorIdentity,
    pub snapshot: String,
}

/// One comment row's like control, and what state it was in when this was read.
#[derive(Debug, Clone, PartialEq)]
pub struct RowLike {
    /// The control to tap. The **heart icon**, not the `Button` around it: measured on
    /// `trill` 38.3.2 the button spans `[819,1000][971,1063]`, the icon `[845,1000][898,1063]`
    /// and the count `[903,1000][971,1063]`, and a tap in the button's own centre — the gap
    /// between icon and count — changed nothing at all.
    ///
    /// Measured again on a second handset of the same build (98895a3355424e484f, UI en, the
    /// comment drawer of a real post): the button is `[781,1140][955,1212]` and its outline
    /// heart `[811,1140][871,1212]` — the same 174x72 button, 38 px further left and 140 px
    /// lower than the first phone. So the absolutes belong to one handset and every rule built
    /// on this control is **relative** for that reason: the icon inside the control's own
    /// rectangle, and the control inside the row's reply strip.
    pub element: ElementBox,
    /// `Some(true)` liked, `Some(false)` not liked, `None` when the row carries neither
    /// measured icon — a row whose state cannot be read, which is not the same as one that is
    /// not liked, because tapping a liked heart *removes* the like.
    pub liked: Option<bool>,
}

/// How far above and below a row's `Reply` control its heart may sit.
///
/// Measured on `trill` 38.3.2: heart `[819,1000][971,1063]` against that row's reply control
/// `[242,1010][334,1052]` — 10 px proud at the top and 11 px at the bottom. 24 covers both and
/// stays far under the ~300 px row pitch, which is what keeps the *next* row's heart out.
///
/// `pub(crate)` because the tap's confirmation re-reads the same band from a fresh
/// observation — see `like_comment_row`.
pub(crate) const LIKE_STRIP_SLACK: f64 = 24.0;

/// The like control of the row `body` belongs to, with its state.
///
/// `None` when this build's catalogue cannot name a comment heart (`CommentLike` missing), or
/// when the strip the row occupies does not hold exactly one — the same "one row, or refuse"
/// rule the reply control is found by, for the same reason: every row on screen carries both,
/// so more than one candidate means the row was not identified.
pub fn row_like_control(
    tree: &Tree,
    labels: TikTokControls,
    body: &ElementBox,
    reply: Option<&ElementBox>,
) -> Option<RowLike> {
    let control_label = labels.label(TikTokControl::CommentLike)?;
    // The row's action strip. Anchored on the row's own `Reply` control when there is one; a
    // build with no measured Reply label falls back to the body and the reach the reply path
    // already uses for it.
    let (top, bottom) = match reply {
        Some(reply) => (
            reply.y - LIKE_STRIP_SLACK,
            reply.y + reply.height + LIKE_STRIP_SLACK,
        ),
        None => (
            body.y + body.height - LIKE_STRIP_SLACK,
            body.y + body.height + crate::interaction_hierarchy::REPLY_REACH,
        ),
    };
    let rects = |label: crate::tiktok_labels::LabelMatch| -> Vec<ElementBox> {
        tree.matching(labels.package(), label.to_query())
            .into_iter()
            .filter(|index| tree.ancestors_visible(*index))
            .filter_map(|index| tree.nodes[index].rect())
            .collect()
    };
    let mut controls: Vec<ElementBox> = rects(control_label)
        .into_iter()
        .filter(|control| control.y < bottom && control.y + control.height > top)
        .collect();
    if controls.len() != 1 {
        return None;
    }
    let control = controls.pop().expect("checked to be exactly one");
    // The icons are the control's own children — measured inside its rectangle, sharing its
    // top and bottom — so a same-strip heart of another row cannot be read as this one's state.
    let icon = |label: Option<crate::tiktok_labels::LabelMatch>| -> Option<ElementBox> {
        let mut found: Vec<ElementBox> = rects(label?)
            .into_iter()
            .filter(|icon| {
                icon.x >= control.x
                    && icon.x + icon.width <= control.x + control.width
                    && icon.y >= control.y
                    && icon.y + icon.height <= control.y + control.height
            })
            .collect();
        (found.len() == 1).then(|| found.pop().expect("checked to be exactly one"))
    };
    let liked = icon(labels.label(TikTokControl::CommentLiked));
    let not_liked = icon(labels.label(TikTokControl::CommentNotLiked));
    let (element, state) = match (liked, not_liked) {
        // Both present is not a state this build produces; reading it as liked means the caller
        // does not tap, which is the direction that cannot remove somebody's like.
        (Some(icon), _) => (icon, Some(true)),
        (None, Some(icon)) => (icon, Some(false)),
        (None, None) => (control, None),
    };
    Some(RowLike {
        element,
        liked: state,
    })
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

/// Keep the reply and its parent in one fresh snapshot. On 45.7.3 the
/// search's upward scroll can leave a found reply at the drawer's top with
/// its parent just above the viewport. Scroll back a small bounded distance;
/// this is observation only and retains the exact-text/author/branch checks.
pub async fn reveal_parent(
    session: &dyn UiSession,
    labels: TikTokControls,
    mut found: FoundComment,
    parent: &CommentLocatorIdentity,
    stop: &AtomicBool,
) -> anyhow::Result<FoundComment> {
    for attempt in 0..4 {
        if parent_matches(&found, labels.package(), parent)? {
            return Ok(found);
        }
        anyhow::ensure!(
            attempt < 3,
            "comment_parent_not_visible: chưa xác minh nhánh của câu đã gửi"
        );
        anyhow::ensure!(!stop.load(Ordering::Relaxed), "comment_search_cancelled");
        let tree = Tree::parse(crate::HierarchySourceSnapshot {
            generation: 1,
            xml: found.snapshot.clone(),
        })?;
        // A visible different branch is a refusal, not a reason to scroll past it.
        anyhow::ensure!(
            row(
                &tree,
                labels.package(),
                &parent.text,
                Some(&parent.author_label)
            )?
            .is_none(),
            "comment_parent_mismatch"
        );
        let field = tree
            .matching(
                labels.package(),
                ElementQuery::ClassName("android.widget.EditText"),
            )
            .into_iter()
            .filter_map(|i| tree.nodes[i].rect())
            .max_by(|a, b| a.y.total_cmp(&b.y))
            .context("comment_drawer_closed")?;
        let (width, height) = session.window_size().await?;
        let top = height * 0.35;
        let bottom = field.y - 40.0;
        anyhow::ensure!(
            bottom - top > 300.0 && found.body.y < top + (bottom - top) * 0.55,
            "comment_parent_not_visible"
        );
        let from = crate::TapPoint {
            x: width * 0.5,
            y: top + (bottom - top) * 0.30,
        };
        let to = crate::TapPoint {
            x: width * 0.5,
            y: top + (bottom - top) * 0.55,
        };
        session
            .swipe(crate::SwipeGesture {
                from,
                to,
                duration_ms: 350,
            })
            .await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let snapshot = session.hierarchy_source_snapshot().await?;
        let xml = snapshot.xml.clone();
        let tree = Tree::parse(snapshot)?;
        let mut current = row(
            &tree,
            labels.package(),
            &found.identity.text,
            Some(&found.identity.author_label),
        )?
        .context("comment_reply_left_view")?;
        current.snapshot = xml;
        found = current;
    }
    unreachable!("bounded observations return or refuse")
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
                // Filled by the caller that has the label set: `row()` matches on strings the
                // file already knows (`Reply`), and the like control's name is catalogue data.
                like: None,
                identity: CommentLocatorIdentity {
                    comment_link: None,
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
                found.like = row_like_control(&tree, labels, &found.body, found.reply.as_ref());
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
