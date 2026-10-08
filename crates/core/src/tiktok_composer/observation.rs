//! Bounded, read-only publish predicates. An observation is never a tap target.
use super::*;
use crate::driver::{
    classify_read_failure, ReadFailureKind, SessionEpochChanged, UnsupportedCapability,
};
use crate::ui_automation::{
    ObservationFieldMask, ObservationRequest, ObservationScope, SemanticLocator, UiObservation,
    MAX_OBSERVATION_BUDGET_MS,
};

#[derive(Default)]
pub(crate) struct Cursor {
    epoch: Option<String>,
    previous: Option<(u64, String)>,
    retried_read: bool,
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

/// A raw final tree must advance beyond the phase's last semantic evidence.
/// Its native generation is checked locally; no observation ID is fabricated.
pub(super) fn check_tree_generation(session: &dyn UiSession, generation: u64) -> anyhow::Result<()> {
    check_session(session)?;
    let epoch = session.gui_session_epoch();
    anyhow::ensure!(!epoch.is_empty() && generation > 0, "final tree binding unavailable");
    PHASE_CURSOR.try_with(|shared| {
        anyhow::ensure!(shared.borrow().previous.as_ref()
            .is_none_or(|(previous, _)| generation > *previous),
            "final tree is stale within sound reproof");
        Ok::<_, anyhow::Error>(())
    }).unwrap_or(Ok(()))
}

pub(crate) fn android(package: &str) -> bool {
    matches!(
        package,
        "com.ss.android.ugc.trill" | "com.zhiliaoapp.musically"
    )
}

pub(crate) fn check(deadline: Instant, stop: Option<&AtomicBool>) -> anyhow::Result<()> {
    anyhow::ensure!(
        !stop.is_some_and(|flag| flag.load(Ordering::Relaxed)),
        "publish observation stopped"
    );
    crate::tiktok_sound::check_wait()?;
    if Instant::now() >= deadline {
        return Err(crate::publish_recovery::observation_deadline());
    }
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
            result = &mut task => break result,
            _ = poll.tick() => check(deadline, stop)?,
        }
    };
    check(deadline, stop)?;
    value.map_err(|_| crate::publish_recovery::observation_deadline())?
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
        .get_or_insert_with(|| session.gui_session_epoch())
        .clone();
    anyhow::ensure!(
        epoch == session.gui_session_epoch(),
        "publish observation epoch changed"
    );
    check_session(session)?;
    let mut request = ObservationRequest {
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
    let observation = loop {
        check(deadline, stop)?;
        check_session(session)?;
        anyhow::ensure!(
            epoch == session.gui_session_epoch(),
            "publish observation epoch changed"
        );
        request.remaining_ms = deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(MAX_OBSERVATION_BUDGET_MS as u128) as u64;
        let mut trace = StageDiagnosticOperation::start("observe", &epoch, Some(deadline));
        let result = read(deadline, stop, session.observe(&request)).await;
        trace.finish(if result.is_ok() { "completed" } else { "error" });
        drop(trace);
        // A driver repair belongs to the phase owner. Do not accept its fresh proof
        // under an old cursor or replace this typed error with a plain epoch mismatch.
        if result
            .as_ref()
            .is_err_and(|error| error.is::<SessionEpochChanged>())
        {
            return result.map(Some);
        }
        check(deadline, stop)?;
        anyhow::ensure!(
            epoch == session.gui_session_epoch(),
            "publish observation epoch changed"
        );
        match result {
            Ok(value) => break value,
            Err(error)
                if error
                    .downcast_ref::<UnsupportedCapability>()
                    .is_some_and(|e| e.capability == "ui_observation") =>
            {
                return Ok(None)
            }
            Err(error) if classify_read_failure(&error) == ReadFailureKind::Transient => {
                // Claim the retry at the enclosing phase, so a new selector/cursor
                // cannot replenish it. A standalone caller keeps it on its cursor.
                let claimed = PHASE_CURSOR
                    .try_with(|shared| {
                        let mut phase = shared.borrow_mut();
                        !std::mem::replace(&mut phase.retried_read, true)
                    })
                    .unwrap_or_else(|_| !std::mem::replace(&mut cursor.retried_read, true));
                if !claimed {
                    return Err(error);
                }
                crate::publish_recovery::note_read(
                    "observation",
                    "sameSessionRead",
                    1,
                    "retry",
                    None,
                );
                let delay = Duration::from_millis(250)
                    .min(deadline.saturating_duration_since(Instant::now()));
                read(deadline, stop, async {
                    tokio::time::sleep(delay).await;
                    Ok(())
                })
                .await?;
            }
            Err(error) => return Err(error),
        }
    };
    anyhow::ensure!(
        !epoch.is_empty()
            && observation.session_epoch == epoch
            && session.gui_session_epoch() == epoch
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

/// Narrow one already-bound snapshot without dropping unknown candidate matches.
/// This is projection, never a new read or evidence of absence.
pub(crate) fn project_query(snapshot: &UiObservation, query: &SemanticLocator) -> UiObservation {
    use crate::ui_automation::resolver::{compile_locator, SemanticMatch};
    let compiled = compile_locator(query);
    let mut unknown_match_count = snapshot.unknown_match_count;
    if snapshot.source == crate::ui_automation::ObservationSource::NativeQuery && query.id.is_some()
    {
        // Native Android exact-id observations count unreadable identifiers.
        // The broad batch request did not have an id, so restore that guard.
        unknown_match_count += snapshot
            .matches
            .iter()
            .filter(|node| node.id.is_none())
            .count();
    }
    let matches = snapshot
        .matches
        .iter()
        .filter_map(|node| match compiled.matches(node) {
            SemanticMatch::Match => Some(node.clone()),
            SemanticMatch::Unknown => {
                unknown_match_count += 1;
                None
            }
            SemanticMatch::NoMatch => None,
        })
        .collect();
    UiObservation {
        device_id: snapshot.device_id.clone(),
        app: snapshot.app.clone(),
        session_epoch: snapshot.session_epoch.clone(),
        observation_id: snapshot.observation_id.clone(),
        generation: snapshot.generation,
        started_at_ms: snapshot.started_at_ms,
        ended_at_ms: snapshot.ended_at_ms,
        source: snapshot.source,
        completeness: snapshot.completeness,
        matches,
        unknown_match_count,
    }
}

pub(crate) fn caption_values(
    snapshot: &UiObservation,
    package: &str,
    id: &str,
) -> anyhow::Result<Option<Vec<String>>> {
    caption_values_from_matches(&snapshot.matches, snapshot.unknown_match_count, package, id)
}

fn caption_values_from_matches(
    matches: &[crate::ui_automation::SemanticNode],
    unknown_match_count: usize,
    package: &str,
    id: &str,
) -> anyhow::Result<Option<Vec<String>>> {
    if unknown_match_count != 0 || matches.is_empty() {
        return Ok(None);
    }
    if matches.len() > 1 {
        anyhow::bail!("caption observation ambiguous");
    }
    let mut values = Vec::new();
    for node in matches {
        if node.id.as_deref() != Some(id)
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
        tracing::debug!(focused = ?node.focused, "caption editor predicate observed");
        values.push(text.clone());
    }
    Ok(Some(values))
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
                role: Some("textbox".into()),
                ..Default::default()
            },
            deadline,
            Some(stop),
            cursor,
        )
        .await?
        {
            return caption_values(&observation, package, &id);
        }
        let rows = read(deadline, Some(stop), session.locate_all_described(query)).await?;
        return Ok(rows.into_iter().map(|row| row.description).collect());
    }
    // WDA requests must not be cancelled mid-flight.
    let rows = session.locate_all_described(query).await?;
    Ok(rows.into_iter().map(|row| row.description).collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaptionState {
    Confirmed,
    Cleared,
    Unknown,
    Mismatch,
}

fn snapshot_caption_cleared(snapshot: &UiObservation, package: &str, id: &str) -> bool {
    caption_cleared_from_matches(&snapshot.matches, snapshot.unknown_match_count, package, id)
}

fn caption_cleared_from_matches(
    matches: &[crate::ui_automation::SemanticNode],
    unknown_match_count: usize,
    package: &str,
    id: &str,
) -> bool {
    let [node] = matches else {
        return false;
    };
    unknown_match_count == 0
        && node.id.as_deref() == Some(id)
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
        })
}

