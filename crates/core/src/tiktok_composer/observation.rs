//! Bounded, read-only publish predicates. An observation is never a tap target.
use super::*;
use crate::driver::UnsupportedCapability;
use crate::ui_automation::{
    ObservationFieldMask, ObservationRequest, ObservationScope, SemanticLocator, UiObservation,
    MAX_OBSERVATION_BUDGET_MS,
};

#[derive(Default)]
pub(crate) struct Cursor {
    epoch: Option<String>,
    previous: Option<(u64, String)>,
}

tokio::task_local! {
    static PHASE_CURSOR: std::cell::RefCell<Cursor>;
}

pub(crate) async fn with_binding<T>(work: impl std::future::Future<Output = T>) -> T {
    if PHASE_CURSOR.try_with(|_| ()).is_ok() {
        work.await
    } else {
        PHASE_CURSOR
            .scope(std::cell::RefCell::new(Cursor::default()), work)
            .await
    }
}

pub(crate) fn check_session(session: &dyn UiSession) -> anyhow::Result<()> {
    PHASE_CURSOR
        .try_with(|shared| {
            anyhow::ensure!(
                shared
                    .borrow()
                    .epoch
                    .as_ref()
                    .is_none_or(|epoch| epoch == &session.gui_session_epoch()),
                "publish observation session changed before navigation"
            );
            Ok(())
        })
        .unwrap_or(Ok(()))
}

pub(crate) fn android(package: &str) -> bool {
    matches!(
        package,
        "com.ss.android.ugc.trill" | "com.zhiliaoapp.musically"
    )
}

pub(crate) fn check(deadline: Instant, stop: Option<&AtomicBool>) -> anyhow::Result<()> {
    crate::tiktok_sound::check_wait()?;
    anyhow::ensure!(
        !stop.is_some_and(|flag| flag.load(Ordering::Relaxed)),
        "publish observation stopped"
    );
    anyhow::ensure!(
        Instant::now() < deadline,
        "publish observation deadline exceeded"
    );
    Ok(())
}

/// Only read futures belong here. Gestures must run to completion.
pub(crate) async fn read<T>(
    deadline: Instant,
    stop: Option<&AtomicBool>,
    work: impl std::future::Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    check(deadline, stop)?;
    let task = tokio::time::timeout_at(deadline, work);
    tokio::pin!(task);
    let mut poll = tokio::time::interval(Duration::from_millis(50));
    let value = loop {
        tokio::select! {
            result = &mut task => break result.context("publish observation deadline exceeded")??,
            _ = poll.tick() => check(deadline, stop)?,
        }
    };
    check(deadline, stop)?;
    Ok(value)
}

/// None means the capability explicitly reported Unsupported, never empty evidence.
pub(crate) async fn observe(
    session: &dyn UiSession,
    package: &str,
    query: SemanticLocator,
    deadline: Instant,
    stop: Option<&AtomicBool>,
    cursor: &mut Cursor,
) -> anyhow::Result<Option<UiObservation>> {
    check(deadline, stop)?;
    let epoch = cursor
        .epoch
        .get_or_insert_with(|| session.gui_session_epoch());
    anyhow::ensure!(
        *epoch == session.gui_session_epoch(),
        "publish observation epoch changed"
    );
    let request = ObservationRequest {
        query,
        scope: Some(ObservationScope {
            package: Some(package.into()),
            root: None,
        }),
        fields: ObservationFieldMask {
            identity: true,
            semantics: true,
            states: true,
            bounds: false,
            // The caption restore predicate compares rendered text with the hint.
            raw_attributes: true,
        },
        remaining_ms: deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(MAX_OBSERVATION_BUDGET_MS as u128) as u64,
    };
    let observation = match read(deadline, stop, session.observe(&request)).await {
        Ok(value) => value,
        Err(error)
            if error
                .downcast_ref::<UnsupportedCapability>()
                .is_some_and(|e| e.capability == "ui_observation") =>
        {
            return Ok(None)
        }
        Err(error) => return Err(error),
    };
    anyhow::ensure!(
        !epoch.is_empty()
            && observation.session_epoch == *epoch
            && session.gui_session_epoch() == *epoch
            && observation.app.package.as_deref() == Some(package)
            && !observation.device_id.is_empty()
            && session
                .gui_scope()
                .is_none_or(|scope| scope.device_id == observation.device_id)
            && observation.generation > 0
            && !observation.observation_id.is_empty()
            && observation.ended_at_ms >= observation.started_at_ms
            && cursor
                .previous
                .as_ref()
                .is_none_or(|(generation, id)| observation.generation > *generation
                    && observation.observation_id != *id),
        "publish observation binding invalid"
    );
    cursor.previous = Some((observation.generation, observation.observation_id.clone()));
    // Independently requested caption and sound predicates are not an atomic batch.
    // They must still advance in one session throughout an enclosing phase.
    PHASE_CURSOR
        .try_with(|shared| {
            let mut shared = shared.borrow_mut();
            anyhow::ensure!(
                shared
                    .epoch
                    .as_ref()
                    .is_none_or(|epoch| epoch == &observation.session_epoch)
                    && shared
                        .previous
                        .as_ref()
                        .is_none_or(|(generation, id)| observation.generation > *generation
                            && observation.observation_id != *id),
                "publish phase observation stale or session changed"
            );
            shared.epoch = Some(observation.session_epoch.clone());
            shared.previous = Some((observation.generation, observation.observation_id.clone()));
            Ok::<_, anyhow::Error>(())
        })
        .unwrap_or(Ok(()))?;
    Ok(Some(observation))
}

