//! Read-only account proof for a measured own-profile screen. Never changes the login.
use crate::tiktok_labels::{LabelMatch, TikTokControls};
use crate::{ElementBox, ElementQuery, UiSession};
use quick_xml::{events::Event, Reader, XmlVersion};
use std::collections::HashMap;

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
    if labels.package() == "com.zhiliaoapp.musically" && labels.language() == "en" {
        return global_username_id(labels.resource_version()).is_some();
    }
    matches!(
        (
            labels.package(),
            labels.resource_version(),
            labels.language(),
        ),
        ("com.ss.android.ugc.trill", Some("38.3.2"), "en")
            | ("com.zhiliaoapp.musically", Some("45.7.3"), "en")
    )
}

fn global_username_id(version: Option<&str>) -> Option<&'static str> {
    match version? {
        "45.4.3" => Some("rq1"),
        "45.7.3" => Some("s0v"),
        "46.0.41" => Some("s7b"),
        "46.1.3" => Some("s8i"),
        "46.2.1" => Some("scn"),
        // AGENTS.md §9.210: ce0517155ab38c390d own Profile snapshot, 10/09/2026.
        "46.2.42" => Some("sao"),
        "46.4.3" => Some("sj8"),
        _ => None,
    }
}

// Measured 08/09/2026, Global 45.7.3/en, SM-G955F Android 9: Edit and Profile menu
// are Buttons without ids; only the username has :id/s0v. All three must belong to
// one source read. Separate locator requests could combine two different screens.
fn global_profile_from_snapshot_with_id(
    xml: &str,
    username_id: &str,
) -> anyhow::Result<Option<String>> {
    anyhow::ensure!(xml.len() <= 16 * 1024 * 1024, "account snapshot size limit");
    let mut reader = Reader::from_str(xml);
    let mut nodes = 0usize;
    let mut depth = 0usize;
    let mut edits = 0usize;
    let mut menus = 0usize;
    let mut usernames = Vec::new();
    loop {
        let event = reader.read_event()?;
        if matches!(event, Event::Start(_)) {
            depth += 1;
            anyhow::ensure!(depth <= 256, "account snapshot depth limit");
        }
        match event {
            Event::Start(node) | Event::Empty(node) => {
                nodes += 1;
                anyhow::ensure!(nodes <= 32768, "account snapshot node limit");
                let mut attrs = HashMap::new();
                for attribute in node.attributes() {
                    let attribute = attribute?;
                    attrs.insert(
                        std::str::from_utf8(attribute.key.as_ref())?.to_owned(),
                        attribute
                            .decoded_and_normalized_value(
                                XmlVersion::Implicit1_0,
                                reader.decoder(),
                            )?
                            .into_owned(),
                    );
                }
                let attr = |name: &str| attrs.get(name).map(String::as_str).unwrap_or("");
                if attr("package") != "com.zhiliaoapp.musically"
                    || attr("class") != "android.widget.Button"
                    || attr("displayed") != "true"
                    || attr("enabled") != "true"
                    || attr("clickable") != "true"
                {
                    continue;
                }
                let edit = attr("text") == "Edit";
                let menu = attr("content-desc") == "Profile menu";
                let username =
                    attr("resource-id") == format!("com.zhiliaoapp.musically:id/{username_id}");
                if !edit && !menu && !username {
                    continue;
                }
                let Some((left, right)) = attr("bounds")
                    .strip_prefix('[')
                    .and_then(|value| value.strip_suffix(']'))
                    .and_then(|value| value.split_once("]["))
                else {
                    return Ok(None);
                };
                let Some((x, y)) = left.split_once(',') else {
                    return Ok(None);
                };
                let Some((right, bottom)) = right.split_once(',') else {
                    return Ok(None);
                };
                let (x, y, right, bottom) = (
                    x.parse::<f64>()?,
                    y.parse::<f64>()?,
                    right.parse::<f64>()?,
                    bottom.parse::<f64>()?,
                );
                if ![x, y, right, bottom].iter().all(|v| v.is_finite())
                    || x < 0.0
                    || y < 0.0
                    || right <= x
                    || bottom <= y
                {
                    return Ok(None);
                }
                edits += usize::from(edit);
                menus += usize::from(menu);
                if username {
                    usernames.push(ElementBox {
                        x,
                        y,
                        width: right - x,
                        height: bottom - y,
                        description: Some(attr("text").to_owned()),
                        enabled: true,
                        clickable: true,
                    });
                }
            }
            Event::End(_) => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| anyhow::anyhow!("account snapshot nesting"))?;
            }
            Event::DocType(_) => anyhow::bail!("doctype in account snapshot"),
            Event::Eof => {
                anyhow::ensure!(depth == 0, "incomplete account snapshot");
                break;
            }
            _ => {}
        }
    }
    Ok((edits == 1 && menus == 1)
        .then(|| single_username(&usernames))
        .flatten())
}

