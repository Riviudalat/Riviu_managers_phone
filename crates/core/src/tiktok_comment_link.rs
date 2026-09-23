//! Read-only identity carried by TikTok's measured comment share link.
//!
//! On `com.zhiliaoapp.musically` 45.7.3/en, long-pressing a comment and choosing
//! `Share with -> Copy link` produced a short `vt.tiktok.com` URL. Following that
//! URL redirected to the post URL and retained `share_item_id` plus
//! `share_comment_id` in its query. This parser accepts either that final URL or
//! the internal `aweme://aweme/detail?id=...&cid=...` form used by comment history.
//! It never treats a post-only URL as a comment identity.

use crate::tiktok_labels::TikTokControls;
use crate::ui_automation::tree::Tree;
use crate::{CommentLocatorIdentity, ElementBox, ElementQuery, UiSession};
use anyhow::{ensure, Context};
use serde::{Deserialize, Serialize};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedCommentLink {
    pub post_id: String,
    pub comment_id: String,
    pub source_url: String,
}

fn numeric(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
}

/// Extracts the post/comment pair without following redirects or trusting display text.
pub fn parse(value: &str) -> Option<SharedCommentLink> {
    let raw = value.trim();
    let parsed = Url::parse(raw).ok()?;
    let internal = parsed.scheme() == "aweme"
        && parsed.host_str() == Some("aweme")
        && parsed.path().trim_matches('/') == "detail";
    let web = trusted(&parsed)
        && matches!(
            parsed.host_str(),
            Some("www.tiktok.com" | "tiktok.com" | "m.tiktok.com")
        );
    if (!internal && !web) || !parsed.username().is_empty() || parsed.password().is_some() {
        return None;
    }
    let values = |keys: &[&str]| {
        parsed
            .query_pairs()
            .filter(|(k, _)| keys.contains(&k.as_ref()))
            .map(|(_, v)| v.into_owned())
            .collect::<Vec<_>>()
    };
    let posts = values(&["share_item_id", "id", "video_id"]);
    let comments = values(&["share_comment_id", "cid", "comment_id"]);
    let ([post], [comment]) = (posts.as_slice(), comments.as_slice()) else {
        return None;
    };
    if !numeric(post) || !numeric(comment) {
        return None;
    }
    let source_url = if web {
        let target = crate::parse_tiktok_links(raw).into_iter().next()?.target?;
        if target.content_id != *post {
            return None;
        }
        format!(
            "{}?share_item_id={post}&share_comment_id={comment}",
            target.normalized_url
        )
    } else {
        format!("aweme://aweme/detail?id={post}&cid={comment}")
    };
    Some(SharedCommentLink {
        post_id: post.clone(),
        comment_id: comment.clone(),
        source_url,
    })
}

fn trusted(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && matches!(
            url.host_str(),
            Some(
                "www.tiktok.com"
                    | "tiktok.com"
                    | "m.tiktok.com"
                    | "vt.tiktok.com"
                    | "vm.tiktok.com"
            )
        )
}

pub async fn resolve(value: &str) -> anyhow::Result<SharedCommentLink> {
    if let Some(link) = parse(value) {
        return Ok(link);
    }
    let mut url = Url::parse(value.trim())?;
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    let client = CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("Mozilla/5.0")
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(35))
            .build()
            .expect("comment redirect client")
    });
    // The measured public short-link redirect took 21.6s on this host.
    let end = tokio::time::Instant::now() + Duration::from_secs(60);
    for _ in 0..5 {
        ensure!(trusted(&url), "Untrusted comment link");
        if let Some(link) = parse(url.as_str()) {
            return Ok(link);
        }
        ensure!(
            matches!(url.host_str(), Some("vt.tiktok.com" | "vm.tiktok.com")),
            "Link does not identify a comment"
        );
        let response = tokio::time::timeout_at(end, client.get(url.clone()).send())
            .await?
            .map_err(|e| anyhow::anyhow!("comment_link_resolution: {e:#}"))?;
        ensure!(
            response.status().is_redirection(),
            "Comment link did not redirect"
        );
        url = url.join(
            response
                .headers()
                .get(reqwest::header::LOCATION)
                .context("Comment redirect missing")?
                .to_str()?,
        )?;
    }
    anyhow::bail!("Comment redirect limit exceeded")
}

