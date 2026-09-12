//! Measured TikTok sound-picker support for the Android publish path.
//!
//! The picker is deliberately version keyed. Resource ids move between TikTok builds, and
//! selecting the wrong row is still a public-post input even though the selection itself is
//! reversible. Unknown packages, versions and locales therefore have no plan.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::Context;
use tokio::time::Instant;

use crate::driver::{ElementBox, ElementQuery, UiSession};
use crate::publish::SoundCandidate;

mod snapshot;

const PICKER_WINDOW: Duration = Duration::from_secs(8);
const READBACK_WINDOW: Duration = Duration::from_secs(8);
const POLL: Duration = Duration::from_millis(250);

/// The exact hierarchy shape measured for one TikTok build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundPickerPlan {
    dynamic: bool,
    package: &'static str,
    entry_id: &'static str,
    current_title_id: &'static str,
    section_label: &'static str,
    canonical_section: &'static str,
    row_id: &'static str,
    title_id: &'static str,
    artist_id: &'static str,
    choose_id: Option<&'static str>,
    layout: SoundPickerLayout,
    selection: SoundSelectionMode,
    post_back_id: Option<&'static str>,
    provenance: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SoundPickerLayout {
    ElementQueries,
    TabbedSnapshot(SoundSnapshotLayout),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SoundSnapshotLayout {
    tab_id: &'static str,
    viewport_id: &'static str,
    boundary_rows: SoundBoundaryRows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SoundBoundaryRows {
    RequireCompleteText,
    ExcludeBottomEdge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SoundSelectionMode {
    Inline { marker_ids: &'static [&'static str] },
}

struct MeasuredSoundPicker {
    version: &'static str,
    language: &'static str,
    plan: SoundPickerPlan,
}

// A measurement owns its behavior as well as its locators. Identical IDs across
// versions are not aliases, and an entry ID never chooses a parser or close route.
const MEASURED_SOUND_PICKERS: &[MeasuredSoundPicker] = &[
    MeasuredSoundPicker {
        version: "45.4.3", language: "en",
        plan: SoundPickerPlan {
            dynamic: false,
            package: "com.zhiliaoapp.musically",
            entry_id: ":id/dmk", current_title_id: ":id/zy1",
            section_label: "Hot", canonical_section: "trending",
            row_id: ":id/vertical_item_music_new_rl", title_id: ":id/title",
            artist_id: ":id/yq_", choose_id: None,
            layout: SoundPickerLayout::TabbedSnapshot(SoundSnapshotLayout {
                tab_id: ":id/wf9", viewport_id: ":id/viewpager_container",
                boundary_rows: SoundBoundaryRows::ExcludeBottomEdge,
            }),
            selection: SoundSelectionMode::Inline { marker_ids: &[":id/nd_"] },
            post_back_id: Some(":id/bgy"),
            provenance: "SM-G955F ce04171435f104080c, Android 9/en, 08/09/2026 code2024504030; isolated album, ordinal 1/2, Hot, selected sound, caption roundtrip, Send empty/typed/cleared",
        },
    },
    MeasuredSoundPicker {
        version: "46.1.3", language: "en",
        plan: SoundPickerPlan {
            dynamic: false,
            package: "com.zhiliaoapp.musically",
            entry_id: ":id/dta", current_title_id: ":id/tv_top_text",
            section_label: "Hot", canonical_section: "trending",
            row_id: ":id/vertical_item_music_new_rl", title_id: ":id/title",
            artist_id: ":id/zba", choose_id: None,
            layout: SoundPickerLayout::TabbedSnapshot(SoundSnapshotLayout {
                tab_id: ":id/wzs", viewport_id: ":id/viewpager_container",
                boundary_rows: SoundBoundaryRows::ExcludeBottomEdge,
            }),
            selection: SoundSelectionMode::Inline { marker_ids: &[":id/ntf"] },
            post_back_id: Some(":id/bn7"),
            provenance: "SM-G955F ce051715e15b2c2e02, Android 9/en, 08/09/2026 code2024601030; isolated album, ordinal 1/2, Hot, selected sound, caption roundtrip, Send empty/typed/cleared",
        },
    },
    MeasuredSoundPicker {
        version: "46.4.3", language: "en",
        plan: SoundPickerPlan {
            dynamic: false,
            package: "com.zhiliaoapp.musically",
            entry_id: ":id/dwh", current_title_id: ":id/tv_top_text",
            section_label: "Hot", canonical_section: "trending",
            row_id: ":id/vertical_item_music_new_rl", title_id: ":id/title",
            artist_id: ":id/zp2", choose_id: None,
            layout: SoundPickerLayout::TabbedSnapshot(SoundSnapshotLayout {
                tab_id: ":id/xbv", viewport_id: ":id/viewpager_container",
                boundary_rows: SoundBoundaryRows::ExcludeBottomEdge,
            }),
            selection: SoundSelectionMode::Inline { marker_ids: &[":id/o2k"] },
            post_back_id: Some(":id/bor"),
            provenance: "SM-G955F ce031713aadf361905, Android 9/en, 08/09/2026 code2024604030; isolated album, ordinal 1/2, Hot, selected sound, caption roundtrip, Send empty/typed/cleared",
        },
    },

    MeasuredSoundPicker {
        version: "45.7.3",
        language: "en",
        plan: SoundPickerPlan {
            dynamic: false,
            package: "com.zhiliaoapp.musically",
            entry_id: ":id/dou",
            current_title_id: ":id/tv_top_text",
            section_label: "Hot",
            canonical_section: "trending",
            row_id: ":id/vertical_item_music_new_rl",
            title_id: ":id/title",
            artist_id: ":id/z3k",
            choose_id: None,
            layout: SoundPickerLayout::TabbedSnapshot(SoundSnapshotLayout {
                tab_id: ":id/wrv",
                viewport_id: ":id/viewpager_container",
                boundary_rows: SoundBoundaryRows::ExcludeBottomEdge,
            }),
            selection: SoundSelectionMode::Inline { marker_ids: &[":id/nms"] },
            post_back_id: Some(":id/bix"),
            provenance: "musically/en/45.7.3 code2024507030, measured 2026-09-08 on ce031713b0c610ab0c; three-photo selection, Hot, selected row/editor/caption return",
        },
    },
    MeasuredSoundPicker {
        version: "38.3.2",
        language: "en",
        plan: SoundPickerPlan {
            dynamic: false,
            package: "com.ss.android.ugc.trill",
            entry_id: ":id/c_4",
            current_title_id: ":id/so9",
            section_label: "Recommended",
            canonical_section: "recommended",
            row_id: ":id/ta8",
            title_id: ":id/title",
            artist_id: ":id/rr5",
            // dfu is the trim scissors, not a choose control (live 2026-09-06).
            choose_id: None,
            layout: SoundPickerLayout::ElementQueries,
            selection: SoundSelectionMode::Inline {
                marker_ids: &[":id/dfu", ":id/jk1"],
            },
            post_back_id: Some(":id/aun"),
            provenance: "trill/en/38.3.2, measured 2026-09-04 on 9889db374744474635",
        },
    },
    MeasuredSoundPicker {
        version: "46.2.1",
        language: "en",
        plan: SoundPickerPlan {
            dynamic: false,
            package: "com.zhiliaoapp.musically",
            entry_id: ":id/dvc",
            current_title_id: ":id/tv_top_text",
            section_label: "Hot",
            canonical_section: "trending",
            row_id: ":id/vertical_item_music_new_rl",
            title_id: ":id/title",
            artist_id: ":id/zgj",
            // This layout has no dedicated choose icon. The measured title area is the
            // row's stable selection target; the readback below still decides whether it
            // took.
            choose_id: None,
            layout: SoundPickerLayout::TabbedSnapshot(SoundSnapshotLayout {
                tab_id: ":id/x4y", viewport_id: ":id/viewpager_container",
                boundary_rows: SoundBoundaryRows::ExcludeBottomEdge,
            }),
            selection: SoundSelectionMode::Inline { marker_ids: &[":id/nx3"] },
            post_back_id: Some(":id/bot"),
            provenance: "musically/en/46.2.1, remeasured 08-09-2026 on ce04171411ae6a1504; Hot inline marker, Back, editor title and caption roundtrip; AGENTS.md ?9.189",
        },
    },
    MeasuredSoundPicker {
        version: "46.2.42",
        language: "en",
        plan: SoundPickerPlan {
            dynamic: false,
            package: "com.zhiliaoapp.musically",
            entry_id: ":id/dv3",
            current_title_id: ":id/tv_top_text",
            section_label: "Hot",
            canonical_section: "trending",
            row_id: ":id/vertical_item_music_new_rl",
            title_id: ":id/title",
            artist_id: ":id/zdw",
            choose_id: None,
            layout: SoundPickerLayout::TabbedSnapshot(SoundSnapshotLayout {
                tab_id: ":id/x2k",
                viewport_id: ":id/viewpager_container",
                boundary_rows: SoundBoundaryRows::RequireCompleteText,
            }),
            selection: SoundSelectionMode::Inline {
                marker_ids: &[":id/nve"],
            },
            post_back_id: Some(":id/bot"),
            provenance: "musically/en/46.2.42, measured 2026-09-04 on ce0517155ab38c390d",
        },
    },
    MeasuredSoundPicker {
        version: "46.0.41",
        language: "en",
        plan: SoundPickerPlan {
            dynamic: false,
            package: "com.zhiliaoapp.musically",
            entry_id: ":id/dsv",
            current_title_id: ":id/tv_top_text",
            section_label: "Hot",
            canonical_section: "trending",
            row_id: ":id/vertical_item_music_new_rl",
            title_id: ":id/title",
            artist_id: ":id/z_g",
            choose_id: None,
            layout: SoundPickerLayout::TabbedSnapshot(SoundSnapshotLayout {
                tab_id: ":id/wy5",
                viewport_id: ":id/viewpager_container",
                // The artist is visibly clipped despite both text nodes existing.
                boundary_rows: SoundBoundaryRows::ExcludeBottomEdge,
            }),
            selection: SoundSelectionMode::Inline { marker_ids: &[":id/nrm"] },
            post_back_id: Some(":id/bmy"),
            provenance: "musically/en/46.0.41 code2024600410, measured 2026-09-08 on ce011711c354be2005; selected row/editor/caption return verified before Post",
        },
    },
];

impl SoundPickerPlan {
    pub fn resolve_runtime(package: &str, locale: &str, version: &str) -> Option<Self> {
        Self::resolve(package, locale, version).or_else(|| {
            let language = crate::tiktok_labels::normalise_language(locale);
            MEASURED_SOUND_PICKERS
                .iter()
                .find(|p| p.plan.package == package && p.language == language)
                .map(|p| {
                    let mut plan = p.plan;
                    plan.dynamic = true;
                    plan
                })
        })
    }
    pub(crate) fn post_back_query(self) -> Option<ElementQuery<'static>> {
        self.post_back_id.map(ElementQuery::ResourceIdSuffix)
    }

    fn snapshot_layout(self) -> Option<SoundSnapshotLayout> {
        match self.layout {
            SoundPickerLayout::ElementQueries => None,
            SoundPickerLayout::TabbedSnapshot(layout) => Some(layout),
        }
    }

    fn closes_with_back(self) -> bool {
        matches!(self.selection, SoundSelectionMode::Inline { .. })
    }

    fn selected_marker_ids(self) -> &'static [&'static str] {
        match self.selection {
            SoundSelectionMode::Inline { marker_ids } => marker_ids,
        }
    }

    /// Resolve only an exact build/locale tuple measured on the attached fleet.
    pub fn resolve(package: &str, locale: &str, version: &str) -> Option<Self> {
        let language = locale
            .trim()
            .split(['-', '_'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        MEASURED_SOUND_PICKERS
            .iter()
            .find(|entry| {
                entry.plan.package == package.trim()
                    && entry.language == language
                    && entry.version == version.trim()
            })
            .map(|entry| entry.plan)
    }

    pub fn provenance(self) -> &'static str {
        self.provenance
    }
}

/// One observed pool plus the exact row targets that produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct ObservedSoundPool {
    effective_plan: Option<SoundPickerPlan>,
    pub candidates: Vec<SoundCandidate>,
    maximum_visible: usize,
    targets: Vec<ElementBox>,
    selected_index: Option<usize>,
}

