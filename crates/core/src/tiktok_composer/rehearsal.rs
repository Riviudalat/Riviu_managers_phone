//! Owned image-only NO-PUBLIC preparation through the production composer.
//! No campaign, submission timestamp, Post intent, verifier or Sheet is created.
use super::*;
use crate::ui_automation::tree::Tree;
use serde::Serialize;
use std::sync::{atomic::AtomicUsize, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PublishPrepareBlocker {
    InvalidInput,
    Unsupported,
    Cancelled,
    Deadline,
    StaleComposer,
    AccountUnproved,
    BindingChanged,
    BeforePostNotReached,
    Transport,
    CleanupUnproved,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OwnedComposerCleanup {
    NotCreated,
    ClearedAndVerified,
    NeedsAttention,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedPublishBeforePost {
    pub account: String,
    pub package: String,
    pub exact_caption: String,
    pub import_id: String,
    pub image_count: usize,
    pub sound: SoundSelectionEvidence,
    pub hierarchy_sha256: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishPreparationOutcome {
    pub prepared: Option<PreparedPublishBeforePost>,
    pub blocker: Option<PublishPrepareBlocker>,
    pub cleanup: OwnedComposerCleanup,
    pub cleanup_safe: bool,
}
impl PublishPreparationOutcome {
    fn refused(blocker: PublishPrepareBlocker) -> Self {
        Self {
            prepared: None,
            blocker: Some(blocker),
            cleanup: OwnedComposerCleanup::NotCreated,
            cleanup_safe: true,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("no-public publish safe-screen navigation needs attention")]
pub struct SafeScreenNeedsAttention;

/// A nonserializable token from fresh, positively observed idle feed + own account.
/// Held by the caller across media staging/import; cannot be constructed from IPC.
pub struct SafePublishScreen<'a> {
    session: &'a dyn UiSession,
    epoch: String,
    package: String,
    account: String,
    generation: u64,
    caption: String,
    image_count: usize,
    deadline: Instant,
}

fn check(
    session: &dyn UiSession,
    epoch: &str,
    stop: &AtomicBool,
    deadline: Instant,
) -> anyhow::Result<()> {
    anyhow::ensure!(!stop.load(Ordering::Relaxed), "no-public publish cancelled");
    anyhow::ensure!(Instant::now() < deadline, "no-public publish deadline");
    anyhow::ensure!(
        !epoch.is_empty() && session.gui_session_epoch() == epoch,
        "no-public publish binding changed"
    );
    Ok(())
}
fn validate(caption: &str, images: usize) -> anyhow::Result<()> {
    anyhow::ensure!(
        (1..=10).contains(&images)
            && !caption.trim().is_empty()
            && caption.chars().count() <= 2000
            && !caption.chars().any(|c| c.is_control()),
        "no-public publish invalid caption/media"
    );
    Ok(())
}
fn indices(tree: &Tree, labels: TikTokControls, query: ElementQuery<'_>) -> Vec<usize> {
    if let ElementQuery::Semantic(role) = query {
        crate::app_automation::tiktok_roles::indices(tree, labels.package(), role)
    } else {
        tree.matching(labels.package(), query)
    }
}
fn unique(
    tree: &Tree,
    labels: TikTokControls,
    query: ElementQuery<'_>,
    actionable: bool,
) -> anyhow::Result<ElementBox> {
    let found = indices(tree, labels, query);
    let [index] = found.as_slice() else {
        anyhow::bail!("no-public publish role ambiguous");
    };
    let node = &tree.nodes[*index];
    anyhow::ensure!(
        node.visibility() == Some(true)
            && tree.ancestors_visible(*index)
            && node.attr("enabled") == "true",
        "no-public publish role unproved"
    );
    if actionable {
        anyhow::ensure!(
            node.attr("clickable") == "true",
            "no-public publish role disabled"
        );
    }
    node.rect().context("no-public publish role geometry")
}
fn navigation_target(tree: &Tree, labels: TikTokControls, query: ElementQuery<'_>) -> anyhow::Result<ElementBox> {
    let target = unique(tree, labels, query, true)?;
    let found = indices(tree, labels, query);
    let [index] = found.as_slice() else { anyhow::bail!("navigation role ambiguous") };
    let target_index = *index;
    for (other_index, node) in tree.nodes.iter().enumerate() {
        if other_index == target_index || !node.visible(labels.package()) || !tree.ancestors_visible(other_index)
            || node.attr("clickable") != "true" { continue }
        if node.rect().is_some_and(|r| r.x < target.x + target.width && r.x + r.width > target.x
            && r.y < target.y + target.height && r.y + r.height > target.y) {
            let background = node.attr("class") == "android.widget.FrameLayout"
                && node.attr("resource-id") == "com.zhiliaoapp.musically:id/hpk"
                && node.attr("text").is_empty() && node.attr("content-desc").is_empty()
                && labels.package() == "com.zhiliaoapp.musically" && labels.resource_version() == Some("45.7.3");
            anyhow::ensure!(background, "no-public navigation actionable overlap");
        }
    }
    Ok(target)
}

fn feed_proof(
    snapshot: crate::HierarchySourceSnapshot,
    labels: TikTokControls,
    plan: ComposerPlan,
) -> anyhow::Result<u64> {
    let tree = Tree::parse(snapshot)?;
    unique(&tree, labels, plan.open, true)?;
    let home = labels
        .label(TikTokControl::HomeTab)
        .context("no-public publish Home unmeasured")?;
    let profile = labels
        .label(TikTokControl::ProfileTab)
        .context("no-public publish Profile unmeasured")?;
    unique(&tree, labels, home.to_query(), true)?;
    unique(&tree, labels, profile.to_query(), true)?;
    for query in [
        Some(plan.shutter),
        Some(plan.album_menu),
        plan.discard,
        plan.publish.map(|p| p.caption),
        plan.post_button(),
    ]
    .into_iter()
    .flatten()
    {
        anyhow::ensure!(
            indices(&tree, labels, query).is_empty(),
            "no-public publish stale composer"
        );
    }
    anyhow::ensure!(
        !tree.nodes.iter().any(|n| n.visible(labels.package())
            && ((n.attr("class") == crate::tiktok_drawer::EDIT_TEXT
                && crate::interaction_hierarchy::rehearsal::input_may_hold_draft(n))
                || [
                    "Uploading",
                    "Posting",
                    "Discard",
                    "Save draft",
                    "Continue editing",
                    "Đang tải lên",
                    "Đang đăng",
                    "Bỏ bản nháp"
                ]
                .iter()
                .any(|s| n.attr("text").contains(s) || n.attr("content-desc").contains(s)))),
        "no-public publish existing draft/upload"
    );
    Ok(tree.generation)
}

/// Refuses stale draft/upload before Back/Discard, account navigation or transfer.
pub async fn prove_safe_publish_screen<'a>(
    session: &'a dyn UiSession,
    labels: TikTokControls,
    plan: ComposerPlan,
    expected_account: &str,
    caption: &str,
    images: usize,
    stop: &AtomicBool,
    deadline: Instant,
) -> anyhow::Result<SafePublishScreen<'a>> {
    validate(caption, images)?;
    anyhow::ensure!(
        labels.language() == "en" && matches!((labels.package(), labels.resource_version()),
            ("com.ss.android.ugc.trill", Some("38.3.2")) | ("com.zhiliaoapp.musically", Some("45.7.3"))),
        "no-public publish first canary tuple unsupported"
    );
    anyhow::ensure!(
        session.supports_accessibility_readback()
            && plan.can_publish_carousel()
            && plan.package == labels.package(),
        "no-public publish unmeasured image route"
    );
    let epoch = session.gui_session_epoch();
    check(session, &epoch, stop, deadline)?;
    let mut guard = PrepareSession::new(session, labels, plan, &epoch, stop, deadline, false);
    guard.account_navigation = true;
    let first = guard.hierarchy_source_snapshot().await?;
    let first_generation = feed_proof(first, labels, plan)?;
    let second = guard.hierarchy_source_snapshot().await?;
    let generation = feed_proof(second, labels, plan)?;
    anyhow::ensure!(
        generation > first_generation,
        "no-public publish stale baseline"
    );
    // Reuse production account navigation while keeping deadline/public-point
    // checks. Profile/Home use exact unique actionable roles rather than rejecting
    // every background container. Any navigation failure retains ownership.
    let account = crate::tiktok_share::observe_publish_account(&guard, &labels).await
        .map_err(|error| error.context(SafeScreenNeedsAttention))?;
    if !account
        .trim_start_matches('@')
        .eq_ignore_ascii_case(expected_account.trim().trim_start_matches('@'))
        || expected_account.trim().is_empty()
    {
        return Err(anyhow::Error::new(SafeScreenNeedsAttention));
    }
    check(session, &epoch, stop, deadline).map_err(|e| e.context(SafeScreenNeedsAttention))?;
    let after = guard
        .hierarchy_source_snapshot()
        .await
        .map_err(|e| e.context(SafeScreenNeedsAttention))?;
    let generation =
        feed_proof(after, labels, plan).map_err(|e| e.context(SafeScreenNeedsAttention))?;
    if generation <= first_generation || guard.uncertain.load(Ordering::Relaxed) {
        return Err(anyhow::Error::new(SafeScreenNeedsAttention));
    }
    Ok(SafePublishScreen {
        session,
        epoch,
        package: labels.package().into(),
        account,
        generation,
        caption: caption.into(),
        image_count: images,
        deadline,
    })
}

#[derive(Debug, thiserror::Error)]
#[error("no-public publish prepared before Post; deny public boundary")]
struct StopBeforePost;

/// The caller's imported media receipt must already be verified and exclusive. This
/// stage uses the real picker/sound/caption/final-Post reproof and denies the callback.
#[allow(clippy::too_many_arguments)]
pub async fn prepare_images_before_post(
    session: &dyn UiSession,
    labels: TikTokControls,
    plan: ComposerPlan,
    sound_plan: SoundPickerPlan,
    sound_policy: &PublishSoundPolicy,
    screen: Screen,
    token: SafePublishScreen<'_>,
    import_id: &str,
    stop: &AtomicBool,
    deadline: Instant,
    phase: &(dyn Fn(&str, &serde_json::Value) -> anyhow::Result<()> + Send + Sync),
) -> PublishPreparationOutcome {
    if validate(&token.caption, token.image_count).is_err()
        || !import_id.starts_with("riviu-")
        || import_id.len() > 128
        || !import_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return PublishPreparationOutcome::refused(PublishPrepareBlocker::InvalidInput);
    }
    let deadline = deadline.min(token.deadline);
    if !std::ptr::eq(token.session, session)
        || token.package != labels.package()
        || check(session, &token.epoch, stop, deadline).is_err()
    {
        return PublishPreparationOutcome::refused(PublishPrepareBlocker::BindingChanged);
    }
    let mut guard = PrepareSession::new(session, labels, plan, &token.epoch, stop, deadline, false);
    guard.import_id = Some(import_id.to_owned());
    let baseline = match guard.hierarchy_source_snapshot().await {
        Ok(snapshot) => snapshot,
        Err(_) => return PublishPreparationOutcome::refused(PublishPrepareBlocker::Transport),
    };
    if feed_proof(baseline.clone(), labels, plan).is_err()
        || baseline.generation <= token.generation
    {
        return PublishPreparationOutcome::refused(PublishPrepareBlocker::StaleComposer);
    }
    if phase("composerStarting",&serde_json::json!({"importId":import_id,"images":token.image_count,"sessionEpoch":token.epoch})).is_err() {
        return PublishPreparationOutcome::refused(PublishPrepareBlocker::Transport);
    }
    let mut selected = None;
    let mut calls = 0;
    let request = CarouselRequest {
        album: import_id,
        images: token.image_count,
        caption: &token.caption,
        screen,
    };
    let result=crate::tiktok_sound::with_deadline_budget(stop,deadline,
        Box::pin(super::publish_selected_media_with_sound_effect_intent(&guard,plan,sound_plan,sound_policy,human_taps(screen),
            PickerSelection::carousel(&request),&token.caption,stop,|sound| {
                calls+=1; selected=Some(sound.clone());
                phase("preparedBeforePost",&serde_json::json!({"importId":import_id,"sound":sound,"publicPostDenied":true}))?;
                Err(StopBeforePost.into())
            },&|_|{},&|_|{},false))).await;
    let reached = calls == 1
        && result
            .as_ref()
            .err()
            .is_some_and(|error| error.is::<StopBeforePost>())
        && selected.as_ref().is_some_and(|s| s.confirmed);
    if !reached {
        let code = result.as_ref().err().map(|error| crate::publish_recovery::describe(error).code);
        let _ = phase("composerPreparationBlocked", &serde_json::json!({"failureCode":code,"boundaryCalls":calls,"publicPostDenied":true}));
    }
    let snapshot = guard.hierarchy_source_snapshot().await;
    let final_proof = if reached {
        snapshot
            .as_ref()
            .ok()
            .and_then(|snapshot| exact_caption(snapshot.clone(), labels, plan, &token.caption).ok())
    } else {
        None
    };
    let prepared = match (selected, final_proof) {
        (Some(sound), Some(hash)) if reached => Some(PreparedPublishBeforePost {
            account: token.account,
            package: token.package.clone(),
            exact_caption: token.caption.clone(),
            import_id: import_id.into(),
            image_count: token.image_count,
            sound,
            hierarchy_sha256: hash,
        }),
        _ => None,
    };
    let cleanup = if guard.mutations.load(Ordering::Relaxed) == 0 {
        OwnedComposerCleanup::NotCreated
    } else if prepared.is_some()
        && !guard.uncertain.load(Ordering::Relaxed)
        && session.gui_session_epoch() == token.epoch
    {
        cleanup_owned(session, labels, plan, &token.epoch, &token.caption, phase).await
    } else {
        OwnedComposerCleanup::NeedsAttention
    };
    let cleanup_safe = matches!(
        cleanup,
        OwnedComposerCleanup::NotCreated | OwnedComposerCleanup::ClearedAndVerified
    );
    let blocker = if stop.load(Ordering::Relaxed) {
        Some(PublishPrepareBlocker::Cancelled)
    } else if Instant::now() >= deadline {
        Some(PublishPrepareBlocker::Deadline)
    } else if prepared.is_none() {
        Some(PublishPrepareBlocker::BeforePostNotReached)
    } else if !cleanup_safe {
        Some(PublishPrepareBlocker::CleanupUnproved)
    } else {
        None
    };
    PublishPreparationOutcome {
        prepared,
        blocker,
        cleanup,
        cleanup_safe,
    }
}
fn exact_caption(
    snapshot: crate::HierarchySourceSnapshot,
    labels: TikTokControls,
    plan: ComposerPlan,
    caption: &str,
) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    let hash = format!("{:x}", Sha256::digest(snapshot.xml.as_bytes()));
    let tree = Tree::parse(snapshot)?;
    let tail = plan.publish.context("no-public publish tail missing")?;
    let found = indices(&tree, labels, tail.caption);
    let [index] = found.as_slice() else {
        anyhow::bail!("no-public caption ambiguous");
    };
    let node = &tree.nodes[*index];
    anyhow::ensure!(
        node.visibility() == Some(true)
            && node.attr("text") == caption
            && node.attr("showing-hint") == "false",
        "no-public exact caption unproved"
    );
    unique(&tree, labels, tail.post_button, true)?;
    Ok(hash)
}
async fn cleanup_owned(
    session: &dyn UiSession,
    labels: TikTokControls,
    plan: ComposerPlan,
    epoch: &str,
    caption: &str,
    phase: &(dyn Fn(&str, &serde_json::Value) -> anyhow::Result<()> + Send + Sync),
) -> OwnedComposerCleanup {
    let stop = AtomicBool::new(false);
    let deadline = Instant::now() + Duration::from_secs(15);
    let guard = PrepareSession::new(session, labels, plan, epoch, &stop, deadline, true);
    let before = match guard.hierarchy_source_snapshot().await {
        Ok(s) => s,
        Err(_) => return OwnedComposerCleanup::NeedsAttention,
    };
    if exact_caption(before.clone(), labels, plan, caption).is_err()
        || phase("cleanupComposer", &serde_json::json!({"owned":true})).is_err()
    {
        return OwnedComposerCleanup::NeedsAttention;
    }
    let composer = Composer::new(&guard, plan, |r: &ElementBox| r.centre());
    if !composer.leave().await || guard.uncertain.load(Ordering::Relaxed) {
        return OwnedComposerCleanup::NeedsAttention;
    }
    let after = match guard.hierarchy_source_snapshot().await {
        Ok(s) => s,
        Err(_) => return OwnedComposerCleanup::NeedsAttention,
    };
    if after.generation <= before.generation || feed_proof(after, labels, plan).is_err() {
        return OwnedComposerCleanup::NeedsAttention;
    }
    OwnedComposerCleanup::ClearedAndVerified
}

/// Private forwarding facade: deadline/cancel checked before every dispatched
/// primitive; already-started effects drain. Public Post/Send/Like/Share roles are
/// never activatable, and a failed effect ACK conservatively invalidates cleanup.
struct PrepareSession<'a> {
    session: &'a dyn UiSession,
    labels: TikTokControls,
    plan: ComposerPlan,
    epoch: &'a str,
    stop: &'a AtomicBool,
    deadline: Instant,
    cleanup: bool,
    account_navigation: bool,
    import_id: Option<String>,
    mutations: AtomicUsize,
    uncertain: AtomicBool,
    last_snapshot: Mutex<Option<crate::HierarchySourceSnapshot>>,
}
impl<'a> PrepareSession<'a> {
    fn new(
        session: &'a dyn UiSession,
        labels: TikTokControls,
        plan: ComposerPlan,
        epoch: &'a str,
        stop: &'a AtomicBool,
        deadline: Instant,
        cleanup: bool,
    ) -> Self {
        Self {
            session,
            labels,
            plan,
            epoch,
            stop,
            deadline,
            cleanup,
            account_navigation: false,
            import_id: None,
            mutations: AtomicUsize::new(0),
            uncertain: AtomicBool::new(false),
            last_snapshot: Mutex::new(None),
        }
    }
    fn check(&self) -> anyhow::Result<()> {
        check(self.session, self.epoch, self.stop, self.deadline)
    }
    async fn read<T>(
        &self,
        future: impl std::future::Future<Output = anyhow::Result<T>>,
    ) -> anyhow::Result<T> {
        self.check()?;
        let result =
            crate::ui_automation::runtime::read_before_deadline(future, self.deadline, self.stop)
                .await?;
        self.check()?;
        match result {
            crate::ui_automation::runtime::ReadWaitResult::Ready(v) => Ok(v),
            _ => anyhow::bail!("no-public publish read interrupted"),
        }
    }
    async fn dispatch(
        &self,
        future: impl std::future::Future<Output = anyhow::Result<()>>,
    ) -> anyhow::Result<()> {
        self.check()?;
        self.mutations.fetch_add(1, Ordering::Relaxed);
        let result = future.await;
        if result.is_err() {
            self.uncertain.store(true, Ordering::Relaxed);
        }
        self.check()?;
        result
    }
    fn sound_navigation_ids(&self) -> &'static [&'static str] {
        match (self.labels.package(), self.labels.resource_version()) {
            ("com.ss.android.ugc.trill", Some("38.3.2")) => &[":id/c_4", ":id/ta8", ":id/title", ":id/aun"],
            ("com.zhiliaoapp.musically", Some("45.7.3")) => &[":id/dou", ":id/tv_top_text", ":id/wrv", ":id/vertical_item_music_new_rl", ":id/title", ":id/bix"],
            _ => &[],
        }
    }
    async fn safe_point(&self, point: &crate::TapPoint) -> anyhow::Result<()> {
        self.check()?;
        anyhow::ensure!(
            self.read(self.session.active_app_bundle()).await? == self.labels.package(),
            "no-public publish foreground changed"
        );
        let tree = Tree::parse(self.hierarchy_source_snapshot().await?)?;
        let forbidden = [
            self.plan.post_button(),
            self.labels
                .label(TikTokControl::CommentSend)
                .map(|l| l.to_query()),
            self.labels
                .label(TikTokControl::Share)
                .map(|l| l.to_query()),
            self.labels
                .label(TikTokControl::Follow)
                .map(|l| l.to_query()),
            self.labels.label(TikTokControl::Like).map(|l| l.to_query()),
            self.labels
                .label(TikTokControl::Bookmark)
                .map(|l| l.to_query()),
        ];
        for query in forbidden.into_iter().flatten() {
            for index in indices(&tree, self.labels, query) {
                if let Some(r) = tree.nodes[index].rect() {
                    anyhow::ensure!(
                        !(point.x >= r.x
                            && point.x < r.x + r.width
                            && point.y >= r.y
                            && point.y < r.y + r.height),
                        "no-public publish public-action tap denied"
                    );
                }
            }
        }
        if self.account_navigation && !self.cleanup {
            for role in [TikTokControl::ProfileTab, TikTokControl::HomeTab] {
                if let Some(label) = self.labels.label(role) {
                    if let Ok(target) = navigation_target(&tree, self.labels, label.to_query()) {
                        if point.x == target.centre().x && point.y == target.centre().y {
                            return Ok(());
                        }
                    }
                }
            }
        }
        let containing: Vec<_> = tree
            .nodes
            .iter()
            .enumerate()
            .filter(|(index, node)| {
                node.visible(self.labels.package())
                    && tree.ancestors_visible(*index)
                    && node.attr("clickable") == "true"
                    && node.rect().is_some_and(|r| {
                        point.x >= r.x
                            && point.x < r.x + r.width
                            && point.y >= r.y
                            && point.y < r.y + r.height
                    })
            })
            .collect();
        anyhow::ensure!(
            containing.len() == 1,
            "no-public publish tap target missing/overlapped"
        );
        let index = containing[0].0;
        let node = containing[0].1;
        let mut allowed_queries = vec![
            self.plan.open,
            self.plan.album_menu,
            self.plan.tabs,
            self.plan.multi_select,
            self.plan.picker_next,
        ];
        allowed_queries.extend(self.plan.gallery_entry);
        allowed_queries.extend(self.plan.publish.map(|tail| tail.edit_next));
        allowed_queries.extend(self.plan.publish.map(|tail| tail.caption));
        allowed_queries.extend(self.plan.discard);
        if self.account_navigation {
            allowed_queries.extend(
                [TikTokControl::ProfileTab, TikTokControl::HomeTab]
                    .into_iter()
                    .filter_map(|control| self.labels.label(control).map(|l| l.to_query())),
            );
        }
        // Keep sound navigation exact-version keyed; never borrow resource IDs from
        // another package. These agree with the production measured SoundPickerPlan.
        let sound_ids = self.sound_navigation_ids();
        allowed_queries.extend(sound_ids.iter().map(|id| ElementQuery::ResourceIdSuffix(id)));
        let measured_role = allowed_queries.into_iter().any(|query| {
            let matches = indices(&tree, self.labels, query);
            matches.as_slice() == [index]
        });
        let album_row = self.import_id.as_deref() == Some(node.attr("text"))
            && !indices(&tree, self.labels, self.plan.album_menu).is_empty();
        let ordinal = !indices(&tree, self.labels, self.plan.album_menu).is_empty()
            && self.plan.selection_controls.is_some_and(|controls| {
                indices(&tree, self.labels, controls.selector).contains(&index)
            });
        anyhow::ensure!(
            measured_role || album_row || ordinal,
            "no-public publish unknown intended role refused"
        );
        if self.cleanup {
            let discard = self
                .plan
                .discard
                .context("no-public cleanup Discard unmeasured")?;
            let r = unique(&tree, self.labels, discard, true)?;
            anyhow::ensure!(
                point.x >= r.x
                    && point.x < r.x + r.width
                    && point.y >= r.y
                    && point.y < r.y + r.height,
                "no-public cleanup only owned Discard"
            );
        }
        self.check()
    }
}
#[async_trait::async_trait]
impl UiSession for PrepareSession<'_> {
    async fn tap(&self, point: crate::TapPoint) -> anyhow::Result<()> {
        self.safe_point(&point).await?;
        let before = self
            .last_snapshot
            .lock()
            .unwrap()
            .clone()
            .context("no-public tap baseline missing")?;
        self.safe_point(&point).await?;
        let after = self
            .last_snapshot
            .lock()
            .unwrap()
            .clone()
            .context("no-public tap reproof missing")?;
        let stable_navigation = if self.account_navigation && !self.cleanup {
            let first = Tree::parse(before.clone())?;
            let second = Tree::parse(after.clone())?;
            [TikTokControl::ProfileTab, TikTokControl::HomeTab].into_iter().any(|role| {
                self.labels.label(role).is_some_and(|label| {
                    let a = navigation_target(&first, self.labels, label.to_query());
                    let b = navigation_target(&second, self.labels, label.to_query());
                    match (a, b) {
                        (Ok(a), Ok(b)) => a == b && a.centre().x == point.x && a.centre().y == point.y,
                        _ => false,
                    }
                })
            })
        } else { false };
        anyhow::ensure!(
            after.generation > before.generation && (before.xml == after.xml || stable_navigation),
            "no-public publish layout changed before tap"
        );
        self.dispatch(self.session.tap(point)).await
    }
    async fn activate_element(&self, query: ElementQuery<'_>) -> anyhow::Result<()> {
        let tree = Tree::parse(self.hierarchy_source_snapshot().await?)?;
        let r = unique(&tree, self.labels, query, true)?;
        self.tap(r.centre()).await
    }
    async fn swipe(&self, gesture: crate::SwipeGesture) -> anyhow::Result<()> {
        anyhow::ensure!(!self.cleanup, "no-public cleanup swipe denied");
        let tree = Tree::parse(self.hierarchy_source_snapshot().await?)?;
        anyhow::ensure!(
            !indices(&tree, self.labels, self.plan.album_menu).is_empty(),
            "no-public swipe requires owned picker"
        );
        self.dispatch(self.session.swipe(gesture)).await
    }
    async fn type_text(&self, text: &str) -> anyhow::Result<()> {
        anyhow::ensure!(!self.cleanup, "no-public cleanup typing denied");
        self.check()?;
        anyhow::ensure!(
            self.read(self.session.active_app_bundle()).await? == self.labels.package(),
            "no-public text foreground changed"
        );
        let tree = Tree::parse(self.hierarchy_source_snapshot().await?)?;
        let focused: Vec<_> = tree
            .nodes
            .iter()
            .enumerate()
            .filter(|(index, n)| {
                n.visible(self.labels.package())
                    && tree.ancestors_visible(*index)
                    && n.attr("class") == crate::tiktok_drawer::EDIT_TEXT
                    && n.attr("focused") == "true"
                    && n.attr("enabled") == "true"
            })
            .collect();
        let [(_, field)] = focused.as_slice() else {
            anyhow::bail!("no-public caption focus ambiguous");
        };
        let caption = self
            .plan
            .publish
            .context("no-public caption tuple missing")?
            .caption;
        let caption_indices = indices(&tree, self.labels, caption);
        anyhow::ensure!(
            caption_indices.len() == 1
                && std::ptr::eq(*field, &tree.nodes[caption_indices[0]])
                && (field.attr("showing-hint") == "true"
                    || field.attr("text").is_empty()
                    || field.attr("text") == text),
            "no-public exact caption focus/empty proof missing"
        );
        self.dispatch(self.session.type_text(text)).await
    }
    async fn type_keys(&self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("no-public publish raw keys denied")
    }
    async fn home(&self) -> anyhow::Result<()> {
        anyhow::bail!("no-public publish Home denied")
    }
    async fn back(&self) -> anyhow::Result<()> {
        self.check()?;
        anyhow::ensure!(
            self.read(self.session.active_app_bundle()).await? == self.labels.package(),
            "no-public Back foreground changed"
        );
        let tree = Tree::parse(self.hierarchy_source_snapshot().await?)?;
        let owned = [
            Some(self.plan.shutter),
            Some(self.plan.album_menu),
            self.plan.edit_step_marker,
            self.plan.publish.map(|p| p.caption),
            self.plan.discard,
            Some(ElementQuery::ResourceIdSuffix(":id/title")),
            Some(ElementQuery::ResourceIdSuffix(":id/mjf")),
        ]
        .into_iter()
        .flatten()
        .any(|query| !indices(&tree, self.labels, query).is_empty());
        anyhow::ensure!(owned, "no-public Back screen unproved");
        self.dispatch(self.session.back()).await
    }
    async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("no-public generic activation denied")
    }
    async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("no-public assert unsupported")
    }
    fn stream_url(&self) -> Option<String> {
        None
    }
    fn supports_element_bounds(&self) -> bool {
        self.session.supports_element_bounds()
    }
    fn supports_accessibility_readback(&self) -> bool {
        self.session.supports_accessibility_readback()
    }
    fn gui_session_epoch(&self) -> String {
        self.session.gui_session_epoch()
    }
    async fn active_app_bundle(&self) -> anyhow::Result<String> {
        self.read(self.session.active_app_bundle()).await
    }
    async fn window_size(&self) -> anyhow::Result<(f64, f64)> {
        self.read(self.session.window_size()).await
    }
    async fn keyboard_shown(&self) -> anyhow::Result<bool> {
        self.read(self.session.keyboard_shown()).await
    }
    async fn locate(&self, q: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
        self.read(self.session.locate(q)).await
    }
    async fn locate_all(&self, q: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
        self.read(self.session.locate_all(q)).await
    }
    async fn locate_all_described(&self, q: ElementQuery<'_>) -> anyhow::Result<Vec<ElementBox>> {
        self.read(self.session.locate_all_described(q)).await
    }
    async fn hierarchy_source_snapshot(&self) -> anyhow::Result<crate::HierarchySourceSnapshot> {
        let snapshot = self.read(self.session.hierarchy_source_snapshot()).await?;
        *self.last_snapshot.lock().unwrap() = Some(snapshot.clone());
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex as Lock;
    use std::sync::atomic::AtomicU64;
    const PACKAGE: &str = "com.ss.android.ugc.trill";
    fn labels() -> TikTokControls {
        crate::tiktok_labels::controls_for(PACKAGE, "en", "38.3.2").unwrap()
    }
    fn plan() -> ComposerPlan {
        ComposerPlan::resolve(&labels()).unwrap()
    }
    fn element(query: ElementQuery<'_>, y: u32, text: &str) -> String {
        let (attribute, value) = match query {
            ElementQuery::Description { value, .. } => ("content-desc", value.to_owned()),
            ElementQuery::Text { value, .. } => ("text", value.to_owned()),
            ElementQuery::ResourceIdSuffix(value) => ("resource-id", format!("{PACKAGE}{value}")),
            _ => panic!("measured test query"),
        };
        format!(
            r#"<node package="{PACKAGE}" class="android.widget.Button" {attribute}="{value}" text="{text}" displayed="true" enabled="true" clickable="true" bounds="[100,{y}][300,{}]"/>"#,
            y + 60
        )
    }
    #[test]
    fn rehearsal_sound_navigation_is_exact_version_keyed() {
        let phone = Phone::new(String::new());
        let stop = AtomicBool::new(false);
        let deadline = Instant::now() + Duration::from_secs(10);
        let global = crate::tiktok_labels::controls_for("com.zhiliaoapp.musically", "en", "45.7.3").unwrap();
        let global_plan = ComposerPlan::resolve(&global).unwrap();
        let guard = PrepareSession::new(&phone, global, global_plan, "fixture-epoch", &stop, deadline, false);
        assert!(guard.sound_navigation_ids().contains(&":id/dou"));
        assert!(!guard.sound_navigation_ids().contains(&":id/c_4"));
        let trill = PrepareSession::new(&phone, labels(), plan(), "fixture-epoch", &stop, deadline, false);
        assert!(trill.sound_navigation_ids().contains(&":id/c_4"));
        assert!(!trill.sound_navigation_ids().contains(&":id/dou"));
    }

    fn feed_xml() -> String {
        let p = plan();
        let mut xml = String::from("<hierarchy>");
        for (query, y) in [
            (p.open, 100),
            (
                labels().label(TikTokControl::HomeTab).unwrap().to_query(),
                200,
            ),
            (
                labels()
                    .label(TikTokControl::ProfileTab)
                    .unwrap()
                    .to_query(),
                300,
            ),
        ] {
            xml += &element(query, y, "");
        }
        xml + "</hierarchy>"
    }
    struct Phone {
        xml: Lock<String>,
        generation: AtomicU64,
        taps: AtomicUsize,
        backs: AtomicUsize,
        post: AtomicUsize,
        stop: AtomicBool,
        lose_ack: bool,
        delay: Duration,
        mutate_layout: bool,
        account_journey: bool,
    }
    impl Phone {
        fn new(xml: String) -> Self {
            Self {
                xml: Lock::new(xml),
                generation: AtomicU64::new(0),
                taps: AtomicUsize::new(0),
                backs: AtomicUsize::new(0),
                post: AtomicUsize::new(0),
                stop: AtomicBool::new(false),
                lose_ack: false,
                delay: Duration::ZERO,
                mutate_layout: false,
                account_journey: false,
            }
        }
    }
    #[async_trait::async_trait]
    impl UiSession for Phone {
        async fn tap(&self, point: crate::TapPoint) -> anyhow::Result<()> {
            if point.y >= 1900.0 {
                self.post.fetch_add(1, Ordering::Relaxed);
                panic!("PUBLIC POST TRAP");
            }
            self.taps.fetch_add(1, Ordering::Relaxed);
            if self.account_journey {
                match point.y as u32 {
                    330 => {
                        let mut xml = String::from("<hierarchy>");
                        xml += &element(labels().label(TikTokControl::HomeTab).unwrap().to_query(), 200, "");
                        xml += &format!(r#"<node package="{PACKAGE}" class="android.widget.TextView" resource-id="{PACKAGE}:id/dby" text="Edit profile" displayed="true" enabled="true" clickable="true" bounds="[100,500][300,560]"/>"#);
                        xml += &element(ElementQuery::ResourceIdSuffix(":id/mjf"), 600, "@fixture.actor");
                        *self.xml.lock() = xml + "</hierarchy>";
                    }
                    230 => *self.xml.lock() = feed_xml(),
                    _ => panic!("unexpected account navigation point"),
                }
            }
            tokio::time::sleep(self.delay).await;
            if self.lose_ack {
                anyhow::bail!("fixture lost ACK");
            }
            Ok(())
        }
        async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
            panic!("fixture swipe not expected")
        }
        async fn type_text(&self, _: &str) -> anyhow::Result<()> {
            panic!("fixture text not expected")
        }
        async fn home(&self) -> anyhow::Result<()> {
            panic!("fixture Home trap")
        }
        async fn back(&self) -> anyhow::Result<()> {
            self.backs.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
            panic!("fixture generic tap trap")
        }
        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
        fn gui_session_epoch(&self) -> String {
            "fixture-epoch".into()
        }
        fn supports_accessibility_readback(&self) -> bool {
            true
        }
        fn supports_element_bounds(&self) -> bool {
            true
        }
        async fn active_app_bundle(&self) -> anyhow::Result<String> {
            Ok(PACKAGE.into())
        }
        async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            let tree = Tree::parse(self.hierarchy_source_snapshot().await?)?;
            let found = tree.matching(PACKAGE, query);
            Ok(found.first().and_then(|index| tree.nodes[*index].rect()))
        }
        async fn hierarchy_source_snapshot(
            &self,
        ) -> anyhow::Result<crate::HierarchySourceSnapshot> {
            let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
            let mut xml = self.xml.lock().clone();
            if self.mutate_layout && generation > 1 {
                xml = xml.replace("[100,100][300,160]", "[100,120][300,180]");
            }
            Ok(crate::HierarchySourceSnapshot { generation, xml })
        }
    }
    #[tokio::test(start_paused = true)]
    async fn measured_profile_background_exception_is_preflight_only_and_exact_center() {
        let global = crate::tiktok_labels::controls_for("com.zhiliaoapp.musically", "en", "45.7.3").unwrap();
        let global_plan = ComposerPlan::resolve(&global).unwrap();
        let xml = include_str!("../../fixtures/tiktok-publish/musically-45.7.3-en/profile-background-order.xml");
        let phone = Phone::new(xml.into());
        // This helper validates a point; no underlying tap or public trap runs.
        struct GlobalPhone<'a>(&'a Phone);
        #[async_trait::async_trait]
        impl UiSession for GlobalPhone<'_> {
            async fn tap(&self, _: crate::TapPoint) -> anyhow::Result<()> { panic!("point proof must not dispatch") }
            async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> { panic!("no swipe") }
            async fn type_text(&self, _: &str) -> anyhow::Result<()> { panic!("no type") }
            async fn home(&self) -> anyhow::Result<()> { panic!("no Home") }
            async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> { panic!("no find tap") }
            async fn assert_visible(&self, _: &str) -> anyhow::Result<()> { Ok(()) }
            fn stream_url(&self) -> Option<String> { None }
            fn gui_session_epoch(&self) -> String { "fixture-epoch".into() }
            async fn active_app_bundle(&self) -> anyhow::Result<String> { Ok("com.zhiliaoapp.musically".into()) }
            async fn hierarchy_source_snapshot(&self) -> anyhow::Result<crate::HierarchySourceSnapshot> { self.0.hierarchy_source_snapshot().await }
        }
        let phone = GlobalPhone(&phone);
        let stop = AtomicBool::new(false);
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut guard = PrepareSession::new(&phone, global, global_plan, "fixture-epoch", &stop, deadline, false);
        let center = crate::TapPoint { x: 972.0, y: 2029.5 };
        assert!(guard.safe_point(&center).await.is_err());
        guard.account_navigation = true;
        guard.safe_point(&center).await.unwrap();
        assert!(guard.safe_point(&crate::TapPoint { x: 973.0, y: 2029.5 }).await.is_err());
        guard.cleanup = true;
        assert!(guard.safe_point(&center).await.is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn publish_preflight_reads_account_and_returns_home_without_recovery_actions() {
        let mut phone = Phone::new(feed_xml());
        phone.account_journey = true;
        let token = prove_safe_publish_screen(&phone, labels(), plan(), "fixture.actor", "fixture caption", 2,
            &phone.stop, Instant::now() + Duration::from_secs(30)).await.unwrap();
        assert_eq!(token.account, "fixture.actor");
        assert_eq!(phone.taps.load(Ordering::Relaxed), 2);
        assert_eq!(phone.backs.load(Ordering::Relaxed), 0);
        assert_eq!(phone.post.load(Ordering::Relaxed), 0);
        assert_eq!(&*phone.xml.lock(), &feed_xml());
    }

    #[test]
    fn stale_composer_is_refused_even_when_feed_tabs_are_visible() {
        let mut xml = feed_xml();
        xml = xml.replace(
            "</hierarchy>",
            &format!(
                "{}</hierarchy>",
                element(plan().post_button().unwrap(), 1900, "Post")
            ),
        );
        assert!(feed_proof(
            crate::HierarchySourceSnapshot { generation: 1, xml },
            labels(),
            plan()
        )
        .is_err());
    }
    #[test]
    fn publish_feed_proof_distinguishes_empty_unfocused_widget_from_draft() {
        let widget = format!(r#"<node package="{PACKAGE}" class="android.widget.EditText" text="" focused="false" displayed="true" bounds="[501,603][579,677]"/>"#);
        let xml = feed_xml().replace("</hierarchy>", &format!("{widget}</hierarchy>"));
        assert!(feed_proof(crate::HierarchySourceSnapshot { generation: 1, xml: xml.clone() }, labels(), plan()).is_ok());
        assert!(feed_proof(crate::HierarchySourceSnapshot { generation: 2, xml: xml.replace("focused=\"false\"", "focused=\"true\"") }, labels(), plan()).is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn stale_draft_refuses_before_account_navigation_or_discard() {
        let xml=feed_xml().replace("</hierarchy>",&format!(r#"<node package="{PACKAGE}" class="android.widget.EditText" text="foreign" displayed="true" enabled="true" bounds="[100,1000][800,1100]"/></hierarchy>"#));
        let phone = Phone::new(xml);
        assert!(prove_safe_publish_screen(
            &phone,
            labels(),
            plan(),
            "fixture.actor",
            "fixture caption",
            1,
            &phone.stop,
            Instant::now() + Duration::from_secs(10)
        )
        .await
        .is_err());
        assert_eq!(phone.taps.load(Ordering::Relaxed), 0);
        assert_eq!(phone.backs.load(Ordering::Relaxed), 0);
    }
    #[tokio::test(start_paused = true)]
    async fn public_post_and_send_points_are_rejected_before_dispatch() {
        let xml = format!(
            "<hierarchy>{}</hierarchy>",
            element(plan().post_button().unwrap(), 1900, "Post")
        );
        let phone = Phone::new(xml);
        let epoch = phone.gui_session_epoch();
        let guard = PrepareSession::new(
            &phone,
            labels(),
            plan(),
            &epoch,
            &phone.stop,
            Instant::now() + Duration::from_secs(10),
            false,
        );
        assert!(guard
            .tap(crate::TapPoint {
                x: 200.0,
                y: 1930.0
            })
            .await
            .is_err());
        assert_eq!(phone.post.load(Ordering::Relaxed), 0);
        assert_eq!(phone.taps.load(Ordering::Relaxed), 0);
    }
    #[tokio::test(start_paused = true)]
    async fn lost_ack_sets_uncertainty_and_never_claims_cleaned() {
        let mut phone = Phone::new(feed_xml());
        phone.lose_ack = true;
        let epoch = phone.gui_session_epoch();
        let guard = PrepareSession::new(
            &phone,
            labels(),
            plan(),
            &epoch,
            &phone.stop,
            Instant::now() + Duration::from_secs(10),
            false,
        );
        assert!(guard
            .tap(crate::TapPoint { x: 200.0, y: 130.0 })
            .await
            .is_err());
        assert!(guard.uncertain.load(Ordering::Relaxed));
        assert_eq!(guard.mutations.load(Ordering::Relaxed), 1);
    }
    #[tokio::test(start_paused = true)]
    async fn cancel_before_dispatch_and_drain_started_effect() {
        let mut phone = Phone::new(feed_xml());
        phone.delay = Duration::from_secs(2);
        let epoch = phone.gui_session_epoch();
        let guard = PrepareSession::new(
            &phone,
            labels(),
            plan(),
            &epoch,
            &phone.stop,
            Instant::now() + Duration::from_secs(1),
            false,
        );
        assert!(guard
            .tap(crate::TapPoint { x: 200.0, y: 130.0 })
            .await
            .is_err());
        assert_eq!(phone.taps.load(Ordering::Relaxed), 1);
        phone.stop.store(true, Ordering::Relaxed);
        assert!(guard
            .tap(crate::TapPoint { x: 200.0, y: 130.0 })
            .await
            .is_err());
        assert_eq!(phone.taps.load(Ordering::Relaxed), 1);
    }
    #[tokio::test(start_paused = true)]
    async fn unknown_public_node_and_changed_picker_geometry_never_authorize_tap() {
        let unknown = format!(
            r#"<hierarchy><node package="{PACKAGE}" class="android.widget.Button" text="Publish now" displayed="true" enabled="true" clickable="true" bounds="[100,100][300,160]"/></hierarchy>"#
        );
        let phone = Phone::new(unknown);
        let epoch = phone.gui_session_epoch();
        let guard = PrepareSession::new(
            &phone,
            labels(),
            plan(),
            &epoch,
            &phone.stop,
            Instant::now() + Duration::from_secs(10),
            false,
        );
        assert!(guard
            .tap(crate::TapPoint { x: 200.0, y: 130.0 })
            .await
            .is_err());
        assert_eq!(phone.taps.load(Ordering::Relaxed), 0);
        let mut phone = Phone::new(feed_xml());
        phone.mutate_layout = true;
        let epoch = phone.gui_session_epoch();
        let guard = PrepareSession::new(
            &phone,
            labels(),
            plan(),
            &epoch,
            &phone.stop,
            Instant::now() + Duration::from_secs(10),
            false,
        );
        assert!(guard
            .tap(crate::TapPoint { x: 200.0, y: 130.0 })
            .await
            .is_err());
        assert_eq!(phone.taps.load(Ordering::Relaxed), 0);
    }
    #[tokio::test(start_paused = true)]
    async fn composer_phase_cannot_navigate_account_and_album_identity_is_exact() {
        let phone = Phone::new(feed_xml());
        let epoch = phone.gui_session_epoch();
        let guard = PrepareSession::new(
            &phone,
            labels(),
            plan(),
            &epoch,
            &phone.stop,
            Instant::now() + Duration::from_secs(10),
            false,
        );
        assert!(guard
            .tap(crate::TapPoint { x: 200.0, y: 330.0 })
            .await
            .is_err());
        assert_eq!(phone.taps.load(Ordering::Relaxed), 0);
        let xml = format!(
            "<hierarchy>{}{}</hierarchy>",
            element(plan().album_menu, 100, "All"),
            element(
                ElementQuery::Text {
                    value: "riviu-np-other",
                    exact: true
                },
                300,
                "riviu-np-other"
            )
        );
        let phone = Phone::new(xml);
        let epoch = phone.gui_session_epoch();
        let mut guard = PrepareSession::new(
            &phone,
            labels(),
            plan(),
            &epoch,
            &phone.stop,
            Instant::now() + Duration::from_secs(10),
            false,
        );
        guard.import_id = Some("riviu-np-approved".into());
        assert!(guard
            .tap(crate::TapPoint { x: 200.0, y: 330.0 })
            .await
            .is_err());
        assert_eq!(phone.taps.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn image_count_and_caption_are_validated_before_any_transfer() {
        assert!(validate("fixture", 0).is_err());
        assert!(validate("fixture\ncaption", 1).is_err());
        assert!(validate(&"x".repeat(2001), 1).is_err());
    }
}