fn snapshot_caption_state(
    snapshot: &UiObservation,
    package: &str,
    id: &str,
    expected: &str,
) -> anyhow::Result<CaptionState> {
    caption_state_from_matches(&snapshot.matches, snapshot.unknown_match_count, package, id, expected)
}

fn caption_state_from_matches(
    matches: &[crate::ui_automation::SemanticNode],
    unknown_match_count: usize,
    package: &str,
    id: &str,
    expected: &str,
) -> anyhow::Result<CaptionState> {
    anyhow::ensure!(matches.len() <= 1, "caption observation ambiguous");
    if caption_cleared_from_matches(matches, unknown_match_count, package, id) {
        return Ok(CaptionState::Cleared);
    }
    let values = caption_values_from_matches(matches, unknown_match_count, package, id)?;
    Ok(match values.as_deref() {
        Some([value]) if super::caption_readback_matches(value, expected) => {
            CaptionState::Confirmed
        }
        Some([_]) => CaptionState::Mismatch,
        _ => CaptionState::Unknown,
    })
}

/// Local projection of ONE already popup-screened tree; no reads or synthetic evidence IDs.
pub(super) fn final_caption_post_from_tree(
    tree: &crate::ui_automation::tree::Tree,
    package: &str,
    caption_query: ElementQuery<'_>,
    post_query: ElementQuery<'_>,
    expected: &str,
) -> anyhow::Result<(CaptionState, Option<ElementBox>)> {
    use crate::ui_automation::resolver::resolve_observation;
    anyhow::ensure!(tree.generation > 0, "final caption/Post snapshot has no generation");
    let ElementQuery::ResourceIdSuffix(suffix) = caption_query else {
        anyhow::bail!("final caption semantic locator unmeasured");
    };
    let id = format!("{package}{suffix}");
    let request = |query| ObservationRequest {
        query,
        scope: Some(ObservationScope { package: Some(package.into()), root: None }),
        fields: ObservationFieldMask::default(),
        remaining_ms: 0, // Local resolution performs no transport.
    };
    let caption = resolve_observation(tree, &request(SemanticLocator {
        id: Some(id.clone()), role: Some("textbox".into()), ..Default::default()
    }))?;
    let state = caption_state_from_matches(
        &caption.matches, caption.unknown_match_count, package, &id, expected,
    )?;
    let query = match post_query {
        ElementQuery::Text { value, exact: true } => SemanticLocator {
            text: Some(value.into()), ..Default::default()
        },
        _ => anyhow::bail!("final Post semantic locator unmeasured"),
    };
    let post = resolve_observation(tree, &request(query))?;
    // Lossless resolver keeps malformed bounds and unknown states in cardinality.
    let button = match post.matches.as_slice() {
        [node] if post.unknown_match_count == 0
            && node.package.as_deref() == Some(package)
            && node.visible == Some(true)
            && node.enabled == Some(true)
            && node.clickable == Some(true) => tree.nodes[node.node_id].rect(),
        _ => None,
    };
    Ok((state, button))
}