impl ObservedSoundPool {
    pub fn effective_plan(&self, fallback: SoundPickerPlan) -> SoundPickerPlan {
        self.effective_plan.unwrap_or(fallback)
    }
    pub fn target(&self, index: usize) -> Option<&ElementBox> {
        self.targets.get(index)
    }
}

/// Open the measured picker and read at most `maximum_visible` rows without selecting one.
pub async fn open_and_observe_sounds(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum_visible: usize,
) -> anyhow::Result<ObservedSoundPool> {
    anyhow::ensure!(
        (1..=5).contains(&maximum_visible),
        "sound observer limit must be within 1..=5"
    );
    if plan.dynamic {
        return open_dynamic_sounds(session, plan, maximum_visible).await;
    }
    let entries = session
        .locate_all(ElementQuery::ResourceIdSuffix(plan.entry_id))
        .await
        .context("locate sound-picker entry")?;
    if entries.is_empty() && !session.gui_session_epoch().is_empty() {
        return open_dynamic_sounds(session, plan, maximum_visible).await;
    }
    let entry = exactly_one(entries, "sound-picker entry")?;
    session
        .tap(entry.centre())
        .await
        .context("open sound picker")?;

    if plan.snapshot_layout().is_some() {
        snapshot::select_section_tab(session, plan).await?;
    }

    observe_sound_pool(session, plan, maximum_visible).await
}

