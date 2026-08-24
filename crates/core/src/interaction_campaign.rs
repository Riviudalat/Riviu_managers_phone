//! The interaction campaign engine: one star or chain of replies, driven to completion.
//!
//! Moved here from `apps/desktop/src-tauri/src/interaction_commands.rs`, where it was 1,712
//! lines with no test of its own while the other half of the same engine
//! (`interaction_hierarchy`) already lived in this crate at 48% test. Splitting one engine
//! across two crates is what let `PARENT_SCROLL_ATTEMPTS` sit at 10 on one side and 4 on the
//! other for three days after a commit that measured the correct value.
//!
//! Nothing here is Tauri-aware, and that is why the move was possible at all: the whole
//! region already took only `riviu-core` types. The two exceptions are gone —
//! `CommandError` was the desktop's IPC error and is now `DeviceControlError`, and the
//! comment composer reached into a Windows OCR module and now takes `FrameTextSource`.

use crate::discover_comment_identity;
use crate::interaction_hierarchy::PARENT_SCROLL_ATTEMPTS;
use crate::interaction_target::{SendFailure, SendOutcome, TargetDriver};
use crate::locate_parent_comment;
use crate::AppEvent;
use crate::CommentLocatorIdentity;
use crate::CommentOcrObservation;
use crate::DeviceControlError;
use crate::DeviceControlPlane;
use crate::DeviceWorkOwner;
use crate::InteractionSessionKind;
use crate::PreparedThreadMessage;
use crate::TapPoint;
use crate::ThreadCampaignRequest;
use crate::ThreadCampaignState;
use crate::ThreadMessageState;
use crate::ThreadPlan;
use anyhow::Context;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;
/// The package travels with the context because every caller needs it again: the
/// arrival check compares it against `active_app_bundle()`, and it is per device —
/// the iOS bundle and Android's two regional builds are three different ids.
pub struct InteractionDevice {
    pub context: crate::UiWithStreamContext,
    pub target_package: String,
}

/// A monotonic-enough stamp for "this campaign changed", in milliseconds.
///
/// The frontend uses it only to decide whether the copy it holds is stale, so it has to move
/// forward on every write and nothing more.
pub fn revision() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

/// Tell the frontend this campaign changed.
///
/// The event carries no state, only the id: it is an invalidation hint, and the UI answers it
/// by re-reading the campaign. That is deliberate — a payload would be a second source of
/// truth for something a cheap SQLite read already answers, and a dropped hint costs nothing
/// because the next one restores the whole picture.
///
/// It exists as a helper because the runner has to send one after **every** state a message
/// passes through. It used to send one only after a send reached a verdict, so a campaign
/// spent its entire preparation phase — the slow part, where the AI is writing — looking
/// motionless, and a campaign that failed before any send ever ran said nothing at all until
/// the final tally.
fn notify(events: &crate::EventBus, campaign_id: &str) {
    events.emit(AppEvent::InteractionUpdated {
        campaign_id: campaign_id.to_string(),
        revision: revision(),
    });
}

/// Whether this campaign is blocked for want of an AI key.
///
/// The gate used to be unconditional, so a manual campaign — one whose every comment the
/// operator had already typed — refused to run on a machine with no key configured, and it
/// refused *after* the campaign row had been written as Running, which reads as a campaign
/// that started and died rather than one that was never allowed to start.
///
/// [`ThreadCampaignRequest::needs_ai_evidence_frames`] is the same predicate the evidence
/// pass keys on, so "does this run touch the AI at all" has exactly one definition.
pub fn ai_key_missing(request: &ThreadCampaignRequest, api_key: &str) -> bool {
    request.needs_ai_evidence_frames() && api_key.trim().is_empty()
}

/// Which assignments a retry may re-send.
///
/// Excluding `Succeeded` is the whole point: tapping Send is not idempotent, so
/// a retry that re-ran a delivered message would post the comment twice.
/// `Uncertain` is excluded for the same reason with less information — delivery
/// is exactly what is unproven there. `Sending` is still in flight.
///
/// `requested` narrows further when the caller names specific assignments;
/// `None` means "everything still retryable".
pub fn retryable_assignments(
    assignments: &[crate::InteractionAssignmentRecord],
    requested: Option<&std::collections::HashSet<String>>,
) -> std::collections::HashSet<String> {
    assignments
        .iter()
        .filter(|assignment| {
            !matches!(
                assignment.state,
                ThreadMessageState::Sending
                    | ThreadMessageState::Succeeded
                    | ThreadMessageState::Uncertain
            )
        })
        .filter(|assignment| requested.is_none_or(|ids| ids.contains(&assignment.id)))
        .map(|assignment| assignment.id.clone())
        .collect()
}

pub async fn open_interaction_context(
    control: &DeviceControlPlane,
    udid: &str,
) -> Result<InteractionDevice, DeviceControlError> {
    // Resolve before acquiring anything: a phone with no drivable TikTok build should
    // refuse without taking a lease or a capacity slot.
    let target_package = control.resolve_tiktok_package(udid).await?;
    let exclusive = control
        .try_acquire_exclusive(udid, DeviceWorkOwner::Interaction)
        .await?;
    let (exclusive, capacity) = control.reserve_ui_capacity(exclusive).await?;
    let kind = if control.requires_fresh_text_session(udid) {
        InteractionSessionKind::FreshText
    } else {
        InteractionSessionKind::Ordinary
    };
    let session = control
        .start_interaction_session(exclusive, &target_package, kind)
        .await?;
    let context = control.start_reserved_stream(session, capacity).await?;
    Ok(InteractionDevice {
        context,
        target_package,
    })
}

/// Pick the frame that can honestly stand as proof of a send, or nothing.
///
/// Two conditions, and both were missing. **Generation**: the caller's own stream
/// generation, so a frame cached by a producer that has since died cannot be published —
/// measured on this farm (`last_frame_age_ms=11373` with `baseline_sequence ==
/// latest_sequence`), the hub will happily hand back stale bytes and `FrameSource::latest`
/// promises nothing about liveness. **Watermark**: strictly newer than the frame that was
/// current before the Send tap, so what gets filed is the screen *after* the comment
/// exists rather than the screen that preceded it.
///
/// Pure and separated from the campaign so both can be pinned by tests without a device.
fn evidence_frame_after(
    frames: &dyn crate::GenerationFrameSource,
    udid: &str,
    generation: u64,
    watermark: Option<u64>,
) -> Option<crate::GenerationFrame> {
    frames
        .latest_in_generation(udid, generation)
        // `map_or(true, ..)` rather than `is_none_or`: this crate pins MSRV 1.77.2 and that
        // helper only stabilised in 1.82.
        // `is_none_or` rather than the `map_or(true, ..)` this carried in the desktop crate:
        // that spelling was there for `rust-version = "1.77.2"`, which `riviu-core` does not
        // declare and could not honour anyway -- five other files here already use the 1.82
        // helper.
        .filter(|frame| watermark.is_none_or(|mark| frame.sequence > mark))
}

/// Save the screen as it stands and return its stored path.
///
/// The campaign used to persist frame hashes without keeping a single frame, so
/// nothing it recorded could be checked afterwards — and `Uncertain`, the state
/// that most needs looking at, wrote no artifact at all. Publishing is
/// best-effort: a campaign must not fail because evidence could not be filed.
///
/// Takes the frame rather than a source, because the caller is the only place that knows
/// which frame is admissible — and because reading it *here* was the bug: this ran after
/// `close_ui_context`, which tears the stream down and removes the device's cached frame,
/// so `latest` returned `None` on every single call and every artefact row was filed with
/// a path of `NULL`.
fn publish_evidence_frame(
    artifacts: &crate::FlowArtifactStore,
    frame: Option<crate::GenerationFrame>,
    campaign_id: &str,
    assignment_id: &str,
    udid: &str,
) -> Option<(String, String)> {
    let Some(frame) = frame else {
        tracing::warn!(
            "interaction {udid}: không có frame nào hợp lệ sau khi gửi \
             (generation đã tiến, hoặc chưa có frame mới hơn mốc trước khi gửi) — \
             không lưu ảnh bằng chứng"
        );
        return None;
    };
    let frame = frame.bytes;
    let campaign = uuid::Uuid::parse_str(campaign_id).ok()?;
    let assignment = uuid::Uuid::parse_str(assignment_id).ok()?;
    let prepared = artifacts
        .prepare_image(
            campaign,
            assignment,
            uuid::Uuid::new_v4(),
            "comment-drawer.jpeg",
            "jpeg",
            &frame,
        )
        .map_err(|error| tracing::warn!("interaction: không chuẩn bị được ảnh bằng chứng: {error}"))
        .ok()?;
    match artifacts.publish_file(&prepared) {
        Ok(relative) => Some((relative.to_string_lossy().into_owned(), prepared.sha256)),
        Err(error) => {
            tracing::warn!("interaction: không lưu được ảnh bằng chứng: {error}");
            let _ = artifacts.rollback_file(&prepared);
            None
        }
    }
}

/// Photograph the target on its root actor, for the AI to write from.
///
/// Extracted so the failure is a value the caller can attribute to one target instead of
/// a `?` that ends the campaign. Every step in here is a step that can fail on a real
/// phone — open the context, choose a driver, prove arrival, get frames — and none of
/// them is a reason to abandon the targets that come after.
async fn collect_target_evidence_frames(
    control: &Arc<DeviceControlPlane>,
    engine: &crate::NurtureEngine,
    plan: &crate::ThreadPlan,
    target: &crate::ResolvedTikTokTarget,
    settle: Duration,
) -> anyhow::Result<Vec<Vec<u8>>> {
    let target_root_actor = plan
        .assignments
        .iter()
        .find(|assignment| assignment.target_key == target.target_key && assignment.ordinal == 0)
        .map(|assignment| assignment.actor_udid.as_str())
        .context("target root actor missing")?;
    let InteractionDevice {
        context,
        target_package,
    } = open_interaction_context(control, target_root_actor)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let session = control.streaming_session(&context)?;
    // Evidence for the AI has to come from the target, not from whatever survived the two
    // seconds this used to sleep.
    let evidence_gestures = tokio::sync::Mutex::new(());
    let frames = async {
        let evidence_driver = choose_target_driver(
            engine,
            target_root_actor,
            session.as_ref(),
            &target_package,
            &evidence_gestures,
        )
        .await?;
        let _proof = evidence_driver
            .open_target(session.as_ref(), target)
            .await?;
        drop(evidence_driver);
        let mut frames = Vec::with_capacity(3);
        for _ in 0..3 {
            if let Some(frame) = engine.frames.latest(target_root_actor) {
                frames.push((*frame).clone());
            }
            tokio::time::sleep(settle).await;
        }
        Ok::<_, anyhow::Error>(frames)
    }
    .await;
    // Closed on every path, including the failing ones: leaving the context open would
    // strand the lease and the stream for a target we are about to give up on.
    let closed = control.close_ui_context(context).await;
    let frames = frames?;
    closed?;
    if frames.is_empty() {
        anyhow::bail!("không có frame stream");
    }
    Ok(frames)
}

/// Mark every assignment of one target failed with the same reason.
///
/// The alternative — which is what happened — is leaving them in `queued`, where they read
/// as "not started yet" for ever. `queued` is also the one state a retry treats as
/// retryable, so silent `queued` rows are a re-send waiting to happen.
///
/// **`protected` is load-bearing, and its absence was a real defect.** The first version of
/// this function stamped `Failed` over *every* assignment of the target, including ones that
/// had already reached `Succeeded`. `Failed` is retryable, so on a retry whose evidence pass
/// failed it erased the record of a comment that was already public and made the next retry
/// re-send it — reintroducing exactly the duplicate-post hazard the `only_assignments` guard
/// in the preparation loop had just closed. Caught by an adversarial re-read of the claim
/// "a retry cannot overwrite a Succeeded state", which was true of the loop and false here.
fn fail_whole_target(
    db: &Arc<crate::db::Database>,
    plan: &crate::ThreadPlan,
    assignment_ids: &HashMap<(String, u8), String>,
    protected: &std::collections::HashSet<String>,
    target: &crate::ResolvedTikTokTarget,
    error_code: &str,
) -> anyhow::Result<()> {
    for assignment in plan
        .assignments
        .iter()
        .filter(|assignment| assignment.target_key == target.target_key)
    {
        let Some(id) = assignment_ids.get(&(assignment.target_key.clone(), assignment.ordinal))
        else {
            continue;
        };
        if protected.contains(id) {
            // Whatever went wrong with this target, it did not un-post a comment that is
            // already live. Leaving the state alone is what keeps it out of a retry.
            continue;
        }
        db.update_interaction_assignment_state(
            id,
            ThreadMessageState::Failed,
            Some(error_code),
            None,
            None,
        )?;
    }
    Ok(())
}

