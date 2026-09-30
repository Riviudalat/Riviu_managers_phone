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