async fn open_dynamic_sounds(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
) -> anyhow::Result<ObservedSoundPool> {
    let source =
        crate::ui_automation::tree::Tree::parse(session.hierarchy_source_snapshot().await?)?;
    let mut entries = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for candidate in MEASURED_SOUND_PICKERS
        .iter()
        .filter(|p| p.plan.package == plan.package)
    {
        if seen.insert(candidate.plan.entry_id) {
            entries.extend(source.matching(
                plan.package,
                ElementQuery::ResourceIdSuffix(candidate.plan.entry_id),
            ));
        }
    }
    entries.sort_unstable();
    entries.dedup();
    let [index] = entries.as_slice() else {
        anyhow::bail!("sound_entry_ambiguous: giao diện chưa có duy nhất nút nhạc");
    };
    let button = source.nodes[*index].rect().context("sound entry bounds")?;
    anyhow::ensure!(button.enabled && button.clickable, "sound entry disabled");
    session.tap(button.centre()).await?;
    let deadline = Instant::now() + std::time::Duration::from_secs(30);
    let selected = loop {
        let tree =
            crate::ui_automation::tree::Tree::parse(session.hierarchy_source_snapshot().await?)?;
        let mut matches = Vec::new();
        for candidate in MEASURED_SOUND_PICKERS
            .iter()
            .filter(|p| p.plan.package == plan.package)
        {
            let p = candidate.plan;
            let exists = |id| {
                !tree
                    .matching(plan.package, ElementQuery::ResourceIdSuffix(id))
                    .is_empty()
            };
            if exists(p.row_id)
                && exists(p.title_id)
                && exists(p.artist_id)
                && p.snapshot_layout()
                    .is_none_or(|l| exists(l.tab_id) && exists(l.viewport_id))
            {
                matches.push(p);
            }
        }
        if let Some(first) = matches.first().copied() {
            anyhow::ensure!(
                matches.iter().all(|p| p.row_id == first.row_id
                    && p.title_id == first.title_id
                    && p.artist_id == first.artist_id
                    && p.layout == first.layout
                    && p.selection == first.selection
                    && p.choose_id == first.choose_id),
                "sound_layout_ambiguous"
            );
            break first;
        }
        anyhow::ensure!(Instant::now() < deadline, "sound_layout_unrecognized");
        tokio::time::sleep(POLL).await;
    };
    if selected.snapshot_layout().is_some() {
        snapshot::select_section_tab(session, selected).await?;
    }
    let mut pool = observe_sound_pool(session, selected, maximum).await?;
    pool.effective_plan = Some(selected);
    Ok(pool)
}

