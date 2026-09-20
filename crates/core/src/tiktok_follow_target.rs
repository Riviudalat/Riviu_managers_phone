//! Follow the exact author profile reached from a separately proven post.
use crate::{ui_automation::tree::Tree, ElementBox, ElementQuery, UiSession};
use anyhow::{ensure, Context};
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Relationship {
    NotFollowing,
    Following,
}
pub(crate) struct Profile {
    pub handle: String,
    pub state: Relationship,
    pub button: ElementBox,
}

pub fn supported(package: &str, version: &str, locale: &str) -> bool {
    locale.split(['-', '_']).next() == Some("en") && profile_ids(package, version).is_some()
}

fn profile_ids(package: &str, version: &str) -> Option<(&'static str, &'static str, &'static str)> {
    match (package, version) {
        ("com.ss.android.ugc.trill", "38.3.2") => Some((":id/mia", ":id/mjf", ":id/dby")),
        ("com.zhiliaoapp.musically", "45.4.3") => Some((":id/rok", ":id/rq1", ":id/f4m")),
        ("com.zhiliaoapp.musically", "45.7.3") => Some((":id/rzl", ":id/s0v", ":id/f72")),
        ("com.zhiliaoapp.musically", "46.0.41") => Some((":id/s5n", ":id/s7b", ":id/fad")),
        ("com.zhiliaoapp.musically", "46.1.3") => Some((":id/s6u", ":id/s8i", ":id/faz")),
        ("com.zhiliaoapp.musically", "46.4.3") => Some((":id/shj", ":id/sj8", ":id/ff8")),
        _ => None,
    }
}

