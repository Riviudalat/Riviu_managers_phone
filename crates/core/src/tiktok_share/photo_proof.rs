//! Measured expanded photo viewer plus independent public metadata proof.
use super::*;
use anyhow::Context;
use hierarchy::Tree;

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Debug)]
pub(super) struct MatchedPhotoCopyFailure(pub LinkCapture);
impl std::fmt::Display for MatchedPhotoCopyFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({:?})", self.0.reason(), self.0)
    }
}
impl std::error::Error for MatchedPhotoCopyFailure {}

fn validate_photo_identity(
    canonical: &str,
    identity: &SubmissionIdentity,
) -> anyhow::Result<String> {
    anyhow::ensure!(
        canonical_account_matches(canonical, &identity.account),
        "canonical account mismatch"
    );
    let parsed = url::Url::parse(canonical)?;
    let id = parsed
        .path()
        .rsplit('/')
        .next()
        .context("post ID missing")?;
    let seconds = id.parse::<u64>()? >> 32;
    let allocated =
        chrono::DateTime::from_timestamp(seconds as i64, 0).context("post ID timestamp invalid")?;
    let submitted = chrono::DateTime::parse_from_rfc3339(&identity.submitted_at)?;
    let prepared = identity
        .prepared_at
        .as_deref()
        .map(chrono::DateTime::parse_from_rfc3339)
        .transpose()?
        .unwrap_or(submitted);
    anyhow::ensure!(
        prepared <= submitted && submitted - prepared <= chrono::Duration::minutes(30),
        "recorded preparation window invalid"
    );
    // IDs have one-second resolution and may be allocated when the app opens.
    // Earlier IDs require the actual recorded session start, never a guessed grace period.
    anyhow::ensure!(
        allocated.timestamp() >= prepared.timestamp()
            && allocated <= submitted + chrono::Duration::minutes(30)
            && allocated <= chrono::Utc::now(),
        "copied post ID is outside recorded publication window"
    );
    Ok(id.to_owned())
}

fn validate_public_metadata(
    embed: &serde_json::Value,
    caption: &str,
    account: &str,
    id: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        normalize(embed["title"].as_str().context("public caption missing")?) == normalize(caption),
        "public caption mismatch"
    );
    let author = url::Url::parse(
        embed["author_url"]
            .as_str()
            .context("public author missing")?,
    )?;
    anyhow::ensure!(
        author.scheme() == "https"
            && author.host_str() == Some("www.tiktok.com")
            && author
                .path()
                .trim_start_matches('/')
                .trim_start_matches('@')
                .eq_ignore_ascii_case(account.trim_start_matches('@')),
        "public author mismatch"
    );
    anyhow::ensure!(
        embed["html"]
            .as_str()
            .is_some_and(|html| html.contains(&format!("data-video-id=\"{id}\""))),
        "public content ID mismatch"
    );
    Ok(())
}

fn viewer_controls(package: &str, version: &str) -> anyhow::Result<(&'static str, &'static str)> {
    match (package, version) {
        ("com.ss.android.ugc.trill", _) => Ok((":id/m3q", ":id/m3h")),
        // SM-G955N Android9/en-US, 18/09/2026: second restart canary;
        // retained sanitized fixture expanded-photo-45.4.3.fixture.
        ("com.zhiliaoapp.musically", "45.4.3") => Ok((":id/r55", ":id/r4k")),
        // SM-G955F fleet18/09/2026: complete caption rey and Share red.
        // Retained sanitized fixture expanded-photo-45.7.3.fixture.
        ("com.zhiliaoapp.musically", "45.7.3") => Ok((":id/rey", ":id/red")),
        // SM-G955F Android9/en-US, 18/09/2026: full-caption expanded photo
        // surface; retained sanitized fixture expanded-photo-46.0.41.fixture.
        ("com.zhiliaoapp.musically", "46.0.41") => Ok((":id/rki", ":id/rjy")),
        // SM-G955F fleet18/09/2026: complete caption rnb and Share rmr.
        // Retained sanitized fixture expanded-photo-46.1.3.fixture.
        ("com.zhiliaoapp.musically", "46.1.3") => Ok((":id/rnb", ":id/rmr")),
        // Phone13, Global46.2.42, 15/09/2026: complete caption rqb and
        // ImageView rpr with content-desc=Share on PostModeDetailActivity.
        ("com.zhiliaoapp.musically", "46.2.42") => Ok((":id/rqb", ":id/rpr")),
        ("com.zhiliaoapp.musically", _) => Ok((":id/rs_", ":id/rrp")),
        _ => anyhow::bail!("photo viewer not measured for package"),
    }
}