/// One supported snapshot distinguishes a positive empty/hint from unknown text.
/// Only explicit Unsupported may use the compatibility reads below.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn caption_state(
    session: &dyn UiSession,
    package: &str,
    query: ElementQuery<'_>,
    expected: &str,
    deadline: Instant,
    stop: &AtomicBool,
    cursor: &mut Cursor,
) -> anyhow::Result<CaptionState> {
    if android(package) {
        if let ElementQuery::ResourceIdSuffix(suffix) = query {
            let id = format!("{package}{suffix}");
            if let Some(snapshot) = observe(
                session,
                package,
                SemanticLocator {
                    id: Some(id.clone()),
                    role: Some("textbox".into()),
                    ..Default::default()
                },
                deadline,
                Some(stop),
                cursor,
            )
            .await?
            {
                return snapshot_caption_state(&snapshot, package, &id, expected);
            }
        }
    }
    // Preserve WDA's no-midflight-cancellation contract on the compatibility path.
    let rows = if android(package) {
        read(deadline, Some(stop), session.locate_all_described(query)).await?
    } else {
        session.locate_all_described(query).await?
    };
    anyhow::ensure!(rows.len() <= 1, "caption observation ambiguous");
    let Some(value) = rows.first().and_then(|row| row.description.as_ref()) else {
        return Ok(CaptionState::Unknown);
    };
    if super::caption_readback_matches(value, expected) {
        return Ok(CaptionState::Confirmed);
    }
    // Text alone cannot distinguish a placeholder from a genuine different caption.
    // Keep the existing positive XML proof before authorizing caption restoration.
    if android(package) && caption_cleared(session, package, query, deadline, stop).await? {
        return Ok(CaptionState::Cleared);
    }
    Ok(CaptionState::Mismatch)
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
                role: Some("textbox".into()),
                ..Default::default()
            },
            deadline,
            Some(stop),
            &mut Cursor::default(),
        )
        .await?
        {
            return Ok(snapshot_caption_cleared(&snapshot, package, &id));
        }
    }
    let actual_package = read(deadline, Some(stop), session.active_app_bundle()).await?;
    anyhow::ensure!(actual_package == package, "caption restore package changed");
    let tree = crate::ui_automation::tree::Tree::parse(
        read(deadline, Some(stop), session.hierarchy_source_snapshot()).await?,
    )?;
    Ok(super::caption_is_empty_after_editor(&tree, package, query))
}