pub(crate) fn positive_query_known(observation: &UiObservation) -> bool {
    // Positive fields may be proved by a partial upstream tree (coordinator policy).
    // This predicate must NEVER be used to prove absence.
    observation.unknown_match_count == 0 && !observation.matches.is_empty()
}

/// Preserve cardinality and unknowns; missing attributes never become empty text.
pub(crate) async fn caption(
    session: &dyn UiSession,
    package: &str,
    query: ElementQuery<'_>,
    deadline: Instant,
    stop: &AtomicBool,
    cursor: &mut Cursor,
) -> anyhow::Result<Option<Vec<String>>> {
    if android(package) && matches!(query, ElementQuery::ResourceIdSuffix(_)) {
        let ElementQuery::ResourceIdSuffix(suffix) = query else {
            anyhow::bail!("caption semantic locator unmeasured");
        };
        let id = format!("{package}{suffix}");
        if let Some(observation) = observe(
            session,
            package,
            SemanticLocator {
                id: Some(id.clone()),
                ..Default::default()
            },
            deadline,
            Some(stop),
            cursor,
        )
        .await?
        {
            if !positive_query_known(&observation) {
                return Ok(None);
            }
            if observation.matches.len() > 1 {
                anyhow::bail!("caption observation ambiguous");
            }
            let mut values = Vec::new();
            for node in &observation.matches {
                if node.id.as_deref() != Some(id.as_str())
                    || node.package.as_deref() != Some(package)
                    || node.visible != Some(true)
                    || node.enabled != Some(true)
                    || node.showing_hint != Some(false)
                    || node.password != Some(false)
                {
                    return Ok(None);
                }
                let Some(text) = &node.text else {
                    return Ok(None);
                };
                // Focus is a caption/IME transition predicate, not proof that the
                // system keyboard itself is visible outside the scoped package.
                tracing::debug!(focused = ?node.focused, "caption editor predicate observed");
                values.push(text.clone());
            }
            return Ok(Some(values));
        }
        let rows = read(deadline, Some(stop), session.locate_all_described(query)).await?;
        return Ok(rows.into_iter().map(|row| row.description).collect());
    }
    // WDA requests must not be cancelled mid-flight.
    let rows = session.locate_all_described(query).await?;
    Ok(rows.into_iter().map(|row| row.description).collect())
}

pub(super) async fn caption_cleared(
    session: &dyn UiSession,
    package: &str,
    query: ElementQuery<'_>,
    deadline: Instant,
    stop: &AtomicBool,
) -> anyhow::Result<bool> {
    if let ElementQuery::ResourceIdSuffix(suffix) = query {
        let id = format!("{package}{suffix}");
        if let Some(snapshot) = observe(
            session,
            package,
            SemanticLocator {
                id: Some(id.clone()),
                ..Default::default()
            },
            deadline,
            Some(stop),
            &mut Cursor::default(),
        )
        .await?
        {
            let [node] = snapshot.matches.as_slice() else {
                return Ok(false);
            };
            return Ok(positive_query_known(&snapshot)
                && node.id.as_deref() == Some(id.as_str())
                && node.package.as_deref() == Some(package)
                && node.class_name.as_deref() == Some("android.widget.EditText")
                && node.enabled == Some(true)
                && node.visible == Some(true)
                && node.password == Some(false)
                && node.text.as_ref().is_some_and(|text| {
                    text.is_empty()
                        || (node.showing_hint == Some(true)
                            && node
                                .raw_attributes
                                .as_ref()
                                .and_then(|attrs| attrs.get("hint"))
                                .is_some_and(|hint| !hint.is_empty() && hint == text))
                }));
        }
    }
    let actual_package = read(deadline, Some(stop), session.active_app_bundle()).await?;
    anyhow::ensure!(actual_package == package, "caption restore package changed");
    let tree = crate::ui_automation::tree::Tree::parse(
        read(deadline, Some(stop), session.hierarchy_source_snapshot()).await?,
    )?;
    Ok(super::caption_is_empty_after_editor(&tree, package, query))
}
