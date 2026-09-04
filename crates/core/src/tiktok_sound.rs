//! Measured TikTok sound-picker support for the Android publish path.
//!
//! The picker is deliberately version keyed. Resource ids move between TikTok builds, and
//! selecting the wrong row is still a public-post input even though the selection itself is
//! reversible. Unknown packages, versions and locales therefore have no plan.

use std::collections::HashSet;
use std::time::Duration;

use anyhow::Context;
use tokio::time::Instant;

use crate::driver::{ElementBox, ElementQuery, UiSession};
use crate::publish::SoundCandidate;

const PICKER_WINDOW: Duration = Duration::from_secs(8);
const READBACK_WINDOW: Duration = Duration::from_secs(8);
const POLL: Duration = Duration::from_millis(250);

/// The exact hierarchy shape measured for one TikTok build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundPickerPlan {
    entry_id: &'static str,
    current_title_id: &'static str,
    section_label: &'static str,
    canonical_section: &'static str,
    row_id: &'static str,
    title_id: &'static str,
    artist_id: &'static str,
    choose_id: Option<&'static str>,
}

impl SoundPickerPlan {
    /// Resolve only an exact build/locale tuple measured on the attached fleet.
    pub fn resolve(package: &str, locale: &str, version: &str) -> Option<Self> {
        let language = locale
            .trim()
            .split(['-', '_'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if language != "en" {
            return None;
        }
        match (package.trim(), version.trim()) {
            ("com.ss.android.ugc.trill", "38.3.2") => Some(Self {
                entry_id: ":id/c_4",
                current_title_id: ":id/so9",
                section_label: "Recommended",
                canonical_section: "recommended",
                row_id: ":id/ta8",
                title_id: ":id/title",
                artist_id: ":id/rr5",
                choose_id: Some(":id/dfu"),
            }),
            ("com.zhiliaoapp.musically", "46.2.1") => Some(Self {
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
            }),
            ("com.zhiliaoapp.musically", "46.2.42") => Some(Self {
                entry_id: ":id/dv3",
                current_title_id: ":id/tv_top_text",
                section_label: "Hot",
                canonical_section: "trending",
                row_id: ":id/vertical_item_music_new_rl",
                title_id: ":id/title",
                artist_id: ":id/zdw",
                choose_id: None,
            }),
            _ => None,
        }
    }

    pub fn provenance(self) -> &'static str {
        match (self.entry_id, self.current_title_id) {
            (":id/c_4", ":id/so9") => "trill/en/38.3.2, measured 2026-09-04 on 9889db374744474635",
            (":id/dvc", ":id/tv_top_text") => {
                "musically/en/46.2.1, measured 2026-09-04 on ce11171beb408a1501"
            }
            (":id/dv3", ":id/tv_top_text") => {
                "musically/en/46.2.42, measured 2026-09-04 on ce0517155ab38c390d"
            }
            _ => "unknown",
        }
    }
}

/// One observed pool plus the exact row targets that produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct ObservedSoundPool {
    pub candidates: Vec<SoundCandidate>,
    targets: Vec<ElementBox>,
}

impl ObservedSoundPool {
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
    let entry = exactly_one(
        session
            .locate_all(ElementQuery::ResourceIdSuffix(plan.entry_id))
            .await
            .context("locate sound-picker entry")?,
        "sound-picker entry",
    )?;
    session
        .tap(entry.centre())
        .await
        .context("open sound picker")?;

    let deadline = Instant::now() + PICKER_WINDOW;
    let (rows, titles, artists, choices) = loop {
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
        if section.len() == 1 && !rows.is_empty() && !titles.is_empty() {
            break (rows, titles, artists, choices);
        }
        if Instant::now() >= deadline {
            anyhow::bail!("sound picker did not expose one measured section with candidate rows");
        }
        tokio::time::sleep(POLL).await;
    };

    assemble_pool(plan, rows, titles, artists, choices, maximum_visible)
}

/// Tap the selected row once and prove the editor now names the same sound.
pub async fn choose_and_confirm_sound(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    pool: &ObservedSoundPool,
    index: usize,
) -> anyhow::Result<()> {
    let candidate = pool
        .candidates
        .get(index)
        .context("sound selection index is outside the observed pool")?;
    let target = pool
        .target(index)
        .context("sound selection target is missing")?;
    session
        .tap(target.centre())
        .await
        .context("select observed sound")?;
    confirm_sound(session, plan, &candidate.title).await
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
        let rows = session
            .locate_all_described(ElementQuery::ResourceIdSuffix(plan.current_title_id))
            .await
            .unwrap_or_default();
        if matches!(rows.as_slice(), [only] if only.description.as_deref().is_some_and(|value| value.trim() == expected))
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            anyhow::bail!("selected sound was not confirmed on the editor");
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
    maximum_visible: usize,
) -> anyhow::Result<ObservedSoundPool> {
    rows.sort_by(|left, right| left.y.total_cmp(&right.y));
    let mut candidates = Vec::new();
    let mut targets = Vec::new();
    for row in rows.into_iter().take(maximum_visible) {
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
    let mut unique_titles = HashSet::with_capacity(candidates.len());
    anyhow::ensure!(
        candidates
            .iter()
            .all(|candidate| unique_titles.insert(candidate.title.clone())),
        "sound picker contains duplicate titles; the editor chip cannot prove which artist was selected"
    );
    Ok(ObservedSoundPool {
        candidates,
        targets,
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
        assert!(SoundPickerPlan::resolve("com.ss.android.ugc.trill", "en-US", "38.3.2").is_some());
        assert!(SoundPickerPlan::resolve("com.zhiliaoapp.musically", "en", "46.2.1").is_some());
        assert!(SoundPickerPlan::resolve("com.zhiliaoapp.musically", "en", "46.2.42").is_some());
        assert!(SoundPickerPlan::resolve("com.ss.android.ugc.trill", "vi", "38.3.2").is_none());
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
        let choices = vec![element(125.0, None), element(225.0, None)];
        let pool = assemble_pool(plan, rows, titles, artists, choices, 5).expect("pool");
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
            vec![element(140.0, None)],
            5,
        )
        .is_err());
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
            vec![element(125.0, None), element(225.0, None)],
            5,
        );
        assert!(result
            .expect_err("title-only readback cannot distinguish the two rows")
            .to_string()
            .contains("duplicate titles"));
    }
}
