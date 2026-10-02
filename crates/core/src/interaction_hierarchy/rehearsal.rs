//! NO-PUBLIC rehearsal: prove identity, prepare a draft, read it exactly, then clear it.
//!
//! This module has no effect gate, assignment recorder, Send operation, mention picker,
//! parent-like option or recipient-share path. The caller retains the exclusive device
//! context through proof, preparation and cleanup. A returned preparation is evidence of
//! an unsent draft, NOT a capability that authorizes a later Send.
use super::*;
use crate::ui_automation::tree::Tree;
use serde::Serialize;
use std::future::Future;

const CLEANUP_WINDOW: Duration = Duration::from_secs(4);
const MAX_DRAFT_CHARS: usize = 150;

/// Validate before proof/navigation, and again before preparing. Control characters
/// (including newline/CR/tab) are not accepted by this conservative root canary.
pub fn validate_rehearsal_text(text: &str) -> Result<(), PrepareFailure> {
    if text.trim().is_empty()
        || text.chars().count() > MAX_DRAFT_CHARS
        || text.chars().any(char::is_control)
    {
        return Err(PrepareFailure::new(PrepareBlocker::InvalidInput));
    }
    Ok(())
}

fn root_hint(value: &str) -> bool {
    [
        "add comment...",
        "add comment…",
        "add comment",
        "thêm bình luận...",
        "thêm bình luận…",
        "thêm bình luận",
    ]
    .contains(&value.trim().to_lowercase().as_str())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PrepareBlocker {
    Cancelled,
    Deadline,
    Unsupported,
    InvalidInput,
    BindingChanged,
    AccountUnproved,
    TargetUnproved,
    StaleDraft,
    ParentNotVisible,
    WrongParent,
    ComposerUnproved,
    DraftMismatch,
    NotArmed,
    Transport,
}

/// Cleanup remains separate even when preparation failed. No raw backend error or
/// draft/account text is formatted into a failure message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DraftCleanup {
    NotCreated,
    ClearedAndVerified,
    RefusedBindingChanged,
    RefusedDraftChanged,
    FailedReadback,
    FailedClear,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("no-public prepare blocked ({blocker:?}); cleanup={cleanup:?}")]
#[serde(rename_all = "camelCase")]
pub struct PrepareFailure {
    pub blocker: PrepareBlocker,
    pub cleanup: DraftCleanup,
}

impl PrepareFailure {
    fn new(blocker: PrepareBlocker) -> Self {
        Self {
            blocker,
            cleanup: DraftCleanup::NotCreated,
        }
    }
}

/// Private, in-process proof. Neither deserialization nor a caller-supplied account
/// string can construct this. It is tied to the exact UiSession object and its epoch.
pub struct RehearsalBinding<'a> {
    session: &'a dyn UiSession,
    epoch: String,
    package: String,
    account: String,
    target: crate::ResolvedTikTokTarget,
    copied_clipboard: String,
    author: String,
    caption: String,
    deadline: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedBeforeSend {
    pub exact_text: String,
    pub account: String,
    pub target: crate::ResolvedTikTokTarget,
    pub parent: Option<CommentLocatorIdentity>,
    /// Caller frame digest is supporting evidence only. The exact readback proof below
    /// hashes the fresh hierarchy rather than treating this frame as live authority.
    pub armed_frame_sha256: String,
    pub readback_hierarchy_sha256: String,
    pub cleanup: DraftCleanup,
}

fn running(
    session: &dyn UiSession,
    epoch: &str,
    stop: &AtomicBool,
    deadline: Instant,
) -> Result<(), PrepareFailure> {
    if stop.load(Ordering::Relaxed) {
        return Err(PrepareFailure::new(PrepareBlocker::Cancelled));
    }
    if Instant::now() >= deadline
        || session
            .gui_scope()
            .and_then(|s| s.deadline_ms)
            .is_some_and(|end| chrono::Utc::now().timestamp_millis() >= end)
    {
        return Err(PrepareFailure::new(PrepareBlocker::Deadline));
    }
    if session.gui_session_epoch() != epoch {
        return Err(PrepareFailure::new(PrepareBlocker::BindingChanged));
    }
    Ok(())
}

async fn read<T>(
    future: impl Future<Output = anyhow::Result<T>>,
    stop: &AtomicBool,
    deadline: Instant,
) -> Result<T, PrepareFailure> {
    match read_before_deadline(future, deadline, stop).await {
        Ok(ReadWaitResult::Ready(value)) => Ok(value),
        Ok(ReadWaitResult::Cancelled) => Err(PrepareFailure::new(PrepareBlocker::Cancelled)),
        Ok(ReadWaitResult::DeadlineExceeded) => Err(PrepareFailure::new(PrepareBlocker::Deadline)),
        Err(_) => Err(PrepareFailure::new(PrepareBlocker::Transport)),
    }
}

fn canonical(value: &str) -> Option<crate::ResolvedTikTokTarget> {
    let url = url::Url::parse(value).ok()?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
        || !matches!(
            url.host_str(),
            Some("www.tiktok.com" | "tiktok.com" | "m.tiktok.com")
        )
    {
        return None;
    }
    crate::parse_tiktok_links(value).into_iter().next()?.target
}

pub(crate) fn input_may_hold_draft(node: &crate::ui_automation::tree::Node) -> bool {
    node.attribute("focused") != Some("false") || node.attribute("text") != Some("")
        || !node.attr("content-desc").is_empty()
}

/// Discovery is separate from preparation approval. It never types or opens a post.
pub async fn inspect_rehearsal_account(
    session: &dyn UiSession,
    labels: TikTokControls,
    stop: &AtomicBool,
    deadline: Instant,
) -> Result<String, PrepareFailure> {
    inspect_rehearsal_account_with_baseline(session, labels, stop, deadline, |_| Ok(())).await
}

/// Observe the exact baseline used by discovery before parsing or navigation.
/// The observer cannot replace that snapshot or grant permission to act.
pub async fn inspect_rehearsal_account_with_baseline(
    session: &dyn UiSession,
    labels: TikTokControls,
    stop: &AtomicBool,
    deadline: Instant,
    observe_baseline: impl FnOnce(&crate::HierarchySourceSnapshot) -> anyhow::Result<()>,
) -> Result<String, PrepareFailure> {
    let epoch = session.gui_session_epoch();
    running(session, &epoch, stop, deadline)?;
    if epoch.is_empty() || !crate::tiktok_account::account_read_supported(labels) {
        return Err(PrepareFailure::new(PrepareBlocker::Unsupported));
    }
    let first = read(session.hierarchy_source_snapshot(), stop, deadline).await?;
    running(session, &epoch, stop, deadline)?;
    observe_baseline(&first).map_err(|_| PrepareFailure::new(PrepareBlocker::Transport))?;
    running(session, &epoch, stop, deadline)?;
    let tree = Tree::parse(first).map_err(|_| PrepareFailure::new(PrepareBlocker::ComposerUnproved))?;
    if tree.matching(labels.package(), ElementQuery::ClassName(crate::tiktok_drawer::EDIT_TEXT))
        .iter().any(|index| {
            let node = &tree.nodes[*index];
            input_may_hold_draft(node)
        }) {
        return Err(PrepareFailure::new(PrepareBlocker::StaleDraft));
    }
    // Discovery only needs a freshly proved Profile target, not an actionable
    // Feed tab. A selected tab may legitimately be non-clickable. NavigationReads
    // still re-proves Profile and excludes overlapping public-action targets.
    for forbidden in [TikTokControl::PostButton, TikTokControl::ComposerDiscard, TikTokControl::PickerNext] {
        if labels.label(forbidden).is_some_and(|label| !role_indices(&tree, labels, label.to_query()).is_empty()) {
            return Err(PrepareFailure::new(PrepareBlocker::StaleDraft));
        }
    }
    if tree.nodes.iter().any(|node| node.attribute("text").is_some_and(|text| matches!(text, "Posting" | "Uploading" | "Discard" | "Save draft"))) {
        return Err(PrepareFailure::new(PrepareBlocker::StaleDraft));
    }
    let bounded = NavigationReads {session, labels, epoch: &epoch, stop, deadline};
    let profile = labels.label(TikTokControl::ProfileTab).ok_or_else(|| PrepareFailure::new(PrepareBlocker::Unsupported))?;
    bounded.activate_element(profile.to_query()).await.map_err(|_| PrepareFailure::new(PrepareBlocker::AccountUnproved))?;
    let account = read(crate::tiktok_account::observe_own_account(&bounded, labels), stop, deadline).await?
        .ok_or_else(|| PrepareFailure::new(PrepareBlocker::AccountUnproved))?;
    running(session, &epoch, stop, deadline)?;
    Ok(account)
}