pub fn supported(labels: TikTokControls) -> bool {
    labels.package() == "com.zhiliaoapp.musically"
        && labels.resource_version() == Some("45.7.3")
        && labels.language() == "en"
}

fn running(stop: &AtomicBool) -> anyhow::Result<()> {
    ensure!(!stop.load(Ordering::Relaxed), "comment_link_cancelled");
    Ok(())
}

fn menu_control(tree: &Tree, package: &str, text: &str) -> anyhow::Result<ElementBox> {
    ensure!(
        tree.nodes
            .iter()
            .any(|n| n.visible(package) && n.attr("content-desc") == "Bottom sheet"),
        "Comment share menu missing"
    );
    let nodes = tree.matching(
        package,
        ElementQuery::Text {
            value: text,
            exact: true,
        },
    );
    ensure!(
        nodes.len() == 1,
        "Comment share control missing or ambiguous: {text}"
    );
    let mut at = Some(nodes[0]);
    for _ in 0..4 {
        if let Some(i) = at {
            let n = &tree.nodes[i];
            if let Some(r) = n.rect() {
                if r.enabled && r.clickable {
                    return Ok(r);
                }
            }
            at = n.parent;
        }
    }
    anyhow::bail!("Comment share control is not clickable")
}

async fn wait_menu(
    session: &dyn UiSession,
    package: &str,
    text: &str,
    stop: &AtomicBool,
) -> anyhow::Result<ElementBox> {
    let end = tokio::time::Instant::now() + Duration::from_secs(6);
    loop {
        running(stop)?;
        let tree = Tree::parse(session.hierarchy_source_snapshot().await?)?;
        if let Ok(r) = menu_control(&tree, package, text) {
            return Ok(r);
        }
        ensure!(
            tokio::time::Instant::now() < end,
            "Comment menu did not expose {text}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Requires an already-proven post, author and exact row. Does not type, send or delete.
pub async fn capture(
    session: &dyn UiSession,
    labels: TikTokControls,
    identity: &CommentLocatorIdentity,
    post_id: &str,
    stop: &AtomicBool,
) -> anyhow::Result<SharedCommentLink> {
    ensure!(supported(labels), "Comment links unavailable on this build");
    running(stop)?;
    let a = Tree::parse(session.hierarchy_source_snapshot().await?)?;
    let first = crate::comment_verification::search::row(
        &a,
        labels.package(),
        &identity.text,
        Some(&identity.author_label),
    )?
    .context("Exact comment missing")?;
    let b = Tree::parse(session.hierarchy_source_snapshot().await?)?;
    let second = crate::comment_verification::search::row(
        &b,
        labels.package(),
        &identity.text,
        Some(&identity.author_label),
    )?
    .context("Exact comment changed")?;
    ensure!(
        (first.body.x - second.body.x).abs() < 2.0 && (first.body.y - second.body.y).abs() < 2.0,
        "Comment moved before copy"
    );
    let sentinel = format!("riviu-comment-link-{}", uuid::Uuid::new_v4());
    session
        .set_clipboard("plaintext", sentinel.as_bytes())
        .await?;
    running(stop)?;
    let point = second.body.centre();
    let (w, h) = session.window_size().await?;
    session.swipe_image(point.clone(), point, w, h, 750).await?;
    let result = async {
        let share = wait_menu(session, labels.package(), "Share with", stop).await?;
        running(stop)?;
        session.tap(share.centre()).await?;
        let copy = wait_menu(session, labels.package(), "Copy link", stop).await?;
        running(stop)?;
        session.tap(copy.centre()).await?;
        let end = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            running(stop)?;
            let (kind, bytes) = session.get_clipboard(4096).await?;
            ensure!(kind.contains("text"), "Non-text comment clipboard");
            let value = String::from_utf8(bytes)?;
            if value != sentinel {
                let link = resolve(&value).await?;
                ensure!(
                    link.post_id == post_id,
                    "Comment link belongs to another post"
                );
                return Ok(link);
            }
            ensure!(
                tokio::time::Instant::now() < end,
                "Copy comment link did not land"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
    .await;
    // Dismiss only the measured menu, without navigating away from another screen.
    if let Ok(snapshot) = session.hierarchy_source_snapshot().await {
        if let Ok(t) = Tree::parse(snapshot) {
            if t.nodes
                .iter()
                .any(|n| n.visible(labels.package()) && n.attr("content-desc") == "Bottom sheet")
            {
                let _ = session.back().await;
            }
        }
    }
    result
}

/// Opens the exact ID, then re-copies the candidate's own link before allowing reply.
pub async fn open_and_verify(
    session: &dyn UiSession,
    labels: TikTokControls,
    identity: &CommentLocatorIdentity,
    stop: &AtomicBool,
) -> anyhow::Result<crate::comment_verification::search::FoundComment> {
    let link = identity
        .comment_link
        .as_ref()
        .context("Comment ID missing")?;
    ensure!(
        parse(&link.source_url).as_ref() == Some(link),
        "Stored comment link is invalid"
    );
    ensure!(
        supported(labels),
        "Comment ID navigation unavailable on this build"
    );
    running(stop)?;
    // musically45.7.3 does not register the Trill aweme scheme. The copied
    // HTTPS comment share destination is handled by its AppLinkHandlerV2.
    ensure!(
        link.source_url.starts_with("https://"),
        "Comment link scheme unavailable on this build"
    );
    let uri = &link.source_url;
    session.open_url_in_app(uri, labels.package()).await?;
    let end = tokio::time::Instant::now() + Duration::from_secs(25);
    let mut opened = false;
    // VIEW completion precedes the comment drawer animation. Do not tap a
    // post-rail coordinate while TikTok is replacing it with comment rows.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    loop {
        running(stop)?;
        let tree = Tree::parse(session.hierarchy_source_snapshot().await?)?;
        if crate::comment_verification::search::row(
            &tree,
            labels.package(),
            &identity.text,
            Some(&identity.author_label),
        )?
        .is_some()
        {
            break;
        }
        let drawer_open = |t: &Tree| {
            t.nodes.iter().enumerate().any(|(i, n)| {
                t.ancestors_visible(i)
                    && n.visible(labels.package())
                    && n.attr("resource-id").ends_with(":id/v80")
                    && n.attr("text").contains("comments")
            })
        };
        if drawer_open(&tree) {
            crate::comment_verification::search::find_for_reply(
                session, labels, identity, None, stop,
            )
            .await?;
            break;
        }
        // Some builds land on the post first. Open the measured drawer once;
        // ID is still proved by copying the exact candidate row below.
        if !opened
            && end.saturating_duration_since(tokio::time::Instant::now()) < Duration::from_secs(20)
        {
            if let Some(control) = labels.label(crate::tiktok_labels::TikTokControl::Comments) {
                let before = tree.matching(labels.package(), control.to_query());
                if let [index] = before.as_slice() {
                    if let Some(button) = tree.nodes[*index].rect() {
                        tokio::time::sleep(Duration::from_millis(350)).await;
                        let fresh = Tree::parse(session.hierarchy_source_snapshot().await?)?;
                        let matches = fresh.matching(labels.package(), control.to_query());
                        if !drawer_open(&fresh) && matches.len() == 1 {
                            if let Some(next) = fresh.nodes[matches[0]].rect() {
                                if (next.x - button.x).abs() < 2.0
                                    && (next.y - button.y).abs() < 2.0
                                {
                                    running(stop)?;
                                    session.tap(next.centre()).await?;
                                    opened = true;
                                }
                            }
                        }
                    }
                }
            }
        }
        ensure!(
            tokio::time::Instant::now() < end,
            "Comment ID not visible on this account"
        );
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    let copied = capture(session, labels, identity, &link.post_id, stop).await?;
    ensure!(
        copied.comment_id == link.comment_id,
        "Different comment ID at destination"
    );
    let snapshot = session.hierarchy_source_snapshot().await?;
    let xml = snapshot.xml.clone();
    let tree = Tree::parse(snapshot)?;
    let mut found = crate::comment_verification::search::row(
        &tree,
        labels.package(),
        &identity.text,
        Some(&identity.author_label),
    )?
    .context("Verified comment left view")?;
    ensure!(
        found.reply.is_some(),
        "Verified comment Reply control missing"
    );
    found.like = crate::comment_verification::search::row_like_control(
        &tree,
        labels,
        &found.body,
        found.reply.as_ref(),
    );
    found.snapshot = xml;
    found.identity.comment_link = Some(copied);
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measured_menu_uses_copy_link_parent_and_refuses_ambiguous_or_other_app() {
        let raw = r#"<hierarchy><node package="com.zhiliaoapp.musically" content-desc="Bottom sheet" displayed="true"><node package="com.zhiliaoapp.musically" clickable="true" enabled="true" bounds="[55,1330][217,1509]"><node package="com.zhiliaoapp.musically" text="Copy link" clickable="false" enabled="true" bounds="[55,1472][217,1509]"/></node><node package="com.zhiliaoapp.musically" text="Copy" clickable="true" enabled="true" bounds="[32,1667][996,1798]"/></node></hierarchy>"#;
        let read = |s: String| {
            Tree::parse(crate::HierarchySourceSnapshot {
                generation: 1,
                xml: s,
            })
            .unwrap()
        };
        let tree = read(raw.into());
        let hit = menu_control(&tree, "com.zhiliaoapp.musically", "Copy link").unwrap();
        assert_eq!(
            (hit.x, hit.y, hit.width, hit.height),
            (55.0, 1330.0, 162.0, 179.0)
        );
        assert!(menu_control(
            &read(raw.replace("text=\"Copy\"", "text=\"Copy link\"")),
            "com.zhiliaoapp.musically",
            "Copy link"
        )
        .is_err());
        assert!(menu_control(&tree, "other.app", "Copy link").is_err());
    }

    #[test]
    fn parses_measured_share_redirect_with_comment_and_post_ids() {
        let link = parse("https://www.tiktok.com/@pht.th.h.slay/photo/7687772173519359239?share_item_id=7687772173519359239&share_comment_id=7688082742043263765").unwrap();
        assert_eq!(link.post_id, "7687772173519359239");
        assert_eq!(link.comment_id, "7688082742043263765");
    }

    #[test]
    fn parses_internal_comment_history_uri() {
        let link = parse("aweme://aweme/detail?id=7679311681721306389&cid=7681527250248090376&refer=comment_history").unwrap();
        assert_eq!(link.comment_id, "7681527250248090376");
    }

    #[test]
    fn post_only_or_untrusted_links_are_not_comment_identities() {
        assert!(parse("https://www.tiktok.com/@a/video/123").is_none());
        assert!(parse("https://evil.example/?share_item_id=1&share_comment_id=2").is_none());
        assert!(parse("https://www.tiktok.com/@a/video/123?share_comment_id=x").is_none());
        assert!(
            parse("https://www.tiktok.com/@a/video/123?share_item_id=999&share_comment_id=2")
                .is_none()
        );
        assert!(parse(
            "https://www.tiktok.com/@a/video/123?share_item_id=123&share_comment_id=2&cid=3"
        )
        .is_none());
        assert!(parse(
            "https://user:pass@www.tiktok.com/@a/video/123?share_item_id=123&share_comment_id=2"
        )
        .is_none());
    }

    #[test]
    fn share_identity_discards_tracking_and_old_records_remain_readable() {
        let link=parse("https://www.tiktok.com/@a/photo/123?share_item_id=123&share_comment_id=2&user_id=private&sec_user_id=private").unwrap();
        assert_eq!(
            link.source_url,
            "https://www.tiktok.com/@a/photo/123?share_item_id=123&share_comment_id=2"
        );
        let old: crate::CommentLocatorIdentity = serde_json::from_str(
            r#"{"authorLabel":"A","text":"body","locatorVersion":"v1","frameSha256":"sha"}"#,
        )
        .unwrap();
        assert!(old.comment_link.is_none());
        let mut enriched = old;
        enriched.comment_link = Some(link);
        let roundtrip: crate::CommentLocatorIdentity =
            serde_json::from_str(&serde_json::to_string(&enriched).unwrap()).unwrap();
        assert_eq!(enriched, roundtrip);
    }
}
