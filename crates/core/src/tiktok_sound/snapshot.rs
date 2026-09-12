//! Read one hierarchy generation using the tuple's measured sound-sheet layout.
//! Rows, selected tab and inline markers always come from the same snapshot.
use super::*;

// AGENTS.md §9.187: 0.2.11 exhausted eight seconds before any Hot tap on the
// local 45.7.3 phones 2/3. Android source reads take multiple seconds; the
// driver's measured worst root-query regime is 11 seconds. Two stable reads,
// one missed tap, two fresh reads for the retry and its confirmation need five
// such reads plus polling. Keep both observation phases bounded independently.
const SECTION_WINDOW: Duration = Duration::from_secs(60);
const SNAPSHOT_POOL_WINDOW: Duration = Duration::from_secs(30);

/// One extra observation per phase after the driver's exhausted read recovery.
/// Clear the previous generation before waiting; a stale rectangle cannot authorize a tap.
async fn read_snapshot(
    session: &dyn UiSession,
    deadline: Instant,
    recovery_used: &mut bool,
) -> anyhow::Result<Option<String>> {
    let started = Instant::now();
    match session.hierarchy_source_snapshot().await {
        Ok(snapshot) => Ok(Some(snapshot.xml)),
        Err(error)
            if error
                .downcast_ref::<crate::driver::AccessibilityReadUnavailable>()
                .is_some()
                && !*recovery_used
                && Instant::now() + POLL < deadline =>
        {
            *recovery_used = true;
            tracing::warn!(elapsed_ms = started.elapsed().as_millis() as u64,
                    recovery = 1, remaining_ms = deadline.saturating_duration_since(Instant::now()).as_millis() as u64,
                    error = %error, "sound screen read unavailable; observing once more");
            tokio::time::sleep(POLL).await;
            Ok(None)
        }
        Err(error) => Err(error.context(format!(
            "đọc bảng nhạc thất bại sau {} lần phục hồi bổ sung; request {} ms",
            usize::from(*recovery_used),
            started.elapsed().as_millis()
        ))),
    }
}

#[derive(Debug)]
struct Node {
    id: String,
    selected: bool,
    rect: ElementBox,
}

fn parse(xml: &str, plan: SoundPickerPlan) -> anyhow::Result<Vec<Node>> {
    let layout = plan
        .snapshot_layout()
        .context("sound snapshot layout unmeasured")?;
    let tree = crate::ui_automation::tree::Tree::parse(crate::HierarchySourceSnapshot {
        generation: 1,
        xml: xml.into(),
    })?;
    let mut out = Vec::new();
    for (index, node) in tree.nodes.iter().enumerate() {
        if !node.visible(plan.package) || !tree.ancestors_visible(index) {
            continue;
        }
        let Some(id) = node.attr("resource-id").strip_prefix(plan.package) else {
            continue;
        };
        if ![
            layout.tab_id,
            layout.viewport_id,
            plan.row_id,
            plan.title_id,
            plan.artist_id,
        ]
        .contains(&id)
            && !plan.selected_marker_ids().contains(&id)
            && plan.choose_id != Some(id)
        {
            continue;
        }
        let rect = node.rect().context("invalid sound bounds")?;
        out.push(Node {
            id: id.into(),
            selected: node.attr("selected") == "true",
            rect,
        });
    }
    Ok(out)
}

fn section_tab(nodes: &[Node], plan: SoundPickerPlan) -> anyhow::Result<(&ElementBox, bool)> {
    let layout = plan
        .snapshot_layout()
        .context("sound snapshot layout unmeasured")?;
    let tabs: Vec<_> = nodes.iter().filter(|n| n.id == layout.tab_id).collect();
    let hot: Vec<_> = tabs
        .iter()
        .filter(|n| n.rect.description.as_deref() == Some(plan.section_label))
        .collect();
    let [hot] = hot.as_slice() else {
        anyhow::bail!("{} tab missing or ambiguous", plan.section_label);
    };
    let selected: Vec<_> = tabs.iter().filter(|n| n.selected).collect();
    anyhow::ensure!(selected.len() == 1, "sound tab selection unreadable");
    anyhow::ensure!(hot.rect.enabled, "{} tab disabled", plan.section_label);
    Ok((&hot.rect, hot.selected))
}