/// Fresh account and canonical post proof using navigation and Copy link only.
/// Short URLs are refused instead of being resolved over a hidden HTTP request.
/// An existing composer is refused BEFORE navigating; it is never discarded or overwritten.
pub async fn prove_rehearsal_binding<'a>(
    session: &'a dyn UiSession,
    labels: TikTokControls,
    expected_account: &str,
    expected_target: &crate::ResolvedTikTokTarget,
    text: &str,
    stop: &AtomicBool,
    deadline: Instant,
) -> Result<RehearsalBinding<'a>, PrepareFailure> {
    validate_rehearsal_text(text)?;
    let epoch = session.gui_session_epoch();
    if epoch.is_empty() {
        return Err(PrepareFailure::new(PrepareBlocker::Unsupported));
    }
    running(session, &epoch, stop, deadline)?;
    if !session.supports_accessibility_readback()
        || !crate::tiktok_account::account_read_supported(labels)
    {
        return Err(PrepareFailure::new(PrepareBlocker::Unsupported));
    }
    let target = canonical(&expected_target.normalized_url)
        .filter(|target| {
            target.target_key == expected_target.target_key
                && target.content_id == expected_target.content_id
                && target.author == expected_target.author
                && target.kind == expected_target.kind
        })
        .map(|_| expected_target.clone())
        .ok_or_else(|| PrepareFailure::new(PrepareBlocker::InvalidInput))?;
    let expected = expected_account.trim().trim_start_matches('@');
    if expected.is_empty() || !super::is_typeable_handle(expected) {
        return Err(PrepareFailure::new(PrepareBlocker::InvalidInput));
    }
    let bounded = NavigationReads {
        session,
        labels,
        epoch: &epoch,
        stop,
        deadline,
    };
    let baseline = read(session.hierarchy_source_snapshot(), stop, deadline).await?;
    let tree =
        Tree::parse(baseline).map_err(|_| PrepareFailure::new(PrepareBlocker::ComposerUnproved))?;
    if !tree
        .matching(
            labels.package(),
            ElementQuery::ClassName(crate::tiktok_drawer::EDIT_TEXT),
        )
        .is_empty()
    {
        return Err(PrepareFailure::new(PrepareBlocker::StaleDraft));
    }
    let profile = labels
        .label(TikTokControl::ProfileTab)
        .ok_or_else(|| PrepareFailure::new(PrepareBlocker::Unsupported))?;
    bounded
        .activate_element(profile.to_query())
        .await
        .map_err(|_| PrepareFailure::new(PrepareBlocker::AccountUnproved))?;
    running(session, &epoch, stop, deadline)?;
    let account = read(
        crate::tiktok_account::observe_own_account(&bounded, labels),
        stop,
        deadline,
    )
    .await?
    .filter(|account| {
        account
            .trim_start_matches('@')
            .eq_ignore_ascii_case(expected)
    })
    .ok_or_else(|| PrepareFailure::new(PrepareBlocker::AccountUnproved))?;
    running(session, &epoch, stop, deadline)?;
    // Drain an already-dispatched navigation; never timeout/drop a device effect.
    session
        .open_url_in_app(&target.normalized_url, labels.package())
        .await
        .map_err(|_| PrepareFailure::new(PrepareBlocker::Transport))?;
    running(session, &epoch, stop, deadline)?;
    // Navigation ACK is not arrival. Wait only for positive identity observations;
    // this loop never reopens the URL, scrolls, or dispatches another navigation.
    let (author, caption) = loop {
        running(session, &epoch, stop, deadline)?;
        let author = read(async { Ok(super::read_author_label(&bounded, labels).await) }, stop, deadline).await?
            .filter(|s|!s.trim().is_empty());
        let caption = read(async { Ok(super::read_target_identity_caption(&bounded).await) }, stop, deadline).await?
            .filter(|s|!s.trim().is_empty());
        if let (Some(author), Some(caption)) = (author, caption) { break (author, caption); }
        if Instant::now() + Duration::from_millis(250) >= deadline {
            return Err(PrepareFailure::new(PrepareBlocker::TargetUnproved));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    let copied = crate::tiktok_share::capture_post_link(&bounded, &labels).await;
    running(session, &epoch, stop, deadline)?;
    let copied_target = copied
        .link()
        .and_then(canonical)
        .ok_or_else(|| PrepareFailure::new(PrepareBlocker::TargetUnproved))?;
    if !super::link_identifies_target(&copied_target.normalized_url, &target) {
        return Err(PrepareFailure::new(PrepareBlocker::TargetUnproved));
    }
    let (clipboard_kind, clipboard_bytes) = bounded.get_clipboard(4096).await
        .map_err(|_|PrepareFailure::new(PrepareBlocker::TargetUnproved))?;
    if clipboard_kind != "plaintext" { return Err(PrepareFailure::new(PrepareBlocker::TargetUnproved)); }
    let raw_clipboard = std::str::from_utf8(&clipboard_bytes).map_err(|_|PrepareFailure::new(PrepareBlocker::TargetUnproved))?;
    if raw_clipboard.trim() != copied.link().expect("captured link exists") {
        return Err(PrepareFailure::new(PrepareBlocker::TargetUnproved));
    }
    let binding = RehearsalBinding {
        session,
        epoch,
        package: labels.package().to_owned(),
        account,
        target,
        copied_clipboard: raw_clipboard.to_owned(),
        author,
        caption,
        deadline,
    };
    binding.check(session, labels, stop, deadline).await?;
    Ok(binding)
}

impl RehearsalBinding<'_> {
    pub fn canonical_clipboard_value(&self) -> &str { &self.copied_clipboard }
    async fn check(
        &self,
        session: &dyn UiSession,
        labels: TikTokControls,
        stop: &AtomicBool,
        deadline: Instant,
    ) -> Result<(), PrepareFailure> {
        let deadline = deadline.min(self.deadline);
        running(session, &self.epoch, stop, deadline)?;
        if !std::ptr::eq(self.session, session) || labels.package() != self.package {
            return Err(PrepareFailure::new(PrepareBlocker::BindingChanged));
        }
        if read(session.active_app_bundle(), stop, deadline).await? != self.package {
            return Err(PrepareFailure::new(PrepareBlocker::BindingChanged));
        }
        let author = read(
            async { Ok(super::read_author_label(session, labels).await) },
            stop,
            deadline,
        )
        .await?;
        let caption = read(
            async { Ok(super::read_target_identity_caption(session).await) },
            stop,
            deadline,
        )
        .await?;
        if author.as_deref() != Some(&self.author) || caption.as_deref() != Some(&self.caption) {
            return Err(PrepareFailure::new(PrepareBlocker::BindingChanged));
        }
        running(session, &self.epoch, stop, deadline)
    }
}

/// Root rehearsal always clears its own exact draft before returning (including failures).
#[allow(clippy::too_many_arguments)]
pub async fn prepare_root_before_send<F: FnMut() -> String>(
    session: &dyn UiSession,
    labels: TikTokControls,
    screen: (f64, f64),
    binding: RehearsalBinding<'_>,
    text: &str,
    stop: &AtomicBool,
    deadline: Instant,
    frame_sha: F,
) -> Result<PreparedBeforeSend, PrepareFailure> {
    prepare(
        session, labels, screen, binding, None, text, stop, deadline, frame_sha,
    )
    .await
}

/// Reply rehearsal is deliberately unsupported on this branch until dispatch-safe
/// parent proof is qualified. Refuses before any preparation navigation or typing.
#[allow(clippy::too_many_arguments)]
pub async fn prepare_reply_before_send<F: FnMut() -> String>(
    session: &dyn UiSession,
    labels: TikTokControls,
    screen: (f64, f64),
    binding: RehearsalBinding<'_>,
    parent: &CommentLocatorIdentity,
    text: &str,
    stop: &AtomicBool,
    deadline: Instant,
    frame_sha: F,
) -> Result<PreparedBeforeSend, PrepareFailure> {
    prepare(
        session,
        labels,
        screen,
        binding,
        Some(parent),
        text,
        stop,
        deadline,
        frame_sha,
    )
    .await
}

struct ComposerState {
    text: String,
    hint: bool,
    focused: bool,
    armed: bool,
    xml_sha256: String,
    generation: u64,
}

fn composer_state(
    snapshot: crate::HierarchySourceSnapshot,
    labels: TikTokControls,
) -> Result<ComposerState, PrepareFailure> {
    use sha2::{Digest, Sha256};
    let hash = format!("{:x}", Sha256::digest(snapshot.xml.as_bytes()));
    let generation = snapshot.generation;
    let tree =
        Tree::parse(snapshot).map_err(|_| PrepareFailure::new(PrepareBlocker::ComposerUnproved))?;
    let fields = tree.matching(
        labels.package(),
        ElementQuery::ClassName(crate::tiktok_drawer::EDIT_TEXT),
    );
    // Unlike the compatibility composer reader, no trim or reconstructed string can
    // turn a changed draft into exact readback. Missing hint/focus bits are unknown.
    let [index] = fields.as_slice() else {
        return Err(PrepareFailure::new(PrepareBlocker::ComposerUnproved));
    };
    let node = &tree.nodes[*index];
    let bit = |name| match node.attribute(name) {
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        _ => Err(PrepareFailure::new(PrepareBlocker::ComposerUnproved)),
    };
    if node.attr("enabled") != "true" || node.attr("password") == "true" {
        return Err(PrepareFailure::new(PrepareBlocker::ComposerUnproved));
    }
    let field = node
        .rect()
        .ok_or_else(|| PrepareFailure::new(PrepareBlocker::ComposerUnproved))?;
    let send = labels
        .label(TikTokControl::CommentSend)
        .ok_or_else(|| PrepareFailure::new(PrepareBlocker::Unsupported))?;
    let sends = role_indices(&tree, labels, send.to_query());
    let [send_index] = sends.as_slice() else {
        return Err(PrepareFailure::new(PrepareBlocker::ComposerUnproved));
    };
    let send_node = &tree.nodes[*send_index];
    if node.visibility() != Some(true)
        || send_node.visibility() != Some(true)
        || !tree.ancestors_visible(*send_index)
        || send_node.attr("clickable") != "true"
    {
        return Err(PrepareFailure::new(PrepareBlocker::ComposerUnproved));
    }
    let send_rect = send_node
        .rect()
        .ok_or_else(|| PrepareFailure::new(PrepareBlocker::ComposerUnproved))?;
    if field.x < send_rect.x + send_rect.width
        && field.x + field.width > send_rect.x
        && field.y < send_rect.y + send_rect.height
        && field.y + field.height > send_rect.y
    {
        return Err(PrepareFailure::new(PrepareBlocker::ComposerUnproved));
    }
    let armed = match tree.nodes[*send_index].attribute("enabled") {
        Some("true") => true,
        Some("false") => false,
        _ => return Err(PrepareFailure::new(PrepareBlocker::ComposerUnproved)),
    };
    Ok(ComposerState {
        text: node
            .attribute("text")
            .ok_or_else(|| PrepareFailure::new(PrepareBlocker::ComposerUnproved))?
            .to_owned(),
        hint: bit("showing-hint")?,
        focused: bit("focused")?,
        armed,
        xml_sha256: hash,
        generation,
    })
}