#[cfg(test)]
mod publish_read_recovery_tests {
    use super::*;
    use crate::driver::{SessionEpochChanged, UiError, UiErrorKind};
    use crate::ui_automation::{ObservationCompleteness, ObservationSource, ObservedAppContext};
    use std::collections::VecDeque;

    enum Reply {
        Transient,
        Fresh,
        Repair,
        Unsupported(&'static str),
        Invalid,
    }

    struct Session {
        replies: parking_lot::Mutex<VecDeque<Reply>>,
        budgets: parking_lot::Mutex<Vec<u64>>,
        epoch: parking_lot::Mutex<String>,
    }

    impl Session {
        fn new(replies: impl IntoIterator<Item = Reply>) -> Self {
            Self {
                replies: parking_lot::Mutex::new(replies.into_iter().collect()),
                budgets: parking_lot::Mutex::new(Vec::new()),
                epoch: parking_lot::Mutex::new("epoch".into()),
            }
        }

        fn fresh(&self, generation: u64) -> UiObservation {
            UiObservation {
                device_id: "phone".into(),
                app: ObservedAppContext {
                    package: Some("com.ss.android.ugc.trill".into()),
                    ..Default::default()
                },
                session_epoch: self.gui_session_epoch(),
                observation_id: format!("observation-{generation}"),
                generation,
                started_at_ms: 1,
                ended_at_ms: 2,
                source: ObservationSource::AccessibilityHierarchy,
                completeness: ObservationCompleteness::Unknown,
                matches: Vec::new(),
                unknown_match_count: 0,
            }
        }
    }

