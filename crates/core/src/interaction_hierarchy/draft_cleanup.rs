//! Production pre-Send draft settlement. Never navigation, Send, or foreign-draft erase.
use super::*;
#[cfg(test)]
#[path = "draft_cleanup_tests.rs"]
mod tests;
use crate::ui_automation::tree::Tree;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftCleanup {
    NotTyped,
    ClearedAndVerified,
    RefusedBindingChanged,
    RefusedDraftChanged,
    FailedReadback,
    FailedClear,
    PublicBoundaryCrossed,
}
impl DraftCleanup {
    pub fn retry_safe(self) -> bool {
        matches!(self, Self::NotTyped | Self::ClearedAndVerified)
    }
    pub fn reason(self) -> &'static str {
        match self {
            Self::NotTyped => "draft_not_typed",
            Self::ClearedAndVerified => "draft_cleared_verified",
            Self::RefusedBindingChanged => "draft_cleanup_binding_changed",
            Self::RefusedDraftChanged => "draft_cleanup_foreign_or_changed",
            Self::FailedReadback => "draft_cleanup_readback_unproved",
            Self::FailedClear => "draft_cleanup_clear_unproved",
            Self::PublicBoundaryCrossed => "public_send_boundary_crossed_cleanup_not_attempted",
        }
    }
}

struct State {
    text: String,
    hint: bool,
    focused: bool,
    armed: bool,
    generation: u64,
}
fn state(
    snapshot: crate::HierarchySourceSnapshot,
    labels: TikTokControls,
) -> anyhow::Result<State> {
    let generation = snapshot.generation;
    anyhow::ensure!(generation != 0, "draft generation unavailable");
    let tree = Tree::parse(snapshot)?;
    let fields = tree.matching(
        labels.package(),
        ElementQuery::ClassName(crate::tiktok_drawer::EDIT_TEXT),
    );
    let focused: Vec<_> = fields
        .iter()
        .copied()
        .filter(|index| tree.nodes[*index].attr("focused") == "true")
        .collect();
    let index = match focused.as_slice() {
        [index] => *index,
        [] if fields.len() == 1 => fields[0],
        _ => anyhow::bail!("draft field ambiguous"),
    };
    let node = &tree.nodes[index];
    anyhow::ensure!(
        node.visibility() == Some(true)
            && tree.ancestors_visible(index)
            && node.attr("enabled") == "true"
            && node.attr("password") != "true"
            && node.rect().is_some(),
        "draft field unavailable"
    );
    let bit = |name| match node.attribute(name) {
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        _ => anyhow::bail!("draft field state unknown"),
    };
    let query = labels
        .label(TikTokControl::CommentSend)
        .context("send label unavailable")?
        .to_query();
    let sends = if let ElementQuery::Semantic(role) = query {
        crate::app_automation::tiktok_roles::indices(&tree, labels.package(), role)
    } else {
        tree.matching(labels.package(), query)
    };
    let [send] = sends.as_slice() else {
        anyhow::bail!("draft send ambiguous")
    };
    let send_node = &tree.nodes[*send];
    anyhow::ensure!(
        send_node.visibility() == Some(true)
            && tree.ancestors_visible(*send)
            && send_node.attr("clickable") == "true",
        "draft send unavailable"
    );
    let field = node.rect().context("draft bounds unavailable")?;
    let send_rect = send_node.rect().context("send bounds unavailable")?;
    anyhow::ensure!(
        !(field.x < send_rect.x + send_rect.width
            && field.x + field.width > send_rect.x
            && field.y < send_rect.y + send_rect.height
            && field.y + field.height > send_rect.y),
        "draft and send overlap"
    );
    let armed = match send_node.attribute("enabled") {
        Some("true") => true,
        Some("false") => false,
        _ => anyhow::bail!("send state unknown"),
    };
    Ok(State {
        text: node
            .attribute("text")
            .context("draft text unavailable")?
            .into(),
        hint: bit("showing-hint")?,
        focused: bit("focused")?,
        armed,
        generation,
    })
}
fn empty_with_hint(value: &State, expected_hint: Option<&str>) -> bool {
    crate::tiktok_drawer::empty_draft(&value.text, value.hint, value.armed, expected_hint)
}