#[allow(clippy::too_many_arguments)]
async fn prepare<F: FnMut() -> String>(
    session: &dyn UiSession,
    labels: TikTokControls,
    screen: (f64, f64),
    binding: RehearsalBinding<'_>,
    parent: Option<&CommentLocatorIdentity>,
    text: &str,
    stop: &AtomicBool,
    deadline: Instant,
    mut frame_sha: F,
) -> Result<PreparedBeforeSend, PrepareFailure> {
    let deadline = deadline.min(binding.deadline);
    validate_rehearsal_text(text)?;
    if parent.is_some() {
        return Err(PrepareFailure::new(PrepareBlocker::Unsupported));
    }
    if text.is_empty()
        || text.trim().is_empty()
        || !screen.0.is_finite()
        || !screen.1.is_finite()
        || screen.0 <= 0.0
        || screen.1 <= 0.0
    {
        return Err(PrepareFailure::new(PrepareBlocker::InvalidInput));
    }
    binding.check(session, labels, stop, deadline).await?;
    let bounded = NavigationReads {
        session,
        labels,
        epoch: &binding.epoch,
        stop,
        deadline,
    };
    let existing = read(session.hierarchy_source_snapshot(), stop, deadline).await?;
    let tree =
        Tree::parse(existing).map_err(|_| PrepareFailure::new(PrepareBlocker::ComposerUnproved))?;
    if !tree
        .matching(
            labels.package(),
            ElementQuery::ClassName(crate::tiktok_drawer::EDIT_TEXT),
        )
        .is_empty()
    {
        return Err(PrepareFailure::new(PrepareBlocker::StaleDraft));
    }
    let comments = labels
        .label(TikTokControl::Comments)
        .ok_or_else(|| PrepareFailure::new(PrepareBlocker::Unsupported))?;
    bounded
        .activate_element(comments.to_query())
        .await
        .map_err(|_| PrepareFailure::new(PrepareBlocker::ComposerUnproved))?;
    running(session, &binding.epoch, stop, deadline)?;
    let mut state = read_state(session, labels, stop, deadline).await?;
    if state.armed || (!state.hint && !state.text.is_empty()) {
        return Err(PrepareFailure::new(PrepareBlocker::StaleDraft));
    }
    // An existing Reply composer is not a root field. A root request must not
    // accidentally prepare underneath the parent left selected by somebody else.
    if state.hint && !root_hint(&state.text) {
        return Err(PrepareFailure::new(PrepareBlocker::ComposerUnproved));
    }
    let observed_parent = None;
    binding.check(session, labels, stop, deadline).await?;
    bounded
        .activate_element(ElementQuery::ClassName(crate::tiktok_drawer::EDIT_TEXT))
        .await
        .map_err(|_| PrepareFailure::new(PrepareBlocker::ComposerUnproved))?;
    running(session, &binding.epoch, stop, deadline)?;
    state = read_state(session, labels, stop, deadline).await?;
    if state.armed || (!state.hint && !state.text.is_empty()) {
        return Err(PrepareFailure::new(PrepareBlocker::StaleDraft));
    }
    if !state.focused {
        return Err(PrepareFailure::new(PrepareBlocker::ComposerUnproved));
    }
    binding.check(session, labels, stop, deadline).await?;
    state = read_state(session, labels, stop, deadline).await?;
    if !state.focused
        || state.armed
        || (!state.hint && !state.text.is_empty())
        || (state.hint && !root_hint(&state.text))
    {
        return Err(PrepareFailure::new(PrepareBlocker::StaleDraft));
    }
    running(session, &binding.epoch, stop, deadline)?;
    let baseline_generation = state.generation;
    // From here cleanup is mandatory, even when type_text loses its ACK or Stop arrives.
    let result = async {
        session
            .type_text(text)
            .await
            .map_err(|_| PrepareFailure::new(PrepareBlocker::Transport))?;
        running(session, &binding.epoch, stop, deadline)?;
        let arm_deadline = deadline.min(Instant::now() + crate::tiktok_drawer::ARM_WINDOW);
        loop {
            state = read_state(session, labels, stop, arm_deadline).await?;
            running(session, &binding.epoch, stop, deadline)?;
            if state.generation <= baseline_generation {
                return Err(PrepareFailure::new(PrepareBlocker::ComposerUnproved));
            }
            if state.hint || !state.focused || state.text != text {
                return Err(PrepareFailure::new(PrepareBlocker::DraftMismatch));
            }
            if state.armed {
                break;
            }
            if Instant::now() >= arm_deadline {
                return Err(PrepareFailure::new(PrepareBlocker::NotArmed));
            }
            crate::nurture::sleep_interruptible(
                crate::tiktok_drawer::DRAWER_POLL
                    .min(arm_deadline.saturating_duration_since(Instant::now())),
                stop,
            )
            .await;
        }
        binding.check(session, labels, stop, deadline).await?;
        Ok(PreparedBeforeSend {
            exact_text: state.text.clone(),
            account: binding.account.clone(),
            target: binding.target.clone(),
            parent: observed_parent,
            armed_frame_sha256: frame_sha(),
            readback_hierarchy_sha256: state.xml_sha256.clone(),
            cleanup: DraftCleanup::NotCreated,
        })
    }
    .await;
    let cleanup = cleanup_owned_draft(session, labels, &binding, text, baseline_generation).await;
    match result {
        Ok(mut prepared) => {
            prepared.cleanup = cleanup;
            running(session, &binding.epoch, stop, deadline).map_err(|mut failure| {
                failure.cleanup = cleanup;
                failure
            })?;
            Ok(prepared)
        }
        Err(mut failure) => {
            failure.cleanup = cleanup;
            Err(failure)
        }
    }
}

async fn read_state(
    session: &dyn UiSession,
    labels: TikTokControls,
    stop: &AtomicBool,
    deadline: Instant,
) -> Result<ComposerState, PrepareFailure> {
    composer_state(
        read(session.hierarchy_source_snapshot(), stop, deadline).await?,
        labels,
    )
}

async fn cleanup_owned_draft(
    session: &dyn UiSession,
    labels: TikTokControls,
    binding: &RehearsalBinding<'_>,
    text: &str,
    baseline_generation: u64,
) -> DraftCleanup {
    // Stop/deadline of the preparation do not suppress cleanup. Its separate bounded
    // read budget cannot grant Send, navigation, or another draft write.
    let cleanup_stop = AtomicBool::new(false);
    let deadline = Instant::now() + CLEANUP_WINDOW;
    if session.gui_session_epoch() != binding.epoch
        || !std::ptr::eq(binding.session, session)
        || labels.package() != binding.package
    {
        return DraftCleanup::RefusedBindingChanged;
    }
    if read(session.active_app_bundle(), &cleanup_stop, deadline)
        .await
        .ok()
        .as_deref()
        != Some(&binding.package)
    {
        return DraftCleanup::RefusedBindingChanged;
    }
    let author = read(
        async { Ok(super::read_author_label(session, labels).await) },
        &cleanup_stop,
        deadline,
    )
    .await
    .ok()
    .flatten();
    let caption = read(
        async { Ok(super::read_target_identity_caption(session).await) },
        &cleanup_stop,
        deadline,
    )
    .await
    .ok()
    .flatten();
    if author.as_deref() != Some(&binding.author)
        || caption.as_deref() != Some(&binding.caption)
        || session.gui_session_epoch() != binding.epoch
    {
        return DraftCleanup::RefusedBindingChanged;
    }
    let Ok(before) = read_state(session, labels, &cleanup_stop, deadline).await else {
        return DraftCleanup::FailedReadback;
    };
    if before.generation <= baseline_generation {
        return DraftCleanup::FailedReadback;
    }
    if !before.hint && before.text.is_empty() && !before.armed {
        return DraftCleanup::ClearedAndVerified;
    }
    if before.hint || !before.focused || before.text != text {
        return DraftCleanup::RefusedDraftChanged;
    }
    if session.gui_session_epoch() != binding.epoch {
        return DraftCleanup::RefusedBindingChanged;
    }
    if Instant::now() >= deadline {
        return DraftCleanup::FailedReadback;
    }
    // Drain the one clear; never back out of or erase a draft not read as exactly ours.
    let dispatched = session.type_text("").await;
    let Ok(after) = read_state(session, labels, &cleanup_stop, deadline).await else {
        return DraftCleanup::FailedReadback;
    };
    if session.gui_session_epoch() != binding.epoch {
        return DraftCleanup::RefusedBindingChanged;
    }
    if after.generation <= before.generation {
        return DraftCleanup::FailedReadback;
    }
    if (after.text.is_empty() || (after.hint && root_hint(&after.text))) && !after.armed {
        DraftCleanup::ClearedAndVerified
    } else if dispatched.is_err() {
        DraftCleanup::FailedClear
    } else {
        DraftCleanup::FailedReadback
    }
}

fn role_indices(tree: &Tree, labels: TikTokControls, query: ElementQuery<'_>) -> Vec<usize> {
    if let ElementQuery::Semantic(role) = query {
        crate::app_automation::tiktok_roles::indices(tree, labels.package(), role)
    } else {
        tree.matching(labels.package(), query)
    }
}

