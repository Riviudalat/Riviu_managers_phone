use super::*;
use crate::tiktok_composer::observation::{self, Cursor};
use crate::ui_automation::SemanticLocator;

#[derive(Debug, thiserror::Error)]
#[error("selected sound editor {0}; chưa bấm Đăng")]
pub(crate) struct SoundMismatch(pub &'static str);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditorState {
    Confirmed,
    Loading,
    Unknown,
}

/// A positive caption proof can authorize the existing conditional Back retry.
/// false means unproved, never that the caption or editor is absent.
pub(crate) async fn inspect_editor_and_caption(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    expected_title: &str,
    caption_query: ElementQuery<'_>,
    expected_caption: &str,
) -> anyhow::Result<(EditorState, Option<bool>)> {
    anyhow::ensure!(
        !expected_title.trim().is_empty(),
        "selected sound title is empty"
    );
    let ElementQuery::ResourceIdSuffix(caption_suffix) = caption_query else {
        return Ok((inspect(session, plan, expected_title, false).await?, None));
    };
    let deadline = phase_deadline(READBACK_WINDOW);
    let mut cursor = Cursor::default();
    let Some(snapshot) = observation::observe(
        session,
        plan.package,
        SemanticLocator::default(),
        deadline,
        None,
        &mut cursor,
    )
    .await?
    else {
        // Explicit Unsupported keeps the compatibility read route. An error or
        // partial observation never falls back to a second independent caption.
        return Ok((inspect(session, plan, expected_title, false).await?, None));
    };
    let mut suffixes = vec![plan.current_title_id];
    if plan.dynamic {
        suffixes.extend(
            MEASURED_SOUND_PICKERS
                .iter()
                .filter(|p| p.plan.package == plan.package)
                .map(|p| p.plan.current_title_id),
        );
        suffixes.sort_unstable();
        suffixes.dedup();
    }
    let mut sound = EditorState::Unknown;
    let mut sound_matches = 0;
    let mut sound_unknown = snapshot.unknown_match_count;
    for suffix in suffixes {
        let projected = observation::project_query(
            &snapshot,
            &SemanticLocator {
                id: Some(format!("{}{suffix}", plan.package)),
                ..Default::default()
            },
        );
        sound_unknown = sound_unknown.max(projected.unknown_match_count);
        sound_matches += projected.matches.len();
        if projected.matches.len() > 1 {
            return Err(SoundMismatch("ambiguous").into());
        }
        if observation::positive_query_known(&projected) {
            if let [node] = projected.matches.as_slice() {
                if node.package.as_deref() == Some(plan.package)
                    && node.visible == Some(true)
                    && node.password == Some(false)
                {
                    if let Some(text) = &node.text {
                        if same_editor_sound_title(text, expected_title) {
                            sound = EditorState::Confirmed;
                        } else if text.trim().eq_ignore_ascii_case("Loading") {
                            sound = EditorState::Loading;
                        } else if !text.trim().is_empty() {
                            return Err(SoundMismatch("title mismatch").into());
                        }
                    }
                }
            }
        }
    }
    if sound_matches > 1 {
        return Err(SoundMismatch("ambiguous").into());
    }
    if sound_unknown > 0 {
        sound = EditorState::Unknown;
    }
    let caption_id = format!("{}{caption_suffix}", plan.package);
    let caption = observation::project_query(
        &snapshot,
        &SemanticLocator {
            id: Some(caption_id.clone()),
            role: Some("textbox".into()),
            ..Default::default()
        },
    );
    // A confirmed editor (or its positively observed Loading state) takes
    // precedence over a caption retained underneath it. Only Unknown needs
    // positive caption evidence to authorize the existing conditional retry.
    let unchanged = if sound == EditorState::Unknown {
        let values = observation::caption_values(&caption, plan.package, &caption_id)?;
        matches!(values.as_deref(), Some([only]) if
            crate::tiktok_composer::caption_readback_matches(only, expected_caption))
    } else {
        false
    };
    tracing::info!(session=%snapshot.session_epoch,package=plan.package,
        generation=snapshot.generation,source=?snapshot.source,completeness=?snapshot.completeness,
        sound=?sound,sound_matches,sound_unknown,caption_matches=caption.matches.len(),
        caption_unknown=caption.unknown_match_count,caption_unchanged=unchanged,
        "editor and caption observed in one snapshot");
    observation::check(deadline, None)?;
    Ok((sound, Some(unchanged)))
}