/// Only controls inside the measured header of the exact canonical handle.
pub(crate) fn profile(
    tree: &Tree,
    package: &str,
    version: &str,
    locale: &str,
    expected: &str,
) -> anyhow::Result<Profile> {
    ensure!(
        supported(package, version, locale),
        "Follow profile tuple unmeasured"
    );
    let (header_id, handle_id, label_id) =
        profile_ids(package, version).context("Unmeasured profile")?;
    let headers = tree.matching(package, ElementQuery::ResourceIdSuffix(header_id));
    let handles = tree.matching(package, ElementQuery::ResourceIdSuffix(handle_id));
    let ([header], [handle]) = (headers.as_slice(), handles.as_slice()) else {
        anyhow::bail!("Follow profile identity missing or ambiguous")
    };
    let name = tree.nodes[*handle]
        .attr("text")
        .trim()
        .trim_start_matches('@');
    ensure!(
        tree.inside(*handle, *header)
            && name.eq_ignore_ascii_case(expected.trim_start_matches('@')),
        "Follow profile account mismatch"
    );
    ensure!(
        !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_'),
        "Follow handle invalid"
    );
    let mut controls = Vec::new();
    for index in tree.matching(package, ElementQuery::ResourceIdSuffix(label_id)) {
        if !tree.inside(index, *header) {
            continue;
        }
        let text = tree.nodes[index].attr("text");
        let state = match text {
            "Follow" | "Follow back" => Relationship::NotFollowing,
            "Following" | "Friends" => Relationship::Following,
            _ => continue,
        };
        let mut current = Some(index);
        let mut button = None;
        for _ in 0..5 {
            let Some(i) = current else { break };
            if i == *header {
                break;
            }
            let n = &tree.nodes[i];
            if !n.visible(package) || !tree.ancestors_visible(i) {
                break;
            }
            if let Some(rect) = n.rect().filter(|r| r.enabled && r.clickable) {
                let labels = tree
                    .matching(package, ElementQuery::ResourceIdSuffix(label_id))
                    .into_iter()
                    .filter(|child| tree.inside(*child, i))
                    .count();
                ensure!(labels == 1, "Follow control includes another action");
                button = Some(rect);
                break;
            }
            current = n.parent;
        }
        if let Some(button) = button {
            controls.push((state, button));
        }
    }
    // The measured post-Follow profile replaces the text button with a
    // non-clickable relationship icon inside its clickable control. The adjacent
    // suggestions icon has the same resource ID but is itself clickable.
    let icons = match (package, version) {
        ("com.ss.android.ugc.trill", "38.3.2") => Some((":id/dbu", ":id/dbx", ":id/dbt")),
        // ce04171435f104080c, 20/09/2026: relationship icon beside Message;
        // the adjacent suggestions icon shares f4i but is itself clickable.
        ("com.zhiliaoapp.musically", "45.4.3") => Some((":id/f4i", ":id/f4l", ":id/f4g")),
        ("com.zhiliaoapp.musically", "45.7.3") => Some((":id/f6y", ":id/f71", ":id/f6w")),
        ("com.zhiliaoapp.musically", "46.0.41") => Some((":id/fa_", ":id/fac", ":id/fa8")),
        ("com.zhiliaoapp.musically", "46.1.3") => Some((":id/fav", ":id/fay", ":id/fat")),
        ("com.zhiliaoapp.musically", "46.4.3") => Some((":id/ff4", ":id/ff7", ":id/ff2")),
        _ => None,
    };
    let (icon_id, icon_parent, icon_group) =
        icons.unwrap_or(("unmeasured", "unmeasured", "unmeasured"));
    for index in tree.matching(package, ElementQuery::ResourceIdSuffix(icon_id)) {
        let node = &tree.nodes[index];
        if !tree.inside(index, *header)
            || node.attr("class") != "android.widget.ImageView"
            || node.attr("clickable") != "false"
            || !node.rect().is_some_and(|r| r.enabled)
            || !tree.ancestors_visible(index)
        {
            continue;
        }
        let Some(parent) = node.parent else { continue };
        let Some(group) = tree.nodes[parent].parent else {
            continue;
        };
        if !tree.nodes[parent]
            .attr("resource-id")
            .ends_with(icon_parent)
            || !tree.nodes[group].attr("resource-id").ends_with(icon_group)
        {
            continue;
        }
        let Some(wrapper) = tree.nodes[group].parent else {
            continue;
        };
        let Some(control) = tree.nodes[wrapper].parent else {
            continue;
        };
        let Some(row) = tree.nodes[control].parent else {
            continue;
        };
        let Some(button) = tree.nodes[control]
            .rect()
            .filter(|r| r.enabled && r.clickable)
        else {
            continue;
        };
        ensure!(
            tree.inside(row, *header),
            "Follow icon outside profile header"
        );
        let labels = tree.matching(package, ElementQuery::ResourceIdSuffix(label_id));
        let row_labels: Vec<_> = labels
            .into_iter()
            .filter(|i| tree.inside(*i, row))
            .collect();
        ensure!(
            row_labels.len() == 1
                && matches!(
                    tree.nodes[row_labels[0]].attr("text").trim(),
                    "Message" | "Send a 👋"
                ),
            "Follow icon conflicts with relationship text"
        );
        controls.push((Relationship::Following, button));
    }
    ensure!(
        controls.len() == 1,
        "Follow relationship control missing or ambiguous"
    );
    let (state, button) = controls.pop().context("Follow control missing")?;
    Ok(Profile {
        handle: name.to_ascii_lowercase(),
        state,
        button,
    })
}