/// Pre-typing identity reads cannot authorize cleanup after session/assignment changes.
/// Account ownership remains the caller's exclusive run/assignment scope; cleanup never navigates
/// to a profile to manufacture a fresh account proof while a draft is on screen.
pub(super) struct DraftGuard<'a> {
    session: &'a dyn UiSession,
    package: String,
    epoch: String,
    scope: Option<(String, Option<String>, String)>,
    author: String,
    caption: String,
    generation: u64,
    text: String,
    expected_hint: Option<String>,
}
fn scope_identity(session: &dyn UiSession) -> Option<(String, Option<String>, String)> {
    session
        .gui_scope()
        .map(|scope| (scope.run_id, scope.assignment_id, scope.device_id))
}
async fn bounded<T>(
    future: impl std::future::Future<Output = anyhow::Result<T>>,
    deadline: Instant,
) -> Option<T> {
    match read_before_deadline(future, deadline, &AtomicBool::new(false)).await {
        Ok(ReadWaitResult::Ready(value)) => Some(value),
        _ => None,
    }
}
pub(super) async fn ensure_empty_before_typing(
    session: &dyn UiSession,
    labels: TikTokControls,
) -> Result<(), DraftCleanup> {
    ensure_empty_before_typing_with_hint(session, labels, None).await
}

pub(super) async fn ensure_empty_before_typing_for_reply(
    session: &dyn UiSession,
    labels: TikTokControls,
    expected_hint: &str,
) -> Result<(), DraftCleanup> {
    ensure_empty_before_typing_with_hint(session, labels, Some(expected_hint)).await
}

async fn ensure_empty_before_typing_with_hint(
    session: &dyn UiSession,
    labels: TikTokControls,
    expected_hint: Option<&str>,
) -> Result<(), DraftCleanup> {
    if !session.supports_accessibility_readback() {
        return Ok(());
    } // Legacy fixture/backend contract, not Android fallback.
    let deadline = Instant::now() + Duration::from_secs(4);
    let epoch = session.gui_session_epoch();
    let scope = scope_identity(session);
    if epoch.is_empty()
        || bounded(session.active_app_bundle(), deadline)
            .await
            .as_deref()
            != Some(labels.package())
    {
        return Err(DraftCleanup::RefusedBindingChanged);
    }
    let snapshot = bounded(session.hierarchy_source_snapshot(), deadline)
        .await
        .ok_or(DraftCleanup::FailedReadback)?;
    let baseline = state(snapshot, labels).map_err(|_| DraftCleanup::FailedReadback)?;
    if session.gui_session_epoch() != epoch || scope_identity(session) != scope {
        return Err(DraftCleanup::RefusedBindingChanged);
    }
    if !baseline.focused {
        return Err(DraftCleanup::FailedReadback);
    }
    if !empty_with_hint(&baseline, expected_hint) {
        return Err(DraftCleanup::RefusedDraftChanged);
    }
    Ok(())
}