pub(super) async fn confirm(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    expected_title: &str,
) -> anyhow::Result<()> {
    inspect(session, plan, expected_title, true)
        .await
        .map(|_| ())
}

pub(crate) async fn inspect(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    expected_title: &str,
    wait: bool,
) -> anyhow::Result<EditorState> {
    let expected = expected_title.trim();
    anyhow::ensure!(!expected.is_empty(), "selected sound title is empty");
    let deadline = phase_deadline(READBACK_WINDOW);
    let mut cursor = Cursor::default();
    let mut suffixes = vec![plan.current_title_id];
    if plan.dynamic {
        suffixes.extend(
            MEASURED_SOUND_PICKERS
                .iter()
                .filter(|p| p.plan.package == plan.package)
                .map(|p| p.plan.current_title_id),
        );
        suffixes.sort_unstable();
        suffixes.dedup();
    }
    let ids: Vec<_> = suffixes
        .iter()
        .map(|id| format!("{}{id}", plan.package))
        .collect();
    loop {
        observation::check(deadline, None)?;
        // Dynamic IDs are examined within ONE read; never combine independent generations.
        let query = SemanticLocator {
            id: if ids.len() == 1 {
                Some(ids[0].clone())
            } else {
                None
            },
            ..Default::default()
        };
        let mut state = EditorState::Unknown;
        match observation::observe(session, plan.package, query, deadline, None, &mut cursor)
            .await?
        {
            Some(snapshot) => {
                let rows: Vec<_> = snapshot
                    .matches
                    .iter()
                    .filter(|node| {
                        node.package.as_deref() == Some(plan.package)
                            && node.id.as_ref().is_some_and(|id| ids.contains(id))
                    })
                    .collect();
                if rows.len() > 1 {
                    return Err(SoundMismatch("ambiguous").into());
                }
                if observation::positive_query_known(&snapshot) {
                    if let [only] = rows.as_slice() {
                        if only.visible == Some(true) && only.password == Some(false) {
                            if let Some(value) = &only.text {
                                if same_editor_sound_title(value, expected) {
                                    observation::check(deadline, None)?;
                                    return Ok(EditorState::Confirmed);
                                }
                                if value.trim().eq_ignore_ascii_case("Loading") {
                                    state = EditorState::Loading;
                                } else if !value.trim().is_empty() {
                                    return Err(SoundMismatch("title mismatch").into());
                                }
                            }
                        }
                    }
                }
            }
            None => {
                // Explicit Unsupported only. Every legacy read shares the same deadline.
                let mut rows = Vec::new();
                if suffixes.len() > 1 {
                    // The legacy dynamic route also needs one tree, not an atomicity
                    // assumption about multiple independently awaited element queries.
                    let epoch = session.gui_session_epoch();
                    let snapshot =
                        observation::read(deadline, None, session.hierarchy_source_snapshot())
                            .await?;
                    anyhow::ensure!(
                        session.gui_session_epoch() == epoch && snapshot.generation > 0,
                        "legacy sound observation session changed"
                    );
                    let tree = crate::ui_automation::tree::Tree::parse(snapshot)?;
                    for suffix in &suffixes {
                        for index in
                            tree.matching(plan.package, ElementQuery::ResourceIdSuffix(suffix))
                        {
                            let node = &tree.nodes[index];
                            if let Some(mut row) = node.rect() {
                                row.description = Some(node.attr("text").to_owned());
                                rows.push(row);
                            }
                        }
                    }
                } else {
                    rows = observation::read(
                        deadline,
                        None,
                        session.locate_all_described(ElementQuery::ResourceIdSuffix(
                            plan.current_title_id,
                        )),
                    )
                    .await?;
                }
                if matches!(rows.as_slice(), [only] if only.description.as_deref()
                    .is_some_and(|text| same_editor_sound_title(text, expected)))
                {
                    observation::check(deadline, None)?;
                    return Ok(EditorState::Confirmed);
                }
            }
        }
        if !wait {
            return Ok(state);
        }
        tokio::time::sleep(POLL.min(deadline.saturating_duration_since(Instant::now()))).await;
    }
}
