//! Measured Trill Recent recovery for an already frozen sound identity.
use super::*;
use crate::ui_automation::tree::Tree;

#[derive(Debug, thiserror::Error)]
#[error("Recent recovery session changed: {previous} -> {current}")]
pub(super) struct SessionChanged {
    pub previous: String,
    pub current: String,
}

async fn require_session(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    epoch: &str,
) -> anyhow::Result<()> {
    check_wait()?;
    anyhow::ensure!(
        read_sound(session.active_app_bundle()).await? == plan.package,
        "Recent recovery app changed"
    );
    let current = session.gui_session_epoch();
    if current != epoch {
        return Err(SessionChanged {
            previous: epoch.into(),
            current,
        }
        .into());
    }
    Ok(())
}

pub(super) fn measured(plan: SoundPickerPlan) -> bool {
    !plan.dynamic && plan.package == "com.ss.android.ugc.trill" && plan.entry_id == ":id/c_4"
}

fn tab(tree: &Tree, plan: SoundPickerPlan) -> anyhow::Result<(ElementBox, bool)> {
    let tabs = tree.matching(plan.package, ElementQuery::ResourceIdSuffix(":id/q_g"));
    let recent: Vec<_> = tabs
        .iter()
        .filter(|i| tree.nodes[**i].attr("text") == "Recent")
        .collect();
    let [index] = recent.as_slice() else {
        anyhow::bail!("Recent tab missing or ambiguous");
    };
    anyhow::ensure!(
        tabs.len() == 3
            && tabs
                .iter()
                .filter(|i| tree.nodes[**i].attr("selected") == "true")
                .count()
                == 1,
        "sound tab selection ambiguous"
    );
    let node = &tree.nodes[**index];
    let rect = node.rect().context("Recent tab bounds missing")?;
    anyhow::ensure!(rect.enabled, "Recent tab disabled");
    // Measured q_g text is not clickable; its enclosing tab owns the action.
    let mut ancestor = Some(**index);
    while let Some(i) = ancestor {
        let n = &tree.nodes[i];
        if n.attr("clickable") == "true" {
            let hit = n.rect().context("Recent ancestor bounds missing")?;
            let centre = rect.centre();
            anyhow::ensure!(
                hit.enabled
                    && centre.x >= hit.x
                    && centre.x <= hit.x + hit.width
                    && centre.y >= hit.y
                    && centre.y <= hit.y + hit.height,
                "Recent tab ancestor changed"
            );
            return Ok((rect, node.attr("selected") == "true"));
        }
        ancestor = n.parent;
    }
    anyhow::bail!("Recent tab actionable ancestor missing")
}

fn pool(tree: &Tree, plan: SoundPickerPlan) -> anyhow::Result<ObservedSoundPool> {
    anyhow::ensure!(tab(tree, plan)?.1, "Recent tab not selected");
    let viewport = tree.matching(plan.package, ElementQuery::ResourceIdSuffix(":id/nof"));
    let [viewport] = viewport.as_slice() else {
        anyhow::bail!("Recent viewport ambiguous");
    };
    let viewport_rect = tree.nodes[*viewport]
        .rect()
        .context("Recent viewport bounds missing")?;
    let mut rows = Vec::new();
    let mut titles = Vec::new();
    let mut artists = Vec::new();
    let mut markers = Vec::new();
    for i in tree.matching(plan.package, ElementQuery::ResourceIdSuffix(plan.row_id)) {
        let row = tree.nodes[i].rect().context("Recent row bounds missing")?;
        // Last live row is clipped at1967; it cannot authorize a sound selection.
        if !tree.inside(i, *viewport)
            || row.y < viewport_rect.y
            || row.y + row.height >= viewport_rect.y + viewport_rect.height
        {
            continue;
        }
        for (j, n) in tree.nodes.iter().enumerate() {
            if !tree.inside(j, i) || !n.visible(plan.package) || !tree.ancestors_visible(j) {
                continue;
            }
            let Some(rect) = n.rect() else { continue };
            if n.matches(ElementQuery::ResourceIdSuffix(plan.title_id)) {
                titles.push(rect.clone());
            }
            if n.matches(ElementQuery::ResourceIdSuffix(plan.artist_id)) {
                artists.push(rect.clone());
            }
            if plan
                .selected_marker_ids()
                .iter()
                .any(|id| n.matches(ElementQuery::ResourceIdSuffix(id)))
            {
                markers.push(rect);
            }
        }
        rows.push(row);
    }
    assemble_pool(plan, rows, titles, artists, Vec::new(), markers, 5)
}