    #[async_trait::async_trait]
    impl UiSession for Session {
        fn stream_url(&self) -> Option<String> {
            None
        }
        fn gui_session_epoch(&self) -> String {
            self.epoch.lock().clone()
        }
        async fn observe(&self, request: &ObservationRequest) -> anyhow::Result<UiObservation> {
            self.budgets.lock().push(request.remaining_ms);
            let reply = self
                .replies
                .lock()
                .pop_front()
                .expect("unexpected extra read");
            tokio::time::sleep(Duration::from_millis(25)).await;
            match reply {
                Reply::Transient => {
                    Err(
                        UiError::new(UiErrorKind::Transport, "observe", "fixture read failure")
                            .into(),
                    )
                }
                Reply::Fresh => Ok(self.fresh(self.budgets.lock().len() as u64)),
                Reply::Repair => {
                    *self.epoch.lock() = "replacement".into();
                    Err(SessionEpochChanged {
                        previous_epoch: "epoch".into(),
                        current_epoch: "replacement".into(),
                        device_id: "phone".into(),
                        package: "com.ss.android.ugc.trill".into(),
                        fresh_observation: Box::new(self.fresh(1)),
                    }
                    .into())
                }
                Reply::Unsupported(capability) => Err(UnsupportedCapability { capability }.into()),
                Reply::Invalid => anyhow::bail!("observation binding invalid"),
            }
        }
        async fn tap(&self, _: crate::TapPoint) -> anyhow::Result<()> {
            panic!("read must not tap")
        }
        async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
            panic!("read must not swipe")
        }
        async fn type_text(&self, _: &str) -> anyhow::Result<()> {
            panic!("read must not type")
        }
        async fn home(&self) -> anyhow::Result<()> {
            panic!("read must not navigate")
        }
        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
            panic!("read must not tap")
        }
        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
            panic!("read must not use legacy assertion")
        }
    }

    #[tokio::test(start_paused = true)]
    async fn transient_retry_budget_is_shared_across_fresh_cursors_in_one_phase() {
        let session = Session::new([
            Reply::Transient,
            Reply::Fresh,
            Reply::Transient,
            Reply::Fresh,
        ]);
        let deadline = Instant::now() + Duration::from_secs(1);
        with_binding(async {
            let first = observe(
                &session,
                "com.ss.android.ugc.trill",
                SemanticLocator::default(),
                deadline,
                None,
                &mut Cursor::default(),
            )
            .await;
            assert!(
                first.is_ok(),
                "a transient read must receive one fresh observation: {first:?}"
            );
            let second = observe(
                &session,
                "com.ss.android.ugc.trill",
                SemanticLocator::default(),
                deadline,
                None,
                &mut Cursor::default(),
            )
            .await;
            assert!(
                second.is_err(),
                "a new selector/cursor must not gain another retry"
            );
        })
        .await;
        let budgets = session.budgets.lock();
        assert_eq!(budgets.len(), 3);
        assert!(
            budgets.windows(2).all(|pair| pair[1] < pair[0]),
            "remaining deadline must shrink: {budgets:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn completed_error_after_stop_or_deadline_never_becomes_a_retry() {
        let stop = AtomicBool::new(false);
        let error = read(
            Instant::now() + Duration::from_secs(1),
            Some(&stop),
            async {
                stop.store(true, Ordering::Relaxed);
                Err::<(), _>(
                    UiError::new(UiErrorKind::Transport, "observe", "fixture read failure").into(),
                )
            },
        )
        .await
        .unwrap_err();
        assert!(
            error.to_string().contains("stopped"),
            "Stop must win over a simultaneously completed error: {error:#}"
        );
        assert_eq!(
            crate::publish_recovery::describe(&error).kind,
            crate::publish_recovery::FailureKind::Terminal
        );

        let error = read(
            Instant::now() + Duration::from_millis(5),
            None,
            std::future::pending::<anyhow::Result<()>>(),
        )
        .await
        .unwrap_err();
        let failure = crate::publish_recovery::describe(&error.context("caption read"));
        assert_eq!(failure.code, "publish_observation_deadline");
        assert_eq!(
            failure.kind,
            crate::publish_recovery::FailureKind::Retryable
        );
    }

    #[tokio::test(start_paused = true)]
    async fn repaired_session_is_typed_owner_recovery_and_never_an_inline_observation() {
        let session = Session::new([Reply::Repair]);
        let mut cursor = Cursor::default();
        let error = observe(
            &session,
            "com.ss.android.ugc.trill",
            SemanticLocator::default(),
            Instant::now() + Duration::from_secs(1),
            None,
            &mut cursor,
        )
        .await
        .unwrap_err();
        assert!(
            error.is::<SessionEpochChanged>(),
            "owner must receive driver repair proof"
        );
        assert!(
            cursor.previous.is_none(),
            "repair proof must not become accepted old-phase evidence"
        );
        let failure = crate::publish_recovery::describe(&error.context("final sound observation"));
        assert_eq!(failure.code, "publish_observation_session_repaired");
        assert_eq!(
            failure.kind,
            crate::publish_recovery::FailureKind::Retryable
        );
        assert_eq!(session.budgets.lock().len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn only_explicit_observation_unsupported_returns_legacy_fallback() {
        for (reply, legacy) in [
            (Reply::Unsupported("ui_observation"), true),
            (Reply::Unsupported("screenshot"), false),
            (Reply::Invalid, false),
        ] {
            let session = Session::new([reply]);
            let result = observe(
                &session,
                "com.ss.android.ugc.trill",
                SemanticLocator::default(),
                Instant::now() + Duration::from_secs(1),
                None,
                &mut Cursor::default(),
            )
            .await;
            assert_eq!(matches!(result, Ok(None)), legacy);
            if !legacy {
                assert!(result.is_err());
            }
            assert_eq!(session.budgets.lock().len(), 1);
        }
    }

    #[test]
    fn caption_state_requires_positive_empty_or_exact_text_proof() {
        use crate::ui_automation::SemanticNode;
        let session = Session::new([]);
        let package = "com.ss.android.ugc.trill";
        let id = format!("{package}:id/caption");
        let known = SemanticNode {
            id: Some(id.clone()),
            package: Some(package.into()),
            class_name: Some("android.widget.EditText".into()),
            text: Some("approved caption".into()),
            enabled: Some(true),
            visible: Some(true),
            password: Some(false),
            showing_hint: Some(false),
            ..Default::default()
        };
        let mut snapshot = session.fresh(1);
        snapshot.matches.push(known.clone());
        assert_eq!(
            snapshot_caption_state(&snapshot, package, &id, "approved caption").unwrap(),
            CaptionState::Confirmed
        );
        snapshot.matches[0].text = Some("different caption".into());
        assert_eq!(
            snapshot_caption_state(&snapshot, package, &id, "approved caption").unwrap(),
            CaptionState::Mismatch
        );
        snapshot.matches[0].text = None;
        assert_eq!(
            snapshot_caption_state(&snapshot, package, &id, "approved caption").unwrap(),
            CaptionState::Unknown
        );
        snapshot.matches[0].text = Some(String::new());
        assert_eq!(
            snapshot_caption_state(&snapshot, package, &id, "approved caption").unwrap(),
            CaptionState::Cleared
        );
        snapshot.unknown_match_count = 1;
        assert_eq!(
            snapshot_caption_state(&snapshot, package, &id, "approved caption").unwrap(),
            CaptionState::Unknown
        );
        snapshot.unknown_match_count = 0;
        snapshot.matches[0].text = Some("Describe your post".into());
        snapshot.matches[0].showing_hint = Some(true);
        assert_eq!(
            snapshot_caption_state(&snapshot, package, &id, "approved caption").unwrap(),
            CaptionState::Unknown
        );
        snapshot.matches[0].raw_attributes =
            Some([("hint".into(), "Describe your post".into())].into());
        assert_eq!(
            snapshot_caption_state(&snapshot, package, &id, "").unwrap(),
            CaptionState::Cleared
        );
        snapshot.matches.push(known);
        assert!(snapshot_caption_state(&snapshot, package, &id, "approved caption").is_err());
    }
}