/// Assignments a failure must not overwrite, because their delivery is settled or in flight.
///
/// The same three states `retryable_assignments` excludes, for the same reason: tapping Send
/// is not idempotent. Built once from the campaign detail rather than re-queried per target.
fn protected_assignment_ids(
    assignments: &[crate::InteractionAssignmentRecord],
) -> std::collections::HashSet<String> {
    assignments
        .iter()
        .filter(|assignment| {
            matches!(
                assignment.state,
                ThreadMessageState::Sending
                    | ThreadMessageState::Succeeded
                    | ThreadMessageState::Uncertain
            )
        })
        .map(|assignment| assignment.id.clone())
        .collect()
}

/// What a campaign's own rows say happened, counted once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CampaignTally {
    succeeded: usize,
    failed: usize,
    /// Messages still in a **non-terminal** state: meant to go, never went, and nothing left
    /// running that would move them. A subset retry leaves these behind by design — the whole
    /// point of `only_assignments` is not to touch the rest — so "not failed" and "not
    /// outstanding" are different questions and answering only the first was the bug.
    outstanding: usize,
}

/// Count a campaign's rows into [`CampaignTally`].
fn tally_assignments(
    assignments: &[crate::interaction::InteractionAssignmentRecord],
) -> CampaignTally {
    let mut tally = CampaignTally {
        succeeded: 0,
        failed: 0,
        outstanding: 0,
    };
    for assignment in assignments {
        match assignment.state {
            ThreadMessageState::Succeeded => tally.succeeded += 1,
            // Everything that was meant to be posted and was not, counted the way
            // `InteractionCampaignSummary::failed_messages` counts it.
            ThreadMessageState::Failed
            | ThreadMessageState::Uncertain
            | ThreadMessageState::SkippedParent => tally.failed += 1,
            ThreadMessageState::Queued
            | ThreadMessageState::Preparing
            | ThreadMessageState::Ready
            | ThreadMessageState::Sending => tally.outstanding += 1,
        }
    }
    tally
}

/// The verdict those rows imply.
///
/// **Pure, and lifted out of the run loop on purpose.** This arithmetic is the campaign's
/// terminal state — what the Monitor draws, and what decides whether it offers Thử lại at all
/// — and it lived inside a two-hundred-line async function that cannot be called without a
/// real driver. So nothing exercised it, and it was wrong: `Queued`, `Preparing`, `Ready` and
/// `Sending` fell into a catch-all arm and were counted as neither success nor failure, which
/// made `failed == 0` read as "nothing outstanding". Retrying one message of fourteen — which
/// the UI offers per row — then filed the campaign `Succeeded` with thirteen comments never
/// sent, and since Thử lại is drawn only for partial/failed/cancelled, put it out of reach.
fn settle_state(tally: CampaignTally, had_error: bool) -> ThreadCampaignState {
    if tally.failed == 0 && tally.outstanding == 0 && !had_error {
        ThreadCampaignState::Succeeded
    } else if tally.succeeded == 0 {
        ThreadCampaignState::Failed
    } else {
        ThreadCampaignState::Partial
    }
}

