//! Measured expanded photo viewer plus independent public metadata proof.
use super::*;
use anyhow::Context;
use hierarchy::Tree;

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

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

pub async fn capture_expanded_photo_link(
    session: &dyn UiSession,
    package: &str,
    caption: &str,
    identity: &SubmissionIdentity,
) -> anyhow::Result<String> {
    let tree = Tree::parse(session.hierarchy_source_snapshot().await?)?;
    let (caption_id, share_id) = match package {
        "com.ss.android.ugc.trill" => (":id/m3q", ":id/m3h"),
        "com.zhiliaoapp.musically" => (":id/rs_", ":id/rrp"),
        _ => anyhow::bail!("photo viewer not measured for package"),
    };
    let captions = tree.matching(package, ElementQuery::ResourceIdSuffix(caption_id));
    let link = if let [index] = captions.as_slice() {
        anyhow::ensure!(
            normalize(tree.nodes[*index].attr("text")) == normalize(caption),
            "expanded caption differs"
        );
        let mut opened = false;
        let link = super::read_through_sheet(
            session,
            ElementQuery::ResourceIdSuffix(share_id),
            &mut opened,
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
        super::capture_post_link(session, &labels).await
    };
    let canonical =
        resolve_canonical_post_link(link.link().context("expanded viewer did not return link")?)
            .await?;
    let id = validate_photo_identity(&canonical, identity)?;
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
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
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