pub(super) async fn select_section_tab(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + SECTION_WINDOW;
    let mut previous: Option<ElementBox> = None;
    let mut attempts = 0;
    let mut retry_after = Instant::now();
    let mut recovery_used = false;
    loop {
        let Some(xml) = read_snapshot(session, deadline, &mut recovery_used).await? else {
            previous = None;
            continue;
        };
        let parsed = parse(&xml, plan)?;
        let observation = match section_tab(&parsed, plan) {
            Ok((_, true)) => return Ok(()),
            Ok((tab, false)) => {
                // LAN 46.0.41 campaigns stopped here without selecting Hot
                // (AGENTS.md §9.186). Wait for stable bounds while the sheet opens.
                // A section tab is idempotent: one fresh, still-unselected readback
                // may authorize one retry. Sound rows themselves remain single-tap.
                let now = Instant::now();
                if previous.as_ref() == Some(tab)
                    && attempts < 2
                    && now >= retry_after
                    && now < deadline
                {
                    attempts += 1;
                    tracing::debug!(
                        section = plan.section_label,
                        attempts,
                        "select sound section"
                    );
                    session
                        .tap(tab.centre())
                        .await
                        .context("select measured sound section")?;
                    retry_after = Instant::now() + Duration::from_secs(1);
                    previous = None;
                } else {
                    previous = Some(tab.clone());
                }
                "another sound section is still selected".to_string()
            }
            Err(error) => {
                previous = None;
                error.to_string()
            }
        };
        anyhow::ensure!(
            Instant::now() < deadline,
            "{} sound section did not become selected after {attempts} tap(s): {observation}",
            plan.section_label,
        );
        tokio::time::sleep(POLL).await;
    }
}

fn contains(outer: &ElementBox, inner: &ElementBox) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.x + inner.width <= outer.x + outer.width
        && inner.y + inner.height <= outer.y + outer.height
}

fn pool(xml: &str, plan: SoundPickerPlan, maximum: usize) -> anyhow::Result<ObservedSoundPool> {
    let nodes = parse(xml, plan)?;
    let layout = plan
        .snapshot_layout()
        .context("sound snapshot layout unmeasured")?;
    anyhow::ensure!(
        section_tab(&nodes, plan)?.1,
        "{} tab visible but not selected",
        plan.section_label
    );
    let find = |id| {
        nodes
            .iter()
            .filter(|n| n.id == id)
            .map(|n| n.rect.clone())
            .collect::<Vec<_>>()
    };
    let viewport = exactly_one(find(layout.viewport_id), "sound list viewport")?;
    let titles = find(plan.title_id);
    let artists = find(plan.artist_id);
    let mut rows = Vec::new();
    for row in find(plan.row_id) {
        anyhow::ensure!(contains(&viewport, &row), "sound row outside list viewport");
        let row_titles = inside(&row, &titles);
        let row_artists = inside(&row, &artists);
        if row.y + row.height == viewport.y + viewport.height
            && (layout.boundary_rows == SoundBoundaryRows::ExcludeBottomEdge
                || row_titles.is_empty()
                || row_artists.is_empty())
        {
            continue;
        }
        anyhow::ensure!(
            row_titles.len() == 1 && row_artists.len() == 1,
            "incomplete sound row"
        );
        anyhow::ensure!(
            contains(&row, &row_titles[0]) && contains(&row, &row_artists[0]),
            "clipped sound text"
        );
        rows.push(row);
    }
    let choices = plan.choose_id.map(find).unwrap_or_default();
    let markers = plan
        .selected_marker_ids()
        .iter()
        .flat_map(|id| find(id))
        .collect();
    assemble_pool(plan, rows, titles, artists, choices, markers, maximum)
}