async fn observe_sound_pool(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum_visible: usize,
) -> anyhow::Result<ObservedSoundPool> {
    if plan.snapshot_layout().is_some() {
        return snapshot::observe(session, plan, maximum_visible).await;
    }
    let deadline = Instant::now() + PICKER_WINDOW;
    loop {
        let section = session
            .locate_all_described(ElementQuery::Text {
                value: plan.section_label,
                exact: true,
            })
            .await
            .unwrap_or_default();
        let rows = session
            .locate_all(ElementQuery::ResourceIdSuffix(plan.row_id))
            .await
            .unwrap_or_default();
        let titles = session
            .locate_all_described(ElementQuery::ResourceIdSuffix(plan.title_id))
            .await
            .unwrap_or_default();
        let artists = session
            .locate_all_described(ElementQuery::ResourceIdSuffix(plan.artist_id))
            .await
            .unwrap_or_default();
        let choices = match plan.choose_id {
            Some(id) => session
                .locate_all(ElementQuery::ResourceIdSuffix(id))
                .await
                .unwrap_or_default(),
            None => Vec::new(),
        };
        let mut markers = Vec::new();
        for id in plan.selected_marker_ids() {
            // An equalizer or trim control can prove the selected row without
            // becoming a tap target. The measured plan names both independently.
            markers.extend(
                session
                    .locate_all(ElementQuery::ResourceIdSuffix(id))
                    .await
                    .unwrap_or_default(),
            );
        }
        if section.len() == 1 && !rows.is_empty() && !titles.is_empty() {
            match assemble_pool(
                plan,
                rows,
                titles,
                artists,
                choices,
                markers,
                maximum_visible,
            ) {
                Ok(pool) => return Ok(pool),
                Err(error) if Instant::now() >= deadline => return Err(error.context(
                    "sound_candidates_incomplete: bảng nhạc chưa tải đủ row/title/artist/choose",
                )),
                Err(_) => {
                    tokio::time::sleep(POLL).await;
                    continue;
                }
            }
        }
        if Instant::now() >= deadline {
            anyhow::bail!("sound picker did not expose one measured section with candidate rows");
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Tap the selected row once and prove the editor now names the same sound.
pub async fn choose_and_confirm_sound(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    pool: &ObservedSoundPool,
    index: usize,
) -> anyhow::Result<()> {
    let plan = pool.effective_plan.unwrap_or(plan);
    let candidate = pool
        .candidates
        .get(index)
        .context("sound selection index is outside the observed pool")?;
    let fresh = observe_sound_pool(session, plan, pool.maximum_visible).await?;
    let target = reproof_target(pool, &fresh, index)?;
    if fresh.selected_index != Some(index) {
        session
            .tap(target.centre())
            .await
            .context("select observed sound")?;
    }
    if plan.closes_with_back() {
        // The measured Android sheet selects inline; Back closes only that sheet.
        // Prove the same pool remains before dismissing it, then prove the editor chip.
        let deadline = Instant::now() + READBACK_WINDOW;
        loop {
            let selected_pool = observe_sound_pool(session, plan, pool.maximum_visible).await?;
            reproof_target(pool, &selected_pool, index)?;
            if selected_pool.selected_index == Some(index) {
                break;
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "selected sound row not confirmed before closing picker"
            );
            tokio::time::sleep(POLL).await;
        }
        session.back().await.context("close inline sound picker")?;
    }
    confirm_sound(session, plan, &candidate.title)
        .await
        .context("initial sound selection readback")
}

fn reproof_target<'a>(
    expected: &ObservedSoundPool,
    fresh: &'a ObservedSoundPool,
    index: usize,
) -> anyhow::Result<&'a ElementBox> {
    anyhow::ensure!(
        expected.candidates == fresh.candidates,
        "sound candidates changed before selection"
    );
    let target = fresh
        .target(index)
        .context("sound selection target is missing")?;
    anyhow::ensure!(target.enabled, "sound selection control disabled");
    Ok(target)
}

/// Re-read the editor chip. The exact title and exactly one node are both required.
pub async fn confirm_sound(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    expected_title: &str,
) -> anyhow::Result<()> {
    let expected = expected_title.trim();
    anyhow::ensure!(!expected.is_empty(), "selected sound title is empty");
    let deadline = Instant::now() + READBACK_WINDOW;
    loop {
        let rows = if plan.dynamic {
            let mut observed = Vec::new();
            let mut ids = std::collections::HashSet::new();
            for candidate in MEASURED_SOUND_PICKERS
                .iter()
                .filter(|p| p.plan.package == plan.package)
            {
                if ids.insert(candidate.plan.current_title_id) {
                    observed.extend(
                        session
                            .locate_all_described(ElementQuery::ResourceIdSuffix(
                                candidate.plan.current_title_id,
                            ))
                            .await
                            .unwrap_or_default(),
                    );
                }
            }
            observed
        } else {
            session
                .locate_all_described(ElementQuery::ResourceIdSuffix(plan.current_title_id))
                .await
                .unwrap_or_default()
        };
        if matches!(rows.as_slice(), [only] if only.description.as_deref().is_some_and(|value| value.trim() == expected))
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            anyhow::bail!("selected sound was not confirmed on the editor: expected {expected:?}, observed {:?}", rows.iter().map(|row| row.description.as_deref()).collect::<Vec<_>>());
        }
        tokio::time::sleep(POLL).await;
    }
}