#[allow(clippy::too_many_arguments)]
/// Run a campaign's cohorts at the same time, then decide what the campaign was.
///
/// **The cohorts are why this is safe to run in parallel.** `partition_actors` gives each
/// cohort its own phones, and `plan_threads` gives each cohort its own links, so two tasks
/// never reach for the same exclusive lease and never share an identity map. Nothing here
/// coordinates them because there is nothing to coordinate — the isolation is in the plan.
///
/// A campaign with no cohort size is one cohort, which is the sequential behaviour this had
/// before: same code path, one task.
///
/// The final state is written **once, here**, after every cohort has finished. Leaving it
/// inside the runner would have each cohort racing to declare the campaign over while its
/// siblings were still posting.
pub async fn execute_thread_campaign(
    db: Arc<crate::db::Database>,
    control: Arc<DeviceControlPlane>,
    engine: crate::NurtureEngine,
    events: crate::EventBus,
    campaign_id: String,
    request: ThreadCampaignRequest,
    plan: ThreadPlan,
    only_assignments: Option<std::collections::HashSet<String>>,
    artifacts: crate::FlowArtifactStore,
    frame_source: Arc<dyn crate::GenerationFrameSource>,
) -> anyhow::Result<()> {
    let mut by_cohort: std::collections::BTreeMap<u16, std::collections::HashSet<String>> =
        Default::default();
    for message in &plan.assignments {
        by_cohort
            .entry(message.cohort)
            .or_default()
            .insert(message.target_key.clone());
    }

    // **Bounded by the stream budget, because exhausting it is a refusal and not a queue.**
    //
    // Each assignment holds one foreground stream slot while it runs, so N cohorts want N
    // slots at once. Past the budget, `preview_foreground_victim` answers
    // `CapacityExhausted` and the assignment *fails* — it does not wait its turn. Starting
    // eight teams against a budget of four would not run slower, it would fail half the
    // campaign for a reason that has nothing to do with the phones.
    //
    // Held for a whole cohort rather than per assignment: coarser than it has to be, and
    // the coarseness is what makes it safe to reason about — a team that has a permit can
    // always open its next phone.
    let gate = Arc::new(tokio::sync::Semaphore::new(
        control.stream_capacity().max(1),
    ));
    // `None` for a single cohort rather than a set holding every key: it is the same work
    // either way, and it keeps the ordinary one-team run on the path that has no filter to
    // get wrong.
    let single = by_cohort.len() <= 1;
    let mut running = Vec::with_capacity(by_cohort.len().max(1));
    for (_, targets) in by_cohort {
        let gate = gate.clone();
        running.push(tokio::spawn(gated_cohort(
            gate,
            (!single).then_some(targets),
            db.clone(),
            control.clone(),
            engine.clone(),
            events.clone(),
            campaign_id.clone(),
            request.clone(),
            plan.clone(),
            only_assignments.clone(),
            artifacts.clone(),
            frame_source.clone(),
        )));
    }

    let mut succeeded = 0usize;
    let mut failed = 0usize;
    let mut first_error: Option<anyhow::Error> = None;
    for handle in running {
        match handle.await {
            Ok(Ok((ok, bad))) => {
                succeeded += ok;
                failed += bad;
            }
            // A cohort that died still leaves its siblings' work standing, and the campaign
            // is still worth finalising from what did land. The first reason is kept for
            // the caller; the rest would only bury it.
            Ok(Err(error)) => {
                tracing::warn!("interaction cohort failed: {error:#}");
                first_error.get_or_insert(error);
            }
            Err(join) => {
                tracing::warn!("interaction cohort panicked: {join}");
                first_error.get_or_insert_with(|| anyhow::anyhow!("cohort panicked: {join}"));
            }
        }
    }

    // Read the campaign, not the run, to decide how it ended.
    //
    // `succeeded`/`failed` count what *this* pass did, and on a retry that is only the
    // messages the retry touched. Measured 24/08/2026: a campaign with five comments already
    // live had every retried message fail, so `succeeded == 0` for the pass and the row was
    // written `Failed` — a campaign the operator could see five posted comments under, filed
    // as a total loss. The question the state answers is about the campaign.
    let detail = db.get_interaction_campaign(&campaign_id)?;
    let cancelled = matches!(
        detail.as_ref().map(|detail| detail.summary.state),
        Some(ThreadCampaignState::Cancelled)
    );
    let tally = match detail.as_ref() {
        Some(detail) => tally_assignments(&detail.assignments),
        // Unreadable: fall back to this pass's own tally rather than inventing a verdict.
        None => CampaignTally {
            succeeded,
            failed,
            outstanding: 0,
        },
    };
    if !cancelled {
        let final_state = settle_state(tally, first_error.is_some());
        // **`Partial` has to say why.** The SQL sets `error_code=?2` unconditionally, so `None`
        // wrote NULL rather than preserving anything — and `Partial` is the state most in need
        // of an explanation and the one guaranteed not to have had one. The counts are the
        // explanation the engine actually holds here; the per-assignment rows carry the detail
        // beneath it.
        let reason = match final_state {
            ThreadCampaignState::Partial => Some(format!(
                "xong {}, lỗi {}, còn dở {}",
                tally.succeeded, tally.failed, tally.outstanding
            )),
            _ => first_error.as_ref().map(|error| format!("{error:#}")),
        };
        db.update_interaction_campaign_state(&campaign_id, final_state, reason.as_deref())?;
    }
    events.emit(AppEvent::InteractionUpdated {
        campaign_id,
        revision: revision(),
    });
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// [`run_cohort`], but holding a slice of the stream budget for as long as it runs.
///
/// A separate function rather than a block inside the spawn, so the permit's lifetime is
/// the task's lifetime by construction: acquiring it inside the future and dropping it at
/// the end is exactly what "this team is using a phone" means, and there is no path out of
/// here that forgets to release it.
#[allow(clippy::too_many_arguments)]
async fn gated_cohort(
    gate: Arc<tokio::sync::Semaphore>,
    mine: Option<std::collections::HashSet<String>>,
    db: Arc<crate::db::Database>,
    control: Arc<DeviceControlPlane>,
    engine: crate::NurtureEngine,
    events: crate::EventBus,
    campaign_id: String,
    request: ThreadCampaignRequest,
    plan: ThreadPlan,
    only_assignments: Option<std::collections::HashSet<String>>,
    artifacts: crate::FlowArtifactStore,
    frame_source: Arc<dyn crate::GenerationFrameSource>,
) -> anyhow::Result<(usize, usize)> {
    // The semaphore is never closed, so this only fails if it is — treat that as "go", since
    // refusing a whole team over a bookkeeping error would lose more than it protects.
    let _permit = gate.acquire().await;
    run_cohort(
        mine,
        db,
        control,
        engine,
        events,
        campaign_id,
        request,
        plan,
        only_assignments,
        artifacts,
        frame_source,
    )
    .await
}

/// Run one cohort's share of a campaign: its links, its phones, its own identity map.
///
/// Renamed from `execute_thread_campaign` when cohorts arrived, and the body is unchanged
/// — which is the point. A cohort is the whole campaign restricted to the links it owns,
/// so the sequencing inside it, the parent-identity map, the cancellation checks and the
/// evidence all keep working exactly as they were proven to.
///
/// `mine` is the set of target keys this cohort owns; `None` means all of them, which is
/// the single-cohort arrangement this had before. The **full** request is still passed in
/// rather than a trimmed one, because `manual_comment_for` deals the operator's pool by
/// global target index — trimming would make link 1 and link 2 open with the same
/// sentence, which is precisely what that dealing exists to avoid.
///
/// Returns `(succeeded, failed)` and deliberately does **not** write the campaign's final
/// state: with several cohorts running, that decision belongs to whoever joins them.
#[allow(clippy::too_many_arguments)]
async fn run_cohort(
    mine: Option<std::collections::HashSet<String>>,
    db: Arc<crate::db::Database>,
    control: Arc<DeviceControlPlane>,
    engine: crate::NurtureEngine,
    events: crate::EventBus,
    campaign_id: String,
    request: ThreadCampaignRequest,
    plan: ThreadPlan,
    only_assignments: Option<std::collections::HashSet<String>>,
    artifacts: crate::FlowArtifactStore,
    // Separate from `engine.frames`, which is an `Arc<dyn FrameSource>` and therefore has
    // no way to ask for a *generation*. Evidence needs that: see `evidence_frame_after`.
    frame_source: Arc<dyn crate::GenerationFrameSource>,
) -> anyhow::Result<(usize, usize)> {
    let settings = db.get_nurture_settings().context("đọc cấu hình AI")?;
    if ai_key_missing(&request, &settings.api_key) {
        anyhow::bail!("AI API key chưa được cấu hình cho Interaction");
    }
    let detail = db
        .get_interaction_campaign(&campaign_id)?
        .context("campaign không tồn tại")?;
    let protected = protected_assignment_ids(&detail.assignments);
    // **What already posted, so a retry knows what to reply to.**
    //
    // An identity is only ever produced by sending, and a message that already succeeded
    // is deliberately never sent again — so on a retry the in-memory map below starts
    // empty and every reply under a succeeded root was skipped with
    // `parent_identity_not_confirmed`. That is the one case Retry exists for, and it could
    // not work. The identity was on disk the whole time, in the assignment's evidence.
    let posted: HashMap<(String, u8), CommentLocatorIdentity> = detail
        .assignments
        .iter()
        .filter_map(|assignment| {
            Some((
                (assignment.target_key.clone(), assignment.ordinal),
                assignment.posted_identity()?,
            ))
        })
        .collect();
    let assignment_ids: HashMap<(String, u8), String> = detail
        .assignments
        .into_iter()
        .map(|assignment| {
            (
                (assignment.target_key.clone(), assignment.ordinal),
                assignment.id,
            )
        })
        .collect();
    // Held false for the life of the campaign, on purpose. It reaches the send
    // functions, where it would abort their waits mid-flight — including the
    // window between tapping Send and confirming the field cleared, which would
    // manufacture `Uncertain` and block retry for a comment that did post.
    // Stopping is handled between assignments instead, where nothing is in
    // flight.
    let stop = AtomicBool::new(false);
    let mut succeeded = 0usize;
    let mut failed = 0usize;

    for (target_index, target) in request.targets.iter().enumerate() {
        // Another cohort's link. Skipped here rather than by trimming the request so that
        // `target_index` keeps meaning what it meant.
        if mine
            .as_ref()
            .is_some_and(|mine| !mine.contains(&target.target_key))
        {
            continue;
        }
        if campaign_is_cancelled(&db, &campaign_id)? {
            return Ok((succeeded, failed));
        }

        // Open the target and collect the same-post evidence before preparing
        // any message. All texts for this target are persisted before send.
        //
        // **Only when the AI is writing.** In manual mode the pool already covers every
        // (target, ordinal), so nothing here is ever read — and opening the target on
        // ordinal 0's own phone is what then makes ordinal 0's arrival check refuse
        // deterministically. See `ThreadCampaignRequest::needs_ai_evidence_frames`.
        //
        // A failure in here now fails **this target**, not the campaign. It used to be a
        // bare `?` on a function returning `anyhow::Result<()>`, so the first target that
        // could not be photographed ended the whole run: every later target was left in
        // `queued` with no error of its own, and the campaign carried one target's message.
        // Measured: a two-target run posted one comment and never touched target two.
        let frames = if request.needs_ai_evidence_frames() {
            match collect_target_evidence_frames(
                &control,
                &engine,
                &plan,
                target,
                Duration::from_millis(500),
            )
            .await
            {
                Ok(frames) => frames,
                Err(error) => {
                    fail_whole_target(
                        &db,
                        &plan,
                        &assignment_ids,
                        &protected,
                        target,
                        &format!("target_evidence_unavailable: {error}"),
                    )?;
                    notify(&events, &campaign_id);
                    failed += request.message_count as usize;
                    continue;
                }
            }
        } else {
            Vec::new()
        };

        let mut prepared_messages = Vec::with_capacity(request.message_count as usize);
        let mut previous = None::<String>;
        for assignment in plan
            .assignments
            .iter()
            .filter(|assignment| assignment.target_key == target.target_key)
        {
            let id = assignment_ids
                .get(&(assignment.target_key.clone(), assignment.ordinal))
                .context("assignment id missing")?;
            // An out-of-scope assignment keeps the state it earned. This guard is the one
            // that makes `retryable_assignments` mean anything: that function excludes
            // `Succeeded` precisely because tapping Send is not idempotent, but this loop
            // used to overwrite **every** assignment of the target with `Preparing`
            // regardless of scope. The send loop below then skipped them, so a first retry
            // posted nothing twice — it merely erased the record that they had succeeded.
            // The *second* retry then read `Preparing`, which is retryable, and would have
            // re-posted a comment that was already public.
            if only_assignments
                .as_ref()
                .is_some_and(|scope| !scope.contains(id))
            {
                continue;
            }
            db.update_interaction_assignment_state(
                id,
                ThreadMessageState::Preparing,
                None,
                None,
                None,
            )?;
            // Skipped in manual mode on purpose: there, Preparing and Ready are microseconds
            // apart because the text is already written, and two events per message would be
            // a burst that says nothing the Ready one does not.
            if !request.is_manual() {
                notify(&events, &campaign_id);
            }
            // The operator's own comments win when they gave any. Dealt across
            // (target, ordinal) so ten links do not all open with the same sentence, and
            // deterministically, so a replay of this campaign sends the same text — which is
            // what makes the stored evidence checkable.
            let text = match request.manual_comment_for(target_index, assignment.ordinal) {
                Some(manual) => manual.to_string(),
                None => {
                    let mut scoped = settings.clone();
                    scoped.max_comment_words = u32::from(request.max_words);
                    let direction = if let Some(parent) = &previous {
                        format!(
                            "{}; trả lời tự nhiên câu trước {:?}, không nhắc lại nguyên văn",
                            request.instruction, parent
                        )
                    } else {
                        request.instruction.clone()
                    };
                    // `{:#}` on the way out, and a log line here. This used to be a bare `?`
                    // on a function returning `anyhow::Result<()>`, so one AI failure on the
                    // first assignment ended the whole campaign, left the rest in `queued`
                    // with no reason of their own, and recorded only the outermost context
                    // (`error.to_string()` keeps one layer). Measured 13/08/2026: a live run
                    // failed with `AI chuẩn bị assignment 0` and the HTTP status, body and
                    // timeout were all unrecoverable — a failure that cannot be diagnosed
                    // from the evidence it left.
                    let prepared_text = match crate::openai_client::prepare_comment_for_frames(
                        &scoped,
                        &frames,
                        Some(&direction),
                        engine.frame_text.as_ref(),
                    )
                    .await
                    {
                        Ok(value) => value,
                        Err(error) => {
                            let detail = format!("{error:#}");
                            tracing::error!(
                                "interaction {}: AI không viết được cho ordinal {}: {detail}",
                                target.target_key,
                                assignment.ordinal
                            );
                            db.update_interaction_assignment_state(
                                id,
                                ThreadMessageState::Failed,
                                Some(&format!(
                                    "ai_comment_unavailable: ordinal {} — {detail}",
                                    assignment.ordinal
                                )),
                                None,
                                None,
                            )?;
                            notify(&events, &campaign_id);
                            failed += 1;
                            continue;
                        }
                    };
                    let (grounded, _evidence_mode) = prepared_text;
                    grounded.text
                }
            };
            // Tag the mentioned accounts on the opening comment only — the replies do not
            // re-tag. Carried beside the text rather than pasted into it: the hierarchy driver
            // turns each handle into a **real** mention by picking it out of TikTok's own
            // suggestion list, and a handle already sitting in the body would just be typed
            // twice. The pixel driver, which cannot reach that list, prepends them as literal
            // text itself — exactly what both used to do.
            let mentions = if assignment.ordinal == 0 {
                request
                    .mentions
                    .iter()
                    .map(|handle| handle.trim().trim_start_matches('@').to_string())
                    .filter(|handle| !handle.is_empty())
                    .collect()
            } else {
                Vec::new()
            };
            let prepared = PreparedThreadMessage::with_mentions(assignment, text, mentions);
            previous = Some(prepared.text.clone());
            db.prepare_interaction_assignment(id, &prepared)?;
            notify(&events, &campaign_id);
            prepared_messages.push((id.clone(), prepared));
        }

        // A root comment is sent with full frame evidence. Each subsequent
        // reply first resolves the exact parent text+author on two OCR frames.
        // Seeded from what this target already posted in an earlier run, then added to as
        // this one sends. A fresh campaign finds nothing here and behaves exactly as before.
        let mut identities: HashMap<u8, CommentLocatorIdentity> = posted
            .iter()
            .filter(|((key, _), _)| key == &target.target_key)
            .map(|((_, ordinal), identity)| (*ordinal, identity.clone()))
            .collect();
        let mut chain_broken_at: Option<u8> = None;
        for (id, prepared) in prepared_messages {
            // A retry runs the same plan but must not re-send anything already
            // posted; the caller decides which assignments are in scope.
            if only_assignments
                .as_ref()
                .is_some_and(|only| !only.contains(&id))
            {
                continue;
            }
            // Cancellation was only ever checked between *targets*, so pressing
            // Dừng left every remaining message of the current target to post —
            // up to six more public comments. Check before each one instead.
            //
            // Deliberately not mid-send: the Send tap is a side effect that has
            // already gone out, and aborting between it and its confirmation
            // would manufacture `Uncertain`, which blocks retry. One in-flight
            // message finishing is the correct cost of stopping.
            if campaign_is_cancelled(&db, &campaign_id)? {
                return Ok((succeeded, failed));
            }
            let parent_identity = prepared
                .parent_ordinal
                .and_then(|ordinal| identities.get(&ordinal).cloned());
            if let Some(parent_ordinal) = prepared
                .parent_ordinal
                .filter(|_| parent_identity.is_none())
            {
                // An identity is only ever learned by sending, so a message whose parent
                // never posted has nothing to reply to. Naming the ordinal that broke it
                // is the difference between "5 messages skipped" and knowing which one to
                // look at.
                //
                // **How far that spreads is the shape's business, not this block's.** In a
                // chain every later message names the one before it, so one gap does end
                // the target — each of them arrives here in turn. In a star they all name
                // ordinal 0, so a reply that fails costs only itself and its siblings carry
                // on. Nothing here needs to know which it is: the lookup is by the parent
                // this message actually has.
                if chain_broken_at.is_none() {
                    chain_broken_at = Some(parent_ordinal);
                }
                let broke_at = chain_broken_at.unwrap_or(parent_ordinal);
                db.update_interaction_assignment_state(
                    &id,
                    ThreadMessageState::SkippedParent,
                    Some(&format!(
                        "parent_identity_not_confirmed_at_ordinal_{broke_at}"
                    )),
                    None,
                    None,
                )?;
                notify(&events, &campaign_id);
                failed += 1;
                continue;
            }
            let opened = match open_interaction_context(&control, &prepared.actor_udid).await {
                Ok(context) => context,
                Err(error) => {
                    db.update_interaction_assignment_state(
                        &id,
                        ThreadMessageState::Failed,
                        Some(&format!("{error}")),
                        None,
                        None,
                    )?;
                    notify(&events, &campaign_id);
                    failed += 1;
                    continue;
                }
            };
            let InteractionDevice {
                context,
                target_package: opened_package,
            } = opened;
            let session = control.streaming_session(&context)?;
            let gestures = tokio::sync::Mutex::new(());
            let mut effect_intent = false;
            // The stream this context owns. A frame from any other generation belongs to a
            // producer that has already been torn down and proves nothing about this send.
            let generation = context.stream_proof().generation;
            // Seeded here so a failure *before* the send (a refused arrival, a driver that
            // will not be chosen) still files the screen that explains it, and re-read just
            // before the tap so the published frame is strictly newer than the comment.
            let mut watermark = frame_source
                .latest_in_generation(&prepared.actor_udid, generation)
                .map(|frame| frame.sequence);
            let result = async {
                // Chosen once per assignment, from the session in hand. A build with no
                // measured labels refuses here — before the link is opened, so before
                // anything at all has happened on the phone.
                let driver = choose_target_driver(
                    &engine,
                    &prepared.actor_udid,
                    session.as_ref(),
                    &opened_package,
                    &gestures,
                )
                .await?;
                // Nothing is typed until the target is on screen: an
                // unconfirmed open posts the campaign's comment to whichever
                // video the phone happens to be showing.
                let proof = driver.open_target(session.as_ref(), target).await?;
                if proof == TargetProof::Structural {
                    // Worth saying out loud: the post is open but unidentified, so nothing
                    // here rules out the link having resolved to a different post.
                    //
                    // This used to blame OCR being macOS-only, which was wrong twice. The
                    // condition is on the proof *level* and knows nothing about platform or
                    // driver: the hierarchy path never calls OCR at all, and the pixel path
                    // also lands here when OCR ran fine and simply did not find the handle
                    // within the grace window. Naming the reader instead of guessing at a
                    // cause is the honest version — a message that sends the operator to
                    // install OCR when the handle merely is not on screen is worse than no
                    // message.
                    tracing::warn!(
                        "interaction {}: đăng vào bài của @{} ở mức bằng chứng Structural — \
                         không có gì trên màn nêu handle nên chưa xác định được đây có đúng \
                         bài đó không (reader={})",
                        target.target_key,
                        target.author,
                        driver.kind()
                    );
                }
                // The like, if the operator asked for one. **After the arrival proof and
                // before anything is typed**, for two reasons: the rail is where the arrival
                // check just found it, so no extra locate is needed, and a like that fails
                // must not consume the comment — so it is logged and the message carries on.
                //
                // Not fatal on purpose. A refusal here is either "this backend cannot" or
                // "the label did not flip", and neither is a reason to abandon a comment the
                // operator queued.
                //
                // **Written into the evidence, not only into the log.** The sentence above
                // used to end "the campaign's own record shows what happened", and it did
                // not: the outcome went to `tracing::warn!` and nowhere else, so an operator
                // watching the Monitor tab saw a message succeed and had no way to learn the
                // like had been refused. A failure nobody is shown is the same shape as the
                // ones this project has spent its time removing.
                let like_note = if request.like_target {
                    match driver.like_target(session.as_ref()).await {
                        Ok(reason) => {
                            tracing::info!(
                                "interaction {}: {} — {reason}",
                                target.target_key,
                                prepared.actor_udid
                            );
                            Some(format!("đã tim: {reason}"))
                        }
                        Err(error) => {
                            tracing::warn!(
                                "interaction {}: không thả tim được ({error:#})",
                                target.target_key
                            );
                            Some(format!("không tim được: {error:#}"))
                        }
                    }
                } else {
                    None
                };

                // `effect_intent` decides `Uncertain` versus `Failed` on the error path,
                // and `Uncertain` is permanently unretryable. So it is **not** set before
                // the call: only the driver knows whether a Send tap actually went out,
                // and the steps that locate a parent can fail with nothing typed. Setting
                // it optimistically is exactly the bug this shape exists to prevent — it
                // made a never-posted reply impossible to retry.
                //
                // The DB `Sending` write still happens first: it is the audit record that
                // this assignment is about to act, and it must survive a crash mid-send.
                if let Some(parent) = parent_identity.as_ref() {
                    db.update_interaction_assignment_state(
                        &id,
                        ThreadMessageState::Sending,
                        None,
                        Some("reply_comment"),
                        None,
                    )?;
                    notify(&events, &campaign_id);
                    // Strictly the frame that was current when Send was tapped, so
                    // anything published afterwards has to be newer than the comment.
                    watermark = frame_source
                        .latest_in_generation(&prepared.actor_udid, generation)
                        .map(|frame| frame.sequence);
                    let sent = match driver
                        .send_reply(session.as_ref(), parent, &prepared, &stop)
                        .await
                    {
                        Ok(sent) => sent,
                        Err(failure) => {
                            effect_intent = failure.effect_may_have_gone_out();
                            return Err(failure.into_error());
                        }
                    };
                    Ok::<serde_json::Value, anyhow::Error>(serde_json::json!({
                        "send": sent.evidence,
                        "parent": parent,
                        "postedIdentity": sent.identity,
                        "reader": driver.kind(),
                        "arrival": proof.as_str(),
                        "like": like_note,
                        // Only the opening comment can carry tags, so this is `None` on every
                        // reply — filed beside the like note because both are outcomes the
                        // comment's own text cannot show.
                        "mention": sent.mention_note,
                    }))
                } else {
                    db.update_interaction_assignment_state(
                        &id,
                        ThreadMessageState::Sending,
                        None,
                        Some("post_comment"),
                        None,
                    )?;
                    notify(&events, &campaign_id);
                    // Strictly the frame that was current when Send was tapped, so
                    // anything published afterwards has to be newer than the comment.
                    watermark = frame_source
                        .latest_in_generation(&prepared.actor_udid, generation)
                        .map(|frame| frame.sequence);
                    let sent = match driver.send_root(session.as_ref(), &prepared, &stop).await {
                        Ok(sent) => sent,
                        Err(failure) => {
                            effect_intent = failure.effect_may_have_gone_out();
                            return Err(failure.into_error());
                        }
                    };
                    Ok::<serde_json::Value, anyhow::Error>(serde_json::json!({
                        "send": sent.evidence,
                        "postedIdentity": sent.identity,
                        "reader": driver.kind(),
                        "arrival": proof.as_str(),
                        "like": like_note,
                        // Only the opening comment can carry tags, so this is `None` on every
                        // reply — filed beside the like note because both are outcomes the
                        // comment's own text cannot show.
                        "mention": sent.mention_note,
                    }))
                }
            }
            .await;
            // Before the teardown, not after. `close_ui_context` stops the stream and
            // removes this device's cached frame, which is why every artefact row until
            // now was filed with a NULL path.
            let evidence = evidence_frame_after(
                frame_source.as_ref(),
                &prepared.actor_udid,
                generation,
                watermark,
            );
            let cleanup = control.close_ui_context(context).await;
            match result {
                Ok(evidence_json) => {
                    let evidence_text = evidence_json.to_string();
                    db.update_interaction_assignment_state(
                        &id,
                        ThreadMessageState::Succeeded,
                        None,
                        None,
                        Some(&evidence_text),
                    )?;
                    let artifact_kind = if prepared.parent_ordinal.is_some() {
                        "comment-reply-evidence"
                    } else {
                        "comment-root-evidence"
                    };
                    // The drawer is still open on the phone — nothing closes it after the
                    // identity pass — so this frame shows the comment that was just posted,
                    // in the list, which is the only thing that settles a dispute later.
                    // The frame itself was taken above, while the stream was still alive.
                    let saved = publish_evidence_frame(
                        &artifacts,
                        evidence.clone(),
                        &campaign_id,
                        &id,
                        &prepared.actor_udid,
                    );
                    // No fallback sha. It used to borrow `postedIdentity.frameSha256` when
                    // nothing was stored, which produced exactly the row observed on
                    // 13/08/2026: a digest that looks like evidence next to a `relative_path`
                    // of NULL and no bytes anywhere. A row that stored nothing must say so.
                    let artifact_sha = saved.as_ref().map(|(_, sha)| sha.as_str()).unwrap_or("");
                    let _ = db.add_interaction_artifact(
                        &campaign_id,
                        &target.target_key,
                        Some(&id),
                        artifact_kind,
                        &evidence_text,
                        artifact_sha,
                        saved.as_ref().map(|(path, _)| path.as_str()),
                    )?;
                    if let Some(identity) = evidence_json.get("postedIdentity").and_then(|value| {
                        serde_json::from_value::<CommentLocatorIdentity>(value.clone()).ok()
                    }) {
                        identities.insert(prepared.ordinal, identity);
                    }
                    succeeded += 1;
                }
                Err(error) => {
                    let state = if effect_intent {
                        ThreadMessageState::Uncertain
                    } else {
                        ThreadMessageState::Failed
                    };
                    db.update_interaction_assignment_state(
                        &id,
                        state,
                        Some(&error.to_string()),
                        None,
                        None,
                    )?;
                    // Especially here. `Uncertain` means the Send tap went out
                    // and its confirmation did not arrive, so whether the
                    // comment posted can only be settled by looking — and this
                    // path used to write no artifact at all.
                    if let Some((path, sha)) = publish_evidence_frame(
                        &artifacts,
                        evidence,
                        &campaign_id,
                        &id,
                        &prepared.actor_udid,
                    ) {
                        let _ = db.add_interaction_artifact(
                            &campaign_id,
                            &target.target_key,
                            Some(&id),
                            "comment-failure-evidence",
                            &serde_json::json!({ "error": error.to_string() }).to_string(),
                            &sha,
                            Some(&path),
                        );
                    }
                    failed += 1;
                }
            }
            if let Err(error) = cleanup {
                tracing::warn!("interaction cleanup {}: {}", prepared.actor_udid, error);
            }
            // Covers both arms of the match above — a send either succeeded or it did not,
            // and this fires once for whichever ran.
            notify(&events, &campaign_id);
        }
    }

    Ok((succeeded, failed))
}

/// Longest the device gets to land on the target before the open is called a
/// failure, and how often the frame is read while waiting. The check costs one
/// OCR pass per poll, so this is deliberately slower than a frame loop — it runs
/// once per target, not per frame.
const OPEN_TARGET_TIMEOUT: Duration = Duration::from_secs(14);
const OPEN_TARGET_POLL: Duration = Duration::from_millis(900);
/// Characters of the author handle that must be legible. TikTok truncates long
/// handles with an ellipsis, so a prefix is all that can be required; six is
/// enough that another account on screen is unlikely to share it.
const OPEN_TARGET_HANDLE_PREFIX: usize = 6;
/// How long to keep looking for the handle once the post has visibly settled,
/// before accepting the structural proof and moving on. The post is already up
/// by then; this is only the identity check catching up.
const OPEN_TARGET_HANDLE_GRACE: Duration = Duration::from_secs(4);

/// Open a target link and prove the device landed on *that* post.
///
/// `open_url` returning Ok means the request was accepted, not that the video is
/// on screen. This module already draws that distinction for typing — "the
/// request returning OK is not evidence of insertion" — but not for opening: two
/// of the three call sites slept two seconds and carried on, and the third only
/// asked whether TikTok was frontmost, which does not say *which post* is up,
/// and skipped even that whenever the query itself failed.
///
/// It matters most on the send path. A campaign that opens nothing then posts
/// its comment to whatever the phone happens to be showing — someone else's
/// video, under the operator's account. On the evidence path it means the AI
/// writes its comment about a post that is not the target.
///
/// The link carries the author handle and the opened post displays it, so that
/// is the proof: wait for a frame whose OCR contains it.
/// How far the open could be proved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetProof {
    /// The author handle from the link was read back off the screen. This is
    /// the only level that identifies *which* post is open.
    Identified,
    /// TikTok came forward, the screen changed, and it settled on a post — but
    /// the handle could not be read, so the post is unidentified.
    ///
    /// This is the level nearly every real send reaches, and **not** because of any
    /// platform limitation — the note that used to sit here blamed OCR for being
    /// macOS-only, which was wrong on both counts. The hierarchy path reads the handle
    /// out of the accessibility tree and calls no OCR at all; it lands here because a
    /// TikTok post page shows the *nickname*, and a nickname folds onto its handle for
    /// roughly one account in three (`interaction_hierarchy::author_matches_handle`
    /// documents the measurements). So sending is deliberately **not** gated on
    /// [`Self::Identified`]: gating it would refuse most posts that opened perfectly.
    Structural,
}

