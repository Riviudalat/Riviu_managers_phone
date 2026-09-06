//! Read-only account proof for a measured own-profile screen. Never changes the login.
use crate::tiktok_labels::{LabelMatch, TikTokControls};
use crate::{ElementBox, ElementQuery, UiSession};

fn single_username(elements: &[ElementBox]) -> Option<String> {
    let [element] = elements else {
        return None;
    };
    let text = element.description.as_deref()?.trim();
    let handle = text.strip_prefix('@')?;
    (!handle.is_empty()
        && handle.len() <= 24
        && !handle.ends_with('.')
        && handle
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
        && element.width > 0.0
        && element.height > 0.0)
        .then(|| handle.to_owned())
}

pub fn account_read_supported(labels: TikTokControls) -> bool {
    (
        labels.package(),
        labels.resource_version(),
        labels.language(),
    ) == ("com.ss.android.ugc.trill", Some("38.3.2"), "en")
}

async fn read_once(session: &dyn UiSession) -> anyhow::Result<Option<String>> {
    // Machine 2, 06/09/2026: own profile mjf Button contains @username; dby TextView
    // contains Edit profile. Neither arbitrary bio @mentions nor another user's profile qualifies.
    if session
        .locate_all_described(ElementQuery::Text {
            value: "Edit profile",
            exact: true,
        })
        .await?
        .len()
        != 1
    {
        return Ok(None);
    }
    let nodes = session
        .locate_all_described(LabelMatch::ResourceId(":id/mjf").to_query())
        .await?;
    Ok(single_username(&nodes))
}

pub async fn observe_own_account(
    session: &dyn UiSession,
    labels: TikTokControls,
) -> anyhow::Result<Option<String>> {
    if !account_read_supported(labels) {
        return Ok(None);
    }
    if session.active_app_bundle().await? != labels.package() {
        return Ok(None);
    }
    let before = read_once(session).await?;
    let after = read_once(session).await?;
    Ok(before.filter(|handle| Some(handle) == after.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(text: &str) -> ElementBox {
        ElementBox {
            x: 386.0,
            y: 552.0,
            width: 308.0,
            height: 50.0,
            description: Some(text.into()),
            enabled: true,
            clickable: true,
        }
    }
    #[test]
    fn rejects_display_names_urls_and_ambiguous_accounts() {
        assert_eq!(
            single_username(&[node("@valid.account")]).as_deref(),
            Some("valid.account")
        );
        for text in [
            "Display Name",
            "https://tiktok.com/@valid",
            "@",
            "@with space",
            "@@wrong",
            "@trailing.",
        ] {
            assert!(single_username(&[node(text)]).is_none());
        }
        assert!(single_username(&[node("@a"), node("@b")]).is_none());
    }
    #[test]
    fn unmeasured_build_and_language_never_claim_a_profile_locator() {
        use crate::tiktok_labels::controls_for;
        assert!(account_read_supported(
            controls_for("com.ss.android.ugc.trill", "en", "38.3.2").unwrap()
        ));
        assert!(!account_read_supported(
            controls_for("com.ss.android.ugc.trill", "vi", "38.3.2").unwrap()
        ));
        assert!(!account_read_supported(
            controls_for("com.ss.android.ugc.trill", "en", "46.3.3").unwrap()
        ));
    }
}