/// Read the exact canonical author profile. The caller first proves the source
/// post and actor. No action gate or relationship button is touched here.
pub async fn observe_follow_profile(
    session: &dyn UiSession,
    labels: crate::tiktok_labels::TikTokControls,
    target: &crate::ResolvedTikTokTarget,
    account: &str,
) -> anyhow::Result<&'static str> {
    let author = crate::publish_submission::normalize_publish_account(&target.author)?;
    ensure!(
        !account.trim().is_empty()
            && !account
                .trim()
                .trim_start_matches('@')
                .eq_ignore_ascii_case(&author),
        "Cannot inspect own or unknown relationship"
    );
    let version = session
        .app_version(labels.package())
        .await
        .context("Follow version missing")?;
    ensure!(
        supported(labels.package(), &version, labels.language()),
        "Follow tuple unavailable"
    );
    let epoch = session.gui_session_epoch();
    session
        .reopen_url_in_app(
            &format!("https://www.tiktok.com/@{author}"),
            labels.package(),
        )
        .await?;
    let until = tokio::time::Instant::now() + Duration::from_secs(12);
    let mut previous: Option<(Relationship, u64)> = None;
    while tokio::time::Instant::now() < until {
        ensure!(
            session.gui_session_epoch() == epoch
                && session.active_app_bundle().await? == labels.package(),
            "Follow readback session changed"
        );
        let tree = Tree::parse(session.hierarchy_source_snapshot().await?)?;
        if let Ok(read) = profile(
            &tree,
            labels.package(),
            &version,
            labels.language(),
            &author,
        ) {
            if previous.as_ref().is_some_and(|(state, generation)| {
                *state == read.state && tree.generation > *generation
            }) {
                return Ok(if read.state == Relationship::Following {
                    "present"
                } else {
                    "absent"
                });
            }
            previous = Some((read.state, tree.generation));
        } else {
            previous = None;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    anyhow::bail!("Follow canonical profile not proved; no relationship action issued")
}

pub(crate) async fn follow_profile(
    session: &dyn UiSession,
    labels: crate::tiktok_labels::TikTokControls,
    target: &crate::ResolvedTikTokTarget,
    account: &str,
    gate: &mut crate::interaction_target::ActionEffectGate<'_>,
) -> Result<&'static str, crate::ActionFailure> {
    let before = async {
        ensure!(
            !account.trim().is_empty()
                && !account
                    .trim_start_matches('@')
                    .eq_ignore_ascii_case(&target.author),
            "Cannot Follow own or unknown account"
        );
        let package = labels.package();
        let version = session
            .app_version(package)
            .await
            .context("Follow version unreadable")?;
        ensure!(
            supported(package, &version, labels.language()),
            "Follow tuple unavailable"
        );
        let query = labels
            .label(crate::tiktok_labels::TikTokControl::AuthorProfileLink)
            .context("Author profile control missing")?
            .to_query();
        let epoch = session.gui_session_epoch();
        // Geometry and identity must come from one current hierarchy. Locator
        // IDs can point at recycled feed nodes between find/rect round trips.
        let tree = Tree::parse(session.hierarchy_source_snapshot().await?)?;
        let matches = tree.matching(package, query);
        let [index] = matches.as_slice() else {
            anyhow::bail!("Author profile is not unique")
        };
        let node = &tree.nodes[*index];
        let identity = (
            node.attr("resource-id").to_owned(),
            node.attr("content-desc").to_owned(),
        );
        let fresh = Tree::parse(session.hierarchy_source_snapshot().await?)?;
        ensure!(
            fresh.generation > tree.generation && session.gui_session_epoch() == epoch,
            "Author snapshot stale"
        );
        let matches = fresh.matching(package, query);
        let [index] = matches.as_slice() else {
            anyhow::bail!("Author profile changed")
        };
        let node = &fresh.nodes[*index];
        ensure!(
            node.attr("resource-id") == identity.0 && node.attr("content-desc") == identity.1,
            "Author profile identity changed"
        );
        let row = node
            .rect()
            .filter(|r| r.enabled)
            .context("Author profile geometry missing")?;
        session.tap(row.centre()).await?;
        let until = Instant::now() + Duration::from_secs(12);
        let mut matched = None;
        while Instant::now() < until {
            ensure!(
                session.gui_session_epoch() == epoch
                    && session.active_app_bundle().await? == package,
                "Follow session changed"
            );
            let tree = Tree::parse(session.hierarchy_source_snapshot().await?)?;
            if let Ok(profile) =
                profile(&tree, package, &version, labels.language(), &target.author)
            {
                matched = Some((profile, tree.generation));
                break;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        let (first, first_generation) = matched.context("Follow target profile not proved")?;
        let tree = Tree::parse(session.hierarchy_source_snapshot().await?)?;
        let fresh = profile(&tree, package, &version, labels.language(), &target.author)?;
        ensure!(
            tree.generation > first_generation
                && first.handle == fresh.handle
                && first.state == fresh.state
                && session.gui_session_epoch() == epoch,
            "Follow target changed before intent"
        );
        Ok::<_, anyhow::Error>((fresh, version, epoch, tree.generation))
    }
    .await
    .map_err(crate::ActionFailure::before)?;
    let (proved, version, epoch, mut generation) = before;
    if proved.state == Relationship::Following {
        return Ok("alreadyFollowing");
    }
    gate.cross()
        .map_err(|e| crate::ActionFailure::before(anyhow::anyhow!(e.to_string())))?;
    session
        .tap(proved.button.centre())
        .await
        .map_err(crate::ActionFailure::after)?;
    let until = Instant::now() + Duration::from_secs(10);
    let mut confirmed = 0;
    while Instant::now() < until {
        let read = async {
            ensure!(
                session.gui_session_epoch() == epoch
                    && session.active_app_bundle().await? == labels.package(),
                "Follow session changed after effect"
            );
            let tree = Tree::parse(session.hierarchy_source_snapshot().await?)?;
            ensure!(
                tree.generation > generation,
                "Follow readback snapshot is stale"
            );
            generation = tree.generation;
            profile(
                &tree,
                labels.package(),
                &version,
                labels.language(),
                &target.author,
            )
        }
        .await
        .map_err(crate::ActionFailure::after)?;
        if read.state == Relationship::Following {
            confirmed += 1;
            if confirmed == 2 {
                // TikTok can optimistically display Following and later reject
                // the relationship. Revisit the canonical profile before the
                // ledger calls this confirmed; a missing relationship remains
                // uncertain after the single tap and must never be replayed.
                let persisted = observe_follow_profile(session, labels, target, account)
                    .await
                    .map_err(crate::ActionFailure::after)?;
                if persisted != "present" {
                    return Err(crate::ActionFailure::after(anyhow::anyhow!(
                        "Follow did not persist on canonical profile readback; never repeat the tap"
                    )));
                }
                return Ok("followed");
            }
        } else {
            confirmed = 0;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    Err(crate::ActionFailure::after(anyhow::anyhow!(
        "Follow not confirmed; never repeat the tap"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[test]
    fn measured_global_following_icon_is_a_relationship_not_the_following_counter() {
        let xml = include_str!("../fixtures/tiktok-follow/global-profile-after.fixture");
        let parse = |xml: String| {
            let tree = Tree::parse(crate::HierarchySourceSnapshot { generation: 1, xml }).unwrap();
            profile(
                &tree,
                "com.zhiliaoapp.musically",
                "45.7.3",
                "en",
                "fixture.target",
            )
        };
        assert_eq!(parse(xml.into()).unwrap().state, Relationship::Following);
        assert!(parse(xml.replace(":id/f6y", ":id/unknown")).is_err());
        assert!(parse(xml.replace("@fixture.target", "@fixture.targe")).is_err());
        assert!(parse(xml.replace("text=\" Message\"", "text=\"Follow\"")).is_err());
        let tree = Tree::parse(crate::HierarchySourceSnapshot {
            generation: 1,
            xml: include_str!("../fixtures/tiktok-follow/trill-profile-after.fixture").into(),
        })
        .unwrap();
        assert_eq!(
            profile(
                &tree,
                "com.ss.android.ugc.trill",
                "38.3.2",
                "en",
                "fixture.target"
            )
            .unwrap()
            .state,
            Relationship::Following
        );
    }
    #[test]
    fn measured_trill_friends_keeps_relationship_icon_with_wave_message() {
        let tree = Tree::parse(crate::HierarchySourceSnapshot {
            generation: 2,
            xml: include_str!("../fixtures/tiktok-follow/trill-profile-friends.fixture").into(),
        })
        .unwrap();
        assert_eq!(
            profile(
                &tree,
                "com.ss.android.ugc.trill",
                "38.3.2",
                "en",
                "fixture.target"
            )
            .unwrap()
            .state,
            Relationship::Following
        );
        let tree = Tree::parse(crate::HierarchySourceSnapshot {
            generation: 3,
            xml: include_str!("../fixtures/tiktok-follow/global-profile-friends.fixture").into(),
        })
        .unwrap();
        assert_eq!(
            profile(
                &tree,
                "com.zhiliaoapp.musically",
                "45.7.3",
                "en",
                "fixture.target"
            )
            .unwrap()
            .state,
            Relationship::Following
        );
    }
    #[derive(Default)]
    struct Phone {
        taps: AtomicUsize,
        opens: AtomicUsize,
        snapshots: AtomicUsize,
        stale: bool,
        after: bool,
        fail_effect: bool,
        reverts_on_revisit: bool,
    }
    #[async_trait::async_trait]
    impl UiSession for Phone {
        async fn open_url_in_app(&self, url: &str, package: &str) -> anyhow::Result<()> {
            assert_eq!(url, "https://www.tiktok.com/@fixture.target");
            assert_eq!(package, "com.zhiliaoapp.musically");
            self.opens.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        async fn tap(&self, _: crate::TapPoint) -> anyhow::Result<()> {
            let n = self.taps.fetch_add(1, Ordering::Relaxed);
            if n == 1 && self.fail_effect {
                anyhow::bail!("lost tap acknowledgement");
            }
            Ok(())
        }
        async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
            unreachable!()
        }
        async fn type_text(&self, _: &str) -> anyhow::Result<()> {
            unreachable!()
        }
        async fn home(&self) -> anyhow::Result<()> {
            unreachable!()
        }
        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
            unreachable!()
        }
        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
            unreachable!()
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
        fn gui_session_epoch(&self) -> String {
            "fixture-session".into()
        }
        async fn active_app_bundle(&self) -> anyhow::Result<String> {
            Ok("com.zhiliaoapp.musically".into())
        }
        async fn app_version(&self, _: &str) -> Option<String> {
            Some("45.7.3".into())
        }
        async fn locate_all_described(
            &self,
            _: ElementQuery<'_>,
        ) -> anyhow::Result<Vec<ElementBox>> {
            panic!("Follow must not reuse locator IDs for author navigation")
        }
        async fn hierarchy_source_snapshot(
            &self,
        ) -> anyhow::Result<crate::HierarchySourceSnapshot> {
            let mut xml =
                include_str!("../fixtures/tiktok-follow/global-profile-before.fixture").to_owned();
            if self.taps.load(Ordering::Relaxed) == 0 && self.opens.load(Ordering::Relaxed) == 0 {
                xml="<hierarchy><node package=\"com.zhiliaoapp.musically\" class=\"android.widget.ImageView\" resource-id=\"com.zhiliaoapp.musically:id/avatar\" content-desc=\"fixture profile\" bounds=\"[10,10][30,30]\" enabled=\"true\" displayed=\"true\"/></hierarchy>".into();
            } else if (self.after || self.taps.load(Ordering::Relaxed) >= 2)
                && !(self.reverts_on_revisit && self.opens.load(Ordering::Relaxed) > 0)
            {
                xml = include_str!("../fixtures/tiktok-follow/global-profile-after.fixture").into();
            }
            let generation = if self.stale {
                1
            } else {
                self.snapshots.fetch_add(1, Ordering::Relaxed) as u64 + 1
            };
            Ok(crate::HierarchySourceSnapshot { generation, xml })
        }
    }
    #[tokio::test(start_paused = true)]
    async fn follow_does_not_confirm_an_optimistic_relationship_that_reverts_on_revisit() {
        let labels =
            crate::tiktok_labels::controls_for("com.zhiliaoapp.musically", "en", "45.7.3").unwrap();
        let target = crate::parse_tiktok_links("https://www.tiktok.com/@fixture.target/video/123")
            .remove(0)
            .target
            .unwrap();
        let phone = Phone {
            reverts_on_revisit: true,
            ..Default::default()
        };
        let arms = AtomicUsize::new(0);
        let mut gate = crate::interaction_target::ActionEffectGate::new(|| {
            arms.fetch_add(1, Ordering::Relaxed);
            Ok(true)
        });
        let error = follow_profile(&phone, labels, &target, "fixture.actor", &mut gate)
            .await
            .expect_err("optimistic in-place state must not count as confirmed");
        assert!(error.effect_may_have_gone_out());
        assert_eq!(arms.load(Ordering::Relaxed), 1);
        assert_eq!(
            phone.taps.load(Ordering::Relaxed),
            2,
            "one profile navigation and one Follow, never replay"
        );
        assert_eq!(phone.opens.load(Ordering::Relaxed), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn follow_uses_one_effect_gate_and_never_replays_or_self_follows() {
        let labels =
            crate::tiktok_labels::controls_for("com.zhiliaoapp.musically", "en", "45.7.3").unwrap();
        let target = crate::ResolvedTikTokTarget {
            original_url: "https://www.tiktok.com/@fixture.target/video/123".into(),
            normalized_url: "https://www.tiktok.com/@fixture.target/video/123".into(),
            target_key: "content:123".into(),
            content_id: "123".into(),
            author: "fixture.target".into(),
            kind: crate::TikTokPostKind::Video,
        };
        for (already, fail, expected_taps) in
            [(false, false, 2), (true, false, 1), (false, true, 2)]
        {
            let phone = Phone {
                taps: AtomicUsize::new(0),
                after: already,
                fail_effect: fail,
                ..Default::default()
            };
            let arms = AtomicUsize::new(0);
            let mut gate = crate::interaction_target::ActionEffectGate::new(|| {
                arms.fetch_add(1, Ordering::Relaxed);
                Ok(true)
            });
            let result = follow_profile(&phone, labels, &target, "fixture.actor", &mut gate).await;
            assert_eq!(phone.taps.load(Ordering::Relaxed), expected_taps);
            assert_eq!(arms.load(Ordering::Relaxed), usize::from(!already));
            if fail {
                assert!(result.unwrap_err().effect_may_have_gone_out());
            } else {
                assert!(result.is_ok());
            }
        }
        for already in [true, false] {
            let phone = Phone {
                taps: AtomicUsize::new(0),
                after: already,
                fail_effect: true,
                ..Default::default()
            };
            assert_eq!(
                observe_follow_profile(&phone, labels, &target, "fixture.actor")
                    .await
                    .unwrap(),
                if already { "present" } else { "absent" }
            );
            assert_eq!(
                phone.taps.load(Ordering::Relaxed),
                0,
                "readback must never tap"
            );
            assert_eq!(phone.opens.load(Ordering::Relaxed), 1);
        }
        let phone = Phone {
            taps: AtomicUsize::new(0),
            after: false,
            fail_effect: false,
            ..Default::default()
        };
        let stale = Phone {
            stale: true,
            ..Default::default()
        };
        let mut denied =
            crate::interaction_target::ActionEffectGate::new(|| panic!("stale proof must not arm"));
        let error = follow_profile(&stale, labels, &target, "fixture.actor", &mut denied)
            .await
            .unwrap_err();
        assert!(!error.effect_may_have_gone_out());
        assert_eq!(stale.taps.load(Ordering::Relaxed), 0);
        let mut gate =
            crate::interaction_target::ActionEffectGate::new(|| panic!("must not arm self-follow"));
        assert!(
            follow_profile(&phone, labels, &target, "fixture.target", &mut gate)
                .await
                .is_err()
        );
        assert_eq!(phone.taps.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn measured_profile_requires_same_handle_and_excludes_neighbour_actions() {
        for (package, version, xml) in [
            (
                "com.ss.android.ugc.trill",
                "38.3.2",
                include_str!("../fixtures/tiktok-follow/trill-profile-before.fixture"),
            ),
            (
                "com.zhiliaoapp.musically",
                "45.7.3",
                include_str!("../fixtures/tiktok-follow/global-profile-before.fixture"),
            ),
        ] {
            let tree = Tree::parse(crate::HierarchySourceSnapshot {
                generation: 1,
                xml: xml.into(),
            })
            .unwrap();
            assert_eq!(
                profile(&tree, package, version, "en", "fixture.target")
                    .unwrap()
                    .state,
                Relationship::NotFollowing
            );
            assert!(profile(&tree, package, version, "en", "fixture.targe").is_err());
            let bad = Tree::parse(crate::HierarchySourceSnapshot {
                generation: 2,
                xml: xml
                    .replace("Follow back", "Message")
                    .replace("text=\"Follow\"", "text=\"Message\""),
            })
            .unwrap();
            assert!(profile(&bad, package, version, "en", "fixture.target").is_err());
        }
    }
}
#[test]
fn current_fleet_profiles_bind_follow_to_exact_measured_header() {
    for (version, source) in [
        (
            "45.4.3",
            include_str!("../fixtures/tiktok-follow/global-45.4.3-not-following.fixture"),
        ),
        (
            "46.0.41",
            include_str!("../fixtures/tiktok-follow/global-46.0.41-not-following.fixture"),
        ),
        (
            "46.1.3",
            include_str!("../fixtures/tiktok-follow/global-46.1.3-not-following.fixture"),
        ),
        (
            "46.4.3",
            include_str!("../fixtures/tiktok-follow/global-46.4.3-not-following.fixture"),
        ),
    ] {
        let tree = Tree::parse(crate::HierarchySourceSnapshot {
            generation: 1,
            xml: source.into(),
        })
        .unwrap();
        let found = profile(
            &tree,
            "com.zhiliaoapp.musically",
            version,
            "en",
            "ghin.lt.sng.sng",
        )
        .unwrap();
        assert_eq!(found.state, Relationship::NotFollowing);
        assert!(found.button.clickable && found.button.enabled);
        assert!(profile(
            &tree,
            "com.zhiliaoapp.musically",
            version,
            "en",
            "other.account"
        )
        .is_err());
        assert!(profile(
            &tree,
            "com.zhiliaoapp.musically",
            "unknown",
            "en",
            "ghin.lt.sng.sng"
        )
        .is_err());
    }
}

#[test]
fn global460_follow_readback_excludes_clickable_suggestion_icon() {
    let source = include_str!("../fixtures/tiktok-follow/global-46.0.41-following.fixture");
    let tree = Tree::parse(crate::HierarchySourceSnapshot {
        generation: 1,
        xml: source.into(),
    })
    .unwrap();
    let found = profile(
        &tree,
        "com.zhiliaoapp.musically",
        "46.0.41",
        "en",
        "ghin.lt.sng.sng",
    )
    .unwrap();
    assert_eq!(found.state, Relationship::Following);
    assert_eq!(found.button.x, 634.0);
    assert!(profile(&tree, "com.zhiliaoapp.musically", "46.0.41", "en", "other").is_err());
}

#[test]
fn global454_following_icon_requires_exact_header_and_excludes_suggestion() {
    let source = include_str!("../fixtures/tiktok-follow/global-45.4.3-following.fixture");
    let parse = |xml: String| {
        let tree = Tree::parse(crate::HierarchySourceSnapshot { generation: 1, xml }).unwrap();
        profile(
            &tree,
            "com.zhiliaoapp.musically",
            "45.4.3",
            "en",
            "ghin.lt.sng.sng",
        )
    };
    let found = parse(source.into()).unwrap();
    assert_eq!(found.state, Relationship::Following);
    assert!(
        found.button.x < 700.0,
        "the suggestions control is outside this relationship button"
    );
    assert!(parse(source.replace("@ghin.lt.sng.sng", "@other.account")).is_err());
    assert!(parse(source.replace("text=\" Message\"", "text=\"Follow\"")).is_err());
    assert!(parse(source.replace(":id/f4i", ":id/unknown")).is_err());
}

#[test]
fn global_newer_follow_readbacks_require_measured_relationship_icon() {
    for (version, source) in [
        (
            "46.1.3",
            include_str!("../fixtures/tiktok-follow/global-46.1.3-following.fixture"),
        ),
        (
            "46.4.3",
            include_str!("../fixtures/tiktok-follow/global-46.4.3-following.fixture"),
        ),
    ] {
        let tree = Tree::parse(crate::HierarchySourceSnapshot {
            generation: 1,
            xml: source.into(),
        })
        .unwrap();
        let found = profile(
            &tree,
            "com.zhiliaoapp.musically",
            version,
            "en",
            "ghin.lt.sng.sng",
        )
        .unwrap();
        assert_eq!(found.state, Relationship::Following);
        assert_eq!(found.button.x, 634.0);
        assert!(profile(&tree, "com.zhiliaoapp.musically", version, "en", "other").is_err());
    }
}