impl TargetProof {
    /// The word that goes into the evidence row.
    ///
    /// Stored rather than only logged: the proof level is the difference between "we know
    /// which post this landed on" and "we know a post was open", and a stored campaign
    /// that does not say which of those it achieved cannot be audited afterwards.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Identified => "identified",
            Self::Structural => "structural",
        }
    }
}

pub async fn open_target_confirmed(
    engine: &crate::NurtureEngine,
    udid: &str,
    session: &dyn crate::UiSession,
    target: &crate::ResolvedTikTokTarget,
    target_package: &str,
) -> anyhow::Result<TargetProof> {
    // The screen as it was before the request, so "nothing happened" is
    // distinguishable from "the post loaded".
    let before = engine
        .frames
        .latest(udid)
        .map(|frame| crate::frame_sha256(&frame))
        .unwrap_or_default();

    session
        .open_url(&target.normalized_url)
        .await
        .with_context(|| format!("mở link {}", target.normalized_url))?;

    let wanted = crate::normalize_locator_text(&target.author);
    let needle: String = wanted
        .chars()
        .take(OPEN_TARGET_HANDLE_PREFIX.min(wanted.chars().count()))
        .collect();

    let deadline = tokio::time::Instant::now() + OPEN_TARGET_TIMEOUT;
    let mut ocr_available = true;
    let mut settled_since: Option<tokio::time::Instant> = None;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(OPEN_TARGET_POLL).await;
        // A wrong frontmost app is decisive; a query that cannot answer it is
        // not, and never stands in for the frame proof either way.
        if let Ok(bundle) = session.active_app_bundle().await {
            if bundle != target_package {
                continue;
            }
        }
        let Some(frame) = engine.frames.latest(udid) else {
            continue;
        };
        if crate::frame_sha256(&frame) == before {
            // Byte-identical to the pre-request screen: the open did nothing.
            continue;
        }
        let Ok(decoded) = image::load_from_memory(&frame) else {
            continue;
        };
        if crate::screen::locate_action_rail(&decoded.to_rgb8()).is_none() {
            // Still loading, or on an interstitial — not a post yet.
            continue;
        }
        let settled_at = *settled_since.get_or_insert(tokio::time::Instant::now());

        if ocr_available && !needle.is_empty() {
            match engine.frame_text.recognize(&frame).await {
                Ok(observations) => {
                    if observations.iter().any(|observation| {
                        crate::normalize_locator_text(&observation.text).contains(&needle)
                    }) {
                        return Ok(TargetProof::Identified);
                    }
                }
                // Not a reason to fail the open — it is how a build with no OCR
                // reports that fact. Fall back to the structural proof rather
                // than blocking the feature on a platform capability.
                Err(_) => ocr_available = false,
            }
        }
        // A missing handle downgrades the proof; it never fails the open.
        //
        // The handle comes from the URL, and what the post displays is the
        // account's *nickname*, which often is not the same string — captures
        // from this device show both cases ("nguyenvantoan8584" on one card,
        // "Lúc này lúc kia" on another). Until that has been checked against a
        // link-opened page on a real device, treating a missing handle as proof
        // of the wrong post would break the feature for every account whose
        // nickname differs from its handle, to guard against a failure that has
        // never been observed. The failure that *has* been observed is the open
        // doing nothing at all, and the checks above catch that.
        if !ocr_available || settled_at.elapsed() >= OPEN_TARGET_HANDLE_GRACE {
            return Ok(TargetProof::Structural);
        }
    }

    if settled_since.is_some() {
        return Ok(TargetProof::Structural);
    }
    anyhow::bail!(
        "mở link {} nhưng máy không chuyển sang bài viết nào",
        target.normalized_url
    )
}