fn overlaps(a: &ElementBox, b: &ElementBox) -> bool {
    a.x < b.x + b.width && a.x + a.width > b.x && a.y < b.y + b.height && a.y + a.height > b.y
}

/// Resolve only the intended role. Any other visible clickable rectangle that
/// overlaps it refuses dispatch, except the exact measured lower Global feed
/// background beneath Profile. No union of allowed coordinates is authorization.
pub(crate) fn intended_target(
    tree: &Tree,
    labels: TikTokControls,
    query: ElementQuery<'_>,
) -> anyhow::Result<ElementBox> {
    let matches = role_indices(tree, labels, query);
    let [index] = matches.as_slice() else {
        anyhow::bail!("no-public role ambiguous");
    };
    let node = &tree.nodes[*index];
    anyhow::ensure!(
        node.visibility() == Some(true)
            && tree.ancestors_visible(*index)
            && node.attr("enabled") == "true"
            && node.attr("clickable") == "true",
        "no-public role not actionable"
    );
    let target = node
        .rect()
        .ok_or_else(|| anyhow::anyhow!("no-public role geometry missing"))?;
    for (other_index, other) in tree.nodes.iter().enumerate() {
        if other_index != *index
            && other.visible(labels.package())
            && tree.ancestors_visible(other_index)
            && other.attr("clickable") == "true"
            && other.rect().is_some_and(|r| overlaps(&target, &r))
            && !measured_profile_container(tree, labels, query, *index, other_index, &target)
        {
            anyhow::bail!("no-public role overlaps another action");
        }
    }
    Ok(target)
}

fn measured_profile_container(
    tree: &Tree, labels: TikTokControls, query: ElementQuery<'_>,
    target_index: usize, other_index: usize, target: &ElementBox,
) -> bool {
    let other = &tree.nodes[other_index];
    labels.package() == "com.zhiliaoapp.musically"
        && labels.resource_version() == Some("45.7.3")
        && labels.language() == "en"
        && [TikTokControl::ProfileTab, TikTokControl::HomeTab].into_iter().any(|role| labels.label(role).is_some_and(|label| label.to_query() == query))
        && other.attr("resource-id") == "com.zhiliaoapp.musically:id/hpk"
        && other.attr("class") == "android.widget.FrameLayout"
        && other.attr("text").is_empty() && other.attr("content-desc").is_empty()
        && other.visibility() == Some(true) && other.attr("enabled") == "true"
        && tree.nodes.iter().filter(|node| node.attr("resource-id") == "com.zhiliaoapp.musically:id/hpk").count() == 1
        && measured_profile_above_background(tree, target_index, other_index)
        && other.rect().is_some_and(|r| r.x <= target.x && r.y <= target.y
            && r.x + r.width >= target.x + target.width
            && r.y + r.height >= target.y + target.height)
}

// Measured Global 45.7.3 feed: the nrh navigation bar is drawn above wr2
// feed content (direct branch order 1) inside the same nrm parent. Rectangle overlap alone cannot prove
// that the full-screen hpk background intercepts a bottom navigation button.
fn measured_profile_above_background(tree: &Tree, target: usize, background: usize) -> bool {
    let unique = |suffix: &str| {
        let found: Vec<_> = tree.nodes.iter().enumerate().filter(|(_, node)|
            node.attr("resource-id") == format!("com.zhiliaoapp.musically:id/{suffix}"))
            .map(|(index, _)| index).collect();
        match found.as_slice() { [index] => Some(*index), _ => None }
    };
    let (Some(bar), Some(root)) = (unique("nrh"), unique("nrm")) else { return false };
    let contents: Vec<_> = tree.nodes.iter().enumerate().filter(|(_, node)|
        node.attr("resource-id") == "com.zhiliaoapp.musically:id/wr2" && node.parent == Some(root))
        .map(|(index, _)| index).collect();
    let [content] = contents.as_slice() else { return false };
    let content = *content;
    matches!(tree.nodes[target].attr("resource-id"), "com.zhiliaoapp.musically:id/nrb" | "com.zhiliaoapp.musically:id/nr_")
        && tree.nodes[target].parent == Some(bar)
        && tree.nodes[bar].parent == Some(root) && tree.nodes[content].parent == Some(root)
        && tree.inside(background, content)
        && tree.nodes[bar].attr("class") == "android.widget.LinearLayout"
        && tree.nodes[content].attr("class") == "android.widget.FrameLayout"
        && tree.nodes[bar].visibility() == Some(true) && tree.nodes[content].visibility() == Some(true)
        && tree.nodes[bar].attr("drawing-order") == "4" && tree.nodes[content].attr("drawing-order") == "1"
        && tree.nodes[bar].attr("clickable") == "false" && tree.nodes[content].attr("clickable") == "false"
}

