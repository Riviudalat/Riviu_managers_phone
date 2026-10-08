//! Read one hierarchy generation using the tuple's measured sound-sheet layout.
//! Rows, selected tab and inline markers always come from the same snapshot.
use super::*;

// AGENTS.md §9.187: 0.2.11 exhausted eight seconds before any Hot tap on the
// local 45.7.3 phones 2/3. Android source reads take multiple seconds; the
// driver's measured worst root-query regime is 11 seconds. Two stable reads,
// one missed tap, two fresh reads for the retry and its confirmation need five
// such reads plus polling. Production shares one three-minute budget across all phases.
const SECTION_WINDOW: Duration = Duration::from_secs(60);
const SNAPSHOT_POOL_WINDOW: Duration = SOUND_WINDOW;

/// A loading sheet may temporarily have no accessibility root. Keep observing
/// within the existing phase/global deadline, clearing prior geometry each time.
/// Only successful fresh snapshots can authorize a later navigation tap.
async fn read_snapshot(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    deadline: Instant,
    recoveries: &mut u32,
    covered_generation: &mut Option<u64>,
    retry_transient: bool,
) -> anyhow::Result<Option<crate::HierarchySourceSnapshot>> {
    let labels = measured_popup_labels(plan);
    if labels.is_none() {
        clear_sound_popup(session, deadline, Some(plan.package)).await?;
    }
    let started = Instant::now();
    check_wait()?;
    anyhow::ensure!(
        started < deadline,
        "đọc bảng nhạc hết thời gian chờ; chưa bấm Đăng"
    );
    let epoch = session.gui_session_epoch();
    let foreground = if labels.is_some() {
        let package = read_sound(session.active_app_bundle()).await?;
        anyhow::ensure!(!epoch.is_empty() && (package == plan.package
            || package == "com.google.android.packageinstaller"), "sound snapshot foreground changed");
        Some(package)
    } else { None };
    let observed =
        tokio::time::timeout_at(deadline, read_sound(session.hierarchy_source_snapshot()))
            .await
            .context("đọc bảng nhạc hết thời gian chờ 3 phút; chưa bấm Đăng")?;
    match observed {
        Ok(snapshot) => {
            check_wait()?;
            if let Some(package) = foreground.as_ref() {
                anyhow::ensure!(session.gui_session_epoch() == epoch
                    && read_sound(session.active_app_bundle()).await? == *package,
                    "sound snapshot foreground or session changed during read");
            }
            if let Some(labels) = labels {
                if covered_generation.is_some_and(|generation| snapshot.generation <= generation) {
                    return Ok(None);
                }
                let tree = crate::ui_automation::tree::Tree::parse(snapshot.clone())?;
                if popup_covered(&tree, labels) {
                    *covered_generation = Some(snapshot.generation);
                    clear_sound_popup(session, deadline, Some(plan.package)).await?;
                    return Ok(None);
                }
            }
            if let Some(package) = foreground.as_ref() {
                anyhow::ensure!(package == plan.package && session.gui_session_epoch() == epoch,
                    "sound snapshot foreground changed before row proof");
            }
            Ok(Some(snapshot))
        }
        Err(error) if retry_transient
            && !error.is::<SoundStopped>()
            && !error.is::<crate::driver::SessionEpochChanged>()
            && transient_sound_read(&error) && Instant::now() + POLL < deadline => {
            *recoveries += 1;
            tracing::warn!(elapsed_ms = started.elapsed().as_millis() as u64,
                    recovery = *recoveries, remaining_ms = deadline.saturating_duration_since(Instant::now()).as_millis() as u64,
                    error = %error, "sound screen read unavailable; observing once more");
            tokio::time::sleep(POLL).await;
            Ok(None)
        }
        Err(error) => Err(error.context(format!(
            "đọc bảng nhạc thất bại sau {} lần phục hồi bổ sung; request {} ms",
            *recoveries,
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
    let deadline = phase_deadline(SECTION_WINDOW);
    let mut previous: Option<ElementBox> = None;
    let mut attempts = 0;
    let mut used_image_navigation = false;
    let mut retry_after = Instant::now();
    let mut recovery_used = 0;
    let mut covered_generation = None;
    let mut observation = "sound sheet not yet observed".to_string();
    loop {
        let source = read_snapshot(session, plan, deadline, &mut recovery_used, &mut covered_generation, true)
            .await
            .with_context(|| {
                format!(
                    "{} sound section did not become selected after {attempts} tap(s): {observation}",
                    plan.section_label
                )
            })?;
        let parsed = source
            .as_ref()
            .map(|source| parse(&source.xml, plan))
            .transpose()?;
        let Some(parsed) = parsed.filter(|nodes| !nodes.is_empty()) else {
            previous = None;
            // 45.7.3 can render all four tabs while returning no accessibility
            // root. Two bound native frames may navigate Hot once, but cannot
            // stand in for the XML tab/row proof required below and by observe.
            if attempts == 0
                && !used_image_navigation
                && covered_generation.is_none()
                && selection_recovery::measured(plan)
                && session.gui_reasoner().is_some()
            {
                let point = selection_recovery::prove_sheet(session, plan).await?;
                anyhow::ensure!(Instant::now() < deadline, "sound tab recovery expired");
                used_image_navigation = true;
                attempts += 1;
                tracing::info!(
                    x = point.x,
                    y = point.y,
                    "sound Hot navigation proved by two fresh OCR frames"
                );
                sound_tap(session, point).await?;
                observation = "Hot navigation dispatched; awaiting XML confirmation".into();
            }
            continue;
        };
        observation = match section_tab(&parsed, plan) {
            Ok((_, true)) => return Ok(()),
            Ok((tab, false)) => {
                // LAN 46.0.41 campaigns stopped here without selecting Hot
                // (AGENTS.md §9.186). Wait for stable bounds while the sheet opens.
                // A section tab is idempotent: one fresh, still-unselected readback
                // may authorize one retry. Sound rows themselves remain single-tap.
                let now = Instant::now();
                if previous.as_ref() == Some(tab)
                    && !used_image_navigation
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
                    sound_tap(session, tab.centre())
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

/// Exact measured profiles can classify a popup from the XML already read.
fn measured_popup_labels(plan: SoundPickerPlan) -> Option<crate::tiktok_labels::TikTokControls> {
    let version = if selection_recovery::measured(plan) {
        "45.7.3"
    } else if SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2") == Some(plan) {
        "38.3.2"
    } else {
        return None;
    };
    crate::tiktok_labels::controls_for(plan.package, "en", version)
}

fn popup_covered(tree: &crate::ui_automation::tree::Tree, labels: crate::tiktok_labels::TikTokControls) -> bool {
    let popup = crate::app_automation::dialogs::optional_decline(tree, labels).is_some();
    let blocked = matches!(
        crate::app_automation::dialogs::account_blocker(tree, labels),
        Some(crate::app_automation::dialogs::AccountBlocker::LoginRequired
            | crate::app_automation::dialogs::AccountBlocker::SecurityPrompt)
    );
    popup || blocked || tree.nodes.iter().enumerate().any(|(index, node)| {
        node.visibility() != Some(false)
            && tree.ancestors_visible(index)
            && node.rect().is_some()
            && (node.attr("content-desc") == "Dialog"
                || node.attr("resource-id") == "com.android.packageinstaller:id/dialog_container")
    })
}

/// Wait on the already-open measured Recommended sheet without navigation.
/// Only an affirmative, uniquely selected other tab authorizes caller fallback.
pub(super) async fn observe_current_recommended_until_loaded(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
) -> anyhow::Result<Option<ObservedSoundPool>> {
    anyhow::ensure!(selection_recovery::measured(plan)
        && plan.section_label == "For You" && plan.canonical_section == "recommended",
        "sound current list tuple unmeasured");
    let deadline = phase_deadline(SOUND_WINDOW);
    Box::pin(with_deadline_budget(&AtomicBool::new(false), deadline,
        Box::pin(observe_current_recommended_inner(session, plan, maximum, deadline)))).await
}

async fn observe_current_recommended_inner(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
    deadline: Instant,
) -> anyhow::Result<Option<ObservedSoundPool>> {
    let epoch = session.gui_session_epoch();
    anyhow::ensure!(!epoch.is_empty(), "sound snapshot session missing");
    let mut previous: Option<(u64, ObservedSoundPool)> = None;
    let mut recoveries = 0;
    let mut covered_generation = None;
    loop {
        check_wait()?;
        anyhow::ensure!(session.gui_session_epoch() == epoch, "sound app/session replaced");
        let source = read_snapshot(session, plan, deadline, &mut recoveries,
            &mut covered_generation, true).await?;
        anyhow::ensure!(session.gui_session_epoch() == epoch, "sound app/session replaced after snapshot");
        let observed = source.as_ref().filter(|source| source.generation > 0)
            .map(|source| -> anyhow::Result<Option<ObservedSoundPool>> {
                // Reuse the same measured tab/parser constraints as pool().
                let nodes = parse(&source.xml, plan)?;
                if !section_tab(&nodes, plan)?.1 {
                    let layout = plan.snapshot_layout().expect("measured current list");
                    anyhow::ensure!(nodes.iter().any(|node| node.id == layout.tab_id
                        && node.selected && matches!(node.rect.description.as_deref(),
                            Some("Hot" | "Favorites" | "Recent"))),
                        "sound selected other tab unmeasured");
                    return Ok(None);
                }
                pool(&source.xml, plan, maximum).map(Some)
            });
        match observed {
            Some(Ok(None)) => return Ok(None),
            Some(Ok(Some(current))) => {
                let generation = source.as_ref().expect("observed source").generation;
                if previous.as_ref().is_some_and(|(before, prior)|
                    generation > *before && prior.stable_with(&current)) {
                    return Ok(Some(current));
                }
                previous = Some((generation, current));
            }
            Some(Err(error)) => {
                previous = None;
                tracing::debug!(error = %error, "current Recommended rows not yet proved; read-only wait");
            }
            None => previous = None,
        }
        check_wait()?;
        if Instant::now() >= deadline {
            return Err(crate::publish_recovery::observation_deadline());
        }
        tokio::time::sleep(POLL.min(deadline.saturating_duration_since(Instant::now()))).await;
    }
}

/// Incomplete/loading roots are not candidates and do not authorize a tap.
pub(super) async fn observe_direct(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
) -> anyhow::Result<Option<ObservedSoundPool>> {
    let epoch = session.gui_session_epoch();
    anyhow::ensure!(!epoch.is_empty(), "sound snapshot session missing");
    let current_list = selection_recovery::measured(plan)
        && plan.section_label == "For You"
        && plan.canonical_section == "recommended";
    let mut previous: Option<(u64, ObservedSoundPool)> = None;
    let mut covered_generation = None;
    let mut reads = 0;
    let mut limit = 2;
    while reads < limit {
        reads += 1;
        check_wait()?;
        let foreground = read_sound(session.active_app_bundle()).await?;
        anyhow::ensure!(
            session.gui_session_epoch() == epoch
                && (foreground == plan.package
                    || (current_list && foreground == "com.google.android.packageinstaller")),
            "sound app/session replaced"
        );
        let source = match tokio::time::timeout(
            Duration::from_secs(12),
            read_sound(session.hierarchy_source_snapshot()),
        )
        .await
        {
            Ok(Ok(source)) => source,
            Ok(Err(error)) if error.is::<SoundStopped>() => return Err(error),
            _ => return Ok(None),
        };
        check_wait()?;
        anyhow::ensure!(
            session.gui_session_epoch() == epoch
                && read_sound(session.active_app_bundle()).await? == foreground,
            "sound app/session replaced after snapshot"
        );
        if current_list {
            if source.generation == 0 {
                return Ok(None);
            }
            if covered_generation.is_some_and(|generation| source.generation <= generation) {
                continue;
            }
            let tree = match crate::ui_automation::tree::Tree::parse(source.clone()) {
                Ok(tree) => tree,
                Err(_) => return Ok(None),
            };
            if popup_covered(&tree, measured_popup_labels(plan).expect("measured current list")) {
                previous = None;
                covered_generation = Some(source.generation);
                clear_sound_popup(session, phase_deadline(SOUND_WINDOW), Some(plan.package)).await?;
                limit = 4;
                continue;
            }
        }
        anyhow::ensure!(foreground == plan.package, "sound app changed before row proof");
        let current = match pool(&source.xml, plan, maximum) {
            Ok(pool) => pool,
            Err(_) => return Ok(None),
        };
        if let Some((generation, prior)) = &previous {
            return Ok(
                (source.generation > *generation && prior.stable_with(&current)).then_some(current),
            );
        }
        previous = Some((source.generation, current));
        tokio::time::sleep(POLL).await;
    }
    Ok(None)
}

/// Image-unavailable path for the measured Global 45.7.3 sheet only.
/// It never infers a tab or row from a missing screenshot.
pub(super) async fn select_section_tab_xml_only(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        selection_recovery::measured(plan),
        "sound XML-only tab unmeasured"
    );
    let epoch = session.gui_session_epoch();
    anyhow::ensure!(!epoch.is_empty(), "sound sheet session missing");
    let deadline = phase_deadline(SECTION_WINDOW);
    let mut previous: Option<(u64, ElementBox, bool)> = None;
    let mut tapped = false;
    loop {
        check_wait()?;
        anyhow::ensure!(
            session.gui_session_epoch() == epoch
                && read_sound(session.active_app_bundle()).await? == plan.package,
            "sound app/session changed during XML-only tab proof"
        );
        let source =
            tokio::time::timeout_at(deadline, read_sound(session.hierarchy_source_snapshot()))
                .await
                .context("sound XML-only tab deadline")??;
        let tab = parse(&source.xml, plan).and_then(|nodes| {
            section_tab(&nodes, plan).map(|(rect, selected)| (rect.clone(), selected))
        });
        if let Ok((rect, selected)) = tab {
            if let Some((generation, before, was_selected)) = &previous {
                if source.generation > *generation && before == &rect && *was_selected == selected {
                    if selected {
                        return Ok(());
                    }
                    if !tapped {
                        anyhow::ensure!(
                            session.gui_session_epoch() == epoch
                                && read_sound(session.active_app_bundle()).await? == plan.package,
                            "sound app/session changed before Hot tap"
                        );
                        sound_tap(session, rect.centre()).await?;
                        tapped = true;
                        previous = None;
                    }
                } else {
                    previous = Some((source.generation, rect, selected));
                }
            } else {
                previous = Some((source.generation, rect, selected));
            }
        } else {
            previous = None;
        }
        anyhow::ensure!(Instant::now() < deadline, "sound Hot XML did not stabilize");
        tokio::time::sleep(POLL).await;
    }
}

pub(super) async fn observe_xml_only(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
) -> anyhow::Result<ObservedSoundPool> {
    anyhow::ensure!(
        selection_recovery::measured(plan),
        "sound XML-only pool unmeasured"
    );
    let epoch = session.gui_session_epoch();
    anyhow::ensure!(!epoch.is_empty(), "sound sheet session missing");
    let deadline = phase_deadline(SNAPSHOT_POOL_WINDOW);
    let mut previous: Option<(u64, ObservedSoundPool)> = None;
    loop {
        check_wait()?;
        anyhow::ensure!(
            session.gui_session_epoch() == epoch
                && read_sound(session.active_app_bundle()).await? == plan.package,
            "sound app/session changed during XML-only pool proof"
        );
        let source =
            tokio::time::timeout_at(deadline, read_sound(session.hierarchy_source_snapshot()))
                .await
                .context("sound XML-only pool deadline")??;
        match pool(&source.xml, plan, maximum) {
            Ok(current) => {
                if previous.as_ref().is_some_and(|(generation, before)| {
                    source.generation > *generation && before.stable_with(&current)
                }) {
                    return Ok(current);
                }
                previous = Some((source.generation, current));
            }
            Err(_) => previous = None,
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "sound Hot XML pool did not stabilize"
        );
        tokio::time::sleep(POLL).await;
    }
}

pub(super) async fn observe(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
) -> anyhow::Result<ObservedSoundPool> {
    observe_until(session, plan, maximum, phase_deadline(SNAPSHOT_POOL_WINDOW)).await
}

pub(super) async fn observe_until(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
    deadline: Instant,
) -> anyhow::Result<ObservedSoundPool> {
    let deadline = deadline.min(phase_deadline(SNAPSHOT_POOL_WINDOW));
    Box::pin(with_deadline_budget(&AtomicBool::new(false), deadline,
        Box::pin(observe_inner(session, plan, maximum, true, deadline)))).await
}

/// Once a sound has been tapped, propagate unreadable UI to the caller so it
/// can prove the sheet visually and verify the editor instead of spending the
/// entire remaining budget polling a broken root. Never repeat the selection.
pub(super) async fn observe_after_selection(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
    deadline: Instant,
) -> anyhow::Result<ObservedSoundPool> {
    let deadline = deadline.min(phase_deadline(SNAPSHOT_POOL_WINDOW));
    Box::pin(with_deadline_budget(&AtomicBool::new(false), deadline,
        Box::pin(observe_inner(session, plan, maximum, false, deadline)))).await
}

async fn observe_inner(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
    retry_unavailable: bool,
    deadline: Instant,
) -> anyhow::Result<ObservedSoundPool> {
    let epoch = session.gui_session_epoch();
    let bound_trill = plan.package == "com.ss.android.ugc.trill";
    if bound_trill { anyhow::ensure!(!epoch.is_empty(), "sound snapshot session missing"); }
    let mut previous: Option<(u64, ObservedSoundPool)> = None;
    let mut recovery_used = 0;
    let mut covered_generation = None;
    loop {
        check_wait()?;
        if bound_trill {
            anyhow::ensure!(session.gui_session_epoch() == epoch
                && read_sound(session.active_app_bundle()).await? == plan.package,
                "sound snapshot app/session changed");
        }
        let source = if retry_unavailable || measured_popup_labels(plan).is_some() {
            let Some(source) = read_snapshot(session, plan, deadline, &mut recovery_used,
                &mut covered_generation, retry_unavailable).await?
            else {
                previous = None;
                continue;
            };
            source
        } else {
            read_sound(session.hierarchy_source_snapshot()).await?
        };
        check_wait()?;
        if Instant::now() >= deadline { return Err(crate::publish_recovery::observation_deadline()); }
        if bound_trill {
            anyhow::ensure!(session.gui_session_epoch() == epoch
                && read_sound(session.active_app_bundle()).await? == plan.package,
                "sound snapshot app/session changed after read");
        }
        let observed = pool(&source.xml, plan, maximum);
        match observed {
            Ok(current) => {
                if previous.as_ref().is_some_and(|(generation, prior)| {
                    (!(selection_recovery::measured(plan) || bound_trill) || source.generation > *generation)
                        && prior.stable_with(&current)
                }) {
                    return Ok(current);
                }
                previous = Some((source.generation, current));
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

    struct CurrentRecommendedLoadingSession {
        snapshots: Vec<crate::HierarchySourceSnapshot>,
        reads: std::sync::atomic::AtomicUsize,
        /// Hierarchy calls (0-based) that wait out UiAutomator2's root timeout.
        root_timeouts: Vec<usize>,
        pins: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        agent_session: std::sync::atomic::AtomicUsize,
    }

    impl CurrentRecommendedLoadingSession {
        fn new(snapshots: Vec<crate::HierarchySourceSnapshot>, root_timeouts: Vec<usize>) -> Self {
            Self {
                snapshots,
                reads: Default::default(),
                root_timeouts,
                pins: Default::default(),
                agent_session: Default::default(),
            }
        }
    }

    /// The literal 500 the driver surfaced on #29/#30 (controller-final-caption.stdout.txt:1600).
    const ROOT_TIMEOUT: &str = "agent /source lỗi 500 Internal Server Error: Timed out after \
        10504ms waiting for the root AccessibilityNodeInfo in the active window";

    struct FixtureReadPin(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    impl Drop for FixtureReadPin {
        fn drop(&mut self) {
            self.0.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    #[async_trait::async_trait]
    impl UiSession for CurrentRecommendedLoadingSession {
        async fn tap(&self, _: crate::TapPoint) -> anyhow::Result<()> {
            panic!("current Recommended loading must not navigate or select a row")
        }
        async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> { unreachable!() }
        async fn type_text(&self, _: &str) -> anyhow::Result<()> { unreachable!() }
        async fn home(&self) -> anyhow::Result<()> { unreachable!() }
        async fn back(&self) -> anyhow::Result<()> { unreachable!() }
        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> { unreachable!() }
        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> { unreachable!() }
        fn stream_url(&self) -> Option<String> { None }
        fn gui_session_epoch(&self) -> String {
            let agent = self
                .agent_session
                .load(std::sync::atomic::Ordering::Relaxed);
            format!("current-song-22-fixture-epoch:{agent}")
        }
        fn pin_read_session(&self) -> crate::driver::ReadSessionPin {
            self.pins.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            crate::driver::ReadSessionPin::holding(FixtureReadPin(self.pins.clone()))
        }
        async fn active_app_bundle(&self) -> anyhow::Result<String> {
            Ok("com.zhiliaoapp.musically".into())
        }
        async fn hierarchy_source_snapshot(&self) -> anyhow::Result<crate::HierarchySourceSnapshot> {
            use std::sync::atomic::Ordering::Relaxed;
            let read = self.reads.fetch_add(1, Relaxed);
            if self.root_timeouts.contains(&read) {
                // The Android agent answers this 500 by replacing its session and reading once
                // more, unless the session is pinned. On the 45.7.3 sheet the repeat timed out
                // as well: about 21 s in controller-final-caption.stdout.txt 04:40:35–04:40:45.
                let pinned = self.pins.load(Relaxed) > 0;
                tokio::time::sleep(Duration::from_millis(if pinned { 10_504 } else { 21_335 }))
                    .await;
                if !pinned {
                    self.agent_session.fetch_add(1, Relaxed);
                }
                return Err(crate::driver::AccessibilityReadUnavailable {
                    message: ROOT_TIMEOUT.into(),
                }
                .into());
            }
            let served = read - self.root_timeouts.iter().filter(|&&t| t < read).count();
            Ok(self.snapshots[served.min(self.snapshots.len() - 1)].clone())
        }
    }

    #[tokio::test(start_paused = true)]
    async fn current_recommended_loading_waits_for_two_fresh_stable_raw_pools() {
        use sha2::{Digest, Sha256};
        use std::sync::atomic::{AtomicBool, Ordering};
        // Native machine22 captures retained byte-for-byte under the measured tuple.
        let captures: [(u64, &[u8], &str); 3] = [
            (34, include_bytes!("../../fixtures/tiktok-publish/musically-45.7.3-en/current-song-machine22-loading-generation34.xml"), "53fcfc03e22b5939a2604b42ecf5ca9ed7e2b43ea982cc522ee8f87ad53192f7"),
            (35, include_bytes!("../../fixtures/tiktok-publish/musically-45.7.3-en/current-song-machine22-loading-generation35.xml"), "62f43802013b63b45b6f535420bd5ee625ea3f61dc48cea239f8f36c064e3ca7"),
            (36, include_bytes!("../../fixtures/tiktok-publish/musically-45.7.3-en/current-song-machine22-loading-generation36.xml"), "faa2be835039a44fde087769607a4cd3febcb1acdeb8467c2e40fcb9d2a7b239"),
        ];
        let base = SoundPickerPlan::resolve("com.zhiliaoapp.musically", "en", "45.7.3").unwrap();
        let current = SoundPickerPlan {
            section_label: "For You", canonical_section: "recommended", ..base
        };
        let mut snapshots = Vec::new();
        for ((generation, raw, expected_hash), expected_count) in captures.into_iter().zip([1, 4, 4]) {
            assert_eq!(format!("{:x}", Sha256::digest(raw)), expected_hash);
            let xml = std::str::from_utf8(raw).unwrap().to_owned();
            assert_eq!(pool(&xml, current, 5).unwrap().candidates.len(), expected_count);
            snapshots.push(crate::HierarchySourceSnapshot { generation, xml });
        }
        let s = CurrentRecommendedLoadingSession::new(snapshots, Vec::new());
        let stop = AtomicBool::new(false);
        let observed = with_sound_budget(&stop,
            observe_current_recommended(&s, base, 5, true)).await.unwrap()
            .expect("one-to-four loading change must wait for generation36 instead of Hot fallback");
        assert_eq!(s.reads.load(Ordering::Relaxed), 3);
        assert_eq!(observed.candidates.len(), 4);
        assert!(observed.candidates.iter().all(|candidate| candidate.section == "recommended"));
        assert_eq!(observed.selected_index(), Some(0));
        assert_eq!(observed.effective_plan(base), current);
    }

    /// #22/#29/#30, Global 45.7.3/en, 2026-10-08 04:36–04:41Z: the just-opened Recommended sheet
    /// held UiAutomator2's root wait. Read recovery replaced the agent session, and the guard after
    /// the snapshot ended each attempt before Post ("sound app/session replaced after snapshot").
    /// The sound flow pins its session, so the timeout is an unknown read observed again inside
    /// the unchanged budget, and only the fresh generations that follow can form the pool.
    #[tokio::test(start_paused = true)]
    async fn current_recommended_root_timeout_is_observed_again_on_the_pinned_session() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let snapshots = [
            (34, include_str!("../../fixtures/tiktok-publish/musically-45.7.3-en/current-song-machine22-loading-generation34.xml")),
            (35, include_str!("../../fixtures/tiktok-publish/musically-45.7.3-en/current-song-machine22-loading-generation35.xml")),
            (36, include_str!("../../fixtures/tiktok-publish/musically-45.7.3-en/current-song-machine22-loading-generation36.xml")),
        ]
        .into_iter()
        .map(|(generation, xml)| crate::HierarchySourceSnapshot { generation, xml: xml.into() })
        .collect();
        let base = SoundPickerPlan::resolve("com.zhiliaoapp.musically", "en", "45.7.3").unwrap();
        let s = CurrentRecommendedLoadingSession::new(snapshots, vec![0]);
        let epoch = s.gui_session_epoch();
        let stop = AtomicBool::new(false);
        let started = Instant::now();
        let observed = with_sound_budget(&stop, resume_open_sounds_current(&s, base, 5, true))
            .await
            .expect("a root timeout on the loading sheet must not end the sound read");
        assert_eq!(
            s.gui_session_epoch(),
            epoch,
            "the agent session was replaced"
        );
        assert_eq!(
            s.reads.load(Ordering::Relaxed),
            4,
            "one unknown read, then generations 34-36"
        );
        assert_eq!(
            s.pins.load(Ordering::Relaxed),
            0,
            "the pin ends with the sound call"
        );
        assert_eq!(observed.candidates.len(), 4);
        assert_eq!(observed.selected_index(), Some(0));
        assert!(started.elapsed() < SOUND_WINDOW);
    }

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
    const TRILL_LIST: &str = include_str!("../../fixtures/tiktok-publish/trill-38.3.2-en/actual-failure-tree-3.xml");
    const TRILL_EDITOR: &str = include_str!("../../fixtures/tiktok-publish/trill-38.3.2-en/sound3-postback-editor.xml");

    struct ActualTrillSession {
        reads: AtomicUsize,
        slow_after: usize,
        closed: AtomicBool,
        backs: AtomicUsize,
        row_taps: AtomicUsize,
    }
    impl ActualTrillSession {
        fn xml(&self) -> &'static str {
            if self.closed.load(Ordering::Relaxed) { TRILL_EDITOR } else { TRILL_LIST }
        }
        async fn delay(&self) {
            if self.reads.fetch_add(1, Ordering::Relaxed) >= self.slow_after {
                tokio::time::sleep(Duration::from_secs(20)).await;
            }
        }
    }
    #[async_trait::async_trait]
    impl UiSession for ActualTrillSession {
        async fn tap(&self, _: crate::TapPoint) -> anyhow::Result<()> {
            self.row_taps.fetch_add(1, Ordering::Relaxed); Ok(())
        }
        async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> { unreachable!() }
        async fn type_text(&self, _: &str) -> anyhow::Result<()> { unreachable!() }
        async fn home(&self) -> anyhow::Result<()> { unreachable!() }
        async fn back(&self) -> anyhow::Result<()> {
            self.backs.fetch_add(1, Ordering::Relaxed);
            self.closed.store(true, Ordering::Relaxed); Ok(())
        }
        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> { unreachable!() }
        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> { unreachable!() }
        fn stream_url(&self) -> Option<String> { None }
        fn gui_session_epoch(&self) -> String { "actual-trill-fixture-epoch".into() }
        async fn active_app_bundle(&self) -> anyhow::Result<String> { Ok("com.ss.android.ugc.trill".into()) }
        async fn hierarchy_source_snapshot(&self) -> anyhow::Result<crate::HierarchySourceSnapshot> {
            self.delay().await;
            Ok(crate::HierarchySourceSnapshot { generation: self.reads.load(Ordering::Relaxed) as u64, xml: self.xml().into() })
        }
        async fn locate_all(&self, query: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
            self.locate_all_described(query).await
        }
        async fn locate_all_described(&self, query: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
            // Compatibility projection reads the same verbatim artifact; it does not
            // supply selected-index or editor-confirmed answers on behalf of production.
            self.delay().await;
            let tree = crate::ui_automation::tree::Tree::parse(crate::HierarchySourceSnapshot {
                generation: self.reads.load(Ordering::Relaxed) as u64, xml: self.xml().into(),
            })?;
            Ok(tree.nodes.iter().enumerate().filter(|(index, node)| {
                node.visible("com.ss.android.ugc.trill") && tree.ancestors_visible(*index)
                    && match query {
                        ElementQuery::ResourceIdSuffix(id) => node.attr("resource-id").ends_with(id),
                        ElementQuery::Text { value, exact: true } => node.attr("text") == value,
                        _ => false,
                    }
            }).filter_map(|(_, node)| node.rect()).collect())
        }
    }

    #[tokio::test(start_paused = true)]
    async fn actual_trill_selected_row_requires_real_editor_proof() {
        let plan = SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2").unwrap();
        let actual = ActualTrillSession { reads: AtomicUsize::new(0), slow_after: usize::MAX,
            closed: AtomicBool::new(false), backs: AtomicUsize::new(0), row_taps: AtomicUsize::new(0) };
        let observed = pool(TRILL_LIST, plan, 5).unwrap();
        assert_eq!(observed.candidates.len(), 5);
        assert_eq!(observed.selected_index, Some(0));
        assert_eq!(observed.candidates[0].artist, concat!("TINH H\u{00c0} ", r#""SAY HI""#));
        assert!(confirm_sound(&actual, plan, &observed.candidates[0].title).await.is_err(),
            "jk1 in an open sheet is not final editor proof");
        actual.reads.store(0, Ordering::Relaxed);
        let stop = AtomicBool::new(false);
        with_sound_budget(&stop, choose_and_confirm_sound(&actual, plan, &observed, 0)).await.unwrap();
        assert_eq!(actual.backs.load(Ordering::Relaxed), 1);
        assert_eq!(actual.row_taps.load(Ordering::Relaxed), 0);
        assert!(confirm_sound(&actual, plan, "a different title").await.is_err());

    }

    #[tokio::test(start_paused = true)]
    async fn actual_trill_post_row_observation_keeps_its_local_deadline() {
        // Use the measured layout explicitly so the old deadline behavior can
        // fail independently of adding this tuple's production snapshot route.
        let plan = SoundPickerPlan {
            layout: SoundPickerLayout::TabbedSnapshot(SoundSnapshotLayout {
                tab_id: ":id/q_g", viewport_id: ":id/tka",
                boundary_rows: SoundBoundaryRows::RequireCompleteText,
            }),
            ..SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2").unwrap()
        };
        let observed = pool(TRILL_LIST, plan, 5).unwrap();
        let stop = AtomicBool::new(false);
        // Two fresh pre-row snapshots succeed; post-row source stalls beyond8s.
        let slow = ActualTrillSession { reads: AtomicUsize::new(0), slow_after: 2,
            closed: AtomicBool::new(false), backs: AtomicUsize::new(0), row_taps: AtomicUsize::new(0) };
        let started = Instant::now();
        assert!(with_sound_budget(&stop, choose_and_confirm_sound(&slow, plan, &observed, 2)).await.is_err());
        assert!(started.elapsed() <= READBACK_WINDOW + POLL,
            "post-row observation spent the global budget instead of its local window");
        assert_eq!(slow.backs.load(Ordering::Relaxed), 0);
        assert_eq!(slow.row_taps.load(Ordering::Relaxed), 1);
    }

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
        // The sheet stays in the bound app; choose_and_confirm_sound reads the
        // foreground before Back closes it.
        async fn active_app_bundle(&self) -> anyhow::Result<String> {
            Ok(PACKAGE.into())
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
        s.failed_snapshots = (1..=1000).collect();
        s.snapshot_delay = Duration::from_secs(21);
        let started = Instant::now();
        let error = select_section_tab(&s, plan()).await.unwrap_err();
        assert!(format!("{error:#}").contains("đọc bảng nhạc"));
        assert!(started.elapsed() <= SECTION_WINDOW);
        assert_eq!(s.taps.load(Ordering::Relaxed), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn loading_sound_sheet_keeps_observing_within_budget_and_discards_stale_geometry() {
        let mut s = session(false);
        s.failed_snapshots = vec![2, 3];
        s.snapshot_delay = Duration::from_secs(4);
        select_section_tab(&s, plan()).await.unwrap();
        assert_eq!(*s.tap_snapshots.lock().unwrap(), vec![5]);
        assert_eq!(s.taps.load(Ordering::Relaxed), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn loading_sound_read_recovery_still_honors_stop_without_a_tap() {
        let mut s = session(false);
        s.failed_snapshots = (1..=1000).collect();
        s.snapshot_delay = Duration::from_secs(4);
        let stop = AtomicBool::new(false);
        let started = Instant::now();
        let work = with_sound_budget(&stop, select_section_tab(&s, plan()));
        let cancel = async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            stop.store(true, Ordering::Relaxed);
        };
        let (result, ()) = tokio::join!(work, cancel);
        assert!(result.unwrap_err().is::<SoundStopped>());
        assert!(started.elapsed() < Duration::from_secs(11));
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
    async fn sound_load_after_150_seconds_is_still_observed_before_three_minute_limit() {
        let mut s = session(false);
        s.hot.store(true, Ordering::Relaxed);
        s.snapshot_delay = Duration::from_secs(75);
        let pool = observe(&s, plan(), 5).await.unwrap();
        assert_eq!(pool.candidates.len(), 2);
        assert_eq!(s.taps.load(Ordering::Relaxed), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn stable_sound_identity_can_use_fresh_bounds_after_layout_moves() {
        let mut s = session(false);
        s.hot.store(true, Ordering::Relaxed);
        s.snapshot_filter = |xml, n| {
            if n % 2 == 0 {
                xml.replace("[60,65]", "[60,66]")
            } else {
                xml
            }
        };
        let pool = observe(&s, plan(), 5).await.unwrap();
        assert_eq!(pool.candidates.len(), 2);
        assert_eq!(s.snapshots.load(Ordering::Relaxed), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn sound_budget_caps_total_wait_and_never_taps_after_cancel() {
        let mut s = session(false);
        s.hot.store(true, Ordering::Relaxed);
        s.snapshot_delay = Duration::from_secs(100);
        let stop = AtomicBool::new(false);
        let at = Instant::now();
        let e = with_sound_budget(&stop, observe(&s, plan(), 5))
            .await
            .unwrap_err();
        let failure = crate::publish_recovery::describe(&e);
        assert_eq!(failure.code, "sound_load_timeout");
        assert_eq!(
            failure.kind,
            crate::publish_recovery::FailureKind::Retryable
        );
        assert_eq!(at.elapsed(), SOUND_WINDOW);
        assert_eq!(s.taps.load(Ordering::Relaxed), 0);
        let at = Instant::now();
        let work = with_sound_budget(&stop, observe(&s, plan(), 5));
        let cancel = async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            stop.store(true, Ordering::Relaxed);
        };
        let (r, ()) = tokio::join!(work, cancel);
        assert!(r.unwrap_err().is::<SoundStopped>());
        assert!(at.elapsed() < Duration::from_secs(3));
        assert_eq!(s.taps.load(Ordering::Relaxed), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn sound_budget_is_shared_across_phases_instead_of_reset() {
        let mut s = session(false);
        s.hot.store(true, Ordering::Relaxed);
        s.snapshot_delay = Duration::from_secs(25);
        let stop = AtomicBool::new(false);
        let at = Instant::now();
        let r = with_sound_budget(&stop, async {
            tokio::time::sleep(Duration::from_secs(140)).await;
            observe(&s, plan(), 5).await
        })
        .await;
        assert!(r.is_err());
        assert_eq!(at.elapsed(), SOUND_WINDOW);
    }

    #[tokio::test(start_paused = true)]
    async fn selected_sound_uses_latest_target_but_rejects_changed_identity() {
        let first = pool(&fixture(true, false), plan(), 5).unwrap();
        let shifted = pool(
            &fixture(true, false).replace("[60,65]", "[60,66]"),
            plan(),
            5,
        )
        .unwrap();
        assert!(first.stable_with(&shifted));
        assert_eq!(reproof_target(&first, &shifted, 0).unwrap().y, 66.0);
        let changed = pool(&fixture(true, false).replace("One", "Different"), plan(), 5).unwrap();
        assert!(!first.stable_with(&changed));
        assert!(reproof_target(&first, &changed, 0).is_err());
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
    async fn other_measured_builds_keep_read_recovery_after_selection() {
        let mut s = session(false);
        select_section_tab(&s, plan()).await.unwrap();
        let p = observe(&s, plan(), 5).await.unwrap();
        // The next two reads reprove the pool before the single selection tap.
        s.failed_snapshots = vec![s.snapshots.load(Ordering::Relaxed) + 3];
        choose_and_confirm_sound(&s, plan(), &p, 1).await.unwrap();
        assert_eq!(s.taps.load(Ordering::Relaxed), 2);
        assert!(s.closed.load(Ordering::Relaxed));
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
        assert!(
            e.downcast_ref::<crate::driver::UiError>()
                .is_some_and(|error| error.op == "observe"
                    && matches!(error.kind, crate::driver::UiErrorKind::Timeout)),
            "selected marker must not accept a wrong editor title before the deadline: {e:#}"
        );
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