/// Recognize only the measured full-caption surface. This permits navigation
/// back to the profile; publication still needs caption/account/ID/time proof.
pub(super) fn expanded_photo_surface(tree: &Tree, package: &str, version: &str) -> bool {
    let Ok((caption_id, share_id)) = viewer_controls(package, version) else {
        return false;
    };
    let captions = tree.matching(package, ElementQuery::ResourceIdSuffix(caption_id));
    let shares = tree.matching(package, ElementQuery::ResourceIdSuffix(share_id));
    let ([caption], [share]) = (captions.as_slice(), shares.as_slice()) else {
        return false;
    };
    tree.nodes[*caption].attr("class") == "android.widget.TextView"
        && !tree.nodes[*caption].attr("text").trim().is_empty()
        && tree.nodes[*share].attr("content-desc") == "Share"
        && tree.nodes[*share]
            .rect()
            .is_some_and(|rect| rect.enabled && rect.clickable)
}

pub async fn capture_expanded_photo_link(
    session: &dyn UiSession,
    package: &str,
    caption: &str,
    identity: &SubmissionIdentity,
) -> anyhow::Result<String> {
    capture_expanded_photo_link_counted(session, package, caption, identity, &mut 0).await
}

pub(super) async fn capture_expanded_photo_link_counted(
    session: &dyn UiSession,
    package: &str,
    caption: &str,
    identity: &SubmissionIdentity,
    copy_attempts: &mut u32,
) -> anyhow::Result<String> {
    let tree = Tree::parse(session.hierarchy_source_snapshot().await?)?;
    anyhow::ensure!(
        !tree.publication_removed(package),
        "publication removed; skip sharing"
    );
    let version = session
        .app_version(package)
        .await
        .context("photo viewer version missing")?;
    let (caption_id, share_id) = viewer_controls(package, &version)?;
    let captions = tree.matching(package, ElementQuery::ResourceIdSuffix(caption_id));
    let matched_caption = matches!(captions.as_slice(), [_]);
    let link = if let [index] = captions.as_slice() {
        anyhow::ensure!(
            normalize(tree.nodes[*index].attr("text")) == normalize(caption),
            "expanded caption differs"
        );
        let mut opened = false;
        let link = super::read_through_sheet_counted(
            session,
            ElementQuery::ResourceIdSuffix(share_id),
            &mut opened,
            copy_attempts,
        )
        .await;
        if opened {
            super::close_sheet(session).await;
        }
        link
    } else {
        anyhow::ensure!(captions.is_empty(), "expanded caption not unique");
        let version = session
            .app_version(package)
            .await
            .context("post version missing")?;
        let locale = session.ui_language().await.context("post locale missing")?;
        let labels = crate::tiktok_labels::controls_for_runtime(package, &locale, &version)
            .context("post labels missing")?;
        // Some photo details hide their caption after the album hint animates.
        // Only inspect a measured post surface here. The public metadata below
        // still must match the complete submitted caption, account, ID and time.
        anyhow::ensure!(
            !tree
                .matching(
                    package,
                    ElementQuery::Description {
                        value: "Back",
                        exact: true
                    }
                )
                .is_empty()
                && labels
                    .label(TikTokControl::Comments)
                    .is_some_and(|q| !tree.matching(package, q.to_query()).is_empty()),
            "not a measured post detail"
        );
        if let Some(share) = labels.label(TikTokControl::Share) {
            let mut opened = false;
            let link = super::read_through_sheet_counted(
                session,
                share.to_query(),
                &mut opened,
                copy_attempts,
            )
            .await;
            if opened {
                super::close_sheet(session).await;
            }
            link
        } else {
            LinkCapture::ShareUnmeasured
        }
    };
    if matched_caption && link.link().is_none() {
        return Err(MatchedPhotoCopyFailure(link).into());
    }
    let canonical =
        resolve_canonical_post_link(link.link().context("expanded viewer did not return link")?)
            .await?;
    validate_photo_identity(&canonical, identity)?;
    validate_public_link(&canonical, caption, identity)
        .await
        .map_err(|error| {
            MatchedPhotoCopyFailure(LinkCapture::ReadFailed(format!(
                "public metadata for {canonical}: {error:#}"
            )))
        })?;
    Ok(canonical)
}