async fn open_comment_for_ocr(
    engine: &crate::NurtureEngine,
    udid: &str,
    session: &dyn crate::UiSession,
    gestures: &tokio::sync::Mutex<()>,
) -> anyhow::Result<(Vec<CommentOcrObservation>, (f64, f64))> {
    let frame = engine
        .frames
        .latest(udid)
        .context("không có frame trước khi mở comment")?;
    let image = image::load_from_memory(&frame)?.to_rgb8();
    // Locate per frame and fail closed rather than tapping the layout-2
    // fallback blind (an already-followed card hides the red badge; a
    // layout-1 card would then bookmark instead of opening comments).
    let rail = crate::screen::locate_action_rail(&image)
        .context("không định vị được action rail để mở comment")?;
    let screen_size = session.window_size().await.unwrap_or((375.0, 667.0));
    {
        let _guard = gestures.lock().await;
        session
            .tap(TapPoint {
                x: screen_size.0 * rail.x,
                y: screen_size.1 * rail.comment_y,
            })
            .await?;
    }
    for _ in 0..30 {
        if let Some(next) = engine.frames.latest(udid) {
            if let Ok(decoded) = image::load_from_memory(&next) {
                let state = crate::screen::comment_drawer_state(&decoded.to_rgb8()).0;
                if !matches!(
                    state,
                    crate::screen::CommentDrawer::Closed | crate::screen::CommentDrawer::Unknown
                ) {
                    tokio::time::sleep(Duration::from_millis(900)).await;
                    let stable = engine.frames.latest(udid).context("drawer frame missing")?;
                    let observations = engine.frame_text.recognize(&stable).await?;
                    return Ok((observations, screen_size));
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(180)).await;
    }
    anyhow::bail!("comment drawer OCR không mở được")
}

async fn discover_after_send(
    engine: &crate::NurtureEngine,
    udid: &str,
    session: &dyn crate::UiSession,
    gestures: &tokio::sync::Mutex<()>,
    exact_text: &str,
) -> anyhow::Result<CommentLocatorIdentity> {
    let (first, _) = open_comment_for_ocr(engine, udid, session, gestures).await?;
    let first_frame = engine
        .frames
        .latest(udid)
        .context("identity frame missing")?;
    let first_sha = crate::frame_sha256(&first_frame);
    let first_identity = discover_comment_identity(
        &first,
        exact_text,
        &first_sha,
        engine.frame_text.locator_version(),
    )
    .context("comment author/text chưa xuất hiện trong OCR")?;
    tokio::time::sleep(Duration::from_millis(700)).await;
    let second_frame = engine
        .frames
        .latest(udid)
        .context("identity second frame missing")?;
    let second = engine.frame_text.recognize(&second_frame).await?;
    let second_identity = discover_comment_identity(
        &second,
        exact_text,
        &crate::frame_sha256(&second_frame),
        engine.frame_text.locator_version(),
    )
    .context("comment identity không ổn định")?;
    if crate::normalize_locator_text(&first_identity.author_label)
        != crate::normalize_locator_text(&second_identity.author_label)
        || crate::normalize_locator_text(&first_identity.text)
            != crate::normalize_locator_text(&second_identity.text)
    {
        anyhow::bail!("comment identity thay đổi giữa hai frame")
    }
    dismiss_comment_drawer(session, gestures).await;
    Ok(first_identity)
}

/// Today's pixel path, behind [`TargetDriver`] and otherwise unchanged.
///
/// Every method here delegates to the function that already did the work, in the same
/// order and with the same arguments. Nothing about the iOS behaviour changes — the
/// point of the trait is that Android can be a peer, not that this path be rewritten.
struct PixelTargetDriver<'a> {
    engine: &'a crate::NurtureEngine,
    udid: &'a str,
    target_package: &'a str,
    gestures: &'a tokio::sync::Mutex<()>,
}

#[async_trait::async_trait]
impl TargetDriver for PixelTargetDriver<'_> {
    fn kind(&self) -> &'static str {
        "pixel"
    }

    async fn open_target(
        &self,
        session: &dyn crate::UiSession,
        target: &crate::ResolvedTikTokTarget,
    ) -> anyhow::Result<TargetProof> {
        open_target_confirmed(self.engine, self.udid, session, target, self.target_package).await
    }

    async fn send_root(
        &self,
        session: &dyn crate::UiSession,
        prepared: &PreparedThreadMessage,
        stop: &AtomicBool,
    ) -> Result<SendOutcome, SendFailure> {
        // `AfterEffect` for the whole call, which is exactly what this path did before
        // the trait existed: the caller used to set `effect_intent = true` immediately
        // before it. `send_prepared_thread_comment` can also fail *before* tapping Send,
        // but distinguishing that would change iOS behaviour, and the safe direction for
        // a comment that may have posted is to refuse the retry.
        let evidence = self
            .engine
            .send_prepared_thread_comment(self.udid, session, self.gestures, prepared, stop)
            .await
            .map_err(SendFailure::after)?;
        let identity = discover_after_send(
            self.engine,
            self.udid,
            session,
            self.gestures,
            &prepared.text,
        )
        .await
        .ok();
        // The pixel path prepends the tags as literal characters — it cannot reach TikTok's
        // suggestion list — so there is nothing to report beyond what the text already shows.
        Ok(SendOutcome {
            evidence,
            identity,
            mention_note: None,
        })
    }

    async fn send_reply(
        &self,
        session: &dyn crate::UiSession,
        parent: &CommentLocatorIdentity,
        prepared: &PreparedThreadMessage,
        stop: &AtomicBool,
    ) -> Result<SendOutcome, SendFailure> {
        // Everything down to the reply tap is `BeforeEffect`: it opens the drawer,
        // scrolls, and reads frames. Nothing is typed and no Send tap goes out, so a
        // failure here must stay retryable — the parent genuinely may not be in the list,
        // because each reply comes from a different device and TikTok re-ranks the
        // comments between them.
        let (observations, screen_size) =
            open_comment_for_ocr(self.engine, self.udid, session, self.gestures)
                .await
                .map_err(SendFailure::before)?;
        // Hunt down the list first; the stability check then runs on the frame that
        // actually shows the parent.
        let observations = scroll_to_parent(
            self.engine,
            self.udid,
            session,
            self.gestures,
            parent,
            screen_size,
            observations,
        )
        .await
        .map_err(SendFailure::before)?;
        let parent_match = stable_parent_match(self.engine, self.udid, parent, observations)
            .await
            .map_err(SendFailure::before)?;
        let reply_point = TapPoint {
            x: screen_size.0 * parent_match.reply_x,
            y: screen_size.1 * parent_match.reply_y,
        };
        let evidence = self
            .engine
            .send_prepared_thread_reply(
                self.udid,
                session,
                self.gestures,
                reply_point,
                prepared,
                stop,
            )
            .await
            .map_err(SendFailure::after)?;
        let identity = discover_after_send(
            self.engine,
            self.udid,
            session,
            self.gestures,
            &prepared.text,
        )
        .await
        .ok();
        // The pixel path prepends the tags as literal characters — it cannot reach TikTok's
        // suggestion list — so there is nothing to report beyond what the text already shows.
        Ok(SendOutcome {
            evidence,
            identity,
            mention_note: None,
        })
    }
}

/// The hierarchy path, for devices that report where their controls are.
///
/// Holds the resolved label set rather than resolving per call: the lookup needs the UI
/// language *and* the app version, and reading the version costs a `dumpsys` (1–2 s
/// measured). Resolving once per assignment is also what makes a build nobody has
/// measured refuse **before** the first tap.
struct HierarchyTargetDriver<'a> {
    engine: &'a crate::NurtureEngine,
    udid: &'a str,
    target_package: &'a str,
    labels: crate::tiktok_labels::TikTokControls,
    screen: (f64, f64),
}

impl HierarchyTargetDriver<'_> {
    /// The current frame's SHA, or empty when no frame is available.
    ///
    /// The same source the pixel path uses, so a stored evidence row means the same
    /// thing whichever driver wrote it. Empty rather than an error: a missing frame is
    /// not a reason to fail a send that otherwise succeeded, it just leaves that field
    /// unproved.
    fn frame_sha(&self) -> String {
        self.engine
            .frames
            .latest(self.udid)
            .map(|frame| crate::frame_sha256(&frame))
            .unwrap_or_default()
    }

    /// Turn a hierarchy outcome into the shared shape, or a classified failure.
    ///
    /// The classification comes from the verdict, and `CommentVerdict`'s own contract is
    /// what makes it sound: **every variant except `Sent` means nothing was posted, and
    /// `NotConfirmed` is the only one where a Send tap went out**. So `NotConfirmed` is
    /// the single `AfterEffect` case; `SendUnmeasured`, `NoDrawer`, `NoSendControl` and
    /// `NotArmed` all provably never tapped Send and must stay retryable.
    fn finish(
        outcome: crate::interaction_hierarchy::HierarchySendOutcome,
        prepared: &PreparedThreadMessage,
    ) -> Result<SendOutcome, SendFailure> {
        use crate::tiktok_drawer::CommentVerdict;
        if !outcome.verdict.is_sent() {
            // The verdict's own reason, verbatim: it names the step that stopped.
            let error = anyhow::anyhow!("{}", outcome.verdict.reason());
            return Err(match outcome.verdict {
                CommentVerdict::NotConfirmed => SendFailure::AfterEffect(error),
                _ => SendFailure::BeforeEffect(error),
            });
        }
        Ok(SendOutcome {
            evidence: crate::ThreadSendEvidence {
                text_sha256: prepared.text_sha256.clone(),
                armed_frame_sha256: outcome.armed_frame_sha256,
                cleared_frame_sha256: outcome.cleared_frame_sha256,
            },
            identity: outcome.identity,
            mention_note: outcome.mention_note,
        })
    }
}