impl<'a> DraftGuard<'a> {
    pub(super) async fn capture(
        session: &'a dyn UiSession,
        labels: TikTokControls,
        text: &str,
    ) -> Option<Self> {
        Self::capture_with_hint(session, labels, text, None).await
    }
    pub(super) async fn capture_for_reply(
        session: &'a dyn UiSession,
        labels: TikTokControls,
        text: &str,
        expected_hint: &str,
    ) -> Option<Self> {
        Self::capture_with_hint(session, labels, text, Some(expected_hint)).await
    }
    async fn capture_with_hint(
        session: &'a dyn UiSession,
        labels: TikTokControls,
        text: &str,
        expected_hint: Option<&str>,
    ) -> Option<Self> {
        let deadline = Instant::now() + Duration::from_secs(4);
        let epoch = session.gui_session_epoch();
        if epoch.is_empty() || !session.supports_accessibility_readback() {
            return None;
        }
        let scope = scope_identity(session);
        if bounded(session.active_app_bundle(), deadline)
            .await?
            .as_str()
            != labels.package()
        {
            return None;
        }
        let author = bounded(
            async { Ok(read_author_label(session, labels).await) },
            deadline,
        )
        .await??;
        let caption = bounded(
            async { Ok(read_target_identity_caption(session).await) },
            deadline,
        )
        .await??;
        let baseline = state(
            bounded(session.hierarchy_source_snapshot(), deadline).await?,
            labels,
        )
        .ok()?;
        if !baseline.focused
            || !empty_with_hint(&baseline, expected_hint)
            || session.gui_session_epoch() != epoch
            || scope_identity(session) != scope
        {
            return None;
        }
        Some(Self {
            session,
            package: labels.package().into(),
            epoch,
            scope,
            author,
            caption,
            generation: baseline.generation,
            text: text.into(),
            expected_hint: expected_hint.map(str::to_owned),
        })
    }
    pub(super) fn exact_text(&mut self, text: &str) {
        self.text = text.into();
    }
    pub(super) async fn cleanup(
        &self,
        session: &dyn UiSession,
        labels: TikTokControls,
    ) -> DraftCleanup {
        let deadline = Instant::now() + Duration::from_secs(4);
        let bound = || {
            std::ptr::eq(self.session, session)
                && labels.package() == self.package
                && session.gui_session_epoch() == self.epoch
                && scope_identity(session) == self.scope
        };
        if !bound() {
            return DraftCleanup::RefusedBindingChanged;
        }
        if bounded(session.active_app_bundle(), deadline)
            .await
            .as_deref()
            != Some(labels.package())
        {
            return DraftCleanup::RefusedBindingChanged;
        }
        let author = bounded(
            async { Ok(read_author_label(session, labels).await) },
            deadline,
        )
        .await
        .flatten();
        let caption = bounded(
            async { Ok(read_target_identity_caption(session).await) },
            deadline,
        )
        .await
        .flatten();
        if author.as_deref() != Some(self.author.as_str())
            || caption.as_deref() != Some(self.caption.as_str())
            || !bound()
        {
            return DraftCleanup::RefusedBindingChanged;
        }
        let Some(snapshot) = bounded(session.hierarchy_source_snapshot(), deadline).await else {
            return DraftCleanup::FailedReadback;
        };
        let Ok(before) = state(snapshot, labels) else {
            return DraftCleanup::FailedReadback;
        };
        if !bound() {
            return DraftCleanup::RefusedBindingChanged;
        }
        if before.generation <= self.generation {
            return DraftCleanup::FailedReadback;
        }
        if empty_with_hint(&before, self.expected_hint.as_deref()) {
            return DraftCleanup::ClearedAndVerified;
        }
        if before.hint || !before.focused || before.text != self.text {
            return DraftCleanup::RefusedDraftChanged;
        }
        if !bound() || Instant::now() >= deadline {
            return DraftCleanup::RefusedBindingChanged;
        }
        // Drain the single clear even if its ACK fails. Never timeout/drop/replay this primitive.
        let dispatched = session.type_text("").await;
        let Some(snapshot) = bounded(session.hierarchy_source_snapshot(), deadline).await else {
            return DraftCleanup::FailedReadback;
        };
        if !bound() {
            return DraftCleanup::RefusedBindingChanged;
        }
        let Ok(after) = state(snapshot, labels) else {
            return DraftCleanup::FailedReadback;
        };
        if after.generation <= before.generation {
            return DraftCleanup::FailedReadback;
        }
        if empty_with_hint(&after, self.expected_hint.as_deref()) {
            DraftCleanup::ClearedAndVerified
        } else if dispatched.is_err() {
            DraftCleanup::FailedClear
        } else {
            DraftCleanup::FailedReadback
        }
    }
}

pub(super) async fn settle(
    session: &dyn UiSession,
    labels: TikTokControls,
    guard: Option<&DraftGuard<'_>>,
    typed: bool,
    ownership_lost: bool,
) -> DraftCleanup {
    if !typed {
        return DraftCleanup::NotTyped;
    }
    if ownership_lost {
        return DraftCleanup::RefusedBindingChanged;
    }
    match guard {
        Some(guard) => guard.cleanup(session, labels).await,
        None => DraftCleanup::RefusedBindingChanged,
    }
}