async fn validate_public_link(
    canonical: &str,
    caption: &str,
    identity: &SubmissionIdentity,
) -> anyhow::Result<()> {
    let id = validate_photo_identity(canonical, identity)?;
    // Measured 14/09/2026: photo ID 7685155573721025810 is exposed by video oEmbed.
    let video_url = format!(
        "https://www.tiktok.com/@{}/video/{id}",
        identity.account.trim_start_matches('@')
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut response = client
        .get("https://www.tiktok.com/oembed")
        .query(&[("url", video_url.as_str())])
        .send()
        .await?
        .error_for_status()?;
    anyhow::ensure!(
        response.content_length().is_none_or(|size| size <= 262144),
        "oembed too large"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(bytes.len() + chunk.len() <= 262144, "oembed too large");
        bytes.extend_from_slice(&chunk);
    }
    let embed: serde_json::Value = serde_json::from_slice(&bytes)?;
    validate_public_metadata(&embed, caption, &identity.account, &id)?;
    Ok(())
}

/// The video viewer truncates its caption and opens a comment drawer on expansion.
/// A prefix authorizes Copy only; publication proof comes from full public metadata
/// and the immutable account/content-id/time window of this submitted attempt.
pub async fn capture_visible_video_link(
    session: &dyn UiSession,
    package: &str,
    caption: &str,
    identity: &SubmissionIdentity,
) -> anyhow::Result<String> {
    capture_visible_video_link_counted(session, package, caption, identity, &mut 0).await
}

pub(super) async fn capture_visible_video_link_counted(
    session: &dyn UiSession,
    package: &str,
    caption: &str,
    identity: &SubmissionIdentity,
    copy_attempts: &mut u32,
) -> anyhow::Result<String> {
    let observed_at = std::time::Instant::now();
    anyhow::ensure!(
        session.active_app_bundle().await? == package,
        "video viewer foreground changed"
    );
    let tree = Tree::parse(session.hierarchy_source_snapshot().await?)?;
    let (caption_id, share) = video_viewer_caption(&tree, package, caption)?;
    let _ = caption_id;
    let mut opened = false;
    anyhow::ensure!(
        observed_at.elapsed() < Duration::from_secs(20)
            && session.active_app_bundle().await? == package,
        "video viewer observation expired or foreground changed"
    );
    let captured =
        super::read_through_sheet_counted(session, share, &mut opened, copy_attempts).await;
    if opened {
        super::close_sheet(session).await;
    }
    // A matching candidate whose Copy failed remains unresolved. Navigating
    // through older posts must not replace its processing/clipboard diagnosis.
    if captured.link().is_none() {
        return Err(MatchedPhotoCopyFailure(captured).into());
    }
    let canonical = resolve_canonical_post_link(
        captured
            .link()
            .context("video viewer did not return link")?,
    )
    .await
    .map_err(|error| {
        MatchedPhotoCopyFailure(LinkCapture::ReadFailed(format!(
            "matched video candidate link resolution: {error:#}"
        )))
    })?;
    anyhow::ensure!(
        url::Url::parse(&canonical)?.path().contains("/video/"),
        "copied content is not a video"
    );
    validate_photo_identity(&canonical, identity)?;
    validate_public_link(&canonical, caption, identity)
        .await
        .map_err(|error| {
            MatchedPhotoCopyFailure(LinkCapture::ReadFailed(format!(
                "public metadata for {canonical}: {error:#}"
            )))
        })?;
    Ok(canonical)
}

fn video_viewer_caption(
    tree: &Tree,
    package: &str,
    caption: &str,
) -> anyhow::Result<(&'static str, ElementQuery<'static>)> {
    anyhow::ensure!(!tree.publication_removed(package), "video removed");
    anyhow::ensure!(
        tree.nodes
            .iter()
            .any(|n| n.visible(package) && n.attr("content-desc") == "Video"),
        "not a video viewer"
    );
    let (caption_id, share) = match package {
        "com.ss.android.ugc.trill" => (
            ":id/dmk",
            ElementQuery::Description {
                value: "Share video",
                exact: false,
            },
        ),
        "com.zhiliaoapp.musically" => (
            ":id/desc",
            ElementQuery::Description {
                value: "Share video",
                exact: false,
            },
        ),
        _ => anyhow::bail!("video viewer package unmeasured"),
    };
    let rows = tree.matching(package, ElementQuery::ResourceIdSuffix(caption_id));
    let [index] = rows.as_slice() else {
        anyhow::bail!("video caption missing or ambiguous");
    };
    // Android accessibility can add direction marks at the caption edges.
    // Ignore only that presentation boundary for candidate selection; exact
    // complete public caption/account/content-id/time proof remains unchanged.
    let ui_caption = |value: &str| {
        normalize(
            value.trim_matches(|c: char| c.is_whitespace() || matches!(c, '\u{200e}' | '\u{200f}')),
        )
    };
    let visible = ui_caption(tree.nodes[*index].attr("text"));
    let expected = ui_caption(caption);
    let prefix = visible
        .strip_suffix("...")
        .or_else(|| visible.strip_suffix('…'))
        // Global 46.2.42/en, phone13 (15/09/2026): desc carries the
        // expansion affordance as one literal suffix, not a separate node.
        .or_else(|| {
            (package == "com.zhiliaoapp.musically")
                .then(|| visible.strip_suffix("…more"))
                .flatten()
        })
        .map(str::trim_end);
    anyhow::ensure!(
        visible == expected
            || prefix.is_some_and(|p| p.chars().count() >= 20 && expected.starts_with(p)),
        "video caption differs"
    );
    anyhow::ensure!(
        tree.matching(package, share).len() == 1,
        "video share control missing or ambiguous"
    );
    Ok((caption_id, share))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn video_prefix_only_allows_copy_and_public_metadata_still_requires_full_identity() {
        let package = "com.ss.android.ugc.trill";
        let xml = r#"<hierarchy><node package="com.ss.android.ugc.trill" content-desc="Video" bounds="[0,0][1080,1965]" displayed="true"/>
        <node package="com.ss.android.ugc.trill" resource-id="com.ss.android.ugc.trill:id/dmk" text="A complete matching caption with..." bounds="[10,1500][900,1700]" displayed="true"/>
        <node package="com.ss.android.ugc.trill" content-desc="Share video. 0 shares" bounds="[950,1200][1030,1300]" displayed="true" clickable="true" enabled="true"/></hierarchy>"#;
        let tree = Tree::parse(crate::HierarchySourceSnapshot {
            generation: 1,
            xml: xml.into(),
        })
        .unwrap();
        let caption = "A complete matching caption with the required suffix";
        assert!(video_viewer_caption(&tree, package, caption).is_ok());
        assert!(video_viewer_caption(&tree, package, "A totally different publication").is_err());
        let embed = serde_json::json!({"title":"A complete matching caption with DIFFERENT suffix","author_url":"https://www.tiktok.com/@fixture","html":"<blockquote data-video-id=\"123\">"});
        assert!(validate_public_metadata(&embed, caption, "fixture", "123").is_err());
    }
    #[test]
    fn video_candidate_caption_accepts_only_explicit_ellipsis_and_edge_direction_marks() {
        let package = "com.ss.android.ugc.trill";
        let expected =
            "Lưu list này rồi đi Đà Lạt cho đỡ mò từng nơi nhé. Lưu list này để chọn điểm";
        let observed = "Lưu list này rồi đi Đà Lạt cho đỡ mò từng nơi nhé. Lưu list ...";
        let xml = |text: &str, description: &str| {
            format!(
                r#"<hierarchy>
            <node package="{package}" content-desc="{description}" bounds="[0,0][1080,1965]" displayed="true"/>
            <node package="{package}" resource-id="{package}:id/dmk" text="{text}" bounds="[10,1500][900,1700]" displayed="true"/>
            <node package="{package}" content-desc="Share video.  shares" bounds="[950,1200][1030,1300]" displayed="true" clickable="true" enabled="true"/>
            </hierarchy>"#
            )
        };
        let parse =
            |xml| Tree::parse(crate::HierarchySourceSnapshot { generation: 1, xml }).unwrap();
        for caption in [
            observed.to_owned(),
            format!(" \u{200e}{observed}\u{200f} "),
            observed.replace("...", "…"),
        ] {
            assert!(
                video_viewer_caption(&parse(xml(&caption, "Video")), package, expected).is_ok()
            );
        }
        for caption in [
            "Lưu list ...",
            "Lưu list này rồi đi Đà Lạt nhưng nội dung khác...",
            "Lưu list này rồi đi Đà Lạt cho đỡ mò từng nơi nhé. Lưu list ...more",
        ] {
            assert!(
                video_viewer_caption(&parse(xml(caption, "Video")), package, expected).is_err()
            );
        }
        assert!(video_viewer_caption(&parse(xml(observed, "Videos")), package, expected).is_err());
        let good = xml(observed, "Video");
        let duplicate=good.replace("</hierarchy>",&format!(r#"<node package="{package}" resource-id="{package}:id/dmk" text="{observed}" bounds="[10,1200][900,1400]" displayed="true"/></hierarchy>"#));
        assert!(video_viewer_caption(&parse(duplicate), package, expected).is_err());
    }
    #[test]
    fn global_video_literal_more_suffix_only_authorizes_copy_not_publication_proof() {
        let package = "com.zhiliaoapp.musically";
        let visible = "Lưu list này rồi đi Đà Lạt cho đỡ mò từng nơi nhé.…more";
        let caption = "Lưu list này rồi đi Đà Lạt cho đỡ mò từng nơi nhé. Lưu list này để có lịch đi Đà Lạt gọn hơn, dễ chọn điểm theo buổi và đỡ mất thời gian mò từng nơi. #riviudalat #dalat #dalatreview #72hdalat #dulichdalat";
        let xml = format!(
            r#"<hierarchy>
          <node package="{package}" content-desc="Video" resource-id="{package}:id/long_press_layout" bounds="[0,0][1080,1965]" displayed="true"/>
          <node package="{package}" resource-id="{package}:id/desc" text="{visible}" bounds="[10,1500][900,1700]" displayed="true"/>
          <node package="{package}" content-desc="Share video.  shares" resource-id="{package}:id/fwo" bounds="[950,1200][1030,1300]" displayed="true" clickable="true" enabled="true"/>
        </hierarchy>"#
        );
        let tree = Tree::parse(crate::HierarchySourceSnapshot { generation: 1, xml }).unwrap();
        assert!(video_viewer_caption(&tree, package, caption).is_ok());
        assert!(video_viewer_caption(&tree, package, "Một bài khác cùng tài khoản").is_err());
        let embed = serde_json::json!({"title":visible,"author_url":"https://www.tiktok.com/@fixture","html":"<blockquote data-video-id=\"123\">"});
        assert!(validate_public_metadata(&embed, caption, "fixture", "123").is_err());
        let mut full = embed;
        full["title"] = caption.into();
        assert!(validate_public_metadata(&full, caption, "fixture", "123").is_ok());
        full["author_url"] = "https://www.tiktok.com/@different".into();
        assert!(validate_public_metadata(&full, caption, "fixture", "123").is_err());
    }
    #[test]
    fn measured_global_46_0_41_expanded_photo_matches_caption_and_share() {
        let package = "com.zhiliaoapp.musically";
        let tree = Tree::parse(crate::HierarchySourceSnapshot {
            generation: 1,
            xml: include_str!("../../fixtures/tiktok-publish/expanded-photo-46.0.41.fixture")
                .into(),
        })
        .unwrap();
        let (caption, share) = viewer_controls(package, "46.0.41").unwrap();
        let captions = tree.matching(package, ElementQuery::ResourceIdSuffix(caption));
        assert_eq!(
            captions.len(),
            1,
            "Full caption must be recognized before Copy"
        );
        assert_eq!(
            tree.nodes[captions[0]].attr("text"),
            "Complete fixture caption identifying the submitted photo post"
        );
        let shares = tree.matching(package, ElementQuery::ResourceIdSuffix(share));
        assert_eq!(shares.len(), 1);
        assert!(tree.nodes[shares[0]]
            .rect()
            .is_some_and(|r| r.enabled && r.clickable));
        assert_ne!(
            viewer_controls(package, "46.2.1").unwrap(),
            (caption, share)
        );
    }
    #[test]
    fn measured_global_45_7_3_expanded_photo_requires_its_exact_controls() {
        let package = "com.zhiliaoapp.musically";
        let xml = include_str!("../../fixtures/tiktok-publish/expanded-photo-45.7.3.fixture");
        let observed = |xml: String| {
            Tree::parse(crate::HierarchySourceSnapshot { generation: 1, xml }).unwrap()
        };
        let tree = observed(xml.into());
        let (caption, share) = viewer_controls(package, "45.7.3").unwrap();
        assert_eq!(
            tree.matching(package, ElementQuery::ResourceIdSuffix(caption))
                .len(),
            1
        );
        assert_eq!(
            tree.matching(package, ElementQuery::ResourceIdSuffix(share))
                .len(),
            1
        );
        assert!(expanded_photo_surface(&tree, package, "45.7.3"));
        for changed in [
            xml.replace(":id/rey", ":id/other"),
            xml.replace(":id/red", ":id/other"),
            xml.replace("displayed=\"true\"", "displayed=\"false\""),
            xml.replace("clickable=\"true\"", "clickable=\"false\""),
            xml.replace("content-desc=\"Share\"", "content-desc=\"Like\""),
        ] {
            assert!(!expanded_photo_surface(
                &observed(changed),
                package,
                "45.7.3"
            ));
        }
        assert!(!expanded_photo_surface(&tree, package, "46.0.41"));
    }
    #[test]
    fn measured_global_45_4_3_expanded_photo_requires_its_exact_controls() {
        let package = "com.zhiliaoapp.musically";
        let xml = include_str!("../../fixtures/tiktok-publish/expanded-photo-45.4.3.fixture");
        let observed = |xml: String| {
            Tree::parse(crate::HierarchySourceSnapshot { generation: 1, xml }).unwrap()
        };
        let tree = observed(xml.into());
        let (caption, share) = viewer_controls(package, "45.4.3").unwrap();
        assert_eq!(
            tree.matching(package, ElementQuery::ResourceIdSuffix(caption))
                .len(),
            1
        );
        assert_eq!(
            tree.matching(package, ElementQuery::ResourceIdSuffix(share))
                .len(),
            1
        );
        assert!(expanded_photo_surface(&tree, package, "45.4.3"));
        for changed in [
            xml.replace(":id/r55", ":id/other"),
            xml.replace(":id/r4k", ":id/other"),
            xml.replace("displayed=\"true\"", "displayed=\"false\""),
            xml.replace("clickable=\"true\"", "clickable=\"false\""),
            xml.replace("content-desc=\"Share\"", "content-desc=\"Like\""),
            xml.replace(
                "</hierarchy>",
                &format!(
                    "{} </hierarchy>",
                    xml.split("<hierarchy>")
                        .nth(1)
                        .unwrap()
                        .split("</hierarchy>")
                        .next()
                        .unwrap()
                ),
            ),
        ] {
            assert!(!expanded_photo_surface(
                &observed(changed),
                package,
                "45.4.3"
            ));
        }
        assert!(!expanded_photo_surface(&tree, package, "46.0.41"));
    }
    #[test]
    fn global_46_2_42_viewer_uses_its_own_caption_and_share_controls() {
        let package = "com.zhiliaoapp.musically";
        let (caption, share) = viewer_controls(package, "46.2.42").unwrap();
        let tree=Tree::parse(crate::HierarchySourceSnapshot{generation:1,xml:r#"<hierarchy>
          <node package="com.zhiliaoapp.musically" resource-id="com.zhiliaoapp.musically:id/rqb" class="android.widget.TextView" text="Complete caption" bounds="[0,1704][1080,1965]" displayed="true"/>
          <node package="com.zhiliaoapp.musically" resource-id="com.zhiliaoapp.musically:id/rpr" class="android.widget.ImageView" content-desc="Share" bounds="[959,1982][1027,2050]" clickable="true" enabled="true" displayed="true"/>
        </hierarchy>"#.into()}).unwrap();
        assert_eq!(
            tree.matching(package, ElementQuery::ResourceIdSuffix(caption))
                .len(),
            1
        );
        assert_eq!(
            tree.matching(package, ElementQuery::ResourceIdSuffix(share))
                .len(),
            1
        );
        assert_ne!(
            viewer_controls(package, "46.2.1").unwrap(),
            (caption, share)
        );
    }
    fn identity() -> SubmissionIdentity {
        SubmissionIdentity {
            account: "fixture".into(),
            submitted_at: "2026-01-01T12:00:58Z".into(),
            prepared_at: Some("2026-01-01T12:00:06Z".into()),
        }
    }
    fn link(at: &str) -> String {
        let id = (chrono::DateTime::parse_from_rfc3339(at)
            .unwrap()
            .timestamp() as u64)
            << 32;
        format!("https://www.tiktok.com/@fixture/photo/{id}")
    }
    #[test]
    fn photo_allocated_before_post_requires_the_recorded_preparation_window() {
        let url = link("2026-01-01T12:00:09Z");
        let mut proof = identity();
        assert!(validate_photo_identity(&url, &proof).is_ok());
        proof.prepared_at = None;
        assert!(validate_photo_identity(&url, &proof).is_err());
    }
    #[test]
    fn old_post_wrong_author_or_invalid_preparation_never_match() {
        let mut proof = identity();
        assert!(validate_photo_identity(&link("2026-01-01T11:59:59Z"), &proof).is_err());
        assert!(validate_photo_identity(&link("2026-01-01T12:31:00Z"), &proof).is_err());
        assert!(validate_photo_identity(
            &link("2026-01-01T12:01:00Z").replace("@fixture", "@other"),
            &proof
        )
        .is_err());
        proof.prepared_at = Some("2026-01-01T12:01:00Z".into());
        assert!(validate_photo_identity(&link("2026-01-01T12:01:00Z"), &proof).is_err());
        proof.prepared_at = Some("2025-12-31T12:00:00Z".into());
        assert!(validate_photo_identity(&link("2026-01-01T12:01:00Z"), &proof).is_err());
    }
    #[test]
    fn public_metadata_requires_complete_caption_author_and_same_id() {
        let valid = serde_json::json!({"title":"Nội dung đầy đủ #dalat","author_url":"https://www.tiktok.com/@fixture","html":"<blockquote data-video-id=\"123\">"});
        assert!(
            validate_public_metadata(&valid, "Nội dung đầy đủ\n#dalat", "fixture", "123").is_ok()
        );
        for (field, value) in [
            ("title", "Nội dung..."),
            ("author_url", "https://www.tiktok.com/@someone"),
            ("html", "<blockquote data-video-id=\"456\">"),
        ] {
            let mut bad = valid.clone();
            bad[field] = value.into();
            assert!(
                validate_public_metadata(&bad, "Nội dung đầy đủ #dalat", "fixture", "123").is_err()
            );
        }
    }
}