async fn read_once(
    session: &dyn UiSession,
    labels: TikTokControls,
) -> anyhow::Result<Option<String>> {
    if labels.package() == "com.zhiliaoapp.musically" {
        let id = global_username_id(labels.resource_version())
            .ok_or_else(|| anyhow::anyhow!("unmeasured account build"))?;
        return global_profile_from_snapshot_with_id(
            &session.hierarchy_source_snapshot().await?.xml,
            id,
        );
    }
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
    let before = read_once(session, labels).await?;
    let after = read_once(session, labels).await?;
    if session.active_app_bundle().await? != labels.package() {
        return Ok(None);
    }
    Ok(before.filter(|handle| Some(handle) == after.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_fleet_accounts_require_their_measured_username_id_and_own_profile_controls() {
        for (version, xml) in [
            (
                "46.2.42",
                include_str!("../fixtures/tiktok-publish/musically-46.2.42-en/profile.xml"),
            ),
            (
                "46.0.41",
                include_str!("../fixtures/tiktok-publish/musically-46.0.41-en/profile.xml"),
            ),
            (
                "46.2.1",
                include_str!("../fixtures/tiktok-publish/musically-46.2.1-en/profile.xml"),
            ),
            (
                "45.4.3",
                include_str!("../fixtures/tiktok-publish/musically-45.4.3-en/profile.xml"),
            ),
            (
                "46.1.3",
                include_str!("../fixtures/tiktok-publish/musically-46.1.3-en/profile.xml"),
            ),
            (
                "46.4.3",
                include_str!("../fixtures/tiktok-publish/musically-46.4.3-en/profile.xml"),
            ),
        ] {
            let id = global_username_id(Some(version)).unwrap();
            assert_eq!(
                global_profile_from_snapshot_with_id(xml, id)
                    .unwrap()
                    .as_deref(),
                Some("fixture.account")
            );
            assert!(global_profile_from_snapshot_with_id(xml, "s0v")
                .unwrap()
                .is_none());
            assert!(global_profile_from_snapshot_with_id(
                &xml.replace("Profile menu", "Other menu"),
                id
            )
            .unwrap()
            .is_none());
        }
        assert!(global_username_id(Some("46.4.4")).is_none());
    }
    fn global_profile_from_snapshot(xml: &str) -> anyhow::Result<Option<String>> {
        global_profile_from_snapshot_with_id(xml, "s0v")
    }
    use crate::tiktok_labels::controls_for;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    const PROFILE: &str = include_str!("../../../fixtures/tiktok/account-musically-45.7.3-en.xml");

    struct ProfileSession {
        snapshots: Mutex<VecDeque<String>>,
        packages: Mutex<VecDeque<String>>,
    }

    impl ProfileSession {
        fn global(before: &str, after: &str) -> Self {
            Self {
                snapshots: Mutex::new(VecDeque::from([before.to_owned(), after.to_owned()])),
                packages: Mutex::new(VecDeque::from([
                    "com.zhiliaoapp.musically".to_owned(),
                    "com.zhiliaoapp.musically".to_owned(),
                ])),
            }
        }
    }

    #[async_trait::async_trait]
    impl UiSession for ProfileSession {
        async fn tap(&self, _: crate::TapPoint) -> anyhow::Result<()> {
            panic!("account observation must not tap")
        }
        async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
            panic!("account observation must not swipe")
        }
        async fn type_text(&self, _: &str) -> anyhow::Result<()> {
            panic!("account observation must not type")
        }
        async fn home(&self) -> anyhow::Result<()> {
            panic!("account observation must not navigate")
        }
        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
            panic!("account observation must not tap")
        }
        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
            panic!("account observation requires complete proof")
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
        async fn active_app_bundle(&self) -> anyhow::Result<String> {
            Ok(self
                .packages
                .lock()
                .unwrap()
                .pop_front()
                .expect("foreground proof"))
        }
        async fn hierarchy_source_snapshot(
            &self,
        ) -> anyhow::Result<crate::HierarchySourceSnapshot> {
            let mut snapshots = self.snapshots.lock().unwrap();
            Ok(crate::HierarchySourceSnapshot {
                generation: 3 - snapshots.len() as u64,
                xml: snapshots.pop_front().expect("two source reads"),
            })
        }
        async fn locate_all_described(
            &self,
            query: ElementQuery<'_>,
        ) -> anyhow::Result<Vec<ElementBox>> {
            // Legacy Trill uses its existing exact Edit profile/mjf locator route.
            Ok(match query {
                ElementQuery::Text {
                    value: "Edit profile",
                    exact: true,
                } => vec![node("Edit profile")],
                ElementQuery::ResourceIdSuffix(":id/mjf") => vec![node("@legacy.account")],
                _ => panic!("unmeasured account locator"),
            })
        }
    }

    #[test]
    fn measured_global_profile_requires_its_three_unique_controls() {
        assert_eq!(
            global_profile_from_snapshot(PROFILE).unwrap().as_deref(),
            Some("fixture.account")
        );
        for changed in [
            PROFILE.replace("text=\"Edit\"", "text=\"Edit profile\""),
            PROFILE.replace("Profile menu", "Profile menu help"),
            PROFILE.replace(":id/s0v", ":id/other"),
            PROFILE.replace("text=\"Edit\" clickable=\"true\"", "text=\"Edit\" clickable=\"false\""),
            PROFILE.replace("[423,510][656,549]", "[423,510][423,549]"),
            PROFILE.replace("resource-id=\"com.zhiliaoapp.musically:id/s0v\" clickable=\"true\" enabled=\"true\"", "resource-id=\"com.zhiliaoapp.musically:id/s0v\" clickable=\"true\" enabled=\"false\""),
            PROFILE.replace("com.zhiliaoapp.musically", "com.other.app"),
            PROFILE.replace("android.widget.Button", "android.widget.TextView"),
            PROFILE.replace("text=\"@bio.mention\"", "text=\"Edit\""),
            PROFILE.replace("text=\"@bio.mention\"", "text=\"@other.account\" resource-id=\"com.zhiliaoapp.musically:id/s0v\""),
            PROFILE.replace("text=\"@bio.mention\"", "text=\"\" content-desc=\"Profile menu\""),
            PROFILE.replace("displayed=\"true\"", "displayed=\"false\""),
        ] {
            assert_eq!(global_profile_from_snapshot(&changed).unwrap(), None);
        }
        assert!(global_profile_from_snapshot(&PROFILE.replace("</hierarchy>", "")).is_err());
    }

    #[tokio::test]
    async fn measured_global_account_is_repeated_without_effects() {
        let session = ProfileSession::global(PROFILE, PROFILE);
        let labels = controls_for("com.zhiliaoapp.musically", "en", "45.7.3").unwrap();
        assert_eq!(
            observe_own_account(&session, labels)
                .await
                .unwrap()
                .as_deref(),
            Some("fixture.account")
        );
        assert!(session.snapshots.lock().unwrap().is_empty());
        assert!(session.packages.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn changed_account_or_own_profile_proof_never_returns_a_handle() {
        let labels = controls_for("com.zhiliaoapp.musically", "en", "45.7.3").unwrap();
        for after in [
            PROFILE.replace("@fixture.account", "@changed.account"),
            PROFILE.replace("Profile menu", "Share profile"),
        ] {
            assert_eq!(
                observe_own_account(&ProfileSession::global(PROFILE, &after), labels)
                    .await
                    .unwrap(),
                None
            );
        }
        let session = ProfileSession::global(PROFILE, PROFILE);
        *session.packages.lock().unwrap() = VecDeque::from([
            "com.zhiliaoapp.musically".to_owned(),
            "com.other.app".to_owned(),
        ]);
        assert_eq!(observe_own_account(&session, labels).await.unwrap(), None);
    }

    #[tokio::test]
    async fn unmeasured_global_build_language_and_wrong_foreground_do_not_read_sources() {
        assert!(controls_for("com.zhiliaoapp.musically", "vi", "45.7.3").is_none());
        for (language, version) in [("en", "46.0.42"), ("en", "45.7.4")] {
            let session = ProfileSession::global(PROFILE, PROFILE);
            let labels = controls_for("com.zhiliaoapp.musically", language, version).unwrap();
            assert_eq!(observe_own_account(&session, labels).await.unwrap(), None);
            assert_eq!(session.snapshots.lock().unwrap().len(), 2);
            assert_eq!(session.packages.lock().unwrap().len(), 2);
        }
        let session = ProfileSession::global(PROFILE, PROFILE);
        session.packages.lock().unwrap()[0] = "com.other.app".to_owned();
        assert_eq!(
            observe_own_account(
                &session,
                controls_for("com.zhiliaoapp.musically", "en", "45.7.3").unwrap()
            )
            .await
            .unwrap(),
            None
        );
        assert_eq!(session.snapshots.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn legacy_trill_keeps_its_measured_profile_locators() {
        let session = ProfileSession::global(PROFILE, PROFILE);
        *session.packages.lock().unwrap() = VecDeque::from([
            "com.ss.android.ugc.trill".to_owned(),
            "com.ss.android.ugc.trill".to_owned(),
        ]);
        assert_eq!(
            observe_own_account(
                &session,
                controls_for("com.ss.android.ugc.trill", "en", "38.3.2").unwrap()
            )
            .await
            .unwrap()
            .as_deref(),
            Some("legacy.account")
        );
        assert_eq!(session.snapshots.lock().unwrap().len(), 2);
    }
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