fn assemble_pool(
    plan: SoundPickerPlan,
    mut rows: Vec<ElementBox>,
    titles: Vec<ElementBox>,
    artists: Vec<ElementBox>,
    choices: Vec<ElementBox>,
    markers: Vec<ElementBox>,
    maximum_visible: usize,
) -> anyhow::Result<ObservedSoundPool> {
    rows.sort_by(|left, right| left.y.total_cmp(&right.y));
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    let mut selected_index = None;
    for (index, row) in rows.into_iter().take(maximum_visible).enumerate() {
        if plan.closes_with_back() && !inside(&row, &markers).is_empty() {
            anyhow::ensure!(selected_index.is_none(), "ambiguous selected sound row");
            selected_index = Some(index);
        }
        let title = exactly_one(inside(&row, &titles), "sound title inside candidate row")?;
        let title_text = title
            .description
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .context("sound title is empty")?;
        let artist = exactly_one(inside(&row, &artists), "sound artist inside candidate row")?
            .description
            .as_deref()
            .map(normalize_artist)
            .unwrap_or_default();
        let target = if plan.choose_id.is_some() {
            exactly_one(
                inside(&row, &choices),
                "sound choose control inside candidate row",
            )?
        } else {
            title.clone()
        };
        candidates.push(SoundCandidate {
            section: plan.canonical_section.to_string(),
            title: title_text.to_string(),
            artist,
        });
        targets.push(target);
    }
    anyhow::ensure!(
        !candidates.is_empty(),
        "sound picker exposed no complete candidate row"
    );
    // Skip every ambiguous title, retaining the original observation window for
    // reproof. One duplicate recommendation must not discard distinct usable songs.
    let mut counts = HashMap::new();
    for candidate in &candidates {
        *counts.entry(candidate.title.clone()).or_insert(0) += 1;
    }
    let mut unique_candidates = Vec::new();
    let mut unique_targets = Vec::new();
    let mut unique_selected = None;
    for (index, (candidate, target)) in candidates.into_iter().zip(targets).enumerate() {
        if counts[&candidate.title] != 1 {
            continue;
        }
        if selected_index == Some(index) {
            unique_selected = Some(unique_candidates.len());
        }
        unique_candidates.push(candidate);
        unique_targets.push(target);
    }
    anyhow::ensure!(!unique_candidates.is_empty(), "sound picker contains only duplicate titles; the editor chip cannot prove which artist was selected");
    let candidates = unique_candidates;
    let targets = unique_targets;
    let selected_index = unique_selected;
    Ok(ObservedSoundPool {
        effective_plan: None,
        candidates,
        maximum_visible,
        targets,
        selected_index,
    })
}

fn inside(row: &ElementBox, values: &[ElementBox]) -> Vec<ElementBox> {
    values
        .iter()
        .filter(|value| {
            let centre = value.centre();
            centre.x >= row.x
                && centre.x <= row.x + row.width
                && centre.y >= row.y
                && centre.y <= row.y + row.height
        })
        .cloned()
        .collect()
}