pub(super) async fn observe(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
) -> anyhow::Result<ObservedSoundPool> {
    let deadline = Instant::now() + SNAPSHOT_POOL_WINDOW;
    let mut previous: Option<ObservedSoundPool> = None;
    let mut recovery_used = false;
    loop {
        let Some(xml) = read_snapshot(session, deadline, &mut recovery_used).await? else {
            previous = None;
            continue;
        };
        let observed = pool(&xml, plan, maximum);
        match observed {
            Ok(current) => {
                if previous.as_ref().is_some_and(|p| p == &current) {
                    return Ok(current);
                }
                previous = Some(current);
            }
            Err(error) => {
                previous = None;
                if Instant::now() >= deadline {
                    return Err(error);
                }
            }
        }
        anyhow::ensure!(Instant::now() < deadline, "sound pool did not stabilize");
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_fleet_sound_tuples_bind_hot_rows_markers_and_exact_readback() {
        for (version, initial, hot, selected, editor) in [
            (
                "46.2.1",
                include_str!("../../fixtures/tiktok-publish/musically-46.2.1-en/sound.xml"),
                include_str!("../../fixtures/tiktok-publish/musically-46.2.1-en/hot.xml"),
                include_str!("../../fixtures/tiktok-publish/musically-46.2.1-en/selected.xml"),
                include_str!("../../fixtures/tiktok-publish/musically-46.2.1-en/editor.xml"),
            ),
            (
                "45.4.3",
                include_str!("../../fixtures/tiktok-publish/musically-45.4.3-en/sound.xml"),
                include_str!("../../fixtures/tiktok-publish/musically-45.4.3-en/hot.xml"),
                include_str!("../../fixtures/tiktok-publish/musically-45.4.3-en/selected.xml"),
                include_str!("../../fixtures/tiktok-publish/musically-45.4.3-en/editor.xml"),
            ),
            (
                "46.1.3",
                include_str!("../../fixtures/tiktok-publish/musically-46.1.3-en/sound.xml"),
                include_str!("../../fixtures/tiktok-publish/musically-46.1.3-en/hot.xml"),
                include_str!("../../fixtures/tiktok-publish/musically-46.1.3-en/selected.xml"),
                include_str!("../../fixtures/tiktok-publish/musically-46.1.3-en/editor.xml"),
            ),
            (
                "46.4.3",
                include_str!("../../fixtures/tiktok-publish/musically-46.4.3-en/sound.xml"),
                include_str!("../../fixtures/tiktok-publish/musically-46.4.3-en/hot.xml"),
                include_str!("../../fixtures/tiktok-publish/musically-46.4.3-en/selected.xml"),
                include_str!("../../fixtures/tiktok-publish/musically-46.4.3-en/editor.xml"),
            ),
        ] {
            let plan = SoundPickerPlan::resolve(PACKAGE, "en-US", version).unwrap();
            assert!(
                pool(initial, plan, 5).is_err(),
                "For You cannot prove Hot: {version}"
            );
            let before = pool(hot, plan, 5).unwrap();
            let after = pool(selected, plan, 5).unwrap();
            assert_eq!(
                before.candidates.len(),
                3,
                "clipped row excluded: {version}"
            );
            assert_eq!(before.candidates, after.candidates);
            assert_eq!(after.selected_index, Some(1));
            assert_eq!(after.candidates[1].title, "Sure Thing (Live)");
            let nodes = parse(
                editor,
                SoundPickerPlan {
                    title_id: plan.current_title_id,
                    ..plan
                },
            )
            .unwrap();
            let names: Vec<_> = nodes
                .iter()
                .filter(|n| n.id == plan.current_title_id)
                .collect();
            assert_eq!(names.len(), 1);
            assert_eq!(
                names[0].rect.description.as_deref(),
                Some("Sure Thing (Live)")
            );
            assert!(
                pool(hot, observed_460_plan(), 5).is_err(),
                "cross-version ID borrowing"
            );
        }
        assert!(SoundPickerPlan::resolve(PACKAGE, "en", "46.4.4").is_none());
    }
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    };
    const PACKAGE: &str = "com.zhiliaoapp.musically";

    struct CarouselSession {
        hot: AtomicBool,
        selected: AtomicBool,
        closed: AtomicBool,
        taps: AtomicUsize,
        title: &'static str,
        selection_takes: bool,
        hot_takes_on: usize,
        snapshots: AtomicUsize,
        tap_snapshots: Mutex<Vec<usize>>,
        snapshot_filter: fn(String, usize) -> String,
        snapshot_delay: Duration,
        failed_snapshots: Vec<usize>,
    }
    #[async_trait::async_trait]
    impl UiSession for CarouselSession {
        async fn tap(&self, point: crate::TapPoint) -> anyhow::Result<()> {
            self.tap_snapshots
                .lock()
                .unwrap()
                .push(self.snapshots.load(Ordering::Relaxed));
            let taps = self.taps.fetch_add(1, Ordering::Relaxed) + 1;
            if point.y < 50.0 {
                self.hot.store(taps >= self.hot_takes_on, Ordering::Relaxed);
            } else if self.selection_takes {
                self.selected.fetch_xor(true, Ordering::Relaxed);
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
        async fn back(&self) -> anyhow::Result<()> {
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
        async fn hierarchy_source_snapshot(
            &self,
        ) -> anyhow::Result<crate::driver::HierarchySourceSnapshot> {
            tokio::time::sleep(self.snapshot_delay).await;
            let snapshot = self.snapshots.fetch_add(1, Ordering::Relaxed) + 1;
            if self.failed_snapshots.contains(&snapshot) {
                return Err(crate::driver::AccessibilityReadUnavailable {
                    message: "agent /source lỗi 500: waiting for the root AccessibilityNodeInfo"
                        .into(),
                }
                .into());
            }
            Ok(crate::driver::HierarchySourceSnapshot {
                generation: snapshot as u64,
                xml: (self.snapshot_filter)(
                    fixture(
                        self.hot.load(Ordering::Relaxed),
                        self.selected.load(Ordering::Relaxed),
                    ),
                    snapshot,
                ),
            })
        }
        async fn locate_all_described(
            &self,
            query: ElementQuery<'_>,
        ) -> anyhow::Result<Vec<ElementBox>> {
            Ok(
                if matches!(query, ElementQuery::ResourceIdSuffix(":id/tv_top_text"))
                    && self.closed.load(Ordering::Relaxed)
                {
                    vec![ElementBox {
                        x: 0.0,
                        y: 0.0,
                        width: 100.0,
                        height: 30.0,
                        description: Some(self.title.into()),
                        enabled: true,
                        clickable: false,
                    }]
                } else {
                    vec![]
                },
            )
        }
    }
    fn session(selected: bool) -> CarouselSession {
        CarouselSession {
            hot: AtomicBool::new(false),
            selected: AtomicBool::new(selected),
            closed: AtomicBool::new(false),
            taps: AtomicUsize::new(0),
            title: "Two",
            selection_takes: true,
            hot_takes_on: 1,
            snapshots: AtomicUsize::new(0),
            tap_snapshots: Mutex::new(Vec::new()),
            snapshot_filter: |xml, _| xml,
            snapshot_delay: Duration::ZERO,
            failed_snapshots: Vec::new(),
        }
    }
    #[tokio::test(start_paused = true)]
    async fn transient_source_failure_requires_two_new_snapshots_before_tapping() {
        let mut s = session(false);
        s.failed_snapshots = vec![2];
        select_section_tab(&s, plan()).await.unwrap();
        assert_eq!(*s.tap_snapshots.lock().unwrap(), vec![4]);
    }

    #[tokio::test(start_paused = true)]
    async fn repeated_source_failure_stops_without_any_tap() {
        let mut s = session(false);
        s.failed_snapshots = vec![1, 2];
        let error = select_section_tab(&s, plan()).await.unwrap_err();
        assert!(format!("{error:#}").contains("1 lần phục hồi"));
        assert_eq!(s.snapshots.load(Ordering::Relaxed), 2);
        assert_eq!(s.taps.load(Ordering::Relaxed), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn pool_source_recovery_discards_the_previous_generation() {
        let mut s = session(false);
        s.hot.store(true, Ordering::Relaxed);
        s.failed_snapshots = vec![2];
        observe(&s, plan(), 5).await.unwrap();
        assert_eq!(s.snapshots.load(Ordering::Relaxed), 4);
        assert_eq!(s.taps.load(Ordering::Relaxed), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn source_recovery_does_not_start_after_the_phase_deadline() {
        let mut s = session(false);
        s.failed_snapshots = vec![1];
        s.snapshot_delay = SECTION_WINDOW;
        assert!(select_section_tab(&s, plan()).await.is_err());
        assert_eq!(s.snapshots.load(Ordering::Relaxed), 1);
        assert_eq!(s.taps.load(Ordering::Relaxed), 0);
    }
    #[tokio::test(start_paused = true)]
    async fn hot_tab_recovers_one_missed_tap_after_unselected_readback() {
        let mut s = session(false);
        s.hot_takes_on = 2;
        select_section_tab(&s, plan()).await.unwrap();
        assert_eq!(s.taps.load(Ordering::Relaxed), 2);
        assert!(s.hot.load(Ordering::Relaxed));
        assert!(!s.selected.load(Ordering::Relaxed));
    }

    #[tokio::test(start_paused = true)]
    async fn slow_sound_snapshots_still_allow_hot_selection_and_pool_readback() {
        let mut s = session(false);
        s.snapshot_delay = Duration::from_secs(11);
        s.hot_takes_on = 2;
        select_section_tab(&s, plan()).await.unwrap();
        let pool = observe(&s, plan(), 5).await.unwrap();
        assert_eq!(s.taps.load(Ordering::Relaxed), 2);
        assert_eq!(pool.candidates.len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn hot_tab_recovery_stops_after_two_verified_attempts() {
        let mut s = session(false);
        s.hot_takes_on = usize::MAX;
        let error = select_section_tab(&s, plan()).await.unwrap_err();
        assert_eq!(s.taps.load(Ordering::Relaxed), 2);
        assert!(error.to_string().contains("did not become selected"));
        assert!(!s.selected.load(Ordering::Relaxed));
    }

    #[tokio::test(start_paused = true)]
    async fn already_selected_hot_tab_needs_no_tap() {
        let s = session(false);
        s.hot.store(true, Ordering::Relaxed);
        select_section_tab(&s, plan()).await.unwrap();
        assert_eq!(s.taps.load(Ordering::Relaxed), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn hot_tab_waits_for_sheet_motion_to_settle_before_tapping() {
        let mut s = session(false);
        s.snapshot_filter = |xml, read| {
            if read == 1 {
                xml.replace("[0,0][80,40]", "[0,5][80,45]")
            } else {
                xml
            }
        };
        select_section_tab(&s, plan()).await.unwrap();
        assert_eq!(*s.tap_snapshots.lock().unwrap(), vec![3]);
    }

    #[tokio::test(start_paused = true)]
    async fn unreadable_section_selection_never_authorizes_a_tap() {
        let mut s = session(false);
        s.snapshot_filter = |xml, _| xml.replace("selected=\"true\"", "selected=\"false\"");
        let error = select_section_tab(&s, plan()).await.unwrap_err();
        assert_eq!(s.taps.load(Ordering::Relaxed), 0);
        assert!(error.to_string().contains("sound tab selection unreadable"));
    }
    #[tokio::test(start_paused = true)]
    async fn hot_tab_and_desired_sound_are_selected_once_then_sheet_closes() {
        for selected in [false, true] {
            let s = session(selected);
            select_section_tab(&s, plan()).await.unwrap();
            let p = observe(&s, plan(), 5).await.unwrap();
            choose_and_confirm_sound(&s, plan(), &p, 1).await.unwrap();
            assert_eq!(s.taps.load(Ordering::Relaxed), 1 + usize::from(!selected));
            assert!(s.closed.load(Ordering::Relaxed));
        }
    }
    #[tokio::test(start_paused = true)]
    async fn missing_selection_marker_never_closes_or_retries() {
        let mut s = session(false);
        s.selection_takes = false;
        select_section_tab(&s, plan()).await.unwrap();
        let p = observe(&s, plan(), 5).await.unwrap();
        assert!(choose_and_confirm_sound(&s, plan(), &p, 1).await.is_err());
        assert_eq!(s.taps.load(Ordering::Relaxed), 2);
        assert!(!s.closed.load(Ordering::Relaxed));
    }
    #[tokio::test(start_paused = true)]
    async fn selected_marker_does_not_replace_exact_editor_readback() {
        let mut s = session(true);
        s.title = "Wrong";
        select_section_tab(&s, plan()).await.unwrap();
        let p = observe(&s, plan(), 5).await.unwrap();
        let e = choose_and_confirm_sound(&s, plan(), &p, 1)
            .await
            .unwrap_err();
        assert!(format!("{e:#}").contains("selected sound was not confirmed"));
        assert_eq!(s.taps.load(Ordering::Relaxed), 1);
    }
    fn node(id: &str, text: &str, bounds: &str, selected: bool) -> String {
        format!(
            r#"<android.widget.TextView package="{PACKAGE}" displayed="true" enabled="true" resource-id="{PACKAGE}{id}" text="{text}" selected="{selected}" bounds="{bounds}"/>"#
        )
    }
    fn fixture(hot: bool, marker: bool) -> String {
        let mut xml = String::from("<hierarchy>");
        xml += &node(":id/x2k", "Hot", "[0,0][80,40]", hot);
        xml += &node(":id/x2k", "For You", "[100,0][200,40]", !hot);
        xml += &node(":id/viewpager_container", "", "[0,50][500,310]", false);
        for (index, title) in ["One", "Two"].iter().enumerate() {
            let y = 60 + index * 100;
            xml += &node(
                ":id/vertical_item_music_new_rl",
                "",
                &format!("[0,{y}][500,{}]", y + 90),
                false,
            );
            xml += &node(
                ":id/title",
                title,
                &format!("[60,{}][400,{}]", y + 5, y + 30),
                false,
            );
            xml += &node(
                ":id/zdw",
                "Artist",
                &format!("[60,{}][400,{}]", y + 40, y + 60),
                false,
            );
        }
        if marker {
            xml += &node(":id/nve", "", "[10,165][30,185]", false);
        }
        xml + "</hierarchy>"
    }
    fn plan() -> SoundPickerPlan {
        SoundPickerPlan::resolve(PACKAGE, "en", "46.2.42").unwrap()
    }
    #[test]
    fn visible_hot_is_not_selected_hot() {
        assert!(pool(&fixture(false, true), plan(), 5)
            .unwrap_err()
            .to_string()
            .contains("not selected"));
    }
    #[test]
    fn snapshot_binds_all_rows_and_equalizer() {
        let p = pool(&fixture(true, true), plan(), 5).unwrap();
        assert_eq!(p.candidates.len(), 2);
        assert_eq!(p.selected_index, Some(1));
        assert!(plan().closes_with_back());
        assert!(matches!(
            plan().post_back_query(),
            Some(ElementQuery::ResourceIdSuffix(":id/bot"))
        ));
    }
    #[test]
    fn wrong_package_cannot_prove_tab() {
        assert!(pool(
            &fixture(true, true).replace(PACKAGE, "other.app"),
            plan(),
            5
        )
        .is_err());
    }
    #[test]
    fn truncated_xml_and_duplicate_tab_are_rejected() {
        let xml = fixture(true, true);
        assert!(pool(xml.trim_end_matches("</hierarchy>"), plan(), 5).is_err());
        let duplicate = xml.replace(
            "</hierarchy>",
            &(node(":id/x2k", "Hot", "[0,0][80,40]", true) + "</hierarchy>"),
        );
        assert!(pool(&duplicate, plan(), 5).is_err());
    }
    #[test]
    fn bottom_partial_row_is_not_a_candidate() {
        let xml = fixture(true, true).replace(
            "</hierarchy>",
            &(node(
                ":id/vertical_item_music_new_rl",
                "",
                "[0,260][500,310]",
                false,
            ) + &node(":id/title", "Partial", "[60,265][400,295]", false)
                + "</hierarchy>"),
        );
        assert_eq!(pool(&xml, plan(), 5).unwrap().candidates.len(), 2);
    }

    #[test]
    fn snapshot_uses_all_measured_layout_fields_without_build_constants() {
        let base = plan();
        let measured = SoundPickerPlan {
            package: "com.fixture.sound",
            entry_id: ":id/fixture_entry",
            section_label: "Recommended",
            canonical_section: "recommended",
            row_id: ":id/fixture_row",
            title_id: ":id/fixture_title",
            artist_id: ":id/fixture_artist",
            layout: SoundPickerLayout::TabbedSnapshot(SoundSnapshotLayout {
                tab_id: ":id/fixture_tab",
                viewport_id: ":id/fixture_viewport",
                boundary_rows: SoundBoundaryRows::RequireCompleteText,
            }),
            selection: SoundSelectionMode::Inline {
                marker_ids: &[":id/fixture_marker"],
            },
            ..base
        };
        let xml = fixture(true, true)
            .replace(PACKAGE, measured.package)
            .replace(":id/x2k", ":id/fixture_tab")
            .replace(":id/viewpager_container", ":id/fixture_viewport")
            .replace(":id/vertical_item_music_new_rl", ":id/fixture_row")
            .replace(":id/title", ":id/fixture_title")
            .replace(":id/zdw", ":id/fixture_artist")
            .replace(":id/nve", ":id/fixture_marker")
            .replace("Hot", "Recommended");
        let observed = pool(&xml, measured, 5).unwrap();
        assert_eq!(observed.candidates.len(), 2);
        assert_eq!(observed.candidates[0].section, "recommended");
        assert_eq!(observed.candidates[1].title, "Two");
        assert_eq!(observed.selected_index, Some(1));
        assert!(pool(&xml, base, 5).is_err());
        assert!(pool(&fixture(true, true), measured, 5).is_err());
    }

    // Extracted relevant nodes from q460-{sound,hot,selected-sound,readback}.xml;
    // original captures remain unchanged under target/remote-publish-192.168.1.43.
    const GLOBAL_460_INITIAL: &str =
        include_str!("../../../../fixtures/tiktok/sound-musically-46.0.41-en-sound.xml");
    const GLOBAL_460_HOT: &str =
        include_str!("../../../../fixtures/tiktok/sound-musically-46.0.41-en-hot.xml");
    const GLOBAL_460_SELECTED: &str =
        include_str!("../../../../fixtures/tiktok/sound-musically-46.0.41-en-selected-sound.xml");
    const GLOBAL_460_READBACK: &str =
        include_str!("../../../../fixtures/tiktok/sound-musically-46.0.41-en-readback.xml");

    fn observed_460_plan() -> SoundPickerPlan {
        SoundPickerPlan::resolve(PACKAGE, "en-US", "46.0.41").unwrap()
    }

    #[test]
    fn measured_45_7_3_requires_hot_and_confirms_selected_vietnamese_title() {
        let plan = SoundPickerPlan::resolve(PACKAGE, "en", "45.7.3").unwrap();
        let initial =
            include_str!("../../fixtures/tiktok-publish/musically-45.7.3-en/11-sound.xml");
        let hot =
            include_str!("../../fixtures/tiktok-publish/musically-45.7.3-en/13-hot-stable.xml");
        let selected =
            include_str!("../../fixtures/tiktok-publish/musically-45.7.3-en/14-selected.xml");
        let readback =
            include_str!("../../fixtures/tiktok-publish/musically-45.7.3-en/15-readback.xml");
        assert!(pool(initial, plan, 5).is_err());
        let before = pool(hot, plan, 5).unwrap();
        let after = pool(selected, plan, 5).unwrap();
        assert_eq!(before.candidates.len(), 3);
        assert_eq!(before.selected_index, None);
        assert_eq!(before.candidates, after.candidates);
        assert_eq!(after.selected_index, Some(1));
        assert_eq!(after.candidates[1].title, "Đến Khi Nào");
        assert_eq!(
            plan.post_back_query(),
            Some(ElementQuery::ResourceIdSuffix(":id/bix"))
        );
        let nodes = parse(
            readback,
            SoundPickerPlan {
                title_id: plan.current_title_id,
                ..plan
            },
        )
        .unwrap();
        let titles: Vec<_> = nodes
            .iter()
            .filter(|node| node.id == plan.current_title_id)
            .collect();
        assert_eq!(titles.len(), 1);
        assert_eq!(titles[0].rect.description.as_deref(), Some("Đến Khi Nào"));
        assert!(pool(hot, observed_460_plan(), 5).is_err());
    }

    #[test]
    fn measured_global_460_hot_pool_excludes_the_visibly_clipped_bottom_artist() {
        let plan = observed_460_plan();
        assert!(
            pool(GLOBAL_460_INITIAL, plan, 5).is_err(),
            "For You is not selected Hot"
        );
        let observed = pool(GLOBAL_460_HOT, plan, 5).unwrap();
        assert_eq!(observed.candidates.len(), 3);
        assert_eq!(observed.candidates[0].title, "Summer Bummer (Lights On)");
        assert_eq!(observed.candidates[1].title, "Sure Thing (Live)");
        assert_eq!(observed.selected_index, None);
        assert!(!observed.candidates.iter().any(|row| row.title == "BbY WOW"));
    }

    #[test]
    fn measured_global_460_marker_binds_to_the_selected_row_and_editor_title() {
        let plan = observed_460_plan();
        let before = pool(GLOBAL_460_HOT, plan, 5).unwrap();
        let after = pool(GLOBAL_460_SELECTED, plan, 5).unwrap();
        assert_eq!(after.candidates, before.candidates);
        assert_eq!(after.selected_index, Some(1));
        assert_eq!(
            plan.post_back_query(),
            Some(ElementQuery::ResourceIdSuffix(":id/bmy"))
        );
        assert!(
            after.target(1).unwrap().x > before.target(1).unwrap().x,
            "equalizer moves the title; reproof must use the fresh rectangle"
        );
        let nodes = parse(
            GLOBAL_460_READBACK,
            SoundPickerPlan {
                title_id: plan.current_title_id,
                ..plan
            },
        )
        .unwrap();
        let titles: Vec<_> = nodes
            .iter()
            .filter(|node| node.id == plan.current_title_id)
            .collect();
        assert_eq!(titles.len(), 1);
        assert_eq!(
            titles[0].rect.description.as_deref(),
            Some("Sure Thing (Live)")
        );
        assert!(
            pool(GLOBAL_460_SELECTED, self::plan(), 5).is_err(),
            "older tuple cannot borrow new labels"
        );
    }
}