async fn read(session: &dyn UiSession, plan: SoundPickerPlan, epoch: &str) -> anyhow::Result<Tree> {
    check_wait()?;
    require_session(session, plan, epoch).await?;
    let tree = Tree::parse(read_sound(session.hierarchy_source_snapshot()).await?)?;
    require_session(session, plan, epoch).await?;
    Ok(tree)
}

pub(super) async fn observe(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
) -> anyhow::Result<ObservedSoundPool> {
    let epoch = session.gui_session_epoch();
    anyhow::ensure!(!epoch.is_empty(), "Recent recovery session missing");
    let mut previous: Option<(u64, ObservedSoundPool)> = None;
    loop {
        let tree = read(session, plan, &epoch).await?;
        if let Ok(current) = pool(&tree, plan) {
            if previous
                .as_ref()
                .is_some_and(|(g, p)| tree.generation > *g && p.stable_with(&current))
            {
                return Ok(current);
            }
            previous = Some((tree.generation, current));
        } else {
            previous = None;
        }
        tokio::time::sleep(POLL).await;
        check_wait()?;
    }
}

pub(super) async fn recover(
    session: &dyn UiSession,
    mut plan: SoundPickerPlan,
    selection: &crate::SoundSelectionEvidence,
) -> anyhow::Result<ObservedSoundPool> {
    anyhow::ensure!(measured(plan), "frozen sound Recent recovery unmeasured");
    let epoch = session.gui_session_epoch();
    anyhow::ensure!(!epoch.is_empty(), "Recent recovery session missing");
    let mut previous: Option<(u64, ElementBox, bool)> = None;
    loop {
        let tree = read(session, plan, &epoch).await?;
        if let Ok((target, selected)) = tab(&tree, plan) {
            if previous
                .as_ref()
                .is_some_and(|(g, r, s)| tree.generation > *g && *r == target && *s == selected)
            {
                let fresh = read(session, plan, &epoch).await?;
                anyhow::ensure!(
                    fresh.generation > tree.generation
                        && tab(&fresh, plan)? == (target.clone(), selected),
                    "Recent tab changed before navigation"
                );
                if !selected {
                    sound_tap(session, target.centre()).await?;
                }
                break;
            }
            previous = Some((tree.generation, target, selected));
        } else {
            previous = None;
        }
        tokio::time::sleep(POLL).await;
    }
    plan.section_label = "Recent";
    let mut observed = observe(session, plan).await?;
    let matches = observed
        .candidates
        .iter()
        .filter(|c| c.title == selection.title && c.artist == selection.artist)
        .count();
    anyhow::ensure!(
        matches == 1,
        "frozen sound absent or ambiguous in Recent; không đổi sang nhạc khác"
    );
    observed.effective_plan = Some(plan);
    Ok(observed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    struct Session {
        taps: AtomicUsize,
        reads: AtomicUsize,
        mode: &'static str,
        closed: AtomicBool,
        recycled: AtomicBool,
        tap_completed: AtomicBool,
    }
    fn node(id: &str, text: &str, bounds: &str, extra: &str) -> String {
        format!(
            r#"<node package="com.ss.android.ugc.trill" resource-id="com.ss.android.ugc.trill:id/{id}" text="{text}" bounds="{bounds}" displayed="true" enabled="true" clickable="true" {extra}/>"#
        )
    }
    fn xml(recent: bool, empty: bool, wrong_artist: bool) -> String {
        let tab = |text: &str, selected: bool, bounds: &str| {
            node("q_g", text, bounds, &format!("selected=\"{selected}\""))
        };
        let rows = if empty {
            String::new()
        } else {
            format!(
                r#"<node package="com.ss.android.ugc.trill" resource-id="com.ss.android.ugc.trill:id/ta8" bounds="[0,1300][996,1480]" displayed="true" enabled="true" clickable="true">{}{}</node>"#,
                node(
                    "title",
                    if recent {
                        "Tùng Zin Zin"
                    } else {
                        "Other recommendation"
                    },
                    "[199,1320][600,1370]",
                    ""
                ),
                node(
                    "rr5",
                    if wrong_artist {
                        "Other artist"
                    } else {
                        "Kafkaf &amp; Hằng ssi &amp; Aries tis"
                    },
                    "[199,1390][800,1440]",
                    ""
                )
            )
        };
        format!("<hierarchy>{}{}{}<node package=\"com.ss.android.ugc.trill\" resource-id=\"com.ss.android.ugc.trill:id/nof\" bounds=\"[0,1185][996,1967]\" displayed=\"true\" enabled=\"true\">{rows}</node></hierarchy>",tab("Recommended",!recent,"[42,1088][392,1142]"),tab("Favorites",false,"[392,1088][617,1142]"),tab("Recent",recent,"[617,1088][802,1142]"))
    }
    #[async_trait::async_trait]
    impl UiSession for Session {
        async fn tap(&self, p: crate::TapPoint) -> anyhow::Result<()> {
            if (self.mode == "select" || self.mode.starts_with("recycle"))
                && self.taps.load(Ordering::Relaxed) == 1
            {
                assert!(p.x >= 199. && p.x <= 600. && p.y >= 1320. && p.y <= 1370.);
                self.taps.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
            assert!(
                p.x >= 617. && p.x <= 802. && p.y >= 1088. && p.y <= 1142.,
                "only Recent tab may be tapped"
            );
            self.taps.fetch_add(1, Ordering::Relaxed);
            if self.mode == "slow-tap" {
                tokio::time::sleep(Duration::from_secs(61)).await;
            }
            self.tap_completed.store(true, Ordering::Relaxed);
            Ok(())
        }
        async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
            panic!("no scrolling");
        }
        async fn type_text(&self, _: &str) -> anyhow::Result<()> {
            panic!("no search typing");
        }
        async fn home(&self) -> anyhow::Result<()> {
            unreachable!()
        }
        async fn back(&self) -> anyhow::Result<()> {
            assert_eq!(self.taps.load(Ordering::Relaxed), 2);
            self.closed.store(true, Ordering::Relaxed);
            Ok(())
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
            if self.recycled.load(Ordering::Relaxed)
                || (self.mode == "session" && self.reads.load(Ordering::Relaxed) > 2)
            {
                "changed"
            } else {
                "owned"
            }
            .into()
        }
        async fn active_app_bundle(&self) -> anyhow::Result<String> {
            Ok(if self.mode == "app"
                || (self.mode == "recycle-app" && self.recycled.load(Ordering::Relaxed))
            {
                "other.app"
            } else {
                "com.ss.android.ugc.trill"
            }
            .into())
        }
        async fn hierarchy_source_snapshot(
            &self,
        ) -> anyhow::Result<crate::HierarchySourceSnapshot> {
            let n = self.reads.fetch_add(1, Ordering::Relaxed) + 1;
            if self.mode.starts_with("recycle") && self.taps.load(Ordering::Relaxed) == 2 {
                self.recycled.store(true, Ordering::Relaxed);
                let title = if self.mode == "recycle-wrong" {
                    "Other sound"
                } else {
                    "Tùng Zin Zin"
                };
                let sheet = if self.mode == "recycle-sheet" {
                    node("q_g", "Recent", "[617,1088][802,1142]", "selected=\"true\"")
                } else {
                    String::new()
                };
                return Ok(crate::HierarchySourceSnapshot {
                    generation: if self.mode == "recycle-stale" {
                        1
                    } else {
                        n as u64
                    },
                    xml: format!(
                        "<hierarchy>{}{}{sheet}</hierarchy>",
                        node("so9", title, "[300,100][700,180]", ""),
                        node("kl_", "Next", "[755,1985][849,2038]", "")
                    ),
                });
            }
            if self.mode == "stop" {
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
            let mut source = xml(
                self.taps.load(Ordering::Relaxed) > 0,
                n == 4,
                self.mode == "artist",
            );
            if self.taps.load(Ordering::Relaxed) == 2 {
                source = source.replacen(
                    "</node>",
                    &format!("{}</node>", node("jk1", "", "[50,1320][90,1360]", "")),
                    1,
                );
            }
            Ok(crate::HierarchySourceSnapshot {
                generation: if self.mode == "stale" { 1 } else { n as u64 },
                xml: source,
            })
        }
        async fn locate_all(&self, _: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
            Ok(Vec::new())
        }
        async fn locate_all_described(
            &self,
            query: ElementQuery<'_>,
        ) -> anyhow::Result<Vec<ElementBox>> {
            Ok(
                if self.closed.load(Ordering::Relaxed)
                    && query == ElementQuery::ResourceIdSuffix(":id/so9")
                {
                    vec![ElementBox {
                        x: 300.,
                        y: 100.,
                        width: 400.,
                        height: 80.,
                        description: Some("Tùng Zin Zin".into()),
                        enabled: true,
                        clickable: false,
                    }]
                } else {
                    vec![]
                },
            )
        }
    }
    #[tokio::test(start_paused = true)]
    async fn frozen_identity_recovers_from_recommendations_only_through_proven_recent_rows() {
        for mode in [
            "found",
            "select",
            "recycle",
            "recycle-wrong",
            "recycle-app",
            "recycle-stale",
            "recycle-sheet",
            "artist",
            "app",
            "session",
            "stale",
            "stop",
        ] {
            let session = Session {
                taps: AtomicUsize::new(0),
                reads: AtomicUsize::new(0),
                mode,
                closed: AtomicBool::new(false),
                recycled: AtomicBool::new(false),
                tap_completed: AtomicBool::new(false),
            };
            let selection = crate::SoundSelectionEvidence {
                section: crate::publish::SoundSectionKind::Trending,
                title: "Tùng Zin Zin".into(),
                artist: "Kafkaf & Hằng ssi & Aries tis".into(),
                index: 3,
                candidates_digest: "frozen-pool".into(),
                confirmed: true,
            };
            let unchanged = selection.clone();
            let stop = AtomicBool::new(false);
            let (result, ()) = tokio::join!(
                with_sound_budget(
                    &stop,
                    recover_frozen_sound_pool(
                        &session,
                        SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2")
                            .unwrap(),
                        &selection
                    )
                ),
                async {
                    if mode == "stop" {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        stop.store(true, Ordering::Relaxed);
                    }
                }
            );
            if mode == "found" || mode == "select" || mode.starts_with("recycle") {
                let pool = result.expect(
                    "missing recommendation must recover exact frozen identity from Recent",
                );
                assert_eq!(pool.candidates[0].title, selection.title);
                assert_eq!(pool.candidates[0].artist, selection.artist);
                assert_eq!(session.taps.load(Ordering::Relaxed), 1);
                if mode == "select" || mode.starts_with("recycle") {
                    let plan = pool.effective_plan.unwrap();
                    let result =
                        with_sound_budget(&stop, recover_sound_selection(&session, plan, &pool, 0))
                            .await;
                    if mode == "select" || mode == "recycle" {
                        result.expect("Recent selection must reprove exact editor after session replacement without replay");
                    } else {
                        assert!(result.is_err());
                    }
                    assert_eq!(session.taps.load(Ordering::Relaxed), 2);
                    assert_eq!(
                        session.closed.load(Ordering::Relaxed),
                        mode == "select",
                        "session recovery must not Back"
                    );
                }
            } else {
                assert!(result.is_err());
                assert!(session.taps.load(Ordering::Relaxed) <= usize::from(mode == "artist"));
            }
            assert_eq!(selection, unchanged);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn recent_recovery_deadline_drains_a_dispatched_tab_tap() {
        let session = Session {
            taps: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
            mode: "slow-tap",
            closed: AtomicBool::new(false),
            recycled: AtomicBool::new(false),
            tap_completed: AtomicBool::new(false),
        };
        let selection = crate::SoundSelectionEvidence {
            section: crate::publish::SoundSectionKind::Trending,
            title: "Tùng Zin Zin".into(),
            artist: "Kafkaf & Hằng ssi & Aries tis".into(),
            index: 3,
            candidates_digest: "frozen-pool".into(),
            confirmed: true,
        };
        let started = Instant::now();
        let stop = AtomicBool::new(false);
        let result = with_sound_budget(
            &stop,
            recover_frozen_sound_pool(
                &session,
                SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2").unwrap(),
                &selection,
            ),
        )
        .await;
        assert!(
            result.is_err(),
            "expired navigation cannot authorize selection"
        );
        assert!(
            session.tap_completed.load(Ordering::Relaxed),
            "the stage timeout abandoned a dispatched device gesture"
        );
        assert_eq!(session.taps.load(Ordering::Relaxed), 1);
        assert!(started.elapsed() >= Duration::from_secs(61));
    }
}