#[async_trait::async_trait]
impl TargetDriver for HierarchyTargetDriver<'_> {
    fn kind(&self) -> &'static str {
        "hierarchy"
    }

    /// Like the open post, through the same measured contract the nurture loop uses.
    ///
    /// `crate::tiktok_like` is shared rather than reimplemented here: the proof is that
    /// the liked label appears, or the not-liked one goes while the action rail stays, and two
    /// copies of that rule would drift into reporting different things.
    ///
    /// The tap goes to the centre of the rectangle the device reported. There is no touch
    /// planner on this path — a campaign taps a given post once, so there is no history to
    /// vary against, and the alternative would be a planner constructed per assignment whose
    /// "history" is a single entry.
    async fn like_target(&self, session: &dyn crate::UiSession) -> anyhow::Result<String> {
        let stop = std::sync::atomic::AtomicBool::new(false);
        let verdict = crate::tiktok_like::like_post(
            session,
            self.labels,
            &mut crate::tiktok_like::centre_of,
            &stop,
        )
        .await?;
        if verdict.is_liked() {
            Ok(verdict.reason().to_string())
        } else {
            anyhow::bail!("{}", verdict.reason())
        }
    }

    async fn open_target(
        &self,
        session: &dyn crate::UiSession,
        target: &crate::ResolvedTikTokTarget,
    ) -> anyhow::Result<TargetProof> {
        use crate::interaction_hierarchy::TargetArrival;
        // The campaign's stop flag is deliberately not threaded in here: aborting an
        // arrival check is safe, but the flag this campaign holds is permanently false
        // on purpose (see `execute_thread_campaign`), so passing it would only add a
        // parameter that never changes.
        let never = AtomicBool::new(false);
        match crate::interaction_hierarchy::open_target_by_hierarchy(
            session,
            self.labels,
            self.target_package,
            &target.normalized_url,
            &target.author,
            &never,
        )
        .await
        {
            Ok(TargetArrival::Identified { .. }) => Ok(TargetProof::Identified),
            Ok(TargetArrival::Structural) => Ok(TargetProof::Structural),
            Err(refusal) => anyhow::bail!("{}: {}", refusal.code(), refusal.message()),
        }
    }

    async fn send_root(
        &self,
        session: &dyn crate::UiSession,
        prepared: &PreparedThreadMessage,
        stop: &AtomicBool,
    ) -> Result<SendOutcome, SendFailure> {
        let outcome = crate::interaction_hierarchy::send_root_by_hierarchy(
            session,
            self.labels,
            self.screen,
            &prepared.text,
            &prepared.mentions,
            stop,
            || self.frame_sha(),
        )
        .await
        // A transport error out of the send flow itself: the agent stopped answering, and
        // whether the tap landed is genuinely unknown.
        .map_err(SendFailure::after)?;
        Self::finish(outcome, prepared)
    }

    async fn send_reply(
        &self,
        session: &dyn crate::UiSession,
        parent: &CommentLocatorIdentity,
        prepared: &PreparedThreadMessage,
        stop: &AtomicBool,
    ) -> Result<SendOutcome, SendFailure> {
        let outcome = crate::interaction_hierarchy::send_reply_by_hierarchy(
            session,
            self.labels,
            self.screen,
            parent,
            &prepared.text,
            stop,
            || self.frame_sha(),
        )
        .await
        .map_err(SendFailure::after)?;
        match outcome {
            Ok(outcome) => Self::finish(outcome, prepared),
            // Every `ReplyRefusal` variant means nothing was typed — that is its
            // documented contract and every one of them is tested for it. Flattening them
            // into the same error as a `NotConfirmed` send would make a message that was
            // never posted permanently unretryable.
            Err(refusal) => Err(SendFailure::BeforeEffect(anyhow::anyhow!(
                "{}: {}",
                refusal.code(),
                refusal.message()
            ))),
        }
    }
}

/// Pick the driver for one assignment, from the session actually in hand.
///
/// `supports_element_bounds()` on the **session** is the runtime authority, and it is the
/// only thing consulted here. `DeviceControlPlane::reports_element_bounds` answers the
/// same question without a session and is what the pre-flight gates use; the two are
/// separate answers from the same driver and no attempt is made to cross-check them,
/// because there is nothing useful to do with a disagreement at this point — the session
/// is the thing about to be driven.
///
/// It refuses, before any tap, when this build has no measured label set **and** when it
/// has no measured Send control: without the latter there is nothing to aim at, and
/// finding that out after opening the drawer leaves the phone sitting in it.
async fn choose_target_driver<'a>(
    engine: &'a crate::NurtureEngine,
    udid: &'a str,
    session: &dyn crate::UiSession,
    target_package: &'a str,
    gestures: &'a tokio::sync::Mutex<()>,
) -> anyhow::Result<Box<dyn TargetDriver + 'a>> {
    if !session.supports_element_bounds() {
        return Ok(Box::new(PixelTargetDriver {
            engine,
            udid,
            target_package,
            gestures,
        }));
    }
    let language = session.ui_language().await.unwrap_or_default();
    let app_version = session
        .app_version(target_package)
        .await
        .unwrap_or_default();
    let labels = crate::tiktok_labels::controls_for(target_package, &language, &app_version)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "chưa đo nhãn cho {target_package} + ngôn ngữ {language:?}; từ chối thay vì \
                 dùng chuỗi của ngôn ngữ khác (nhãn nào silently không khớp thì đọc thành \
                 'không có control đó')"
            )
        })?;
    // The Send control is keyed by app *version*, so a build whose translations are
    // catalogued can still have no Send id — which is exactly what a TikTok update
    // produces. Refusing here rather than inside the drawer is the difference between
    // "nothing happened" and "the phone is sitting in an open comment drawer".
    if labels
        .label(crate::tiktok_labels::TikTokControl::CommentSend)
        .is_none()
    {
        anyhow::bail!(
            "chưa đo nút Gửi cho {target_package} phiên bản {app_version:?} — resource id của \
             nó bị gán lại mỗi lần app rebuild nên không dùng được id của bản khác. Chạy \
             `probe --measure-comment` trên máy này rồi thêm một entry TIKTOK_RESOURCE_SETS."
        );
    }
    let screen = session
        .window_size()
        .await
        .context("đọc kích thước màn hình để lập kế hoạch tap")?;
    Ok(Box::new(HierarchyTargetDriver {
        engine,
        udid,
        target_package,
        labels,
        screen,
    }))
}

/// The gesture used to hunt for the parent. How *far* to hunt is
/// `PARENT_SCROLL_ATTEMPTS`, shared with the hierarchy path — the budget was measured
/// against TikTok's comment list, which is the same list whichever path is scrolling it.
const PARENT_SCROLL_FROM_Y: f64 = 0.62;
const PARENT_SCROLL_TO_Y: f64 = 0.38;
const PARENT_SCROLL_SETTLE: Duration = Duration::from_millis(900);

/// Scroll the comment list until the parent is on screen.
///
/// Each scroll has to prove it happened, and here the proof is cheap in a way it
/// never was for the feed: the drawer is static, so a frame that changed is a
/// list that moved. (In the feed the opposite held — a playing video changed
/// every frame, which is what made the old swipe check worthless.) A swipe that
/// changes nothing means the list is at its end, and a frame that stops
/// classifying as an open drawer means the gesture closed it instead of
/// scrolling it; both stop the hunt rather than swiping on blindly.
async fn scroll_to_parent(
    engine: &crate::NurtureEngine,
    udid: &str,
    session: &dyn crate::UiSession,
    gestures: &tokio::sync::Mutex<()>,
    identity: &CommentLocatorIdentity,
    screen_size: (f64, f64),
    first: Vec<CommentOcrObservation>,
) -> anyhow::Result<Vec<CommentOcrObservation>> {
    if locate_parent_comment(&first, identity).is_some() {
        return Ok(first);
    }
    for attempt in 1..=PARENT_SCROLL_ATTEMPTS {
        let before = engine
            .frames
            .latest(udid)
            .context("no frame before scrolling the comment list")?;
        {
            let _guard = gestures.lock().await;
            session
                .swipe(crate::SwipeGesture {
                    from: TapPoint {
                        x: screen_size.0 * 0.5,
                        y: screen_size.1 * PARENT_SCROLL_FROM_Y,
                    },
                    to: TapPoint {
                        x: screen_size.0 * 0.5,
                        y: screen_size.1 * PARENT_SCROLL_TO_Y,
                    },
                    duration_ms: 320,
                })
                .await
                .context("scroll the comment list")?;
        }
        tokio::time::sleep(PARENT_SCROLL_SETTLE).await;

        let after = engine
            .frames
            .latest(udid)
            .context("no frame after scrolling the comment list")?;
        if crate::frame_sha256(&after) == crate::frame_sha256(&before) {
            anyhow::bail!(
                "cuộn {attempt} lần nhưng danh sách bình luận không nhúc nhích —                  nhiều khả năng đã hết danh sách"
            );
        }
        let decoded = image::load_from_memory(&after)
            .context("decode comment drawer frame")?
            .to_rgb8();
        if matches!(
            crate::screen::comment_drawer_state(&decoded).0,
            crate::screen::CommentDrawer::Closed | crate::screen::CommentDrawer::Unknown
        ) {
            anyhow::bail!("cú vuốt đã đóng khay bình luận thay vì cuộn nó");
        }

        let observations = engine.frame_text.recognize(&after).await?;
        if locate_parent_comment(&observations, identity).is_some() {
            return Ok(observations);
        }
    }
    anyhow::bail!("không thấy bình luận cha sau {PARENT_SCROLL_ATTEMPTS} lần cuộn danh sách")
}

async fn stable_parent_match(
    engine: &crate::NurtureEngine,
    udid: &str,
    identity: &CommentLocatorIdentity,
    first: Vec<CommentOcrObservation>,
) -> anyhow::Result<crate::CommentParentMatch> {
    let first_match =
        locate_parent_comment(&first, identity).context("parent/reply control không khớp")?;
    tokio::time::sleep(Duration::from_millis(700)).await;
    let second_frame = engine
        .frames
        .latest(udid)
        .context("parent second frame missing")?;
    let second = engine.frame_text.recognize(&second_frame).await?;
    let second_match = locate_parent_comment(&second, identity).context("parent không ổn định")?;
    if (first_match.reply_x - second_match.reply_x).abs() > 0.04
        || (first_match.reply_y - second_match.reply_y).abs() > 0.04
    {
        anyhow::bail!("reply control thay đổi giữa hai frame")
    }
    Ok(first_match)
}

async fn dismiss_comment_drawer(session: &dyn crate::UiSession, gestures: &tokio::sync::Mutex<()>) {
    let screen_size = session.window_size().await.unwrap_or((375.0, 667.0));
    let _guard = gestures.lock().await;
    let _ = session
        .tap(TapPoint {
            x: screen_size.0 * crate::screen::DRAWER_DISMISS.0,
            y: screen_size.1 * crate::screen::DRAWER_DISMISS.1,
        })
        .await;
}