/// Private forwarding facade for the existing account/Copy-link readers. Only explicit
/// navigation targets are enabled; defaults remain unsupported. No raw driver, transport,
/// IME/package management, recovery, public actions or reasoner is exposed.
struct NavigationReads<'a> {
    session: &'a dyn UiSession,
    labels: TikTokControls,
    epoch: &'a str,
    stop: &'a AtomicBool,
    deadline: Instant,
}
impl NavigationReads<'_> {
    fn check(&self) -> anyhow::Result<()> {
        running(self.session, self.epoch, self.stop, self.deadline).map_err(Into::into)
    }
    async fn foreground(&self) -> anyhow::Result<()> {
        self.check()?;
        anyhow::ensure!(
            read(self.session.active_app_bundle(), self.stop, self.deadline).await?
                == self.labels.package(),
            "no-public foreground changed"
        );
        self.check()
    }
    fn allowed(&self, query: ElementQuery<'_>) -> bool {
        [
            TikTokControl::ProfileTab,
            TikTokControl::Share,
            TikTokControl::Comments,
        ]
        .into_iter()
        .any(|control| {
            self.labels
                .label(control)
                .is_some_and(|label| label.to_query() == query)
        }) || query == ElementQuery::ClassName(crate::tiktok_drawer::EDIT_TEXT)
            || ["Copy link", "Sao chép liên kết", "Sao chép link"]
                .into_iter()
                .any(|label| {
                    query
                        == ElementQuery::Description {
                            value: label,
                            exact: true,
                        }
                })
    }
    async fn resolve_dispatch(&self, query: ElementQuery<'_>) -> anyhow::Result<ElementBox> {
        self.foreground().await?;
        let first = self.hierarchy_source_snapshot().await?;
        let before = intended_target(&Tree::parse(first.clone())?, self.labels, query)?;
        self.foreground().await?;
        let second = self.hierarchy_source_snapshot().await?;
        anyhow::ensure!(
            second.generation > first.generation,
            "no-public stale dispatch snapshot"
        );
        let target = intended_target(&Tree::parse(second)?, self.labels, query)?;
        anyhow::ensure!(
            before == target,
            "no-public target moved during dispatch proof"
        );
        // No await/read is inserted between the final snapshot role proof and tap.
        self.check()?;
        Ok(target)
    }
}
#[async_trait::async_trait]
impl UiSession for NavigationReads<'_> {
    async fn tap(&self, _: crate::TapPoint) -> anyhow::Result<()> {
        anyhow::bail!("no-public coordinate navigation refused")
    }
    async fn activate_element(&self, query: ElementQuery<'_>) -> anyhow::Result<()> {
        self.check()?;
        anyhow::ensure!(self.allowed(query), "no-public activation refused");
        let target = self.resolve_dispatch(query).await?;
        self.session.tap(target.centre()).await?;
        self.check()
    }
    async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
        anyhow::bail!("no-public swipe unsupported")
    }
    async fn type_text(&self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("no-public navigation text unsupported")
    }
    async fn home(&self) -> anyhow::Result<()> {
        anyhow::bail!("no-public home unsupported")
    }
    async fn back(&self) -> anyhow::Result<()> {
        self.check()?;
        let mut copy = Vec::new();
        for label in ["Copy link", "Sao chép liên kết", "Sao chép link"] {
            copy.extend(
                self.locate_all_described(ElementQuery::Description {
                    value: label,
                    exact: true,
                })
                .await?,
            );
            copy.extend(
                self.locate_all_described(ElementQuery::Text {
                    value: label,
                    exact: true,
                })
                .await?,
            );
        }
        anyhow::ensure!(
            !copy.is_empty(),
            "no-public back requires observed Copy sheet"
        );
        let _target = self
            .resolve_dispatch(ElementQuery::Description {
                value: "Copy link",
                exact: true,
            })
            .await?;
        self.session.back().await?;
        self.check()
    }
    async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("no-public generic activation unsupported")
    }
    async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("no-public assert unsupported")
    }
    fn stream_url(&self) -> Option<String> {
        None
    }
    fn supports_element_bounds(&self) -> bool {
        true
    }
    fn supports_accessibility_readback(&self) -> bool {
        true
    }
    fn gui_session_epoch(&self) -> String {
        self.session.gui_session_epoch()
    }
    async fn active_app_bundle(&self) -> anyhow::Result<String> {
        self.check()?;
        let value = read(self.session.active_app_bundle(), self.stop, self.deadline).await?;
        self.check()?;
        Ok(value)
    }
    async fn hierarchy_source_snapshot(&self) -> anyhow::Result<crate::HierarchySourceSnapshot> {
        self.check()?;
        let value = read(
            self.session.hierarchy_source_snapshot(),
            self.stop,
            self.deadline,
        )
        .await?;
        self.check()?;
        Ok(value)
    }
    async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
        self.check()?;
        let value = read(self.session.locate(query), self.stop, self.deadline).await?;
        self.check()?;
        Ok(value)
    }
    async fn locate_all(&self, query: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
        self.check()?;
        let value = read(self.session.locate_all(query), self.stop, self.deadline).await?;
        self.check()?;
        Ok(value)
    }
    async fn locate_all_described(
        &self,
        query: ElementQuery<'_>,
    ) -> anyhow::Result<Vec<ElementBox>> {
        self.check()?;
        let value = read(
            self.session.locate_all_described(query),
            self.stop,
            self.deadline,
        )
        .await?;
        self.check()?;
        Ok(value)
    }
    async fn set_clipboard(&self, kind: &str, bytes: &[u8]) -> anyhow::Result<()> {
        self.check()?;
        anyhow::ensure!(
            kind == "plaintext"
                && bytes.starts_with(b"riviu-clipboard-sentinel-")
                && bytes.len() < 100,
            "no-public clipboard write refused"
        );
        self.session.set_clipboard(kind, bytes).await?;
        self.check()
    }
    async fn get_clipboard(&self, limit: usize) -> anyhow::Result<(String, Vec<u8>)> {
        self.check()?;
        let value = read(
            self.session.get_clipboard(limit.min(4096)),
            self.stop,
            self.deadline,
        )
        .await?;
        self.check()?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use std::sync::atomic::{AtomicU64, AtomicUsize};

    const PACKAGE: &str = "com.ss.android.ugc.trill";
    const URL: &str = "https://www.tiktok.com/@fixture.creator/video/7000000000000000001";
    fn labels() -> TikTokControls {
        crate::tiktok_labels::controls_for(PACKAGE, "en", "38.3.2").unwrap()
    }
    fn target() -> crate::ResolvedTikTokTarget {
        canonical(URL).unwrap()
    }
    fn rect(y: f64, text: &str) -> ElementBox {
        ElementBox {
            x: 100.0,
            y,
            width: 200.0,
            height: 60.0,
            description: Some(text.into()),
            enabled: true,
            clickable: true,
        }
    }
    fn node(class: &str, text: &str, id: &str, y: u32) -> String {
        format!(
            r#"<node package="{PACKAGE}" class="{class}" text="{text}" resource-id="{PACKAGE}:id/{id}" displayed="true" enabled="true" clickable="true" bounds="[100,{y}][300,{}]"/>"#,
            y + 60
        )
    }
    struct Phone {
        page: Mutex<&'static str>,
        draft: Mutex<String>,
        focused: AtomicBool,
        generation: AtomicU64,
        epoch: Mutex<String>,
        typed: Mutex<Vec<String>>,
        send_attempts: AtomicUsize,
        like_attempts: AtomicUsize,
        taps: AtomicUsize,
        clipboard: Mutex<Vec<u8>>,
        stop: AtomicBool,
        cancel_after_type: bool,
        mutate_after_type: bool,
        fail_type: bool,
        fail_clear: bool,
        nonempty_hint_after_clear: bool,
        fail_readback_after_type: bool,
        stale_generation_after_type: bool,
        parent_present: bool,
        wrong_parent: bool,
        copied_url: String,
        dispatch_delay: Duration,
        completed_taps: AtomicUsize,
        hide_feed_tab: bool,
        feed_empty_edit: bool,
    }
    impl Default for Phone {
        fn default() -> Self {
            Self {
                page: Mutex::new("feed"),
                draft: Mutex::new(String::new()),
                focused: AtomicBool::new(false),
                generation: AtomicU64::new(0),
                epoch: Mutex::new("fixture-epoch".into()),
                typed: Mutex::new(vec![]),
                send_attempts: AtomicUsize::new(0),
                like_attempts: AtomicUsize::new(0),
                taps: AtomicUsize::new(0),
                clipboard: Mutex::new(vec![]),
                stop: AtomicBool::new(false),
                cancel_after_type: false,
                mutate_after_type: false,
                fail_type: false,
                fail_clear: false,
                nonempty_hint_after_clear: false,
                fail_readback_after_type: false,
                stale_generation_after_type: false,
                parent_present: true,
                wrong_parent: false,
                copied_url: URL.into(),
                dispatch_delay: Duration::ZERO,
                completed_taps: AtomicUsize::new(0),
                hide_feed_tab: false,
                feed_empty_edit: false,
            }
        }
    }
    impl Phone {
        fn assert_no_public(&self) {
            assert_eq!(
                self.send_attempts.load(Ordering::Relaxed),
                0,
                "never attempt Send, including failed dispatch"
            );
            assert_eq!(
                self.like_attempts.load(Ordering::Relaxed),
                0,
                "never parent Like"
            );
        }
        fn snapshot(&self) -> crate::HierarchySourceSnapshot {
            let page = *self.page.lock();
            let mut xml = String::from("<hierarchy>");
            for control in [
                TikTokControl::ProfileTab,
                TikTokControl::FeedTab,
                TikTokControl::Share,
                TikTokControl::Comments,
                TikTokControl::AuthorProfileLink,
            ] {
                if let Some(label) = labels().label(control) {
                    for r in self.matches(label.to_query()) {
                        let (attr, value) = match label.to_query() {
                            ElementQuery::Description { value, .. } => {
                                ("content-desc", value.to_owned())
                            }
                            ElementQuery::Text { value, .. } => ("text", value.to_owned()),
                            ElementQuery::ResourceIdSuffix(value) => {
                                ("resource-id", format!("{PACKAGE}{value}"))
                            }
                            _ => continue,
                        };
                        xml += &format!(
                            r#"<node package="{PACKAGE}" class="android.widget.Button" {attr}="{value}" displayed="true" enabled="true" clickable="true" bounds="[{},{}][{},{}]"/>"#,
                            r.x,
                            r.y,
                            r.x + r.width,
                            r.y + r.height
                        );
                    }
                }
            }
            if page == "feed" && self.feed_empty_edit {
                xml += &format!(r#"<node package="{PACKAGE}" class="android.widget.EditText" text="" focused="false" displayed="true" enabled="true" clickable="false" bounds="[501,603][579,677]"/>"#);
            }
            if page == "sheet" {
                xml += &format!(
                    r#"<node package="{PACKAGE}" class="android.widget.Button" content-desc="Copy link" text="Copy link" displayed="true" enabled="true" clickable="true" bounds="[100,900][300,960]"/>"#
                );
            }
            if page == "own" {
                xml += &node("android.widget.TextView", "Edit profile", "dby", 100);
                xml += &node("android.widget.Button", "@fixture.actor", "mjf", 200);
            }
            if matches!(page, "drawer" | "reply") {
                let draft = self.draft.lock().clone();
                let hint = draft.is_empty();
                let text = if hint {
                    if page == "reply" {
                        if self.wrong_parent {
                            "Replying to stranger"
                        } else {
                            "Replying to fixture.parent"
                        }
                    } else {
                        "Add comment..."
                    }
                } else {
                    &draft
                };
                xml += &format!(
                    r#"<node package="{PACKAGE}" class="android.widget.EditText" text="{text}" displayed="true" enabled="true" clickable="true" focused="{}" showing-hint="{hint}" bounds="[100,1000][800,1100]"/>"#,
                    self.focused.load(Ordering::Relaxed)
                );
                let send = labels().label(TikTokControl::CommentSend).unwrap();
                let attr = match send.to_query() {
                    ElementQuery::Description { .. } => "content-desc",
                    ElementQuery::ResourceIdSuffix(_) => "resource-id",
                    ElementQuery::Text { .. } => "text",
                    _ => panic!("measured Send query"),
                };
                xml += &format!(
                    r#"<node package="{PACKAGE}" class="android.widget.Button" {attr}="{}" displayed="true" enabled="{}" clickable="true" bounds="[900,1000][1000,1100]"/>"#,
                    send.value(),
                    !draft.is_empty()
                );
                if self.parent_present {
                    xml += "<node>";
                    xml += &node("android.widget.Button", "fixture.parent", "author", 300);
                    xml += &node(
                        "android.widget.TextView",
                        "fixture parent body",
                        "body",
                        370,
                    );
                    xml += &node(
                        "android.widget.Button",
                        labels().label(TikTokControl::CommentReply).unwrap().value(),
                        "reply",
                        450,
                    );
                    xml += "</node>";
                }
            }
            xml += "</hierarchy>";
            crate::HierarchySourceSnapshot {
                generation: self.generation.fetch_add(1, Ordering::Relaxed) + 1,
                xml,
            }
        }
        fn matches(&self, query: ElementQuery<'_>) -> Vec<ElementBox> {
            let label = |control| {
                labels()
                    .label(control)
                    .is_some_and(|l| l.to_query() == query)
            };
            let page = *self.page.lock();
            if label(TikTokControl::ProfileTab) && page != "own" {
                return vec![rect(100.0, "Profile")];
            }
            if label(TikTokControl::FeedTab) && page == "feed" && !self.hide_feed_tab {
                return vec![rect(400.0, "Home")];
            }
            if label(TikTokControl::Share) && page == "post" {
                return vec![rect(700.0, "Share")];
            }
            if label(TikTokControl::Comments) && matches!(page, "post" | "drawer" | "reply") {
                return vec![rect(800.0, "Comments")];
            }
            if label(TikTokControl::AuthorProfileLink)
                && matches!(page, "post" | "drawer" | "reply")
            {
                return vec![rect(600.0, "fixture.creator profile")];
            }
            if query == ElementQuery::ResourceIdSuffix(":id/desc")
                && matches!(page, "post" | "drawer" | "reply")
            {
                return vec![rect(600.0, "fixture caption")];
            }
            if page == "sheet"
                && (query
                    == (ElementQuery::Description {
                        value: "Copy link",
                        exact: true,
                    })
                    || query
                        == (ElementQuery::Text {
                            value: "Copy link",
                            exact: true,
                        })
                    || query == ElementQuery::ClassName("android.widget.TextView"))
            {
                return vec![rect(900.0, "Copy link")];
            }
            Vec::new()
        }
    }
    #[async_trait::async_trait]
    impl UiSession for Phone {
        async fn tap(&self, point: crate::TapPoint) -> anyhow::Result<()> {
            if point.x >= 900.0 && point.y >= 1000.0 && point.y < 1100.0 {
                self.send_attempts.fetch_add(1, Ordering::Relaxed);
                panic!("PUBLIC SEND TRAP");
            }
            if point.x >= 850.0 && point.y >= 400.0 && point.y < 500.0 {
                self.like_attempts.fetch_add(1, Ordering::Relaxed);
                panic!("PUBLIC PARENT LIKE TRAP");
            }
            self.taps.fetch_add(1, Ordering::Relaxed);
            if !self.dispatch_delay.is_zero() {
                tokio::time::sleep(self.dispatch_delay).await;
            }
            let mut page = self.page.lock();
            match point.y as u32 {
                130 => *page = "own",
                730 => *page = "sheet",
                930 => {
                    *self.clipboard.lock() = self.copied_url.as_bytes().to_vec();
                    *page = "post";
                }
                830 => *page = "drawer",
                480 => *page = "reply",
                1000..=1100 => self.focused.store(true, Ordering::Relaxed),
                _ => panic!("unapproved fixture tap {point:?}"),
            }
            self.completed_taps.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
            panic!("no rehearsal swipe")
        }
        async fn type_text(&self, text: &str) -> anyhow::Result<()> {
            self.typed.lock().push(text.into());
            if text.is_empty() && self.fail_clear {
                anyhow::bail!("fixture clear failed");
            }
            *self.draft.lock() = if !text.is_empty() && self.mutate_after_type {
                "different draft".into()
            } else {
                text.into()
            };
            if !text.is_empty() && self.cancel_after_type {
                self.stop.store(true, Ordering::Relaxed);
            }
            if !text.is_empty() && self.fail_type {
                anyhow::bail!("fixture text ACK lost");
            }
            Ok(())
        }
        async fn home(&self) -> anyhow::Result<()> {
            panic!("no rehearsal Home")
        }
        async fn back(&self) -> anyhow::Result<()> {
            *self.page.lock() = "post";
            Ok(())
        }
        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
            panic!("no generic tap")
        }
        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
            panic!("no generic assert")
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
        fn supports_accessibility_readback(&self) -> bool {
            true
        }
        fn supports_element_bounds(&self) -> bool {
            true
        }
        fn gui_session_epoch(&self) -> String {
            self.epoch.lock().clone()
        }
        async fn active_app_bundle(&self) -> anyhow::Result<String> {
            Ok(PACKAGE.into())
        }
        async fn hierarchy_source_snapshot(
            &self,
        ) -> anyhow::Result<crate::HierarchySourceSnapshot> {
            if self.fail_readback_after_type && !self.typed.lock().is_empty() {
                anyhow::bail!("fixture readback failed");
            }
            let mut snapshot = self.snapshot();
            if self.stale_generation_after_type && !self.typed.lock().is_empty() {
                snapshot.generation = 1;
            }
            if self.nonempty_hint_after_clear
                && self.typed.lock().last().is_some_and(|text| text.is_empty())
            {
                snapshot.xml = snapshot
                    .xml
                    .replace("text=\"Add comment...\"", "text=\"changed foreign draft\"");
            }
            Ok(snapshot)
        }
        async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            Ok(self.matches(query).into_iter().next())
        }
        async fn locate_all(&self, query: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
            Ok(self.matches(query))
        }
        async fn locate_all_described(
            &self,
            query: ElementQuery<'_>,
        ) -> anyhow::Result<Vec<ElementBox>> {
            Ok(self.matches(query))
        }
        async fn open_url_in_app(&self, url: &str, package: &str) -> anyhow::Result<()> {
            assert_eq!(url, URL);
            assert_eq!(package, PACKAGE);
            *self.page.lock() = "post";
            Ok(())
        }
        async fn set_clipboard(&self, _: &str, bytes: &[u8]) -> anyhow::Result<()> {
            *self.clipboard.lock() = bytes.to_vec();
            Ok(())
        }
        async fn get_clipboard(&self, _: usize) -> anyhow::Result<(String, Vec<u8>)> {
            Ok(("plaintext".into(), self.clipboard.lock().clone()))
        }
    }
    async fn binding(phone: &Phone, deadline: Instant) -> RehearsalBinding<'_> {
        prove_rehearsal_binding(
            phone,
            labels(),
            "fixture.actor",
            &target(),
            "fixture draft",
            &phone.stop,
            deadline,
        )
        .await
        .unwrap()
    }
    fn parent() -> CommentLocatorIdentity {
        CommentLocatorIdentity {
            author_label: "fixture.parent".into(),
            text: "fixture parent body".into(),
            locator_version: HIERARCHY_LOCATOR_VERSION.into(),
            frame_sha256: "fixture-parent-frame".into(),
            comment_link: None,
        }
    }
    #[tokio::test(start_paused = true)]
    async fn discovery_reads_account_without_typing_or_clipboard() {
        let phone = Phone::default();
        let account = inspect_rehearsal_account(
            &phone, labels(), &phone.stop, Instant::now() + Duration::from_secs(30),
        ).await.unwrap();
        assert_eq!(account, "fixture.actor");
        assert!(phone.typed.lock().is_empty());
        assert!(phone.clipboard.lock().is_empty());
        phone.assert_no_public();
    }

    #[test]
    fn discovery_unobserved_or_nonempty_input_is_not_an_empty_feed_widget() {
        for attributes in [r#"text="""#, r#"focused="false""#, r#"text="" focused="true""#, r#"text="foreign draft" focused="false""#, r#"text="" focused="false" content-desc="Add comment""#] {
            let tree = Tree::parse(crate::HierarchySourceSnapshot { generation: 1,
                xml: format!(r#"<hierarchy><node package="{PACKAGE}" class="android.widget.EditText" {attributes}/></hierarchy>"#) }).unwrap();
            assert!(input_may_hold_draft(&tree.nodes[1]));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn discovery_empty_unfocused_feed_widgets_are_not_a_draft() {
        let phone = Phone { feed_empty_edit: true, ..Phone::default() };
        assert_eq!(inspect_rehearsal_account(&phone, labels(), &phone.stop,
            Instant::now() + Duration::from_secs(30)).await.unwrap(), "fixture.actor");
        assert!(phone.typed.lock().is_empty());
        phone.assert_no_public();
    }

    #[tokio::test(start_paused = true)]
    async fn discovery_does_not_require_an_actionable_feed_tab() {
        let phone = Phone { hide_feed_tab: true, ..Phone::default() };
        let account = inspect_rehearsal_account(
            &phone, labels(), &phone.stop, Instant::now() + Duration::from_secs(30),
        ).await.unwrap();
        assert_eq!(account, "fixture.actor");
        assert!(phone.typed.lock().is_empty());
        assert!(phone.clipboard.lock().is_empty());
        phone.assert_no_public();
    }

    #[tokio::test(start_paused = true)]
    async fn discovery_persists_exact_refusal_baseline_before_any_navigation() {
        let phone = Phone::default();
        *phone.page.lock() = "drawer";
        *phone.draft.lock() = "existing unrelated draft".into();
        let mut observed = None;
        let failure = inspect_rehearsal_account_with_baseline(
            &phone, labels(), &phone.stop, Instant::now() + Duration::from_secs(30),
            |snapshot| { observed = Some(snapshot.clone()); Ok(()) },
        ).await.unwrap_err();
        assert_eq!(failure.blocker, PrepareBlocker::StaleDraft);
        let snapshot = observed.expect("refusal baseline must reach observer");
        assert_eq!(snapshot.generation, 1);
        assert!(snapshot.xml.contains("existing unrelated draft"));
        assert_eq!(phone.generation.load(Ordering::Relaxed), 1, "no second read substitutes refusal input");
        assert_eq!(phone.taps.load(Ordering::Relaxed), 0);
        assert!(phone.typed.lock().is_empty());
        phone.assert_no_public();
    }

    #[tokio::test(start_paused = true)]
    async fn discovery_evidence_failure_prevents_navigation() {
        let phone = Phone::default();
        let failure = inspect_rehearsal_account_with_baseline(
            &phone, labels(), &phone.stop, Instant::now() + Duration::from_secs(30),
            |_| anyhow::bail!("fixture receipt write failed"),
        ).await.unwrap_err();
        assert_eq!(failure.blocker, PrepareBlocker::Transport);
        assert_eq!(phone.taps.load(Ordering::Relaxed), 0);
        assert!(phone.typed.lock().is_empty());
        phone.assert_no_public();
    }

    #[tokio::test(start_paused = true)]
    async fn discovery_refuses_existing_composer_before_navigation() {
        let phone = Phone::default();
        *phone.page.lock() = "drawer";
        *phone.draft.lock() = "existing unrelated draft".into();
        assert!(inspect_rehearsal_account(
            &phone, labels(), &phone.stop, Instant::now() + Duration::from_secs(30),
        ).await.is_err());
        assert_eq!(phone.taps.load(Ordering::Relaxed), 0);
        assert_eq!(&*phone.draft.lock(), "existing unrelated draft");
        phone.assert_no_public();
    }

    #[tokio::test(start_paused = true)]
    async fn root_exact_prepared_before_send_and_cleared_without_public_effects() {
        let phone = Phone::default();
        let deadline = Instant::now() + Duration::from_secs(30);
        let binding = binding(&phone, deadline).await;
        let result = prepare_root_before_send(
            &phone,
            labels(),
            (1080.0, 2400.0),
            binding,
            "  fixture draft exact  ",
            &phone.stop,
            deadline,
            || "fixture-frame".into(),
        )
        .await
        .unwrap();
        assert_eq!(
            result.exact_text, "  fixture draft exact  ",
            "no trim can fake exact readback"
        );
        assert_eq!(result.account, "fixture.actor");
        assert_eq!(result.target, target());
        assert_eq!(result.parent, None);
        assert_eq!(result.cleanup, DraftCleanup::ClearedAndVerified);
        assert_eq!(result.readback_hierarchy_sha256.len(), 64);
        assert_eq!(
            phone.typed.lock().as_slice(),
            &["  fixture draft exact  ", ""]
        );
        phone.assert_no_public();
    }
    #[tokio::test(start_paused = true)]
    async fn reply_is_explicitly_unsupported_without_any_prepare_dispatch() {
        let phone = Phone::default();
        let deadline = Instant::now() + Duration::from_secs(30);
        let binding = binding(&phone, deadline).await;
        let result = prepare_reply_before_send(
            &phone,
            labels(),
            (1080.0, 2400.0),
            binding,
            &parent(),
            "fixture reply",
            &phone.stop,
            deadline,
            String::new,
        )
        .await
        .unwrap_err();
        assert_eq!(result.blocker, PrepareBlocker::Unsupported);
        assert!(phone.typed.lock().is_empty());
        phone.assert_no_public();
    }
    #[tokio::test(start_paused = true)]
    async fn stale_draft_never_overwritten_or_cleaned_as_ours() {
        let phone = Phone::default();
        let deadline = Instant::now() + Duration::from_secs(30);
        let binding = binding(&phone, deadline).await;
        *phone.page.lock() = "drawer";
        *phone.draft.lock() = "stale fixture draft".into();
        let failure = prepare_root_before_send(
            &phone,
            labels(),
            (1080.0, 2400.0),
            binding,
            "fresh",
            &phone.stop,
            deadline,
            String::new,
        )
        .await
        .unwrap_err();
        assert_eq!(failure.blocker, PrepareBlocker::StaleDraft);
        assert_eq!(failure.cleanup, DraftCleanup::NotCreated);
        assert!(phone.typed.lock().is_empty());
        assert_eq!(*phone.draft.lock(), "stale fixture draft");
        phone.assert_no_public();
    }
    #[tokio::test(start_paused = true)]
    async fn cancellation_after_typing_still_clears_exact_own_draft() {
        let phone = Phone {
            cancel_after_type: true,
            ..Default::default()
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        let binding = binding(&phone, deadline).await;
        let failure = prepare_root_before_send(
            &phone,
            labels(),
            (1080.0, 2400.0),
            binding,
            "fixture draft",
            &phone.stop,
            deadline,
            String::new,
        )
        .await
        .unwrap_err();
        assert_eq!(failure.blocker, PrepareBlocker::Cancelled);
        assert_eq!(failure.cleanup, DraftCleanup::ClearedAndVerified);
        assert_eq!(phone.typed.lock().as_slice(), &["fixture draft", ""]);
        phone.assert_no_public();
    }
    #[tokio::test(start_paused = true)]
    async fn failure_after_typing_retains_separate_cleanup_result() {
        let phone = Phone {
            fail_type: true,
            fail_clear: true,
            ..Default::default()
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        let binding = binding(&phone, deadline).await;
        let failure = prepare_root_before_send(
            &phone,
            labels(),
            (1080.0, 2400.0),
            binding,
            "fixture draft",
            &phone.stop,
            deadline,
            String::new,
        )
        .await
        .unwrap_err();
        assert_eq!(failure.blocker, PrepareBlocker::Transport);
        assert_eq!(failure.cleanup, DraftCleanup::FailedClear);
        phone.assert_no_public();
    }
    #[tokio::test(start_paused = true)]
    async fn changed_draft_is_not_exact_and_never_erased() {
        let phone = Phone {
            mutate_after_type: true,
            ..Default::default()
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        let binding = binding(&phone, deadline).await;
        let failure = prepare_root_before_send(
            &phone,
            labels(),
            (1080.0, 2400.0),
            binding,
            "fixture draft",
            &phone.stop,
            deadline,
            String::new,
        )
        .await
        .unwrap_err();
        assert_eq!(failure.blocker, PrepareBlocker::DraftMismatch);
        assert_eq!(failure.cleanup, DraftCleanup::RefusedDraftChanged);
        assert_eq!(phone.typed.lock().len(), 1);
        phone.assert_no_public();
    }
    #[tokio::test(start_paused = true)]
    async fn failed_readback_is_not_prepared_and_cleanup_failure_is_visible() {
        let phone = Phone {
            fail_readback_after_type: true,
            ..Default::default()
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        let binding = binding(&phone, deadline).await;
        let failure = prepare_root_before_send(
            &phone,
            labels(),
            (1080.0, 2400.0),
            binding,
            "fixture draft",
            &phone.stop,
            deadline,
            String::new,
        )
        .await
        .unwrap_err();
        assert_eq!(failure.blocker, PrepareBlocker::Transport);
        assert_eq!(failure.cleanup, DraftCleanup::FailedReadback);
        phone.assert_no_public();
    }
    #[tokio::test(start_paused = true)]
    async fn cached_exact_text_cannot_fake_fresh_prepare_or_cleanup() {
        let phone = Phone {
            stale_generation_after_type: true,
            ..Default::default()
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        let proof = binding(&phone, deadline).await;
        let failure = prepare_root_before_send(
            &phone,
            labels(),
            (1080.0, 2400.0),
            proof,
            "fixture draft",
            &phone.stop,
            deadline,
            String::new,
        )
        .await
        .unwrap_err();
        assert_eq!(failure.blocker, PrepareBlocker::ComposerUnproved);
        assert_eq!(failure.cleanup, DraftCleanup::FailedReadback);
        phone.assert_no_public();
    }
    #[tokio::test(start_paused = true)]
    async fn reply_missing_or_wrong_parent_refuses_before_typing() {
        for wrong in [false, true] {
            let phone = Phone {
                parent_present: wrong,
                wrong_parent: wrong,
                ..Default::default()
            };
            let deadline = Instant::now() + Duration::from_secs(30);
            let binding = binding(&phone, deadline).await;
            let failure = prepare_reply_before_send(
                &phone,
                labels(),
                (1080.0, 2400.0),
                binding,
                &parent(),
                "fixture reply",
                &phone.stop,
                deadline,
                String::new,
            )
            .await
            .unwrap_err();
            assert_eq!(failure.blocker, PrepareBlocker::Unsupported);
            assert!(phone.typed.lock().is_empty());
            phone.assert_no_public();
        }
    }
    #[tokio::test(start_paused = true)]
    async fn binding_other_session_or_new_epoch_refuses_every_dispatch() {
        let phone = Phone::default();
        let other = Phone::default();
        let deadline = Instant::now() + Duration::from_secs(30);
        let proof = binding(&phone, deadline).await;
        let failure = prepare_root_before_send(
            &other,
            labels(),
            (1080.0, 2400.0),
            proof,
            "fixture",
            &other.stop,
            deadline,
            String::new,
        )
        .await
        .unwrap_err();
        assert_eq!(failure.blocker, PrepareBlocker::BindingChanged);
        assert_eq!(other.taps.load(Ordering::Relaxed), 0);
        let proof = binding(&phone, deadline).await;
        *phone.epoch.lock() = "new-epoch".into();
        assert_eq!(
            prepare_root_before_send(
                &phone,
                labels(),
                (1080.0, 2400.0),
                proof,
                "fixture",
                &phone.stop,
                deadline,
                String::new
            )
            .await
            .unwrap_err()
            .blocker,
            PrepareBlocker::BindingChanged
        );
        assert!(phone.typed.lock().is_empty());
        phone.assert_no_public();
    }
    #[tokio::test(start_paused = true)]
    async fn canonical_proof_retains_raw_clipboard_for_conditional_restoration() {
        let phone = Phone { copied_url: format!("\n{URL}\n"), ..Phone::default() };
        let proof = binding(&phone, Instant::now() + Duration::from_secs(30)).await;
        assert_eq!(proof.canonical_clipboard_value(), format!("\n{URL}\n"));
        assert_eq!(proof.target.normalized_url, URL);
        phone.assert_no_public();
    }

    #[tokio::test(start_paused = true)]
    async fn copied_short_url_or_wrong_post_never_grants_binding() {
        for copied in [
            "https://vm.tiktok.com/fixture/",
            "https://www.tiktok.com/@fixture.creator/video/7000000000000000002",
        ] {
            let phone = Phone {
                copied_url: copied.into(),
                ..Default::default()
            };
            let failure = prove_rehearsal_binding(
                &phone,
                labels(),
                "fixture.actor",
                &target(),
                "fixture draft",
                &phone.stop,
                Instant::now() + Duration::from_secs(30),
            )
            .await
            .err()
            .unwrap();
            assert_eq!(failure.blocker, PrepareBlocker::TargetUnproved);
            assert!(phone.typed.lock().is_empty());
            phone.assert_no_public();
        }
    }
    #[tokio::test(start_paused = true)]
    async fn expired_or_cancelled_facade_never_dispatches_and_unknown_actions_fail_closed() {
        let phone = Phone::default();
        let epoch = phone.gui_session_epoch();
        let facade = NavigationReads {
            session: &phone,
            labels: labels(),
            epoch: &epoch,
            stop: &phone.stop,
            deadline: Instant::now(),
        };
        assert!(facade.tap(rect(100.0, "Profile").centre()).await.is_err());
        assert!(facade
            .activate_element(
                labels()
                    .label(TikTokControl::CommentSend)
                    .unwrap()
                    .to_query()
            )
            .await
            .is_err());
        assert!(facade.type_keys("fixture").await.is_err());
        assert!(facade.open_url(URL).await.is_err());
        let active = NavigationReads {
            deadline: Instant::now() + Duration::from_secs(1),
            ..facade
        };
        assert!(active
            .activate_element(
                labels()
                    .label(TikTokControl::CommentSend)
                    .unwrap()
                    .to_query()
            )
            .await
            .is_err());
        assert!(active.type_text("not allowed").await.is_err());
        assert!(active.open_url(URL).await.is_err());
        phone.stop.store(true, Ordering::Relaxed);
        assert!(active
            .activate_element(
                labels()
                    .label(TikTokControl::ProfileTab)
                    .unwrap()
                    .to_query()
            )
            .await
            .is_err());
        assert_eq!(phone.taps.load(Ordering::Relaxed), 0);
        phone.assert_no_public();
    }
    #[tokio::test(start_paused = true)]
    async fn nonempty_changed_hint_does_not_verify_cleanup() {
        let phone = Phone {
            nonempty_hint_after_clear: true,
            ..Default::default()
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        let proof = binding(&phone, deadline).await;
        let result = prepare_root_before_send(
            &phone,
            labels(),
            (1080.0, 2400.0),
            proof,
            "fixture draft",
            &phone.stop,
            deadline,
            String::new,
        )
        .await
        .unwrap();
        assert_eq!(result.cleanup, DraftCleanup::FailedReadback);
        phone.assert_no_public();
    }
    #[tokio::test(start_paused = true)]
    async fn invalid_text_blocks_proof_before_any_navigation() {
        for text in ["", "line\nfeed", "carriage\rreturn", "tab\ttext", "\u{7f}"] {
            let phone = Phone::default();
            let failure = prove_rehearsal_binding(
                &phone,
                labels(),
                "fixture.actor",
                &target(),
                text,
                &phone.stop,
                Instant::now() + Duration::from_secs(30),
            )
            .await
            .err()
            .unwrap();
            assert_eq!(failure.blocker, PrepareBlocker::InvalidInput);
            assert_eq!(phone.taps.load(Ordering::Relaxed), 0);
            phone.assert_no_public();
        }
        assert!(validate_rehearsal_text(&"x".repeat(151)).is_err());
    }

    fn dispatch_tree(targets: &str, generation: u64) -> Tree {
        Tree::parse(crate::HierarchySourceSnapshot {
            generation,
            xml: format!("<hierarchy>{targets}</hierarchy>"),
        })
        .unwrap()
    }
    #[test]
    fn measured_global_profile_excludes_only_exact_lower_background_branch() {
        let labels = crate::tiktok_labels::controls_for("com.zhiliaoapp.musically", "en", "45.7.3").unwrap();
        let query = labels.label(TikTokControl::ProfileTab).unwrap().to_query();
        let xml = include_str!("../../fixtures/tiktok-publish/musically-45.7.3-en/profile-background-order.xml");
        let tree = |xml: String| Tree::parse(crate::HierarchySourceSnapshot { generation: 1, xml }).unwrap();
        assert!(intended_target(&tree(xml.into()), labels, query).is_ok());
        let home_xml = xml.replace("id/nrb", "id/nr_").replace("content-desc=\"Profile\"", "content-desc=\"Home\"").replace("[864,1965][1080,2094]", "[0,1965][216,2094]");
        let home = labels.label(TikTokControl::HomeTab).unwrap().to_query();
        assert!(intended_target(&tree(home_xml.clone()), labels, home).is_ok());
        assert!(intended_target(&tree(home_xml.replace("drawing-order=\"1\"", "drawing-order=\"5\"")), labels, home).is_err());
        for changed in [
            xml.replace("drawing-order=\"1\"", "drawing-order=\"5\""),
            xml.replace("drawing-order=\"4\"", "drawing-order=\"2\""),
            xml.replace("id/hpk\"", "id/hpk\" content-desc=\"Send\""),
            xml.replace("id/nrh\"", "id/wrong_branch\""),
            xml.replace("id/hpk\" displayed=\"true\" clickable=\"true\" enabled=\"true\"", "id/hpk\" displayed=\"true\" clickable=\"true\" enabled=\"false\""),
        ] {
            assert!(intended_target(&tree(changed), labels, query).is_err());
        }
        for id in ["hpk", "nrh", "nrm"] {
            let duplicate = format!(r#"<node package="com.zhiliaoapp.musically" resource-id="com.zhiliaoapp.musically:id/{id}"/>"#);
            assert!(intended_target(&tree(xml.replace("</hierarchy>", &format!("{duplicate}</hierarchy>"))), labels, query).is_err());
        }
        let unrelated = r#"<node package="com.zhiliaoapp.musically" resource-id="com.zhiliaoapp.musically:id/wr2"/>"#;
        assert!(intended_target(&tree(xml.replace("</hierarchy>", &format!("{unrelated}</hierarchy>"))), labels, query).is_ok(), "wr2 is unique within the measured parent, not the entire tree");
        let duplicate_branch = r#"<node package="com.zhiliaoapp.musically" resource-id="com.zhiliaoapp.musically:id/wr2"/>"#;
        assert!(intended_target(&tree(xml.replace("    <node package=\"com.zhiliaoapp.musically\" class=\"android.widget.LinearLayout\"", &format!("{duplicate_branch}    <node package=\"com.zhiliaoapp.musically\" class=\"android.widget.LinearLayout\""))), labels, query).is_err());
        let unmeasured = crate::tiktok_labels::controls_for("com.zhiliaoapp.musically", "en", "46.4.3").unwrap();
        assert!(intended_target(&tree(xml.into()), unmeasured, unmeasured.label(TikTokControl::ProfileTab).unwrap().to_query()).is_err());
        let overlay = r#"<node package="com.zhiliaoapp.musically" class="android.widget.Button" content-desc="Send" displayed="true" clickable="true" enabled="true" bounds="[864,1965][1080,2094]"/>"#;
        assert!(intended_target(&tree(xml.replace("</hierarchy>", &format!("{overlay}</hierarchy>"))), labels, query).is_err());
    }

    #[test]
    fn intended_role_must_be_unique_visible_and_disjoint_from_other_clickable_actions() {
        let query = ElementQuery::ClassName(crate::tiktok_drawer::EDIT_TEXT);
        let field = node("android.widget.EditText", "", "field", 100);
        assert!(intended_target(&dispatch_tree(&field, 1), labels(), query).is_ok());
        assert!(intended_target(
            &dispatch_tree(&(field.clone() + &field), 1),
            labels(),
            query
        )
        .is_err());
        assert!(intended_target(
            &dispatch_tree(
                &field.replace("displayed=\"true\"", "displayed=\"false\""),
                1
            ),
            labels(),
            query
        )
        .is_err());
        let overlapping_public = node("android.widget.Button", "Send", "send", 100);
        assert!(intended_target(
            &dispatch_tree(&(field + &overlapping_public), 1),
            labels(),
            query
        )
        .is_err());
    }
    #[tokio::test(start_paused = true)]
    async fn raw_coordinate_inside_allowed_rect_never_dispatches() {
        let phone = Phone::default();
        let epoch = phone.gui_session_epoch();
        let facade = NavigationReads {
            session: &phone,
            labels: labels(),
            epoch: &epoch,
            stop: &phone.stop,
            deadline: Instant::now() + Duration::from_secs(30),
        };
        assert!(facade.tap(rect(100.0, "Profile").centre()).await.is_err());
        assert_eq!(phone.taps.load(Ordering::Relaxed), 0);
        phone.assert_no_public();
    }
    #[test]
    fn composer_send_requires_explicit_visibility_actionability_and_geometry() {
        let phone = Phone::default();
        *phone.page.lock() = "drawer";
        let snapshot = phone.snapshot();
        let send = labels().label(TikTokControl::CommentSend).unwrap();
        let marker = format!(
            "{}\" displayed=\"true\" enabled=\"false\" clickable=\"true\"",
            send.value()
        );
        for replacement in [
            format!(
                "{}\" displayed=\"false\" enabled=\"false\" clickable=\"true\"",
                send.value()
            ),
            format!(
                "{}\" displayed=\"true\" enabled=\"false\" clickable=\"false\"",
                send.value()
            ),
        ] {
            let mut changed = snapshot.clone();
            changed.xml = changed.xml.replace(&marker, &replacement);
            assert!(composer_state(changed, labels()).is_err());
        }
        let mut changed = snapshot;
        changed.xml = changed.xml.replace(
            "bounds=\"[900,1000][1000,1100]\"",
            "bounds=\"[900,1000][900,1100]\"",
        );
        assert!(composer_state(changed, labels()).is_err());
        assert!(!root_hint("changed draft"));
    }
    #[tokio::test(start_paused = true)]
    async fn already_dispatched_navigation_drains_then_refuses_late_dispatch() {
        let phone = Phone {
            dispatch_delay: Duration::from_secs(2),
            ..Default::default()
        };
        let epoch = phone.gui_session_epoch();
        let facade = NavigationReads {
            session: &phone,
            labels: labels(),
            epoch: &epoch,
            stop: &phone.stop,
            deadline: Instant::now() + Duration::from_secs(1),
        };
        assert!(facade
            .activate_element(
                labels()
                    .label(TikTokControl::ProfileTab)
                    .unwrap()
                    .to_query()
            )
            .await
            .is_err());
        assert_eq!(
            phone.completed_taps.load(Ordering::Relaxed),
            1,
            "effect future drained, not timed out/dropped"
        );
        assert!(facade.tap(rect(100.0, "Profile").centre()).await.is_err());
        assert_eq!(phone.taps.load(Ordering::Relaxed), 1);
        phone.assert_no_public();
    }
}