fn exactly_one(mut values: Vec<ElementBox>, what: &str) -> anyhow::Result<ElementBox> {
    if values.len() != 1 {
        anyhow::bail!("expected exactly one {what}, found {}", values.len());
    }
    Ok(values.remove(0))
}

fn normalize_artist(value: &str) -> String {
    value
        .trim()
        .split_once(" · ")
        .map_or_else(|| value.trim(), |(artist, _)| artist.trim())
        .trim_matches('\u{200e}')
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct InlineSession {
        selected: AtomicBool,
        closed: AtomicBool,
        taps: AtomicUsize,
        select_takes: bool,
        marker_id: &'static str,
        editor_title: &'static str,
    }

    #[async_trait::async_trait]
    impl UiSession for InlineSession {
        async fn tap(&self, _: crate::TapPoint) -> anyhow::Result<()> {
            self.taps.fetch_add(1, Ordering::Relaxed);
            if self.select_takes {
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
        async fn locate_all(&self, query: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
            Ok(match query {
                ElementQuery::ResourceIdSuffix(":id/ta8") => vec![ElementBox {
                    height: 200.0,
                    ..element(100.0, None)
                }],
                ElementQuery::ResourceIdSuffix(id)
                    if id == self.marker_id && self.selected.load(Ordering::Relaxed) =>
                {
                    vec![element(125.0, None)]
                }
                _ => vec![],
            })
        }
        async fn locate_all_described(
            &self,
            query: ElementQuery<'_>,
        ) -> anyhow::Result<Vec<ElementBox>> {
            Ok(match query {
                ElementQuery::Text {
                    value: "Recommended",
                    ..
                } => vec![element(0.0, Some("Recommended"))],
                ElementQuery::ResourceIdSuffix(":id/title") => vec![element(120.0, Some("One"))],
                ElementQuery::ResourceIdSuffix(":id/rr5") => vec![element(180.0, Some("Artist"))],
                ElementQuery::ResourceIdSuffix(":id/so9")
                    if self.closed.load(Ordering::Relaxed)
                        && self.selected.load(Ordering::Relaxed) =>
                {
                    vec![element(20.0, Some(self.editor_title))]
                }
                _ => vec![],
            })
        }
    }

    #[tokio::test(start_paused = true)]
    async fn inline_sound_desired_state_never_toggles_an_already_selected_track_off() {
        for selected in [false, true] {
            let session = InlineSession {
                selected: AtomicBool::new(selected),
                closed: AtomicBool::new(false),
                taps: AtomicUsize::new(0),
                select_takes: true,
                marker_id: ":id/dfu",
                editor_title: "One",
            };
            let plan =
                SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2").unwrap();
            let pool = observe_sound_pool(&session, plan, 1).await.unwrap();
            choose_and_confirm_sound(&session, plan, &pool, 0)
                .await
                .unwrap();
            assert_eq!(session.taps.load(Ordering::Relaxed), usize::from(!selected));
            assert!(session.closed.load(Ordering::Relaxed));
            assert!(session.selected.load(Ordering::Relaxed));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn unconfirmed_sound_selection_stops_without_another_tap_or_closing_picker() {
        let session = InlineSession {
            selected: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            taps: AtomicUsize::new(0),
            select_takes: false,
            marker_id: ":id/dfu",
            editor_title: "One",
        };
        let plan = SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2").unwrap();
        let pool = observe_sound_pool(&session, plan, 1).await.unwrap();
        assert!(choose_and_confirm_sound(&session, plan, &pool, 0)
            .await
            .is_err());
        assert_eq!(session.taps.load(Ordering::Relaxed), 1);
        assert!(!session.closed.load(Ordering::Relaxed));
    }

    #[tokio::test(start_paused = true)]
    async fn carousel_sound_equalizer_confirms_selected_track_without_trim_control() {
        // Measured carousel row on trill/en/38.3.2: jk1 appears inside the selected
        // ta8 row; dfu is absent. Editor so9 must still confirm the exact title.
        for selected in [false, true] {
            let session = InlineSession {
                selected: AtomicBool::new(selected),
                closed: AtomicBool::new(false),
                taps: AtomicUsize::new(0),
                select_takes: true,
                marker_id: ":id/jk1",
                editor_title: "One",
            };
            let plan =
                SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2").unwrap();
            let pool = observe_sound_pool(&session, plan, 1).await.unwrap();
            choose_and_confirm_sound(&session, plan, &pool, 0)
                .await
                .unwrap();
            assert_eq!(session.taps.load(Ordering::Relaxed), usize::from(!selected));
            assert!(session.closed.load(Ordering::Relaxed));
            assert!(session.selected.load(Ordering::Relaxed));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn carousel_marker_does_not_replace_editor_title_readback() {
        let session = InlineSession {
            selected: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            taps: AtomicUsize::new(0),
            select_takes: true,
            marker_id: ":id/jk1",
            editor_title: "Different sound",
        };
        let plan = SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2").unwrap();
        let pool = observe_sound_pool(&session, plan, 1).await.unwrap();
        let error = choose_and_confirm_sound(&session, plan, &pool, 0)
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("selected sound was not confirmed"));
        assert_eq!(session.taps.load(Ordering::Relaxed), 1);
        assert!(session.closed.load(Ordering::Relaxed));
    }

    #[test]
    fn inline_markers_must_all_belong_to_one_candidate_row() {
        let plan = SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2").unwrap();
        let rows = vec![
            ElementBox {
                height: 100.0,
                ..element(100.0, None)
            },
            ElementBox {
                height: 100.0,
                ..element(200.0, None)
            },
        ];
        let titles = vec![element(115.0, Some("One")), element(215.0, Some("Two"))];
        let artists = vec![element(155.0, Some("A")), element(255.0, Some("B"))];
        let same_row = assemble_pool(
            plan,
            rows.clone(),
            titles.clone(),
            artists.clone(),
            vec![],
            vec![element(120.0, None), element(130.0, None)],
            5,
        )
        .unwrap();
        assert_eq!(same_row.selected_index, Some(0));
        let different_rows = assemble_pool(
            plan,
            rows,
            titles,
            artists,
            vec![],
            vec![element(120.0, None), element(220.0, None)],
            5,
        )
        .unwrap_err();
        assert!(different_rows
            .to_string()
            .contains("ambiguous selected sound row"));
    }

    #[test]
    fn sound_reproof_rejects_changed_pool_and_uses_fresh_position() {
        let expected = ObservedSoundPool {
            effective_plan: None,
            maximum_visible: 5,
            selected_index: None,
            candidates: vec![SoundCandidate {
                section: "recommended".into(),
                title: "One".into(),
                artist: "Artist".into(),
            }],
            targets: vec![element(100.0, None)],
        };
        let mut fresh = expected.clone();
        fresh.targets[0].y = 200.0;
        assert_eq!(reproof_target(&expected, &fresh, 0).unwrap().y, 200.0);
        fresh.candidates[0].artist = "Other".into();
        assert!(reproof_target(&expected, &fresh, 0).is_err());
        fresh.candidates = expected.candidates.clone();
        fresh.targets[0].enabled = false;
        assert!(reproof_target(&expected, &fresh, 0).is_err());
    }

    fn element(y: f64, description: Option<&str>) -> ElementBox {
        ElementBox {
            x: 0.0,
            y,
            width: 500.0,
            height: 50.0,
            description: description.map(str::to_string),
            enabled: true,
            clickable: false,
        }
    }

    #[test]
    fn plans_are_exactly_version_and_locale_keyed() {
        let trill = SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2").unwrap();
        assert!(trill.choose_id.is_none() && trill.closes_with_back());
        assert!(SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en-US", "38.3.2").is_some());
        assert!(SoundPickerPlan::resolve("com.zhiliaoapp.musically", "en", "46.2.1").is_some());
        assert!(SoundPickerPlan::resolve("com.zhiliaoapp.musically", "en", "46.2.42").is_some());
        assert!(SoundPickerPlan::resolve("com.ss.android.ugc.trill", "vi", "38.3.2").is_none());
        assert!(SoundPickerPlan::resolve("com.zhiliaoapp.musically", "en-US", "46.0.41").is_some());
        assert!(SoundPickerPlan::resolve("com.zhiliaoapp.musically", "en-US", "45.7.3").is_some());
        for version in [
            "45.7.2", "45.7.4", "46.1.4", "46.0.40", "46.0.42", "46.2", "",
        ] {
            assert!(SoundPickerPlan::resolve("com.zhiliaoapp.musically", "en", version).is_none());
        }
    }

    #[test]
    fn entry_ids_do_not_choose_layout_close_route_or_provenance() {
        for measurement in MEASURED_SOUND_PICKERS {
            let plan = measurement.plan;
            let moved = SoundPickerPlan {
                entry_id: ":id/new_entry_fixture",
                ..plan
            };
            assert_eq!(moved.snapshot_layout(), plan.snapshot_layout());
            assert_eq!(moved.closes_with_back(), plan.closes_with_back());
            assert_eq!(moved.post_back_query(), plan.post_back_query());
            assert_eq!(moved.provenance(), plan.provenance());
        }
        let inline = SoundPickerPlan::resolve("com.zhiliaoapp.musically", "en", "46.2.1").unwrap();
        let decoy = SoundPickerPlan {
            entry_id: ":id/dv3",
            ..inline
        };
        assert_eq!(decoy.snapshot_layout().unwrap().tab_id, ":id/x4y");
        assert!(decoy.closes_with_back());
        assert_eq!(
            decoy.post_back_query(),
            Some(ElementQuery::ResourceIdSuffix(":id/bot"))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn inline_marker_locator_is_plan_data_not_a_trill_constant() {
        let base = SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2").unwrap();
        let plan = SoundPickerPlan {
            entry_id: ":id/fixture_entry",
            selection: SoundSelectionMode::Inline {
                marker_ids: &[":id/fixture_marker"],
            },
            ..base
        };
        let session = InlineSession {
            selected: AtomicBool::new(true),
            closed: AtomicBool::new(false),
            taps: AtomicUsize::new(0),
            select_takes: true,
            marker_id: ":id/fixture_marker",
            editor_title: "One",
        };
        let pool = observe_sound_pool(&session, plan, 1).await.unwrap();
        assert_eq!(pool.selected_index, Some(0));
        choose_and_confirm_sound(&session, plan, &pool, 0)
            .await
            .unwrap();
        assert_eq!(session.taps.load(Ordering::Relaxed), 0);
        assert!(session.closed.load(Ordering::Relaxed));
    }

    #[test]
    fn choose_control_and_selected_marker_are_independent() {
        let base = SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2").unwrap();
        let plan = SoundPickerPlan {
            choose_id: Some(":id/fixture_choose"),
            ..base
        };
        let row = ElementBox {
            height: 100.0,
            ..element(100.0, None)
        };
        let choice = element(125.0, None);
        let pool = assemble_pool(
            plan,
            vec![row],
            vec![element(115.0, Some("One"))],
            vec![element(155.0, Some("Artist"))],
            vec![choice.clone()],
            vec![],
            1,
        )
        .unwrap();
        assert_eq!(pool.selected_index, None);
        assert_eq!(pool.target(0), Some(&choice));
    }

    #[test]
    fn row_assembly_keeps_title_artist_and_tap_target_bound_together() {
        let plan = SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2")
            .expect("measured plan");
        let rows = vec![
            ElementBox {
                height: 100.0,
                ..element(100.0, None)
            },
            ElementBox {
                height: 100.0,
                ..element(200.0, None)
            },
        ];
        let titles = vec![
            element(120.0, Some("First")),
            element(220.0, Some("Second")),
        ];
        let artists = vec![
            element(160.0, Some("Artist A · 10K posts")),
            element(260.0, Some("Artist B · 20K posts")),
        ];
        let markers = vec![element(125.0, None)];
        let pool = assemble_pool(plan, rows, titles, artists, vec![], markers, 5).expect("pool");
        assert_eq!(
            pool.candidates,
            vec![
                SoundCandidate {
                    section: "recommended".into(),
                    title: "First".into(),
                    artist: "Artist A".into(),
                },
                SoundCandidate {
                    section: "recommended".into(),
                    title: "Second".into(),
                    artist: "Artist B".into(),
                },
            ]
        );
        assert_eq!(pool.targets.len(), 2);
        assert_eq!(pool.selected_index, Some(0));
    }

    #[test]
    fn incomplete_or_ambiguous_rows_fail_closed() {
        let plan = SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2")
            .expect("measured plan");
        let row = ElementBox {
            height: 100.0,
            ..element(100.0, None)
        };
        assert!(assemble_pool(
            plan,
            vec![row],
            vec![element(120.0, Some("One")), element(130.0, Some("Two"))],
            vec![element(160.0, Some("Artist"))],
            vec![],
            vec![element(140.0, None)],
            5,
        )
        .is_err());
    }

    #[test]
    fn duplicate_recommendations_keep_unique_rows_and_the_original_read_window() {
        let plan = SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2").unwrap();
        let rows = [100.0, 200.0, 300.0]
            .into_iter()
            .map(|y| ElementBox {
                height: 90.0,
                ..element(y, None)
            })
            .collect();
        let pool = assemble_pool(
            plan,
            rows,
            vec![
                element(120.0, Some("Same")),
                element(220.0, Some("Same")),
                element(320.0, Some("Distinct")),
            ],
            vec![
                element(160.0, Some("A")),
                element(260.0, Some("B")),
                element(360.0, Some("C")),
            ],
            vec![],
            vec![element(325.0, None)],
            5,
        )
        .unwrap();
        assert_eq!(pool.candidates.len(), 1);
        assert_eq!(pool.candidates[0].title, "Distinct");
        assert_eq!(pool.selected_index, Some(0));
        assert_eq!(pool.maximum_visible, 5);
        assert_eq!(pool.target(0).unwrap().y, 320.0);
    }

    #[test]
    fn duplicate_titles_with_different_artists_fail_closed() {
        let plan = SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en", "38.3.2")
            .expect("measured plan");
        let rows = vec![
            ElementBox {
                height: 100.0,
                ..element(100.0, None)
            },
            ElementBox {
                height: 100.0,
                ..element(200.0, None)
            },
        ];
        let result = assemble_pool(
            plan,
            rows,
            vec![
                element(120.0, Some("Same title")),
                element(220.0, Some("Same title")),
            ],
            vec![
                element(160.0, Some("Artist A")),
                element(260.0, Some("Artist B")),
            ],
            vec![],
            vec![element(125.0, None)],
            5,
        );
        assert!(result
            .expect_err("title-only readback cannot distinguish the two rows")
            .to_string()
            .contains("duplicate titles"));
    }
}