/// `interaction_cancel` only flips the campaign row, so the running worker has
/// to read it back to notice. Nothing else signals it.
fn campaign_is_cancelled(db: &crate::db::Database, campaign_id: &str) -> anyhow::Result<bool> {
    Ok(matches!(
        db.get_interaction_campaign(campaign_id)?
            .map(|detail| detail.summary.state),
        Some(ThreadCampaignState::Cancelled)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Who actually needs the AI key.
    mod ai_key {
        use super::*;
        use crate::interaction::{ResolvedTikTokTarget, ThreadMode, TikTokPostKind};

        fn request(manual_comments: Vec<String>) -> ThreadCampaignRequest {
            ThreadCampaignRequest {
                request_id: "ai-key".into(),
                targets: vec![ResolvedTikTokTarget {
                    original_url: "https://www.tiktok.com/@creator/photo/1".into(),
                    normalized_url: "https://www.tiktok.com/@creator/photo/1".into(),
                    target_key: "content:1".into(),
                    content_id: "1".into(),
                    author: "creator".into(),
                    kind: TikTokPostKind::Photo,
                }],
                actor_udids: vec!["a".into(), "b".into()],
                message_count: 2,
                instruction: "tự nhiên".into(),
                max_words: 12,
                manual_comments,
                like_target: false,
                mode: ThreadMode::Standalone,
                shape: crate::interaction::ThreadShape::Star,
                cohort_size: None,
                mentions: Vec::new(),
            }
        }

        #[test]
        fn a_manual_campaign_runs_without_an_ai_key() {
            // The operator typed every comment already. Refusing here was refusing to use
            // the work they had done, and it happened after the campaign row said Running.
            let manual = request(vec!["đẹp quá".into(), "chỗ này ở đâu ạ".into()]);
            assert!(!ai_key_missing(&manual, ""));
            assert!(!ai_key_missing(&manual, "   "));
        }

        #[test]
        fn an_ai_campaign_still_refuses_without_a_key() {
            // The gate still has to close where it matters, and whitespace is not a key.
            let ai = request(Vec::new());
            assert!(ai_key_missing(&ai, ""));
            assert!(ai_key_missing(&ai, "  \t "));
            assert!(!ai_key_missing(&ai, "sk-real"));
        }
    }

    /// The states a target-level failure must not touch.
    mod protected_deliveries {
        use super::*;
        use crate::InteractionAssignmentRecord;

        fn assignment(id: &str, state: ThreadMessageState) -> InteractionAssignmentRecord {
            InteractionAssignmentRecord {
                id: id.to_string(),
                target_key: "content:1".to_string(),
                ordinal: 0,
                actor_udid: "udid".to_string(),
                parent_assignment_id: None,
                state,
                prepared_text: None,
                error_code: None,
                evidence_json: None,
                like: None,
                mention: None,
            }
        }

        #[test]
        fn a_target_failure_never_reopens_a_comment_that_is_already_public() {
            // The defect this guards was mine and it was subtle: the `only_assignments` guard
            // in the preparation loop stopped a retry overwriting `Succeeded`, and then
            // `fail_whole_target` overwrote it anyway from a different path. `Failed` is
            // retryable, so that erased the record of a live comment and let the next retry
            // post it a second time.
            let assignments = vec![
                assignment("done", ThreadMessageState::Succeeded),
                assignment("in-flight", ThreadMessageState::Sending),
                assignment("unproven", ThreadMessageState::Uncertain),
                assignment("waiting", ThreadMessageState::Queued),
                assignment("broken", ThreadMessageState::Failed),
            ];
            let protected = protected_assignment_ids(&assignments);

            // The same three `retryable_assignments` excludes, for the same reason.
            for settled in ["done", "in-flight", "unproven"] {
                assert!(
                    protected.contains(settled),
                    "{settled} must survive a target-level failure"
                );
            }
            // And the ones a retry is *for* stay writable, or a failed target could never be
            // recorded as failed at all.
            for retryable in ["waiting", "broken"] {
                assert!(
                    !protected.contains(retryable),
                    "{retryable} must stay writable"
                );
            }
        }
    }

    /// The two conditions an evidence frame has to satisfy, pinned without a device.
    mod evidence_frames {
        use super::*;
        use crate::{GenerationFrame, GenerationFrameStream};

        /// A hub that hands back exactly one frame, for one generation.
        struct FakeHub {
            generation: u64,
            sequence: u64,
        }

        impl crate::FrameSource for FakeHub {
            fn subscribe(&self, _udid: &str) -> Box<dyn crate::FrameStream> {
                unimplemented!("evidence uses the latest read, never a subscription")
            }

            /// Deliberately answers even when the generation-qualified read would not.
            ///
            /// That asymmetry *is* the bug this guards: the unqualified read is what the
            /// campaign used to call, and it happily returns a dead producer's bytes.
            fn latest(&self, _udid: &str) -> Option<crate::Frame> {
                Some(std::sync::Arc::new(vec![0xff, 0xd8, 0xff]))
            }
        }

        impl crate::GenerationFrameSource for FakeHub {
            fn subscribe_generation(
                &self,
                _udid: &str,
                _generation: u64,
            ) -> Box<dyn GenerationFrameStream> {
                unimplemented!("evidence uses the latest read, never a subscription")
            }

            fn latest_in_generation(
                &self,
                _udid: &str,
                generation: u64,
            ) -> Option<GenerationFrame> {
                // A real hub answers `None` once the generation has moved on; anything else
                // would hand out a dead producer's bytes.
                (generation == self.generation).then(|| GenerationFrame {
                    generation: self.generation,
                    sequence: self.sequence,
                    bytes: std::sync::Arc::new(vec![0xff, 0xd8, 0xff]),
                })
            }
        }

        #[test]
        fn an_evidence_frame_from_before_the_send_is_not_published_as_proof_of_it() {
            // The watermark is the frame that was current when Send was tapped. A frame at
            // or below it shows the screen *without* the comment, and filing that as proof
            // of the comment is worse than filing nothing — it looks checkable and is not.
            let hub = FakeHub {
                generation: 7,
                sequence: 42,
            };
            assert!(evidence_frame_after(&hub, "udid", 7, Some(42)).is_none());
            assert!(evidence_frame_after(&hub, "udid", 7, Some(99)).is_none());
            assert_eq!(
                evidence_frame_after(&hub, "udid", 7, Some(41)).map(|frame| frame.sequence),
                Some(42)
            );
        }

        #[test]
        fn an_evidence_frame_is_refused_once_the_stream_generation_has_advanced() {
            // Measured on this farm: the hub will return stale bytes for a producer that has
            // already died (`last_frame_age_ms=11373` with the sequence unmoved), and
            // `FrameSource::latest` promises nothing about liveness. Asking for a specific
            // generation is what makes the answer mean something.
            let hub = FakeHub {
                generation: 7,
                sequence: 42,
            };
            assert!(evidence_frame_after(&hub, "udid", 8, None).is_none());
            assert!(evidence_frame_after(&hub, "udid", 6, None).is_none());
        }

        #[test]
        fn with_no_watermark_any_frame_of_the_right_generation_is_admissible() {
            // The pre-send seed case: a refused arrival files whatever explains it, and
            // there is no "after the send" to be newer than.
            let hub = FakeHub {
                generation: 3,
                sequence: 1,
            };
            assert!(evidence_frame_after(&hub, "udid", 3, None).is_some());
        }
    }
    use crate::InteractionAssignmentRecord;

    fn assignment(id: &str, ordinal: u8, state: ThreadMessageState) -> InteractionAssignmentRecord {
        InteractionAssignmentRecord {
            id: id.into(),
            target_key: "content:1".into(),
            ordinal,
            actor_udid: format!("actor-{ordinal}"),
            parent_assignment_id: None,
            state,
            prepared_text: Some("nội dung".into()),
            error_code: None,
            evidence_json: None,
            like: None,
            mention: None,
        }
    }

    /// The state a broken chain actually leaves behind: one message posted, the
    /// rest skipped. Retry used to refuse the whole campaign on sight of that
    /// single `Succeeded`, so the most common failure was also the one that
    /// could never be repaired.
    #[test]
    fn a_posted_message_is_never_re_sent_but_the_skipped_ones_are_retryable() {
        let assignments = vec![
            assignment("a0", 0, ThreadMessageState::Succeeded),
            assignment("a1", 1, ThreadMessageState::SkippedParent),
            assignment("a2", 2, ThreadMessageState::SkippedParent),
        ];

        let retryable = retryable_assignments(&assignments, None);

        assert!(
            !retryable.contains("a0"),
            "tapping Send is not idempotent — a delivered comment must never be re-sent"
        );
        assert_eq!(retryable.len(), 2);
        assert!(retryable.contains("a1") && retryable.contains("a2"));
    }

    /// Delivery is exactly what `Uncertain` does not establish, and `Sending`
    /// is still in flight. Both stay out for the same reason as `Succeeded`.
    #[test]
    fn unproven_and_in_flight_deliveries_are_excluded() {
        let assignments = vec![
            assignment("u", 0, ThreadMessageState::Uncertain),
            assignment("s", 1, ThreadMessageState::Sending),
            assignment("f", 2, ThreadMessageState::Failed),
        ];

        let retryable = retryable_assignments(&assignments, None);

        assert_eq!(retryable.len(), 1);
        assert!(retryable.contains("f"));
    }

    /// Naming assignments narrows the set; it can never widen it past the
    /// safety filter above.
    #[test]
    fn naming_an_already_posted_assignment_does_not_authorise_re_sending_it() {
        let assignments = vec![
            assignment("a0", 0, ThreadMessageState::Succeeded),
            assignment("a1", 1, ThreadMessageState::SkippedParent),
        ];
        let requested: std::collections::HashSet<String> =
            ["a0".to_string(), "a1".to_string()].into_iter().collect();

        let retryable = retryable_assignments(&assignments, Some(&requested));

        assert_eq!(retryable.len(), 1);
        assert!(retryable.contains("a1"));
    }

    /// Thirteen messages still queued is not a success, however well the fourteenth went.
    ///
    /// The reachable sequence: run fourteen assignments with no AI key, `run_cohort` bails and
    /// all fourteen stay `Queued`, campaign `Failed` — correct. Configure the key and retry
    /// **one** row, which the detail view offers per row. `only_assignments` filters, so the
    /// other thirteen are never touched and stay `Queued`. The one succeeds. `failed == 0` and
    /// no error, so the old arithmetic wrote `Succeeded` — thirteen comments never sent, and
    /// Thử lại is drawn only for partial/failed/cancelled, so the campaign left the UI.
    #[test]
    fn a_subset_retry_never_settles_a_campaign_that_still_has_work() {
        let mut assignments = vec![assignment("a0", 0, ThreadMessageState::Succeeded)];
        for ordinal in 1..14u8 {
            assignments.push(assignment(
                &format!("a{ordinal}"),
                ordinal,
                ThreadMessageState::Queued,
            ));
        }

        let tally = tally_assignments(&assignments);

        assert_eq!(tally.succeeded, 1);
        assert_eq!(tally.failed, 0, "nothing is in a terminal-bad state…");
        assert_eq!(tally.outstanding, 13, "…and yet thirteen never went");
        assert_eq!(
            settle_state(tally, false),
            ThreadCampaignState::Partial,
            "a campaign with unsent messages is not a success"
        );
    }

    /// Every non-terminal message state counts as outstanding, by name and not by catch-all.
    ///
    /// Listed one at a time so that adding a state to `ThreadMessageState` makes the compiler
    /// ask where it belongs. A catch-all arm is what silently swallowed four of them.
    #[test]
    fn every_state_lands_in_exactly_one_bucket() {
        let tally = tally_assignments(&[
            assignment("q", 0, ThreadMessageState::Queued),
            assignment("p", 1, ThreadMessageState::Preparing),
            assignment("r", 2, ThreadMessageState::Ready),
            assignment("s", 3, ThreadMessageState::Sending),
            assignment("ok", 4, ThreadMessageState::Succeeded),
            assignment("f", 5, ThreadMessageState::Failed),
            assignment("u", 6, ThreadMessageState::Uncertain),
            assignment("k", 7, ThreadMessageState::SkippedParent),
        ]);
        assert_eq!(tally.outstanding, 4);
        assert_eq!(tally.succeeded, 1);
        assert_eq!(tally.failed, 3);
    }

    /// The three verdicts a settled campaign can carry.
    #[test]
    fn a_settled_campaign_gets_the_verdict_its_rows_imply() {
        let all_done = CampaignTally {
            succeeded: 14,
            failed: 0,
            outstanding: 0,
        };
        assert_eq!(
            settle_state(all_done, false),
            ThreadCampaignState::Succeeded
        );
        // An error the run reported keeps it out of `Succeeded` even with clean rows: the
        // failure may be the one that stopped a cohort before its rows were written.
        assert_eq!(settle_state(all_done, true), ThreadCampaignState::Partial);
        let nothing_posted = CampaignTally {
            succeeded: 0,
            failed: 14,
            outstanding: 0,
        };
        assert_eq!(
            settle_state(nothing_posted, true),
            ThreadCampaignState::Failed
        );
        let some_posted = CampaignTally {
            succeeded: 5,
            failed: 9,
            outstanding: 0,
        };
        assert_eq!(
            settle_state(some_posted, true),
            ThreadCampaignState::Partial
        );
    }
}

#[cfg(test)]
mod boundary_tests {
    /// The body of one `#[tauri::command]` in the desktop wrapper, for the two tests below.
    ///
    /// They are source scans because what they pin is an *ordering* inside a function that
    /// needs a Tauri `State`, a database, a control plane and a spawned worker to call. The
    /// ordering is the whole fix, and a test that cannot run is worse than a crude one that
    /// can.
    /// A refusal from the arrival check is never turned into a success.
    ///
    /// `interaction_measure_post` retries once, and only when the refusal is
    /// `target_open_screen_unchanged` — the ordinary case of pressing Đo bài twice, because the
    /// first press leaves the phone on the post and the check decides it arrived by watching the
    /// author label *change*. The retry is a real second arrival: the phone is stepped off the
    /// post and asked again.
    ///
    /// What this pins is the tempting shortcut: matching that code and carrying on as though the
    /// phone were there. The arrival gate is what keeps a campaign from commenting under the
    /// wrong post, and a *measurement* that bypassed it would be the first crack in it.
    #[test]
    fn measuring_a_post_never_swallows_an_arrival_refusal() {
        let body = command_body("interaction_measure_post");
        assert_eq!(
            body.matches("open_target_by_hierarchy").count(),
            2,
            "one arrival and one retry: a single call is no retry, a third is a loop"
        );
        // The **comparison**, not the bare string: the code is named in the comment above it
        // too, and counting mentions counts prose.
        assert_eq!(
            body.matches(r#"refusal.code() == "target_open_screen_unchanged""#)
                .count(),
            1,
            "the code is compared once, to decide whether to retry — nothing else"
        );
        assert!(
            body.contains("arrival.map_err("),
            "whatever the second arrival said still has to reach the operator"
        );
    }

    fn command_body(name: &str) -> &'static str {
        let desktop = include_str!("../../../apps/desktop/src-tauri/src/interaction_commands.rs");
        let start = desktop
            .find(&format!("pub fn {name}("))
            .or_else(|| desktop.find(&format!("pub async fn {name}(")))
            .unwrap_or_else(|| panic!("{name} must exist in the wrapper"));
        let rest = &desktop[start..];
        let end = rest[1..]
            .find("\n#[tauri::command]")
            .map(|at| at + 1)
            .unwrap_or(rest.len());
        &rest[..end]
    }

    /// Every state a message passes through has to reach the operator.
    ///
    /// The Monitor tab cannot show progress it is never told about, and the runner used to
    /// tell it only after a send reached a verdict — so the whole preparation phase, which is
    /// the slow one because the AI is writing, looked motionless, and a campaign that failed
    /// before any send ran was silent until the final tally.
    ///
    /// Counted rather than observed because `run_cohort` needs a device to call. The rule it
    /// encodes: a write of an assignment state that no `notify` follows is a change the
    /// operator cannot see.
    #[test]
    fn every_assignment_transition_in_the_runner_is_followed_by_a_notify() {
        let source = include_str!("interaction_campaign.rs");
        let runner = {
            let start = source
                .find("async fn run_cohort(")
                .expect("the runner still exists");
            let rest = &source[start..];
            let end = rest.find("\n#[cfg(test)]").unwrap_or(rest.len());
            &rest[..end]
        };
        // The rule is about the paths that *leave* — a write followed by `continue` skips
        // the emit at the bottom of the loop, and that is exactly the set that was silent.
        // The two terminal writes (a send succeeded, a send failed) are deliberately not in
        // it: they are the two arms of one attempt and the emit after the teardown covers
        // whichever ran, so demanding one each would double every send.
        let mut silent = Vec::new();
        for marker in [
            "update_interaction_assignment_state",
            "prepare_interaction_assignment",
            "fail_whole_target(",
        ] {
            let mut from = 0;
            while let Some(offset) = runner[from..].find(marker) {
                let at = from + offset;
                from = at + marker.len();
                let tail = &runner[at..];
                let Some(leaves) = tail.find("continue;") else {
                    continue;
                };
                if !tail[..leaves].contains("notify(&events") {
                    silent.push(format!("{marker} on line {}", runner[..at].lines().count()));
                }
            }
        }
        assert!(
            silent.is_empty(),
            "these writes change an assignment and then leave without telling the \
             frontend: {silent:?}"
        );
    }

    /// A retry that is going to be refused must leave the campaign exactly as it found it.
    ///
    /// It did not: `Queued` was written before the request could be re-read and before
    /// `require_parent_locator` could refuse, so a refused retry parked the campaign in a
    /// state the Monitor draws no buttons for — not Cancel (running only), not Retry
    /// (partial/failed/cancelled only). Asking to repair a campaign made it unreachable.
    #[test]
    fn a_refused_retry_never_touches_the_campaign_state() {
        let body = command_body("interaction_retry");
        let first_write = body
            .find("update_interaction_campaign_state")
            .expect("retry still writes the campaign state");
        for refusal in [
            "get_interaction_campaign_request",
            "require_parent_locator",
            "ai_key_missing",
            "RetryNotAllowed",
        ] {
            let at = body.find(refusal).unwrap_or_else(|| {
                panic!("{refusal} must still be a refusal in interaction_retry")
            });
            assert!(
                at < first_write,
                "{refusal} can refuse after the campaign state was already moved"
            );
        }
    }

    /// A retry whose worker dies has to say so, the way the start path always has.
    ///
    /// The worker writes `Failed` either way; without the emit the Monitor keeps showing
    /// "Đang chạy" until some unrelated event happens to refresh it, and the operator waits
    /// on a campaign that ended.
    #[test]
    fn a_retry_that_dies_in_the_worker_still_tells_the_frontend() {
        for command in ["interaction_retry", "interaction_start_thread"] {
            let emits = command_body(command)
                .matches("AppEvent::InteractionUpdated")
                .count();
            assert_eq!(
                emits, 2,
                "{command} must emit both when it accepts the work and when its worker dies"
            );
        }
    }

    /// The engine has one home, and the tuning number that proves it has one definition.
    ///
    /// This is the test that would have caught the bug that motivated the move.
    /// `PARENT_SCROLL_ATTEMPTS` existed twice — 10 here, 4 in the desktop crate — because the
    /// engine was split across two crates, so a commit that measured the correct value could
    /// land it in one half and leave the other running the number it had just disproved. It
    /// stayed that way for three days. Two definitions of one tuning number is not a style
    /// problem; it is two different machines.
    #[test]
    fn the_scroll_budget_is_defined_once_in_the_whole_workspace() {
        const SOURCES: &[(&str, &str)] = &[
            (
                "core/interaction_hierarchy.rs",
                include_str!("interaction_hierarchy.rs"),
            ),
            (
                "core/interaction_campaign.rs",
                include_str!("interaction_campaign.rs"),
            ),
            (
                "desktop/interaction_commands.rs",
                include_str!("../../../apps/desktop/src-tauri/src/interaction_commands.rs"),
            ),
        ];
        let definitions: Vec<&str> = SOURCES
            .iter()
            .filter(|(_, src)| {
                src.lines().any(|l| {
                    l.trim_start().starts_with("const PARENT_SCROLL_ATTEMPTS")
                        || l.trim_start()
                            .starts_with("pub const PARENT_SCROLL_ATTEMPTS")
                })
            })
            .map(|(name, _)| *name)
            .collect();
        assert_eq!(
            definitions,
            vec!["core/interaction_hierarchy.rs"],
            "PARENT_SCROLL_ATTEMPTS is defined in more than one place again"
        );
    }

    /// The desktop half stays a set of wrappers.
    ///
    /// The point of the move is not that the file got shorter — it is that campaign logic now
    /// has nowhere to accumulate on the Tauri side. So the wrapper file must reach the engine
    /// only through the handful of entry points it imports, and must not name the engine's
    /// internals. Naming one of them is how a "small exception" starts.
    #[test]
    fn the_command_wrappers_do_not_reach_into_the_engine() {
        let desktop = include_str!("../../../apps/desktop/src-tauri/src/interaction_commands.rs");
        // Newline-agnostic for the same reason as the twin scan in `lib.rs`: with
        // `core.autocrlf=true` a checked-out file is CRLF, an LF needle misses, and the cut
        // silently does not happen. This one only survived that by luck — every function in the
        // test module is indented and the column-zero check below skips indented lines — but
        // luck is not a boundary.
        let cut = desktop
            .match_indices("#[cfg(test)]")
            .find(|(at, marker)| {
                desktop[at + marker.len()..]
                    .trim_start_matches(['\r', '\n'])
                    .starts_with("mod ")
            })
            .map(|(at, _)| at);
        let body = match cut {
            Some(at) => &desktop[..at],
            None => desktop,
        };
        for internal in [
            "SendFailure",
            "SendOutcome",
            "TargetDriver",
            // Precise: `interaction_ocr::recognizer_language()` is a different call and a
            // legitimate one -- it tells the operator which OCR language pack is installed.
            "interaction_ocr::recognize(",
            "run_cohort",
            "PixelTargetDriver",
            "HierarchyTargetDriver",
        ] {
            assert!(
                !body.contains(internal),
                "`{internal}` is engine internals and the wrapper file names it again"
            );
        }
    }

    /// And the wrappers are wrappers: every free function there is a command.
    ///
    /// A helper is where the next 1,712 lines would start. The two that remain are named, so
    /// adding a third is a decision someone has to write down rather than a drift.
    #[test]
    fn nothing_but_commands_and_two_named_helpers_live_beside_them() {
        const ALLOWED_HELPERS: &[(&str, &str)] = &[
            (
                "interaction_error",
                "shapes one CommandError for this module",
            ),
            (
                "require_parent_locator",
                "argument validation for the two thread commands",
            ),
        ];
        let desktop = include_str!("../../../apps/desktop/src-tauri/src/interaction_commands.rs");
        // Newline-agnostic for the same reason as the twin scan in `lib.rs`: with
        // `core.autocrlf=true` a checked-out file is CRLF, an LF needle misses, and the cut
        // silently does not happen. This one only survived that by luck — every function in the
        // test module is indented and the column-zero check below skips indented lines — but
        // luck is not a boundary.
        let cut = desktop
            .match_indices("#[cfg(test)]")
            .find(|(at, marker)| {
                desktop[at + marker.len()..]
                    .trim_start_matches(['\r', '\n'])
                    .starts_with("mod ")
            })
            .map(|(at, _)| at);
        let body = match cut {
            Some(at) => &desktop[..at],
            None => desktop,
        };
        let mut stray = Vec::new();
        let lines: Vec<&str> = body.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let t = line.trim_start();
            let is_fn = t.starts_with("fn ")
                || t.starts_with("pub fn ")
                || t.starts_with("async fn ")
                || t.starts_with("pub async fn ");
            if !is_fn || !line.starts_with(['f', 'p', 'a']) {
                continue;
            }
            // Walk back over the doc comment to find the attribute, if any.
            let mut j = i;
            let mut is_command = false;
            while j > 0 {
                j -= 1;
                let prev = lines[j].trim();
                if prev.starts_with("///") || prev.is_empty() {
                    continue;
                }
                is_command = prev.starts_with("#[tauri::command");
                break;
            }
            if is_command {
                continue;
            }
            let name = t
                .trim_start_matches("pub ")
                .trim_start_matches("async ")
                .trim_start_matches("fn ")
                .split(['(', '<'])
                .next()
                .unwrap_or("")
                .trim();
            if !ALLOWED_HELPERS.iter().any(|(n, _)| *n == name) {
                stray.push(name.to_string());
            }
        }
        assert!(
            stray.is_empty(),
            "engine logic is growing back in the wrapper file: {stray:?}"
        );
    }
}
