//! Campaign persistence and the live one-shot publish runtime adapter.

use super::*;
/// Why one phone did not post, and **whether anything may be live because of it**.
///
/// The empty-string sentinel this replaces was a real trap: the caller told a missing bundle
/// apart from every other failure by testing `reason.is_empty()`, so any failure whose message
/// happened to be empty became "bundle missing" — and, worse, the campaign could not tell a
/// phone that refused before opening anything from one that tapped Post and lost the answer.
pub(super) enum PhoneFailure {
    /// This campaign holds no bundle for this assignment. Nothing was opened.
    NoBundle,
    /// The run stopped and **nothing reached TikTok**.
    NothingPublished(String),
    /// A tap may have gone out and the result is unknown.
    MayBeLive(String),
}

/// One phone's whole posting attempt, from the permit to the state write.
///
/// **The cancel check is inside, before the claim.** A campaign the operator stopped must not
/// start another phone — and a phone already inside the composer must not be abandoned there,
/// which is why this is the only place it is checked.
#[allow(clippy::too_many_arguments)]
pub(super) async fn post_one_phone(
    stagger: Duration,
    gate: Arc<tokio::sync::Semaphore>,
    control: Arc<DeviceControlPlane>,
    db: Arc<Database>,
    frames: Arc<dyn FrameSource>,
    events: riviu_core::events::EventBus,
    campaign_id: String,
    assignment: riviu_core::PublishAssignmentRecord,
    bundle: riviu_core::PublishBundle,
    sound_policy: riviu_core::PublishSoundPolicy,
) -> Result<(), PhoneFailure> {
    tokio::time::sleep(stagger).await;
    let _permit = gate.acquire().await.map_err(|error| {
        PhoneFailure::NothingPublished(format!("{}: hết chỗ stream ({error})", assignment.udid))
    })?;

    // Read the operator's cancel **before** claiming, and only here. Stopping between phones
    // costs nothing and leaves every untouched assignment still `Imported`, which is where a
    // later run expects to find it. Stopping a phone already in the composer would produce the
    // `uncertain` state that can never be retried.
    //
    // A read that *fails* is not "not cancelled": stopping here costs nothing, and carrying on
    // because the database did not answer is how a stopped run keeps posting.
    match db.publish_campaign_state(&campaign_id) {
        Ok(Some(riviu_core::PublishCampaignState::Cancelled)) | Ok(None) => return Ok(()),
        Ok(Some(_)) => {}
        Err(error) => {
            return Err(PhoneFailure::NothingPublished(format!(
                "{}: không đọc được trạng thái chiến dịch ({error}) — dừng thay vì đăng tiếp",
                assignment.udid
            )))
        }
    }

    // The assignment CAS is deliberately *not* here. `post_one_assignment` gives the driver a
    // one-shot callback and the driver invokes it on the last line before tapping Post. A crash
    // anywhere between this point and that callback therefore leaves the row retryable; a crash
    // after it leaves `posting`, which startup reconciliation makes `uncertain`.
    let attempt = post_one_assignment(
        &control,
        &db,
        frames.as_ref(),
        &campaign_id,
        &assignment,
        &bundle,
        &sound_policy,
    )
    .await;
    if attempt.claim_refused {
        announce(&events, &db, &campaign_id);
        return Err(PhoneFailure::NothingPublished(match attempt.outcome {
            PostOutcome::NothingPublished(reason) | PostOutcome::Unknown(reason) => reason,
            PostOutcome::Posted(_) => format!(
                "{}: assignment claim was refused before Post",
                assignment.udid
            ),
        }));
    }
    let outcome = attempt.outcome;
    let (state, code) = state_for_outcome(&outcome);
    let (message, evidence) = match &outcome {
        PostOutcome::Posted(evidence) => (None, evidence.to_string()),
        PostOutcome::NothingPublished(reason) | PostOutcome::Unknown(reason) => (
            Some(reason.clone()),
            serde_json::json!({"message": reason, "effectIntent":"post_carousel"}).to_string(),
        ),
    };
    // A posted assignment whose evidence carries a link settles through the one-transaction
    // write: state **and** the sheet's obligation row go in together, or neither does — a
    // link recorded in evidence with no outbox row behind it is a debt the sweeper can never
    // see. Everything else (no link yet — which before the M7 measurement is every run — or
    // any non-posted outcome) keeps the plain state write.
    // **Keyed on the settled state, not merely on a link being present.**
    //
    // `record_publish_success_with_sheet_row` writes `succeeded` unconditionally, so routing
    // by "evidence carries a link" would let any future outcome that kept a captured link
    // while settling to something else — an `Unknown` that may be live, say — be recorded as
    // a clean success, erasing the one state this path treats as permanently unclaimable.
    // The two agree today (only `Posted` maps to `Succeeded`); this makes them agree by
    // construction rather than by coincidence.
    let owed = match &outcome {
        PostOutcome::Posted(value) if state == riviu_core::PublishCampaignState::Succeeded => {
            post_url_owed(value).map(str::to_string)
        }
        _ => None,
    };
    let written = match owed {
        Some(post_url) => db.record_publish_success_with_sheet_row(
            &assignment.id,
            &evidence,
            &campaign_id,
            &post_url,
            poster_identity(),
            &bundle.partners,
        ),
        None => db.update_publish_assignment_state(&assignment.id, state, code, Some(&evidence)),
    };
    if let Err(error) = written {
        // The state write failed *after* the attempt, so what the phone did is unknown to the
        // database whatever it did on screen.
        return Err(PhoneFailure::MayBeLive(format!(
            "{}: {error}",
            assignment.udid
        )));
    }
    announce(&events, &db, &campaign_id);
    match (message, &outcome) {
        (None, _) => Ok(()),
        (Some(reason), PostOutcome::NothingPublished(_)) => Err(PhoneFailure::NothingPublished(
            format!("{}: {reason}", assignment.udid),
        )),
        (Some(reason), _) => Err(PhoneFailure::MayBeLive(format!(
            "{}: {reason}",
            assignment.udid
        ))),
    }
}

#[tauri::command]
pub fn publish_auto_assign(
    state: State<'_, AppState>,
    source_root: String,
    udids: Vec<String>,
    wanted: usize,
) -> Result<riviu_core::publish::AutoAssignment, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let manifest = scan_publish_folder(PathBuf::from(source_root), PublishScanOptions::default())
        .map_err(err)?;
    let all: Vec<String> = manifest
        .bundles
        .iter()
        .map(|bundle| bundle.id.clone())
        .collect();
    let already = state.db.bundle_ids_already_dispatched().map_err(err)?;
    riviu_core::publish::auto_assign_bundles(&all, &already, &udids, wanted)
        .map_err(|error| err(anyhow::anyhow!("{error}")))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri exposes each wire field as a named command argument.
pub async fn publish_create_campaign(
    state: State<'_, AppState>,
    mut source_root: String,
    bundle_ids: Vec<String>,
    udids: Vec<String>,
    run_at: Option<String>,
    caption_overrides: Option<HashMap<String, String>>,
    sound_policy: Option<riviu_core::PublishSoundPolicy>,
    target_ref: Option<riviu_core::TargetRef>,
    confirmed: Option<bool>,
    approved_input_digest: String,
) -> Result<PublishCampaignRecord, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    source_root = source_root.trim().to_string();
    let sound_policy = sound_policy.unwrap_or_default();
    let confirmed = confirmed.unwrap_or(false);
    let preflight_request = riviu_core::PublishPreflightRequest {
        source_root: source_root.clone(),
        bundle_ids: bundle_ids.clone(),
        udids: udids.clone(),
        target_ref,
        run_at: run_at.clone(),
        caption_overrides: caption_overrides
            .clone()
            .unwrap_or_default()
            .into_iter()
            .collect(),
        sound_policy: sound_policy.clone(),
    };
    let prepared = build_publish_preflight(
        &state.control,
        &state.registry,
        &state.db,
        preflight_request,
    )
    .await
    .map_err(err)?;
    require_current_preflight_digest(&prepared.report, &approved_input_digest).map_err(err)?;
    if !prepared.report.can_execute {
        return Err(err(format!(
            "preflight từ chối chiến dịch: {}",
            prepared
                .report
                .issues
                .iter()
                .map(|issue| issue.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        )));
    }
    let selected = prepared.bundles;
    let request_id = Uuid::new_v4().to_string();
    let staging_root = state.artifacts_dir.join("publish").join(&request_id);
    let mut managed = Vec::with_capacity(selected.len());
    for bundle in selected {
        let source_bundle_id = bundle.id.clone();
        let destination = staging_root.join(&source_bundle_id);
        match copy_bundle_to_managed(&bundle, &destination) {
            Ok(mut bundle) => {
                // The scanner's stable id (for example, `bundle-1`) is useful
                // in the preview but the database keeps bundle ids globally
                // unique. Namespace the staged record by this campaign so a
                // later run of the same folder is independent of old runs.
                bundle.id = format!("{request_id}:{source_bundle_id}");
                managed.push(bundle);
            }
            Err(error) => {
                let _ = fs::remove_dir_all(&staging_root);
                return Err(err(error));
            }
        }
    }
    let managed_bundle_ids = managed.iter().map(|bundle| bundle.id.clone()).collect();
    let request = PublishCampaignRequest {
        request_id: request_id.clone(),
        source_root,
        bundle_ids: managed_bundle_ids,
        udids,
        // Trimmed here as well as in `parse_run_at`, or the scheduler re-parses the
        // untrimmed original and rejects a value this command just accepted — moving a
        // campaign the operator scheduled to `failed_before_dispatch` for a leading space.
        run_at: run_at.map(|value| value.trim().to_string()),
        visibility: PublishVisibility::Public,
        cleanup_policy: PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
        sound_policy,
        execution_confirmed: confirmed,
        target_snapshot: Some(prepared.report.target_snapshot.clone()),
    };
    let initial_snapshot = riviu_core::PublishExecutionSnapshotDraft {
        input_digest: prepared.report.input_digest.clone(),
        status: riviu_core::PublishExecutionStatus::Partial,
        retry_scope: riviu_core::PublishRetryScope::FullPipeline,
        report_json: serde_json::to_value(&prepared.report).map_err(err)?,
    };
    match state
        .db
        .create_publish_campaign_with_snapshot(&request, &managed, &initial_snapshot)
    {
        Ok(campaign) => {
            let _ = state.db.log_op("publish.campaign.create", &campaign.id);
            Ok(campaign)
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&staging_root);
            Err(err(error))
        }
    }
}

/// Apply the editor's caption snapshot to the selected manifest before its managed copy.
///
/// Keys are scanner/source bundle ids. The source `caption*.txt` is deliberately never written:
/// `copy_bundle_to_managed` materializes this updated value into the campaign-owned directory.
pub(super) fn apply_caption_overrides(
    bundles: &mut [riviu_core::PublishBundle],
    overrides: Option<&HashMap<String, String>>,
) -> anyhow::Result<()> {
    let Some(overrides) = overrides else {
        return Ok(());
    };
    for (bundle_id, caption) in overrides {
        anyhow::ensure!(
            bundles.iter().any(|bundle| bundle.id == *bundle_id),
            "caption override names an unselected bundle: {bundle_id}"
        );
        anyhow::ensure!(
            !caption.trim().is_empty(),
            "caption override for {bundle_id} is empty"
        );
    }
    for bundle in bundles {
        let Some(caption) = overrides.get(&bundle.id) else {
            continue;
        };
        bundle.caption = caption.trim().to_string();
        bundle.caption_sha256 = frame_sha256(bundle.caption.as_bytes());
    }
    Ok(())
}

#[tauri::command]
pub fn publish_list(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<PublishCampaignRecord>, CommandError> {
    state
        .db
        .list_publish_campaigns(limit.unwrap_or(50).clamp(1, 200))
        .map_err(err)
}

#[tauri::command]
pub fn publish_get(
    state: State<'_, AppState>,
    campaign_id: String,
) -> Result<Option<PublishCampaignDetail>, CommandError> {
    state.db.get_publish_campaign(&campaign_id).map_err(err)
}

#[tauri::command]
pub fn publish_reconcile(
    state: State<'_, AppState>,
    campaign_id: String,
) -> Result<riviu_core::PublishExecutionSnapshot, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    reconcile_publish_execution_and_announce(&state.db, &state.events, &campaign_id).map_err(err)
}

/// Commit one terminal projection before telling subscribers to re-read it.
///
/// The event is deliberately inside this helper rather than at its call sites. A failed save
/// therefore cannot advertise a state that does not exist yet, and every successful subscriber
/// wake-up observes at least the projection that caused it.
pub(super) fn persist_publish_snapshot_then_announce<T>(
    db: &Database,
    events: &riviu_core::events::EventBus,
    campaign_id: &str,
    persist: impl FnOnce() -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let persisted = persist()?;
    announce(events, db, campaign_id);
    Ok(persisted)
}

pub(super) fn persist_reconciled_publish_execution(
    db: &Database,
    campaign_id: &str,
) -> anyhow::Result<riviu_core::PublishExecutionSnapshot> {
    let previous = db.get_publish_execution_snapshot(campaign_id)?;
    let request = db
        .publish_campaign_request(campaign_id)?
        .context("publish campaign request not found")?;
    let detail = db
        .get_publish_campaign(campaign_id)?
        .context("publish campaign not found")?;
    // A campaign transition may happen after the last projection was written (for example, the
    // process can die after the Post effect-intent CAS). Never return that old snapshot blindly:
    // the typed campaign/assignment states are the durable authority after restart.
    let input_digest = previous
        .as_ref()
        .map(|snapshot| snapshot.input_digest.clone())
        .unwrap_or(stored_campaign_input_digest(&request, &detail)?);
    let (status, retry_scope) = db.reconciled_publish_execution_status(campaign_id)?;
    db.save_publish_execution_snapshot(
        campaign_id,
        &input_digest,
        status,
        retry_scope,
        &serde_json::json!({
            "campaignId": campaign_id,
            "status": status,
            "retryScope": retry_scope,
            "source": "typed_state_reconciliation",
            "targetSnapshot": request.target_snapshot,
        }),
    )
}

pub(crate) fn reconcile_publish_execution_and_announce(
    db: &Database,
    events: &riviu_core::events::EventBus,
    campaign_id: &str,
) -> anyhow::Result<riviu_core::PublishExecutionSnapshot> {
    persist_publish_snapshot_then_announce(db, events, campaign_id, || {
        persist_reconciled_publish_execution(db, campaign_id)
    })
}

pub(super) fn publish_reconciliation_identity(
    db: &Database,
    campaign_id: &str,
) -> anyhow::Result<(Option<String>, Option<riviu_core::ResolvedTargetSnapshot>)> {
    let Some(request) = db.publish_campaign_request(campaign_id)? else {
        // Sheet obligations deliberately outlive a deleted campaign. There is no projection to
        // refresh in that case, but the already-delivered row must still be allowed to settle.
        return Ok((None, None));
    };
    let input_digest = match db.get_publish_execution_snapshot(campaign_id)? {
        Some(snapshot) => snapshot.input_digest,
        None => {
            let detail = db
                .get_publish_campaign(campaign_id)?
                .context("publish campaign disappeared while deriving its input digest")?;
            stored_campaign_input_digest(&request, &detail)?
        }
    };
    Ok((Some(input_digest), request.target_snapshot))
}

pub(super) fn stored_campaign_input_digest(
    request: &PublishCampaignRequest,
    detail: &PublishCampaignDetail,
) -> anyhow::Result<String> {
    let stable = serde_json::json!({
        "schemaVersion": 1,
        "request": request,
        "bundles": detail.bundles,
        "targets": detail.assignments.iter().map(|assignment| serde_json::json!({
            "ordinal": assignment.ordinal,
            "bundleId": assignment.bundle_id,
            "udid": assignment.udid,
        })).collect::<Vec<_>>(),
    });
    Ok(frame_sha256(&serde_json::to_vec(&stable)?))
}

pub(super) fn publish_execution_report(
    request: &PublishCampaignRequest,
    result: &riviu_core::PublishCampaignExecutionResult,
) -> anyhow::Result<serde_json::Value> {
    Ok(serde_json::json!({
        "targetSnapshot": request.target_snapshot,
        "result": serde_json::to_value(result)?,
    }))
}

#[tauri::command]
pub fn publish_cancel(state: State<'_, AppState>, campaign_id: String) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    // The database refuses to cancel a terminal campaign (see
    // `Database::cancel_publish_campaign` for the two release bugs that closes); this layer's
    // job is to tell the operator which answer they got instead of pretending both are "done".
    match state
        .db
        .cancel_publish_campaign(&campaign_id)
        .map_err(err)?
    {
        Some(riviu_core::PublishCampaignState::Cancelled) => {
            reconcile_publish_execution_and_announce(&state.db, &state.events, &campaign_id)
                .map_err(err)?;
            Ok(())
        }
        Some(actual) => Err(err(format!(
            "không huỷ được: chiến dịch đang ở {actual:?}. Một chiến dịch đã đăng xong hoặc \
             không xác nhận được thì phải giữ nguyên trạng thái đó — huỷ nó là thả các bài \
             đã lên ngược về kho chia tự động."
        ))),
        None => Err(err("publish campaign not found")),
    }
}

/// Whether some phone may already hold this assignment's post — or the media that becomes one.
///
/// The set a transfer must leave alone: re-staging a `succeeded` row rebuilds nothing (the
/// import is idempotent) but re-*preflighting* it can block the whole campaign, and touching
/// `posting`/`verifying`/`uncertain` rows is how a retry walks into a run that may be live.
/// The same four states the auto-deal pool reserves on, for the same reason.
pub(super) fn assignment_may_hold_the_post(state: &riviu_core::PublishCampaignState) -> bool {
    matches!(
        state,
        riviu_core::PublishCampaignState::Succeeded
            | riviu_core::PublishCampaignState::Posting
            | riviu_core::PublishCampaignState::Verifying
            | riviu_core::PublishCampaignState::Uncertain
    )
}

/// The link a posted assignment owes the sheet, when its evidence carries one.
///
/// Pure, because it is the fork in the road below: `Some` routes the settle through
/// `record_publish_success_with_sheet_row` — state and outbox row in one transaction — and
/// `None` through the plain state write. An empty or whitespace `postUrl` is `None`: the
/// outbox schema refuses blank links (migration 18's CHECK), and a post whose link was
/// never read owes the sheet nothing yet.
///
/// # It reads through the fold, and the first version did not
///
/// By the time this runs, the evidence has been through [`fold_cleanup_into`], which wraps a
/// posted outcome as `{"post": …, "cleanup": …}` — so the link the composer wrote at the top
/// level lives one layer down. The first version looked only at the top level, which meant
/// the `Some` arm was **unreachable**: a captured link would have been recorded in
/// `evidence_json` and never written to the outbox, leaving a debt the sweeper cannot see —
/// exactly the failure the one-transaction write exists to prevent. Nothing caught it because
/// no path sets `postUrl` yet (see the `Posted` arm's note on the M7 route) and the test fed
/// this function **unfolded** evidence. Found by an independent review on 31/08/2026; the
/// test now folds through the real function, so the two shapes cannot drift apart again.
///
/// The unfolded level stays readable because the fold is one caller's shape, not this
/// function's contract, and an outcome that reaches here unfolded is still owed.
pub(super) fn post_url_owed(evidence: &serde_json::Value) -> Option<&str> {
    evidence
        .get("post")
        .and_then(|post| post.get("postUrl"))
        .or_else(|| evidence.get("postUrl"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|url| riviu_core::tiktok_share::looks_like_a_post_link(url))
}

/// Who the sheet says posted. Always `bot`, and the sheet is why.
///
/// # This was the device's handle for one day, on a guess that the sheet refuted
///
/// The handle version shipped on 31/08 with a confident reason: "twenty accounts publish
/// through this app, and a column that always reads `bot` cannot tell the operator whose
/// post a row is." Then the operator's real sheet was read, and column B is **`Nhân Viên`** —
/// a staff column, holding eleven people's names across 1892 rows (`Phúc`, `Lành`, `Quỳnh`,
/// …). It answers *who did this*, not *which account*. So `bot` is not a placeholder there;
/// it is the twelfth value, and it is the one that tells a human at a glance which rows a
/// person posted and which the app did.
///
/// Which account a row belongs to is not lost by this: the canonical link in column D
/// carries `@handle` in its own path. (Column E `Tên Kênh` exists and is empty on all 1892
/// rows — filling it is the operator's call, not this function's.)
///
/// Kept as a named function rather than inlined so the decision has one home, and so the
/// `.gs`'s own `payload.poster || 'bot'` fallback stays a second belt rather than the only
/// one. Migration 18's CHECK refuses a blank poster; this can never produce one.
pub(super) const fn poster_identity() -> &'static str {
    "bot"
}

/// Whether this assignment's carousel is already up, settled, done.
///
/// The post fan-out used to run every assignment and count the claim's "already succeeded"
/// refusal as a failure — so a retry that finished the one remaining phone still ended the
/// campaign `failed_before_dispatch`, the Transfer button came back, and the loop never
/// closed. A settled row is not a participant: it is the part of the campaign that already
/// went right.
pub(super) fn assignment_already_posted(state: &riviu_core::PublishCampaignState) -> bool {
    matches!(state, riviu_core::PublishCampaignState::Succeeded)
}

pub(super) fn record_transfer_write_ahead(
    db: &Database,
    assignment_id: &str,
) -> anyhow::Result<()> {
    db.update_publish_assignment_state(
        assignment_id,
        riviu_core::PublishCampaignState::Transferring,
        None,
        None,
    )
    .with_context(|| format!("record transfer write-ahead for {assignment_id}"))
}

pub(crate) async fn transfer_publish_campaign_inner(
    control: Arc<DeviceControlPlane>,
    db: Arc<Database>,
    events: riviu_core::events::EventBus,
    agent_bundle_id: String,
    campaign_id: String,
) -> anyhow::Result<PublishCampaignDetail> {
    let detail = db
        .get_publish_campaign(&campaign_id)?
        .ok_or_else(|| anyhow::anyhow!("publish campaign not found"))?;
    if detail.bundles.is_empty() || detail.assignments.is_empty() {
        anyhow::bail!("publish campaign has no staged bundle or assignment");
    }
    refuse_unmeasured_video_assignments_before_transfer(&control, &detail).await?;
    // Refused here too, not only before posting. Transferring first would push media onto a
    // phone that can never be posted from, and then leave it there.
    //
    // **Only the assignments this run will actually touch.** The loop below skips rows that
    // may already hold the post, so preflighting them too meant a retry could be blocked by
    // the one phone that already finished — its TikTok updated to an unmeasured build after
    // it posted, and the campaign's *remaining* phones were refused on its behalf.
    let mut reports = Vec::new();
    for assignment in &detail.assignments {
        if assignment_may_hold_the_post(&assignment.state) {
            continue;
        }
        reports.push((
            assignment.udid.as_str(),
            readiness_of(&control, &assignment.udid).await,
        ));
    }
    refuse_devices_whose_composer_is_not_measured(reports)?;
    let sound_participants: Vec<_> = detail
        .assignments
        .iter()
        .filter(|assignment| !assignment_may_hold_the_post(&assignment.state))
        .collect();
    refuse_devices_whose_sound_picker_is_not_measured(&control, &sound_participants).await?;
    // And the same argument for the bundle rather than the device: an image count this
    // composer's grid cannot reach fails at `post_one_assignment`, which is *after* the media
    // is on the phone and visible to TikTok.
    //
    // **Per device now, not one number for the campaign.** The two routes have different
    // ceilings — twelve tap points somebody wrote down, against however many grid cells fit on
    // the screen — and using the smaller for both refused Android bundles that post fine.
    refuse_assignments_whose_bundle_is_too_large(
        detail
            .assignments
            .iter()
            .filter(|assignment| !assignment_may_hold_the_post(&assignment.state))
            .filter_map(|assignment| {
                detail
                    .bundles
                    .iter()
                    .find(|bundle| bundle.id == assignment.bundle_id)
                    .map(|bundle| {
                        (
                            assignment.udid.as_str(),
                            bundle,
                            max_images_for(route_of(&control, &assignment.udid)),
                        )
                    })
            }),
    )?;
    // **Claimed, not written.** Transfer used to move the campaign to `Transferring`
    // unconditionally and then walk every assignment back to `Imported` — so a second Transfer
    // on a campaign that had already *succeeded* rebuilt exactly the state the posting claim
    // accepts, and the next Post published the same carousels again. Every compare-and-swap
    // downstream was sound only while nothing could manufacture a claimable state.
    if !db.claim_publish_campaign_for_transfer(&campaign_id)? {
        anyhow::bail!(
            "chiến dịch này không ở trạng thái chuyển được: {:?}. Chuyển lại một chiến dịch đã \
             đăng xong là dựng lại đúng trạng thái để nó đăng lần nữa.",
            detail.campaign.state
        );
    }
    announce(&events, &db, &campaign_id);

    for assignment in &detail.assignments {
        // An assignment that already reached a phone is not re-imported. The campaign claim
        // above makes some of these unreachable today; the guard stays because the two answer
        // different questions, and a retry of a partially posted campaign arrives here with
        // `succeeded` rows it must step over.
        if assignment_may_hold_the_post(&assignment.state) {
            continue;
        }
        // The operator's cancel is read **between phones**, the same place the post loop
        // reads it. Transfer used to run to the end regardless, and its final write then
        // erased the cancel (see `settle_publish_transfer`); stopping here costs nothing —
        // every untouched assignment is still where a later run expects it. A read that
        // fails is not "not cancelled": carrying on because the database did not answer is
        // how a stopped transfer keeps pushing media.
        match db.publish_campaign_state(&campaign_id) {
            Ok(Some(riviu_core::PublishCampaignState::Transferring)) => {}
            Ok(Some(riviu_core::PublishCampaignState::Cancelled)) | Ok(None) => break,
            Ok(Some(other)) => {
                anyhow::bail!(
                    "campaign left the transfer underneath this run: {other:?} — dừng thay vì \
                     chuyển tiếp"
                );
            }
            Err(error) => {
                let _ = db.settle_publish_transfer(
                    &campaign_id,
                    riviu_core::db::PublishTransferSettle::FailedBeforeDispatch {
                        error_code: "campaign_state_unreadable",
                    },
                );
                anyhow::bail!(
                    "không đọc được trạng thái chiến dịch giữa lượt chuyển ({error}) — dừng \
                     thay vì chuyển tiếp"
                );
            }
        }
        // **Each phone gets its own bundle, and only its own.**
        //
        // This used to take one `source_root` for the whole campaign —
        // `bundles[0].source_path.parent()` — and stage it to every phone in the loop. The
        // managed layout is `…/<request_id>/<bundle_id>/<images>`, so that parent is the
        // directory *containing* the bundles: every assignment was handed the same tree,
        // and the mapping UI that pairs N folders with N phones decided nothing at all.
        //
        // The rest of the path was already built for per-bundle work and only this input
        // was wrong: `import_id` is `riviu-<campaign>-<manifest sha>`, and the id passed
        // below is the bundle's own `"{request_id}:{bundle}"` — the exact shape
        // `the_import_id_is_the_directory_and_changes_with_the_content` pins with
        // `"req-7:bundle-3"`. Two bundles therefore land in two device directories and two
        // albums, which is what the post step needs to pick the right one.
        let bundle = bundle_for_assignment(&detail.bundles, assignment)?;
        let staged = stage_one_bundle(bundle, assignment.ordinal)?;
        let source_root = staged.path().to_path_buf();
        let device_scope = device_campaign_id(&campaign_id, assignment.ordinal);
        let device_scope = device_scope.as_str();

        // **Recorded before the device work, so a crash during it is recoverable.**
        //
        // Without this the assignment stayed at whatever it was — `queued`, usually — for the
        // whole transfer, and startup recovery's `transferring -> failed_before_dispatch`
        // branch never fired on a real database: the only row that ever said `transferring`
        // was the campaign. A crash mid-transfer left a child nobody would settle, under a
        // campaign that got cancelled, and the media on the phone with no record of it.
        //
        // The write-ahead row is the durable owner of any bytes that leave the desktop.
        // If it cannot be committed, no lease, copy, MediaStore insert or device command may
        // follow: after a crash startup can only reconcile children that are actually marked
        // `transferring`.
        if let Err(error) = record_transfer_write_ahead(&db, &assignment.id) {
            let _ = db.settle_publish_transfer(
                &campaign_id,
                riviu_core::db::PublishTransferSettle::FailedBeforeDispatch {
                    error_code: "transfer_write_ahead_failed",
                },
            );
            anyhow::bail!(
                "không ghi được transfer write-ahead cho {} ({error}); chưa chạm thiết bị",
                assignment.udid
            );
        }
        let context = match control
            .acquire_exclusive(&assignment.udid, DeviceWorkOwner::Script)
            .await
        {
            Ok(context) => context,
            // **`failed_before_dispatch`, not `uncertain` — a transfer cannot publish.**
            // Every failure in this loop used to settle both rows to `uncertain`, the one
            // state no claim accepts, so a copy error or a busy phone parked the campaign
            // (and this child) beyond every retry button for good. Nothing on this path can
            // reach TikTok's composer; the name that keeps the work reachable is the true
            // one. The campaign write is conditional on still being `transferring`, so a
            // cancel that landed mid-loop is not overwritten.
            Err(error) => {
                let message = error.to_string();
                db.update_publish_assignment_state(
                    &assignment.id,
                    riviu_core::PublishCampaignState::FailedBeforeDispatch,
                    Some("media_transfer_context_failed"),
                    Some(&serde_json::json!({"message": message}).to_string()),
                )?;
                db.settle_publish_transfer(
                    &campaign_id,
                    riviu_core::db::PublishTransferSettle::FailedBeforeDispatch {
                        error_code: "media_transfer_context_failed",
                    },
                )?;
                anyhow::bail!("media transfer context failed: {message}");
            }
        };
        match control
            .stage_publish_media(&context, &agent_bundle_id, device_scope, &source_root)
            .await
        {
            Ok(evidence) => {
                let native_result: anyhow::Result<serde_json::Value> = async {
                    if control.supports_push_media(&assignment.udid) {
                        let manifest_sha256 = evidence
                            .get("manifestSha256")
                            .and_then(serde_json::Value::as_str)
                            .ok_or_else(|| {
                                anyhow::anyhow!("media-stage evidence has no manifestSha256")
                            })?;
                        let native_prepare = control
                            .prepare_publish_media(&context, device_scope, manifest_sha256)
                            .await
                            .map_err(anyhow::Error::new)?;
                        let native_import = control
                            .import_publish_media(&context, device_scope, manifest_sha256)
                            .await
                            .map_err(anyhow::Error::new)?;
                        Ok(serde_json::json!({
                            "mediaStage": evidence,
                            "nativePrepare": native_prepare,
                            "nativeImport": native_import,
                        }))
                    } else {
                        Ok(evidence)
                    }
                }
                .await;
                match native_result {
                    Ok(evidence) => {
                        db.update_publish_assignment_state(
                            &assignment.id,
                            riviu_core::PublishCampaignState::Imported,
                            None,
                            Some(&evidence.to_string()),
                        )?;
                    }
                    // Retryable for the same reason as the context arm: a half-finished
                    // native import is re-run under the same deterministic import id, and
                    // nothing here has touched a composer.
                    Err(error) => {
                        let message = error.to_string();
                        db.update_publish_assignment_state(
                            &assignment.id,
                            riviu_core::PublishCampaignState::FailedBeforeDispatch,
                            Some("media_transfer_native_failed"),
                            Some(&serde_json::json!({"message": message}).to_string()),
                        )?;
                        db.settle_publish_transfer(
                            &campaign_id,
                            riviu_core::db::PublishTransferSettle::FailedBeforeDispatch {
                                error_code: "media_transfer_native_failed",
                            },
                        )?;
                        anyhow::bail!("native media import failed: {message}");
                    }
                }
            }
            // Same rule as the two arms above: nothing reached a composer, so the truthful
            // settle is the retryable one, and the cancel — if one landed — wins.
            Err(error) => {
                let message = error.to_string();
                db.update_publish_assignment_state(
                    &assignment.id,
                    riviu_core::PublishCampaignState::FailedBeforeDispatch,
                    Some("media_transfer_failed"),
                    Some(&serde_json::json!({"message": message}).to_string()),
                )?;
                db.settle_publish_transfer(
                    &campaign_id,
                    riviu_core::db::PublishTransferSettle::FailedBeforeDispatch {
                        error_code: "media_transfer_failed",
                    },
                )?;
                anyhow::bail!("media transfer failed: {message}");
            }
        }
    }
    // Conditional on the campaign still being `transferring` — see
    // `Database::settle_publish_transfer`. The unconditional write here is what used to walk
    // a mid-transfer cancel back to `imported`, and the scheduler's very next line then
    // posted the campaign the operator had stopped.
    // A settle that moves nothing — the operator cancelled mid-transfer — is not an error:
    // the phones that were staged, were staged, and the detail below reads `cancelled`,
    // which is the answer the caller must see. A bail here would push the operator toward a
    // retry the cancel exists to prevent. (The scheduler's follow-up Post is stopped by the
    // posting claim, which does not accept `cancelled`.)
    db.settle_publish_transfer(
        &campaign_id,
        riviu_core::db::PublishTransferSettle::Imported,
    )?;
    announce(&events, &db, &campaign_id);
    db.get_publish_campaign(&campaign_id)?
        .ok_or_else(|| anyhow::anyhow!("campaign disappeared after transfer"))
}

pub(super) const PLUS_BUTTON: TapPoint = TapPoint { x: 187.0, y: 640.0 };
pub(super) const GALLERY_BUTTON: TapPoint = TapPoint { x: 32.0, y: 632.0 };
pub(super) const ALBUM_PICKER: TapPoint = TapPoint { x: 187.0, y: 47.0 };
pub(super) const ALBUM_ROW: TapPoint = TapPoint { x: 187.0, y: 385.0 };
pub(super) const COMPOSER_NEXT: TapPoint = TapPoint { x: 300.0, y: 618.0 };
pub(super) const EDIT_NEXT: TapPoint = TapPoint { x: 280.0, y: 635.0 };
pub(super) const CAPTION_FIELD: TapPoint = TapPoint { x: 180.0, y: 240.0 };
pub(super) const POST_BUTTON: TapPoint = TapPoint { x: 330.0, y: 42.0 };
pub(super) const PUBLIC_POST_CONFIRM: TapPoint = TapPoint { x: 275.0, y: 444.0 };

/// How many images this path can select, and **why that number**.
///
/// `grid_x` has three columns and `grid_y` four rows, so there are twelve tap points and the
/// twelfth image is the last one that can be reached. Eleven is the guard that keeps a
/// thirteenth image from indexing `grid_y[4]` and panicking mid-post, on a real account, with
/// media already imported.
///
/// It lives here rather than in the scanner because it is a fact about *this composer's
/// coordinates*, not about TikTok: a carousel may hold 35 images and a hierarchy-driven
/// composer that locates its cells is not bound by a grid nobody measured.
pub(crate) const IOS_PIXEL_GRID_MAX_IMAGES: usize = 11;

/// How far apart the fanned-out publish tasks start.
///
/// The same two seconds the interaction path measured on this fleet, and for the same reason
/// rather than by imitation: twenty cold starts at once share one USB bus and one host, and the
/// tail of that contention runs past the 40-second foreground window. A publish task opens the
/// app exactly the way an interaction task does.
pub(super) const PUBLISH_FAN_OUT_STAGGER: Duration = Duration::from_secs(2);

/// Tell the UI that a campaign moved, once per state write.
///
/// An id and a revision, never the row itself: the page re-reads, so a payload that was
/// already stale by the time it arrived cannot be rendered as current. Same shape as the
/// interaction path's event, and for the same reason.
///
/// **Best effort, and deliberately silent on failure.** Nothing about a post depends on the
/// screen having heard about it, and turning a broadcast error into a failed publish would be
/// the same mistake as letting a Sheets write fail one.
pub(super) fn announce(events: &riviu_core::events::EventBus, db: &Database, campaign_id: &str) {
    let revision = db
        .publish_campaign_revision(campaign_id)
        .unwrap_or_default();
    events.emit(riviu_core::events::AppEvent::PublishUpdated {
        campaign_id: campaign_id.to_string(),
        revision,
    });
}

/// Execute the approved publish contract or resume only the obligations after a confirmed post.
///
/// Fresh requests use only an exact measured sound-picker tuple; video remains fail-closed until
/// its picker is measured. A succeeded campaign is different: its public effect is already
/// durable, so this command may safely retry own-post link capture and the idempotent Sheet
/// outbox, and it never enters the composer again.
#[tauri::command]
pub async fn publish_execute(
    state: State<'_, AppState>,
    campaign_id: String,
    confirmed: bool,
) -> Result<riviu_core::PublishCampaignExecutionResult, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    execute_publish_campaign_inner(
        state.control.clone(),
        state.registry.clone(),
        state.db.clone(),
        state.events.clone(),
        state.active_agent_bundle_id.clone(),
        Arc::new(state.streams.clone()),
        campaign_id,
        confirmed,
    )
    .await
    .map_err(err)
}

/// Non-Tauri entry point shared by the manual command, scheduled runs and fleet orchestration.
// These arguments are the independently owned runtime authorities passed by each caller. Keeping
// them explicit makes it possible to audit that the effect boundary uses the same DB, event bus,
// control plane and frame source instead of hiding one behind a partially initialized context.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_publish_campaign_inner(
    control: Arc<DeviceControlPlane>,
    registry: riviu_core::DeviceRegistry,
    db: Arc<Database>,
    events: riviu_core::events::EventBus,
    agent_bundle_id: String,
    frames: Arc<dyn FrameSource>,
    campaign_id: String,
    confirmed: bool,
) -> anyhow::Result<riviu_core::PublishCampaignExecutionResult> {
    let request = db
        .publish_campaign_request(&campaign_id)?
        .ok_or_else(|| anyhow::anyhow!("publish campaign request not found"))?;
    let mut detail = db
        .get_publish_campaign(&campaign_id)?
        .ok_or_else(|| anyhow::anyhow!("publish campaign not found"))?;
    let input_digest = db
        .get_publish_execution_snapshot(&campaign_id)?
        .map(|snapshot| snapshot.input_digest)
        .unwrap_or(stored_campaign_input_digest(&request, &detail)?);
    let fresh_confirmed = confirmed && request.execution_confirmed;
    let fresh_issues =
        fresh_publish_preflight_issues(&detail, &request.sound_policy, fresh_confirmed);
    let mut issues = Vec::new();
    let mut results = Vec::new();
    let mut ambiguous = false;
    let has_fresh_assignment = detail.assignments.iter().any(|assignment| {
        matches!(
            assignment.state,
            riviu_core::PublishCampaignState::Queued
                | riviu_core::PublishCampaignState::Scheduled
                | riviu_core::PublishCampaignState::Ready
                | riviu_core::PublishCampaignState::Imported
                | riviu_core::PublishCampaignState::FailedBeforeDispatch
        )
    });
    if has_fresh_assignment || detail.assignments.is_empty() {
        issues.extend(fresh_issues.iter().cloned());
    }

    // The production command owns the complete phone-side transaction. The typed core
    // runtime remains the reconciler for an already-confirmed post, but its desktop adapter
    // cannot split the legacy import/composer context across trait calls without dropping the
    // lease between phases. Run the proven transfer + composer path once, then feed only the
    // durable `Succeeded` rows through link/Sheet reconciliation below.
    if has_fresh_assignment && issues.is_empty() {
        let fresh_assignments: Vec<_> = detail
            .assignments
            .iter()
            .filter(|assignment| {
                matches!(
                    assignment.state,
                    riviu_core::PublishCampaignState::Queued
                        | riviu_core::PublishCampaignState::Scheduled
                        | riviu_core::PublishCampaignState::Ready
                        | riviu_core::PublishCampaignState::Imported
                        | riviu_core::PublishCampaignState::FailedBeforeDispatch
                )
            })
            .collect();
        if let Err(error) =
            refuse_devices_whose_sound_picker_is_not_measured(&control, &fresh_assignments).await
        {
            issues.push(publish_issue(
                "sound_picker_unmeasured",
                None,
                &error.to_string(),
            ));
        }
    }
    if has_fresh_assignment && issues.is_empty() {
        if !matches!(
            detail.campaign.state,
            riviu_core::PublishCampaignState::Imported
        ) {
            if let Err(error) = transfer_publish_campaign_inner(
                control.clone(),
                db.clone(),
                events.clone(),
                agent_bundle_id,
                campaign_id.clone(),
            )
            .await
            {
                issues.push(publish_issue(
                    "transfer_failed_before_post",
                    None,
                    &error.to_string(),
                ));
            }
            detail = db
                .get_publish_campaign(&campaign_id)?
                .ok_or_else(|| anyhow::anyhow!("publish campaign disappeared after transfer"))?;
        }
        if issues.is_empty()
            && matches!(
                detail.campaign.state,
                riviu_core::PublishCampaignState::Imported
                    | riviu_core::PublishCampaignState::FailedBeforeDispatch
            )
        {
            if let Err(error) = post_publish_campaign_inner(
                control.clone(),
                db.clone(),
                frames,
                events.clone(),
                campaign_id.clone(),
            )
            .await
            {
                issues.push(publish_issue(
                    "publish_phone_failed",
                    None,
                    &error.to_string(),
                ));
            }
            detail = db
                .get_publish_campaign(&campaign_id)?
                .ok_or_else(|| anyhow::anyhow!("publish campaign disappeared after post"))?;
        }
    }

    for assignment in detail.assignments.clone() {
        let Some(bundle) = detail
            .bundles
            .iter()
            .find(|bundle| bundle.id == assignment.bundle_id)
            .cloned()
        else {
            issues.push(publish_issue(
                "bundle_missing",
                Some(&assignment),
                "assignment trỏ tới bundle không tồn tại",
            ));
            continue;
        };
        let current_evidence = assignment
            .evidence_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());

        let resume = match assignment.state {
            riviu_core::PublishCampaignState::Succeeded => {
                riviu_core::PublishResumePoint::ConfirmedPost {
                    post_evidence: current_evidence
                        .clone()
                        .unwrap_or_else(|| serde_json::json!({"state":"posted"})),
                    canonical_link: current_evidence
                        .as_ref()
                        .and_then(post_url_owed)
                        .map(str::to_string),
                    sound_selection: current_evidence
                        .as_ref()
                        .and_then(sound_selection_from_evidence),
                }
            }
            riviu_core::PublishCampaignState::Posting
            | riviu_core::PublishCampaignState::Verifying
            | riviu_core::PublishCampaignState::Uncertain => {
                ambiguous = true;
                issues.push(publish_issue(
                    "post_may_be_live",
                    Some(&assignment),
                    "assignment có thể đã phát Post; chỉ reconcile thủ công, không tự đăng lại",
                ));
                continue;
            }
            riviu_core::PublishCampaignState::Cancelled
            | riviu_core::PublishCampaignState::Missed => {
                issues.push(publish_issue(
                    "assignment_terminal",
                    Some(&assignment),
                    "assignment đã kết thúc mà không được phép tự chạy lại",
                ));
                continue;
            }
            riviu_core::PublishCampaignState::Preparing
            | riviu_core::PublishCampaignState::Transferring => {
                issues.push(publish_issue(
                    "assignment_in_flight",
                    Some(&assignment),
                    "assignment đang do worker khác sở hữu; không khởi động worker thứ hai",
                ));
                continue;
            }
            _ => {
                issues.push(publish_issue(
                    "assignment_not_confirmed",
                    Some(&assignment),
                    "assignment chưa có bằng chứng bài đã đăng; chỉ đường full pipeline được phép xử lý trạng thái này",
                ));
                continue;
            }
        };

        let is_resume = matches!(resume, riviu_core::PublishResumePoint::ConfirmedPost { .. });
        let mut port = DesktopPublishRuntimePort {
            control: control.clone(),
            registry: registry.clone(),
            db: db.clone(),
            events: events.clone(),
            campaign_id: campaign_id.clone(),
            assignment: assignment.clone(),
            current_evidence,
        };
        let effect_db = db.clone();
        let effect_assignment_id = assignment.id.clone();
        let result = riviu_core::run_publish_pipeline(
            riviu_core::PublishExecutionInput {
                assignment_id: assignment.id.clone(),
                bundle,
                sound_policy: request.sound_policy.clone(),
                confirmed: if is_resume {
                    confirmed
                } else {
                    fresh_confirmed
                },
                resume,
            },
            &mut port,
            move |selection| {
                let intent = serde_json::json!({
                    "effectIntent": "post",
                    "soundSelection": selection,
                })
                .to_string();
                match effect_db
                    .claim_publish_assignment_for_posting(&effect_assignment_id, &intent)
                    .map_err(|error| error.to_string())?
                {
                    true => Ok(()),
                    false => Err("assignment effect-intent claim was refused".into()),
                }
            },
        )
        .await;
        if is_resume {
            issues.extend(runtime_issues(&assignment, &result));
        }
        results.push(result);
    }

    let detail = db
        .get_publish_campaign(&campaign_id)?
        .ok_or_else(|| anyhow::anyhow!("publish campaign disappeared during execution"))?;
    let status = if ambiguous
        || results
            .iter()
            .any(|result| result.status == riviu_core::PublishExecutionStatus::Uncertain)
    {
        riviu_core::PublishExecutionStatus::Uncertain
    } else if !results.is_empty()
        && results
            .iter()
            .all(|result| result.status == riviu_core::PublishExecutionStatus::Complete)
        && issues.is_empty()
    {
        riviu_core::PublishExecutionStatus::Complete
    } else {
        riviu_core::PublishExecutionStatus::Partial
    };
    let retry_scope = campaign_retry_scope(status, &results, &issues);
    let output = riviu_core::PublishCampaignExecutionResult {
        campaign_id,
        status,
        retry_scope,
        issues,
        detail,
    };
    persist_publish_snapshot_then_announce(&db, &events, &output.campaign_id, || {
        db.save_publish_execution_snapshot(
            &output.campaign_id,
            &input_digest,
            output.status,
            output.retry_scope,
            &publish_execution_report(&request, &output)?,
        )
    })?;
    Ok(output)
}

/// Run one due schedule from its immutable request snapshot, then leave a retryable terminal
/// state when fail-closed preflight stops before any device or public effect.
pub(crate) async fn execute_scheduled_publish_campaign_inner(
    control: Arc<DeviceControlPlane>,
    registry: riviu_core::DeviceRegistry,
    db: Arc<Database>,
    events: riviu_core::events::EventBus,
    agent_bundle_id: String,
    frames: Arc<dyn FrameSource>,
    campaign_id: String,
) -> anyhow::Result<riviu_core::PublishCampaignExecutionResult> {
    let request = db
        .publish_campaign_request(&campaign_id)?
        .ok_or_else(|| anyhow::anyhow!("scheduled publish request not found"))?;
    let mut result = execute_publish_campaign_inner(
        control,
        registry,
        db.clone(),
        events.clone(),
        agent_bundle_id,
        frames,
        campaign_id.clone(),
        request.execution_confirmed,
    )
    .await?;
    if result.status == riviu_core::PublishExecutionStatus::Partial
        && result.retry_scope == riviu_core::PublishRetryScope::FullPipeline
    {
        let error_code = result
            .issues
            .first()
            .map(|issue| issue.code.as_str())
            .unwrap_or("publish_preflight_failed");
        db.update_publish_campaign_state(
            &campaign_id,
            riviu_core::PublishCampaignState::FailedBeforeDispatch,
            Some(error_code),
        )?;
        result.detail = db
            .get_publish_campaign(&campaign_id)?
            .ok_or_else(|| anyhow::anyhow!("scheduled publish disappeared while settling"))?;
        let snapshot = db
            .get_publish_execution_snapshot(&campaign_id)?
            .context("scheduled publish execution snapshot disappeared while settling")?;
        persist_publish_snapshot_then_announce(&db, &events, &campaign_id, || {
            db.save_publish_execution_snapshot(
                &campaign_id,
                &snapshot.input_digest,
                result.status,
                result.retry_scope,
                &publish_execution_report(&request, &result)?,
            )
        })?;
    }
    Ok(result)
}

pub(super) struct DesktopPublishRuntimePort {
    control: Arc<DeviceControlPlane>,
    registry: riviu_core::DeviceRegistry,
    db: Arc<Database>,
    events: riviu_core::events::EventBus,
    campaign_id: String,
    assignment: riviu_core::PublishAssignmentRecord,
    current_evidence: Option<serde_json::Value>,
}

#[async_trait::async_trait]
impl riviu_core::PublishRuntimePort for DesktopPublishRuntimePort {
    async fn preflight(&mut self, input: &riviu_core::PublishExecutionInput) -> Result<(), String> {
        if matches!(
            input.resume,
            riviu_core::PublishResumePoint::ConfirmedPost {
                canonical_link: None,
                ..
            }
        ) {
            let device = self.registry.get(&self.assignment.udid).ok_or_else(|| {
                "device_missing: confirmed post link still needs its phone".to_string()
            })?;
            if !matches!(device.platform, riviu_core::DevicePlatform::Android) {
                return Err(
                    "android_required: confirmed-link route is measured only on Android".into(),
                );
            }
            let (package, version, locale) = self
                .control
                .tiktok_build(&self.assignment.udid)
                .await
                .map_err(|error| format!("tiktok_build_unreadable: {error}"))?;
            let missing = missing_link_locators(&package, &locale, &version);
            if !missing.is_empty() {
                return Err(format!("link_locator_missing: {}", missing.join(", ")));
            }
        }
        Ok(())
    }

    async fn transfer(&mut self, _bundle: &riviu_core::PublishBundle) -> Result<(), String> {
        Err("production transfer adapter is unreachable until sound preflight is measured".into())
    }

    async fn observe_sound_candidates(
        &mut self,
        _maximum_visible: usize,
    ) -> Result<Vec<riviu_core::SoundCandidate>, String> {
        Err("sound_picker_unmeasured".into())
    }

    async fn choose_sound(
        &mut self,
        _selection: &riviu_core::SoundSelectionEvidence,
    ) -> Result<(), String> {
        Err("sound_picker_unmeasured".into())
    }

    async fn confirm_sound(
        &mut self,
        _selection: &riviu_core::SoundSelectionEvidence,
    ) -> Result<bool, String> {
        Err("sound_picker_unmeasured".into())
    }

    async fn dispatch_post(
        &mut self,
        _before_post: &mut (dyn FnMut() -> Result<(), String> + Send),
        _selection: &riviu_core::SoundSelectionEvidence,
    ) -> Result<(), String> {
        Err("production Post is unreachable until sound read-back is measured".into())
    }

    async fn confirm_post(&mut self) -> Result<serde_json::Value, String> {
        Err("production Post confirmation is unreachable before dispatch".into())
    }

    async fn capture_canonical_link(
        &mut self,
        bundle: &riviu_core::PublishBundle,
    ) -> Result<String, String> {
        capture_confirmed_assignment_link(&self.control, &self.assignment, bundle)
            .await
            .map_err(|error| error.to_string())
    }

    async fn write_sheet(
        &mut self,
        assignment_id: &str,
        canonical_link: &str,
        bundle: &riviu_core::PublishBundle,
    ) -> Result<(), String> {
        if !riviu_core::tiktok_share::looks_like_a_post_link(canonical_link) {
            return Err("canonical TikTok post link is required before Sheet".into());
        }
        let evidence = evidence_with_post_url(self.current_evidence.clone(), canonical_link);
        self.db
            .record_publish_success_with_sheet_row(
                assignment_id,
                &evidence.to_string(),
                &self.campaign_id,
                canonical_link,
                poster_identity(),
                &bundle.partners,
            )
            .map_err(|error| error.to_string())?;
        self.current_evidence = Some(evidence);
        deliver_assignment_sheet_row(&self.db, &self.events, assignment_id).await
    }

    async fn cleanup(&mut self) -> Result<(), String> {
        Ok(())
    }
}

pub(super) fn sound_selection_from_evidence(
    evidence: &serde_json::Value,
) -> Option<riviu_core::SoundSelectionEvidence> {
    evidence
        .get("post")
        .and_then(|post| post.get("soundSelection"))
        .or_else(|| evidence.get("soundSelection"))
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

pub(super) fn runtime_issues(
    assignment: &riviu_core::PublishAssignmentRecord,
    result: &riviu_core::PublishExecutionResult,
) -> Vec<riviu_core::PublishExecutionIssue> {
    result
        .phases
        .iter()
        .filter_map(|phase| {
            let detail = phase.detail.as_ref()?;
            Some(publish_issue(
                &format!("{:?}_failed", phase.phase).to_ascii_lowercase(),
                Some(assignment),
                detail,
            ))
        })
        .collect()
}

pub(super) fn campaign_retry_scope(
    status: riviu_core::PublishExecutionStatus,
    results: &[riviu_core::PublishExecutionResult],
    issues: &[riviu_core::PublishExecutionIssue],
) -> riviu_core::PublishRetryScope {
    if status == riviu_core::PublishExecutionStatus::Uncertain
        || issues.iter().any(|issue| {
            matches!(
                issue.code.as_str(),
                "assignment_terminal"
                    | "assignment_in_flight"
                    | "campaign_terminal"
                    | "post_may_be_live"
            )
        })
    {
        return riviu_core::PublishRetryScope::None;
    }
    if issues.iter().any(|issue| {
        matches!(
            issue.code.as_str(),
            "transfer_failed_before_post"
                | "publish_phone_failed"
                | "assignment_not_confirmed"
                | "sound_picker_unmeasured"
                | "video_composer_unmeasured"
                | "confirmation_required"
                | "campaign_empty"
                | "bundle_missing"
        )
    }) {
        return riviu_core::PublishRetryScope::FullPipeline;
    }
    for scope in [
        riviu_core::PublishRetryScope::FullPipeline,
        riviu_core::PublishRetryScope::LinkAndSheet,
        riviu_core::PublishRetryScope::SheetOnly,
    ] {
        if results.iter().any(|result| result.retry_scope == scope) {
            return scope;
        }
    }
    if issues.is_empty() {
        riviu_core::PublishRetryScope::None
    } else {
        riviu_core::PublishRetryScope::FullPipeline
    }
}

pub(super) fn fresh_publish_preflight_issues(
    detail: &PublishCampaignDetail,
    sound_policy: &riviu_core::PublishSoundPolicy,
    confirmed: bool,
) -> Vec<riviu_core::PublishExecutionIssue> {
    let mut issues = Vec::new();
    if !confirmed {
        issues.push(publish_issue(
            "confirmation_required",
            None,
            "cần đúng một xác nhận cho toàn bộ chuỗi trước khi chuyển media",
        ));
    }
    if sound_policy.pool_size().is_err() {
        issues.push(publish_issue(
            "sound_pool_invalid",
            None,
            "sound pool phải nằm trong 1..=10; runtime chỉ dùng tối đa năm hàng đang hiện",
        ));
    }
    if detail.assignments.is_empty() || detail.bundles.is_empty() {
        issues.push(publish_issue(
            "campaign_empty",
            None,
            "campaign không có đủ bundle và assignment",
        ));
    }
    if matches!(
        detail.campaign.state,
        riviu_core::PublishCampaignState::Posting
            | riviu_core::PublishCampaignState::Verifying
            | riviu_core::PublishCampaignState::Uncertain
    ) {
        issues.push(publish_issue(
            "post_may_be_live",
            None,
            "campaign có thể đã phát Post; chỉ reconcile, tuyệt đối không tự đăng lại",
        ));
    }
    if matches!(
        detail.campaign.state,
        riviu_core::PublishCampaignState::Cancelled | riviu_core::PublishCampaignState::Missed
    ) {
        issues.push(publish_issue(
            "campaign_terminal",
            None,
            "campaign đã bị huỷ hoặc lỡ lịch; không tự chạy lại",
        ));
    }
    for assignment in &detail.assignments {
        if matches!(
            assignment.state,
            riviu_core::PublishCampaignState::Succeeded
                | riviu_core::PublishCampaignState::Posting
                | riviu_core::PublishCampaignState::Verifying
                | riviu_core::PublishCampaignState::Uncertain
                | riviu_core::PublishCampaignState::Cancelled
                | riviu_core::PublishCampaignState::Missed
        ) {
            continue;
        }
        let bundle = detail
            .bundles
            .iter()
            .find(|bundle| bundle.id == assignment.bundle_id);
        let Some(bundle) = bundle else {
            issues.push(publish_issue(
                "bundle_missing",
                Some(assignment),
                "assignment trỏ tới bundle không tồn tại",
            ));
            continue;
        };
        if bundle.caption.trim().is_empty() {
            issues.push(publish_issue(
                "caption_empty",
                Some(assignment),
                "caption rỗng không thể chứng minh bài của chính lượt này khi lấy link",
            ));
        }
    }
    issues
}

pub(super) fn publish_issue(
    code: &str,
    assignment: Option<&riviu_core::PublishAssignmentRecord>,
    message: &str,
) -> riviu_core::PublishExecutionIssue {
    riviu_core::PublishExecutionIssue {
        code: code.into(),
        assignment_id: assignment.map(|value| value.id.clone()),
        udid: assignment.map(|value| value.udid.clone()),
        bundle_id: assignment.map(|value| value.bundle_id.clone()),
        message: message.into(),
    }
}

pub(super) fn missing_link_locators(
    package: &str,
    locale: &str,
    version: &str,
) -> Vec<&'static str> {
    let Some(labels) = riviu_core::tiktok_labels::controls_for(package, locale, version) else {
        return vec!["build_label_set"];
    };
    let mut missing = Vec::new();
    for (control, name) in [
        (
            riviu_core::tiktok_labels::TikTokControl::ProfileTab,
            "profile_tab",
        ),
        (riviu_core::tiktok_labels::TikTokControl::Share, "share"),
    ] {
        if labels.label(control).is_none() {
            missing.push(name);
        }
    }
    if labels.post_tile_id().is_none() {
        missing.push("own_post_tile");
    }
    missing
}

pub(super) async fn capture_confirmed_assignment_link(
    control: &DeviceControlPlane,
    assignment: &riviu_core::PublishAssignmentRecord,
    bundle: &riviu_core::PublishBundle,
) -> anyhow::Result<String> {
    anyhow::ensure!(
        !bundle.caption.trim().is_empty(),
        "caption rỗng nên không có identity proof cho bài"
    );
    let context = open_publish_context(control, &assignment.udid).await?;
    let session = match control.streaming_session(&context) {
        Ok(session) => session,
        Err(error) => {
            control
                .close_ui_context(context)
                .await
                .map_err(anyhow::Error::new)?;
            return Err(anyhow::Error::new(error));
        }
    };
    let outcome = async {
        let package = control.resolve_tiktok_package(&assignment.udid).await?;
        let language = session.ui_language().await.unwrap_or_default();
        let version = session.app_version(&package).await.unwrap_or_default();
        let labels = riviu_core::tiktok_labels::controls_for(&package, &language, &version)
            .ok_or_else(|| anyhow::anyhow!("build TikTok chưa đo cho đường lấy link"))?;
        anyhow::ensure!(
            missing_link_locators(&package, &language, &version).is_empty(),
            "thiếu locator lấy link bài"
        );
        let capture = riviu_core::tiktok_share::capture_own_post_link(
            session.as_ref(),
            &labels,
            &bundle.caption,
        )
        .await;
        capture
            .link()
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!(capture.reason()))
    }
    .await;
    let closed = control
        .close_ui_context(context)
        .await
        .map_err(anyhow::Error::new);
    match (outcome, closed) {
        (Ok(link), Ok(_)) => Ok(link),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(anyhow::anyhow!(
            "đã lấy link nhưng không đóng được UI context: {error}"
        )),
    }
}

pub(super) fn evidence_with_post_url(
    existing: Option<serde_json::Value>,
    link: &str,
) -> serde_json::Value {
    let mut evidence = existing.unwrap_or_else(|| serde_json::json!({"state":"posted"}));
    if let Some(post) = evidence
        .get_mut("post")
        .and_then(serde_json::Value::as_object_mut)
    {
        post.insert("postUrl".into(), serde_json::Value::String(link.into()));
    } else if let Some(object) = evidence.as_object_mut() {
        object.insert("postUrl".into(), serde_json::Value::String(link.into()));
    } else {
        evidence = serde_json::json!({"state":"posted", "postUrl":link});
    }
    evidence
}

pub(crate) async fn post_publish_campaign_inner(
    control: Arc<DeviceControlPlane>,
    db: Arc<Database>,
    frames: Arc<dyn FrameSource>,
    events: riviu_core::events::EventBus,
    campaign_id: String,
) -> anyhow::Result<PublishCampaignDetail> {
    let request = db
        .publish_campaign_request(&campaign_id)?
        .ok_or_else(|| anyhow::anyhow!("publish campaign request not found"))?;
    let detail = db
        .get_publish_campaign(&campaign_id)?
        .ok_or_else(|| anyhow::anyhow!("publish campaign not found"))?;
    if detail.assignments.is_empty() || detail.bundles.is_empty() {
        anyhow::bail!("publish campaign has no imported assignment");
    }
    // Read once for the message; the authority is the claim below, not this check.
    // The same two states the claim below accepts. Reading only `Imported` here refused a
    // campaign that startup recovery had correctly settled to `failed_before_dispatch` —
    // labelled retryable and unreachable, which is the pairing this whole path keeps closing.
    if !matches!(
        detail.campaign.state,
        riviu_core::PublishCampaignState::Imported
            | riviu_core::PublishCampaignState::FailedBeforeDispatch
    ) {
        anyhow::bail!(
            "campaign must be imported before post: {:?}",
            detail.campaign.state
        );
    }
    // **Only the assignments this run will post.** A `succeeded` row is not a participant:
    // its phone is not opened, not preflighted, and — further down — its claim refusal is
    // not a failure. Before this filter, a retry of a partially posted campaign was judged
    // by the phones that had already finished: their "đã đăng — bỏ qua" refusals counted as
    // failures, the campaign went back to `failed_before_dispatch` with every carousel
    // live, and the screen offered another Transfer forever.
    let participants: Vec<&riviu_core::PublishAssignmentRecord> = detail
        .assignments
        .iter()
        .filter(|assignment| !assignment_already_posted(&assignment.state))
        .collect();
    let mut reports = Vec::new();
    for assignment in &participants {
        reports.push((
            assignment.udid.as_str(),
            readiness_of(&control, &assignment.udid).await,
        ));
    }
    refuse_devices_whose_composer_is_not_measured(reports)?;
    refuse_devices_whose_sound_picker_is_not_measured(&control, &participants).await?;
    // Asked per device, not once for the campaign. A campaign spans several
    // phones and a fleet can be mixed, so a single fleet-wide answer would
    // report one device's agent on behalf of the rest. Still fails fast, before
    // any state is mutated, and now names the devices that are short.
    let without_push_media: Vec<&str> = participants
        .iter()
        .filter(|assignment| !control.supports_push_media(&assignment.udid))
        .map(|assignment| assignment.udid.as_str())
        .collect();
    if !without_push_media.is_empty() {
        anyhow::bail!(
            "these devices' agents do not advertise pushMedia: {}; install the combined candidate",
            without_push_media.join(", ")
        );
    }
    // **Claim it, and stop if somebody else already did.** This used to be an
    // unconditional write, so two commands that both read `Imported` both proceeded and both
    // posted the same bundles -- serialised on the device lease, therefore invisible, and a
    // carousel post cannot be taken back.
    if !db.claim_publish_campaign_for_posting(&campaign_id)? {
        anyhow::bail!(
            "chiến dịch này đang được đăng bởi một lượt khác, hoặc đã đăng xong -- \
             không đăng lại"
        );
    }
    announce(&events, &db, &campaign_id);

    // **Fanned out, bounded by the stream budget, staggered.** The same three properties the
    // interaction path measured its way to, for the same fleet and the same reasons.
    //
    // Sequential was the shape before, and its cost is arithmetic: five phones inside TikTok's
    // composer take about as long each, so a run took five times one phone and four phones sat
    // idle throughout. Nothing required that — every assignment is claimed by compare-and-swap
    // (`claim_publish_assignment_for_posting`), so two tasks cannot reach the same row, and the
    // device control plane already serialises per device.
    //
    // The permit count is `stream_capacity`, because each post holds a UI-with-stream context.
    // Running past it does not queue, it fails — which on this path means a phone with media
    // already in its gallery.
    // A campaign whose every assignment already posted has nothing left to run — which is
    // exactly the state the counting bug used to manufacture: children all `succeeded` under
    // a parent stuck `failed_before_dispatch`. Settling it here is what lets one press of
    // Post close that loop instead of reporting the finished phones as failures.
    if participants.is_empty() {
        db.finish_publish_campaign(&campaign_id, riviu_core::db::PublishRunOutcome::AllPosted)?;
        return db
            .get_publish_campaign(&campaign_id)?
            .ok_or_else(|| anyhow::anyhow!("campaign disappeared after post"));
    }
    let gate = Arc::new(tokio::sync::Semaphore::new(
        control.stream_capacity().max(1),
    ));
    let mut running = Vec::with_capacity(participants.len());
    for (index, assignment) in participants.iter().enumerate() {
        let Some(bundle) = detail
            .bundles
            .iter()
            .find(|bundle| bundle.id == assignment.bundle_id)
        else {
            running.push(tokio::spawn(async move { Err(PhoneFailure::NoBundle) }));
            continue;
        };
        running.push(tokio::spawn(post_one_phone(
            PUBLISH_FAN_OUT_STAGGER * index as u32,
            gate.clone(),
            control.clone(),
            db.clone(),
            frames.clone(),
            events.clone(),
            campaign_id.clone(),
            (*assignment).clone(),
            bundle.clone(),
            request.sound_policy.clone(),
        )));
    }

    let mut failures = Vec::new();
    // **Whether anything may be live, tracked separately from whether anything failed.**
    // Collapsing the two made a run where every phone refused before opening anything end as
    // `uncertain` — permanently unclaimable — with children correctly marked retryable
    // underneath it.
    let mut may_be_live = false;
    for (assignment, task) in participants.iter().zip(running) {
        match task.await {
            Ok(Ok(())) => {}
            Ok(Err(PhoneFailure::NoBundle)) => {
                failures.push(format!("bundle {} missing", assignment.bundle_id))
            }
            Ok(Err(PhoneFailure::NothingPublished(reason))) => failures.push(reason),
            Ok(Err(PhoneFailure::MayBeLive(reason))) => {
                may_be_live = true;
                failures.push(reason);
            }
            // A panic in one phone's task must not lose the other four — and it is the one
            // failure that says nothing about where the phone got to, so it counts as live.
            Err(join) => {
                may_be_live = true;
                failures.push(format!("{}: task hỏng ({join})", assignment.udid));
            }
        }
    }
    // A cancel that landed mid-run is honoured by the tasks themselves, and read back here for
    // the message.
    let cancelled = matches!(
        db.publish_campaign_state(&campaign_id)?,
        Some(riviu_core::PublishCampaignState::Cancelled)
    );

    // Conditional on the campaign still being `Posting`, in SQL — see
    // `Database::finish_publish_campaign`. An unconditional write here is what used to erase
    // a cancel that landed while the run was going.
    db.finish_publish_campaign(
        &campaign_id,
        match (failures.is_empty(), may_be_live) {
            (true, _) => riviu_core::db::PublishRunOutcome::AllPosted,
            (false, false) => riviu_core::db::PublishRunOutcome::NothingPublished,
            (false, true) => riviu_core::db::PublishRunOutcome::SomethingMayBeLive,
        },
    )?;
    let output = db
        .get_publish_campaign(&campaign_id)?
        .ok_or_else(|| anyhow::anyhow!("campaign disappeared after post"))?;
    if cancelled {
        // Not an error: the phones that posted, posted, and the rest were never touched.
        // Reporting a failure here would push the operator toward a retry they do not need.
        return Ok(output);
    }
    if !failures.is_empty() {
        anyhow::bail!("{}", failures.join("; "));
    }
    Ok(output)
}

/// What one assignment's posting attempt achieved, and **whether it may be tried again**.
///
/// The distinction the campaign loop could not make before: every failure became `Uncertain`,
/// which is permanently unclaimable. That is the right answer when a tap may have reached
/// TikTok and the wrong one when the run refused before opening anything — and refusing early
/// is what the composer does most of the time, so most of what got stranded never needed to be.
pub(super) enum PostOutcome {
    /// The carousel is on the account.
    Posted(serde_json::Value),
    /// Nothing was published, and that is *known* rather than assumed.
    ///
    /// Only produced where the driver can prove it — `ComposerVerdict::may_retry`. The pixel
    /// path has no equivalent, so its failures all land in [`Self::Unknown`].
    NothingPublished(String),
    /// A tap may have reached TikTok and the result could not be read.
    Unknown(String),
}

pub(super) struct AssignmentPostAttempt {
    outcome: PostOutcome,
    claim_refused: bool,
}

pub(super) async fn post_one_assignment(
    control: &DeviceControlPlane,
    db: &Database,
    frames: &dyn FrameSource,
    campaign_id: &str,
    assignment: &riviu_core::PublishAssignmentRecord,
    bundle: &riviu_core::PublishBundle,
    sound_policy: &riviu_core::PublishSoundPolicy,
) -> AssignmentPostAttempt {
    let finish = |outcome| AssignmentPostAttempt {
        outcome,
        claim_refused: false,
    };
    // **Everything before the phone opens is a refusal, not an unknown.** These used to be
    // `bail!`s that the caller turned into `uncertain` — permanently unclaimable — for a
    // caption nobody could have posted and a phone nobody had touched.
    match bundle.media_kind {
        riviu_core::PublishMediaKind::Image
            if !bundle_media_shape_is_ready(bundle, PublishRoute::Hierarchy) =>
        {
            return finish(PostOutcome::NothingPublished(format!(
                "bundle {} has an invalid image snapshot",
                bundle.id
            )))
        }
        riviu_core::PublishMediaKind::Video
            if !bundle_media_shape_is_ready(bundle, PublishRoute::Hierarchy) =>
        {
            return finish(PostOutcome::NothingPublished(format!(
                "bundle {} has an invalid video snapshot",
                bundle.id
            )))
        }
        riviu_core::PublishMediaKind::Image | riviu_core::PublishMediaKind::Video => {}
    }
    if bundle.caption.chars().count() > 2200 {
        return finish(PostOutcome::NothingPublished(format!(
            "caption for {} exceeds TikTok's 2200 character limit",
            bundle.id
        )));
    }
    let Some(import) = assignment
        .evidence_json
        .as_deref()
        .and_then(import_id_from_evidence)
    else {
        return finish(PostOutcome::NothingPublished(format!(
            "native import proof is missing for {}",
            assignment.udid
        )));
    };
    let context = match open_publish_context(control, &assignment.udid).await {
        Ok(context) => context,
        // The lease, the capacity reservation and the app launch all live in here. None of
        // them reaches TikTok's composer, so a failure means nothing was published — and the
        // media is still on the phone with no context to clean it from, which the operator
        // needs told rather than hidden inside `uncertain`.
        Err(error) => {
            return finish(PostOutcome::NothingPublished(format!(
                "{}: không mở được phiên ({error}); ảnh vẫn còn trên máy",
                assignment.udid
            )))
        }
    };
    let session = match control.streaming_session(&context) {
        Ok(session) => session,
        Err(error) => {
            let cleanup =
                tidy_up_the_imported_media(control, context, &assignment.udid, &import).await;
            return finish(fold_cleanup_into(
                PostOutcome::NothingPublished(format!("{}: {error}", assignment.udid)),
                cleanup,
            ));
        }
    };
    if let Some(refusal) = refuse_when_the_route_authorities_disagree(
        &assignment.udid,
        control.reports_element_bounds(&assignment.udid),
        session.supports_element_bounds(),
    ) {
        let cleanup = tidy_up_the_imported_media(control, context, &assignment.udid, &import).await;
        return finish(fold_cleanup_into(refusal, cleanup));
    }
    if matches!(bundle.media_kind, riviu_core::PublishMediaKind::Video)
        && !session.supports_element_bounds()
    {
        let cleanup = tidy_up_the_imported_media(control, context, &assignment.udid, &import).await;
        return finish(fold_cleanup_into(
            PostOutcome::NothingPublished(format!(
                "{}: video picker is measured only on the Android hierarchy route",
                assignment.udid
            )),
            cleanup,
        ));
    }
    let mut effect_claimed = false;
    let mut claim_refused = false;
    let action_result = {
        let mut before_post =
            |sound_selection: Option<&riviu_core::SoundSelectionEvidence>| -> anyhow::Result<()> {
                if effect_claimed {
                    anyhow::bail!("effect-intent callback invoked more than once");
                }
                let intent = serde_json::json!({
                    "effectIntent": "post",
                    "mediaKind": bundle.media_kind,
                    "bundleId": bundle.id,
                    "captionSha256": bundle.caption_sha256,
                    "soundSelection": sound_selection,
                })
                .to_string();
                match db.claim_publish_assignment_for_posting(&assignment.id, &intent) {
                    Ok(true) => {
                        effect_claimed = true;
                        Ok(())
                    }
                    Ok(false) => {
                        claim_refused = true;
                        anyhow::bail!(
                            "{} đã đăng, đang được đăng, hoặc chiến dịch đã dừng — không tap Post",
                            assignment.udid
                        )
                    }
                    Err(error) => {
                        claim_refused = true;
                        Err(error)
                    }
                }
            };
        if session.supports_element_bounds() {
            post_through_the_composer(
                control,
                session.as_ref(),
                campaign_id,
                &assignment.udid,
                bundle,
                &import,
                sound_policy,
                &mut before_post,
            )
            .await
        } else {
            let mut before_pixel_post = || before_post(None);
            post_through_the_pixel_grid(
                frames,
                session.as_ref(),
                campaign_id,
                &assignment.udid,
                bundle,
                &import,
                &mut before_pixel_post,
            )
            .await
        }
    };
    let action_result =
        if effect_claimed && matches!(action_result, PostOutcome::NothingPublished(_)) {
            PostOutcome::Unknown(
                "the Post boundary was crossed but the route reported a retryable refusal".into(),
            )
        } else {
            action_result
        };
    // **Cleanup runs whatever the route said.** It used to sit behind `action_result?`, so
    // every error path left the campaign's images in a real phone's gallery with nothing
    // owning them — including the Android build gate, which refuses *before its first tap*.
    let cleanup = tidy_up_the_imported_media(control, context, &assignment.udid, &import).await;
    AssignmentPostAttempt {
        outcome: fold_cleanup_into(action_result, cleanup),
        claim_refused,
    }
}

/// Attach the cleanup result to a posting outcome **without changing what the post did**.
///
/// A pure function because the rule it encodes was unreachable by any test while it lived
/// inside `post_one_assignment`, and a reversal proved it: making a cleanup failure downgrade
/// a published post to `Unknown` left the whole suite green. That downgrade is the exact bug
/// the rest of this path is built to prevent — `Unknown` is permanently unclaimable, so a
/// carousel that went out cleanly and left some files behind would need a person to look at a
/// phone where the only problem is disk space.
pub(super) fn fold_cleanup_into(
    outcome: PostOutcome,
    cleanup: anyhow::Result<serde_json::Value>,
) -> PostOutcome {
    let note = match &cleanup {
        Ok(value) => value.clone(),
        Err(error) => serde_json::json!({"state": "not_cleaned", "message": error.to_string()}),
    };
    match outcome {
        PostOutcome::Posted(evidence) => {
            PostOutcome::Posted(serde_json::json!({"post": evidence, "cleanup": note}))
        }
        // Nothing was published, so a cleanup failure is worth saying out loud — and still
        // does not change *what happened to the post*.
        PostOutcome::NothingPublished(reason) => PostOutcome::NothingPublished(match cleanup {
            Ok(_) => reason,
            Err(error) => format!("{reason}; ảnh còn lại trên máy: {error}"),
        }),
        PostOutcome::Unknown(reason) => PostOutcome::Unknown(match cleanup {
            Ok(_) => reason,
            Err(error) => format!("{reason}; ảnh còn lại trên máy: {error}"),
        }),
    }
}

/// The assignment state one outcome earns, and the code that goes with it.
///
/// **The whole point is that these are three answers, not two.** Every failure used to become
/// `Uncertain`, which [`riviu_core::db::Database::claim_publish_assignment_for_posting`]
/// refuses forever — correct when a tap may have reached TikTok, and wrong when the run
/// refused before opening anything. The composer can tell those apart
/// ([`riviu_core::tiktok_composer::ComposerVerdict::may_retry`]), so the loop must stop
/// throwing that away.
/// **The gate and the dispatch have to agree about which composer this is.**
///
/// Two authorities answer that question. [`DeviceControlPlane::reports_element_bounds`] is a
/// *preflight* answer, given before any session exists, and it is what the campaign gate and
/// the per-device image ceiling were decided from. [`riviu_core::UiSession::supports_element_bounds`]
/// is the live session's own answer, and the driver contract says that one is authoritative.
///
/// The contract also allows them to differ, and nothing checked. A `true` preflight followed by
/// a `false` session passed the measured-label gate and then pressed iOS pixel coordinates at a
/// screen nobody had measured; the reverse transferred media to a phone whose build was never
/// checked at all. Neither is a state to publish from.
///
/// `None` when they agree. Otherwise a refusal — **`NothingPublished`, deliberately**: nothing
/// has reached the composer at this point, so the campaign stays retryable rather than becoming
/// permanently unclaimable over a disagreement that a re-read may not repeat.
pub(super) fn refuse_when_the_route_authorities_disagree(
    udid: &str,
    preflight: bool,
    session: bool,
) -> Option<PostOutcome> {
    (preflight != session).then(|| {
        PostOutcome::NothingPublished(format!(
            "{udid}: máy báo hai câu trả lời khác nhau về cách điều khiển (trước phiên: \
             {preflight}, trong phiên: {session}) — không đăng khi chưa biết composer nào"
        ))
    })
}

pub(super) fn state_for_outcome(
    outcome: &PostOutcome,
) -> (riviu_core::PublishCampaignState, Option<&'static str>) {
    match outcome {
        PostOutcome::Posted(_) => (riviu_core::PublishCampaignState::Succeeded, None),
        PostOutcome::NothingPublished(_) => (
            riviu_core::PublishCampaignState::FailedBeforeDispatch,
            Some("post_refused_before_dispatch"),
        ),
        PostOutcome::Unknown(_) => (
            riviu_core::PublishCampaignState::Uncertain,
            Some("post_or_cleanup_failed"),
        ),
    }
}

/// Take the campaign's images back off the phone, with one retry on a fresh lease.
///
/// Split out of `post_one_assignment` so the outcome above can be decided without this
/// function's four failure modes in the same block. Returns the cleanup evidence or the reason
/// it could not — never a reason to change what the post did.
pub(super) async fn tidy_up_the_imported_media(
    control: &DeviceControlPlane,
    context: riviu_core::UiWithStreamContext,
    udid: &str,
    import: &str,
) -> anyhow::Result<serde_json::Value> {
    let cleanup_while_live = control
        .cleanup_publish_media_with_ui(&context, import)
        .await;
    control
        .close_ui_context(context)
        .await
        .map_err(anyhow::Error::new)?;
    let cleanup = match cleanup_while_live {
        Ok(cleanup) => cleanup,
        Err(first_error) => {
            let retry_context = control
                .acquire_exclusive(udid, DeviceWorkOwner::Script)
                .await
                .map_err(anyhow::Error::new)?;
            let retry_result = control.cleanup_publish_media(&retry_context, import).await;
            let close_retry = control.close_exclusive_context(retry_context);
            close_retry.map_err(anyhow::Error::new)?;
            retry_result.map_err(|retry_error| {
                anyhow::anyhow!(
                    "native cleanup failed while UI was live ({first_error}); retry failed: {retry_error}"
                )
            })?
        }
    };
    let cleaned = cleanup
        .get("value")
        .and_then(|value| value.get("state"))
        .and_then(serde_json::Value::as_str)
        == Some("cleaned")
        || cleanup.get("state").and_then(serde_json::Value::as_str) == Some("cleaned");
    anyhow::ensure!(cleaned, "native media cleanup did not return cleaned");
    Ok(cleanup)
}

pub(super) async fn open_publish_context(
    control: &DeviceControlPlane,
    udid: &str,
) -> anyhow::Result<riviu_core::UiWithStreamContext> {
    let exclusive = control
        .acquire_exclusive(udid, DeviceWorkOwner::Script)
        .await
        .map_err(anyhow::Error::new)?;
    let (exclusive, capacity) = control
        .reserve_ui_capacity(exclusive)
        .await
        .map_err(anyhow::Error::new)?;
    // TikTok restores its last composer/gallery screen when launched with
    // kill_existing=false. Publish needs a deterministic feed entry point, so
    // terminate only the target bundle while the exclusive lease is held;
    // start_interaction_session then launches a clean process before creating
    // the fresh WDA session.
    // **Per device, not a module constant.** The publish path handed the *iOS* bundle to
    // every backend, so on Android this terminated a package that does not exist there and
    // the session that followed was opened against nothing. The interaction path fixed the
    // same defect in its own helper; this one kept it.
    let target_package = control
        .resolve_tiktok_package(exclusive.udid())
        .await
        .map_err(anyhow::Error::new)?;
    control
        .terminate_app(&exclusive, &target_package)
        .await
        .map_err(anyhow::Error::new)?;
    let kind = if control.requires_fresh_text_session(udid) {
        InteractionSessionKind::FreshText
    } else {
        InteractionSessionKind::Ordinary
    };
    let session = control
        .start_interaction_session(exclusive, &target_package, kind)
        .await
        .map_err(anyhow::Error::new)?;
    control
        .start_reserved_stream(session, capacity)
        .await
        .map_err(anyhow::Error::new)
}

pub(super) async fn tap_transition(
    frames: &dyn FrameSource,
    udid: &str,
    session: &dyn riviu_core::UiSession,
    point: TapPoint,
    baseline_sha: &str,
) -> anyhow::Result<Arc<Vec<u8>>> {
    session.tap(point).await?;
    wait_for_changed_frame(frames, udid, baseline_sha, Duration::from_secs(8)).await
}

pub(super) async fn wait_for_frame(
    frames: &dyn FrameSource,
    udid: &str,
    timeout: Duration,
) -> anyhow::Result<Arc<Vec<u8>>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(frame) = frames.latest(udid) {
            return Ok(frame);
        }
        if Instant::now() >= deadline {
            anyhow::bail!("no MJPEG frame for {udid}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub(super) async fn wait_for_changed_frame(
    frames: &dyn FrameSource,
    udid: &str,
    baseline_sha: &str,
    timeout: Duration,
) -> anyhow::Result<Arc<Vec<u8>>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(frame) = frames.latest(udid) {
            if frame_sha256(&frame) != baseline_sha {
                return Ok(frame);
            }
        }
        if Instant::now() >= deadline {
            anyhow::bail!("screen did not advance after TikTok publish action");
        }
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
}

/// Wait for the frame that counts as "posted", and answer with **that frame's** screening.
///
/// The screening travels with the frame because the two were once separated: the evidence
/// recorded the screening of `after_post_tap` while hashing the frame this loop accepted —
/// so `accountLockScreened: "not_locked"` could describe a frame nobody kept, one or two
/// screens before the one the verdict stands on, whose own OCR read had failed and been
/// discarded right here. Returning the pair makes the association structural; a caller
/// cannot hash one frame and report another's reading.
pub(super) async fn wait_for_post_frame(
    frames: &dyn FrameSource,
    udid: &str,
    baseline_sha: &str,
    before_redness: f64,
    timeout: Duration,
) -> anyhow::Result<(Arc<Vec<u8>>, LockScreening)> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(frame) = frames.latest(udid) {
            if frame_sha256(&frame) != baseline_sha {
                let screening = frame_reports_account_lock(&frame).await;
                if screening.is_locked() {
                    anyhow::bail!("TikTok account status blocked the post: account_locked");
                }
                let after_redness = bottom_right_redness(&frame);
                if before_redness < 0.01 || after_redness < before_redness * 0.65 {
                    return Ok((frame, screening));
                }
            }
        }
        if Instant::now() >= deadline {
            anyhow::bail!("post frame did not leave the TikTok composer");
        }
        tokio::time::sleep(Duration::from_millis(160)).await;
    }
}

/// What reading the account-status alert off a frame actually produced.
///
/// Three states, because the difference between them is what the evidence has to record.
/// Collapsing `Unavailable` into `NotLocked` is what let a run on a host with no reader write
/// `accountLockScreened: true`.
// On a host with no OCR the reader only ever produces `Unavailable`, so the other two
// variants are unconstructed in that build. They stay because the *type* is the contract:
// dropping them off non-macOS would put the host back inside the verdict, which is the bug
// this enum replaced.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LockScreening {
    /// The frame was read and carries no account-status alert.
    NotLocked,
    /// The frame was read and it is an account-status alert.
    Locked,
    /// **Nobody read it.** No OCR on this host, or the reader failed on this frame.
    Unavailable,
}

impl LockScreening {
    pub(super) fn is_locked(self) -> bool {
        matches!(self, Self::Locked)
    }

    /// What goes in the evidence, so a run can be told apart afterwards.
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::NotLocked => "not_locked",
            Self::Locked => "locked",
            Self::Unavailable => "unavailable",
        }
    }
}

/// A post leaving the composer is only a transport transition. TikTok can
/// replace that transition with an account-status alert, which must remain an
/// uncertain effect rather than being reported as a successful post.
///
/// # This screen is only ever read on macOS, and the caller's success is a colour guess
///
/// **Off macOS nothing reads the frame at all**, including a frame that is an account-status
/// alert: the reader is Vision OCR, which exists on no other host, and there is no iOS hardware
/// on this fleet to measure a replacement against. On macOS the reader can also simply fail on
/// a given frame. Both of those are [`LockScreening::Unavailable`] — **not** `NotLocked`, which
/// is a positive statement that somebody looked. Returning `false` for "nobody looked" is how
/// the evidence came to claim a screening that never happened.
///
/// That matters because of what the caller does with it. [`wait_for_post_frame`] decides a post
/// succeeded from two signals — the frame changed, and the red of the Post button left the
/// bottom-right corner. **Any** screen that satisfies both is accepted: an account-status
/// alert, an error modal, a network sheet, a system permission dialog. The known alert text is
/// the only one anything screens for, and only where OCR runs. With screening unavailable, a
/// blocked post is recorded as `Posted`, with a `frameSha256` of whatever screen stopped it.
///
/// What this does and does not do about that:
///
/// * The evidence names how the verdict was reached (`postConfirmedBy`) and what the screening
///   actually produced (`accountLockScreened`: `not_locked` / `unavailable`), so a run that was
///   never screened is distinguishable afterwards from one that passed.
/// * It does **not** refuse. Failing closed here — declining the pixel route on a host with no
///   reader — is a real option and costs this fleet nothing, because its twenty phones are
///   Android and [`route_of`] sends them through the measured composer instead. It is left
///   undone deliberately: it would disable iOS publishing outright on every non-macOS host, and
///   that is the operator's call to make, not a side effect of a review fix.
///
/// Measuring a text-free signal for the alert — its layout, or a button label read through the
/// element tree rather than OCR — is the real fix, and it needs an iOS device to measure on.
pub(super) async fn frame_reports_account_lock(frame: &[u8]) -> LockScreening {
    #[cfg(target_os = "macos")]
    {
        // A reader that errored did not say "no alert". It said nothing.
        let Ok(observations) = crate::interaction_ocr::recognize(frame).await else {
            return LockScreening::Unavailable;
        };
        let text = observations
            .iter()
            .map(|observation| observation.text.to_lowercase())
            .collect::<Vec<_>>()
            .join(" ");
        if account_status_text_is_locked(&text) {
            LockScreening::Locked
        } else {
            LockScreening::NotLocked
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = frame;
        LockScreening::Unavailable
    }
}

/// Read by the macOS OCR path; the tests exercise the matcher on every platform.
#[cfg(any(target_os = "macos", test))]
pub(super) fn account_status_text_is_locked(text: &str) -> bool {
    let account_status = text.contains("trạng thái tài khoản")
        || text.contains("trang thai tai khoan")
        || text.contains("account status");
    let locked = text.contains("tài khoản của bạn đã bị khóa")
        || text.contains("tai khoan cua ban da bi khoa")
        || (text.contains("account") && text.contains("locked"));
    account_status && locked
}

pub(super) fn bottom_right_redness(frame: &[u8]) -> f64 {
    let Ok(image) = image::load_from_memory(frame) else {
        return 0.0;
    };
    let image = image.to_rgb8();
    let (width, height) = image.dimensions();
    let x0 = (width as f64 * 0.60) as u32;
    let y0 = (height as f64 * 0.82) as u32;
    let mut red = 0usize;
    let mut total = 0usize;
    for y in (y0..height).step_by(4) {
        for x in (x0..width).step_by(4) {
            let pixel = image.get_pixel(x, y).0;
            total += 1;
            if pixel[0] > 150
                && pixel[0] > pixel[1].saturating_add(45)
                && pixel[0] > pixel[2].saturating_add(35)
            {
                red += 1;
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        red as f64 / total as f64
    }
}

pub(super) fn is_public_post_confirmation(frame: &[u8]) -> bool {
    let Ok(image) = image::load_from_memory(frame) else {
        return false;
    };
    let image = image.to_rgb8();
    let (width, height) = image.dimensions();
    if width < 20 || height < 20 {
        return false;
    }
    let sample = |x0: f64, y0: f64, x1: f64, y1: f64| -> f64 {
        let mut sum = 0.0;
        let mut count = 0.0;
        let left = (width as f64 * x0) as u32;
        let top = (height as f64 * y0) as u32;
        let right = (width as f64 * x1).min(width as f64) as u32;
        let bottom = (height as f64 * y1).min(height as f64) as u32;
        for y in (top..bottom).step_by(8) {
            for x in (left..right).step_by(8) {
                let p = image.get_pixel(x, y).0;
                sum += (p[0] as f64 + p[1] as f64 + p[2] as f64) / 3.0;
                count += 1.0;
            }
        }
        if count == 0.0 {
            0.0
        } else {
            sum / count
        }
    };
    let corner = sample(0.0, 0.0, 0.10, 0.15);
    let dialog = sample(0.14, 0.30, 0.86, 0.70);
    corner < 190.0 && dialog > 190.0
}

pub(super) fn frame_sha256(frame: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(frame);
    format!("{:x}", digest.finalize())
}

/// Which bundle a phone is supposed to receive.
///
/// Pulled out as a function because getting it wrong is invisible from anywhere else in the
/// system: the state machine, the evidence and the toast all look identical whether the phone
/// was handed its own pictures or somebody else's. The only place that can tell is here.
pub(super) fn bundle_for_assignment<'a>(
    bundles: &'a [riviu_core::PublishBundle],
    assignment: &riviu_core::PublishAssignmentRecord,
) -> anyhow::Result<&'a riviu_core::PublishBundle> {
    bundles
        .iter()
        .find(|bundle| bundle.id == assignment.bundle_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "assignment {} names bundle {} which this campaign does not have",
                assignment.id,
                assignment.bundle_id
            )
        })
}

/// A directory holding exactly one bundle directory, for exactly one assignment.
///
/// **The shape matters and the two backends disagree about it.** The iOS sidecar's
/// `_media_file_manifest` iterates `source_root.iterdir()`, skips anything that is not a
/// directory, and only then reads the files inside — so `source_root` must be a directory of
/// *bundle directories*. (Android's `publish::stage` reads files directly instead, but this
/// whole path refuses Android at `refuse_devices_this_path_cannot_drive`, so the iOS shape is
/// the only one that runs here.) Handing it the bundle directory itself would produce an
/// empty manifest and stage nothing at all.
///
/// So a per-assignment root is built by copying, rather than by pointing at a bundle inside
/// the shared campaign root. Copying also re-verifies every image hash and the caption hash
/// immediately before the bytes leave for the phone, which the shared root never did.
pub(super) struct SingleBundleRoot(PathBuf);

impl SingleBundleRoot {
    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for SingleBundleRoot {
    /// Removed even when the transfer bails out mid-loop, which it does on the first phone
    /// that fails.
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Build the one-bundle tree this assignment will be staged from.
///
/// The `.transfer` parent is dot-prefixed on purpose: both the iOS manifest walker and the
/// Android stage skip dot entries, so a scratch directory sitting beside the real bundles can
/// never be mistaken for one.
pub(super) fn stage_one_bundle(
    bundle: &riviu_core::PublishBundle,
    ordinal: u32,
) -> anyhow::Result<SingleBundleRoot> {
    let bundle_dir = PathBuf::from(&bundle.source_path);
    let campaign_root = bundle_dir
        .parent()
        .ok_or_else(|| anyhow::anyhow!("managed bundle {} has no parent", bundle.id))?;
    let name = bundle_dir
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("managed bundle {} has no directory name", bundle.id))?;
    let root = campaign_root.join(".transfer").join(ordinal.to_string());
    // A leftover from an interrupted run must not merge with this one.
    let _ = fs::remove_dir_all(&root);
    let guard = SingleBundleRoot(root);
    // Reuse the copier the campaign was built with: it verifies every image SHA-256 and the
    // caption hash as it writes.
    riviu_core::copy_bundle_to_managed(bundle, &guard.path().join(name))?;
    Ok(guard)
}

/// One assignment, one device-side identity.
///
/// Deliberately not the campaign id. That is one value for the whole campaign, so every phone
/// shared one staging directory, one manifest hash and one album name — which is how the
/// pairing the operator set up stopped meaning anything. `campaign_id` is a UUID and the
/// ordinal is small, so this stays inside the 128-character `[A-Za-z0-9._-]` component that
/// every backend validates.
pub(super) fn device_campaign_id(campaign_id: &str, ordinal: u32) -> String {
    format!("{campaign_id}-{ordinal}")
}

pub(super) fn import_id_from_evidence(raw: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let native = value.get("nativeImport")?;
    let value = native.get("value").unwrap_or(native);
    value
        .get("importId")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

pub(super) fn parse_run_at(raw: &str) -> Result<NaiveDateTime, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("runAt cannot be empty".into());
    }
    let parsed = NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S"))
        .map_err(|_| "runAt must use YYYY-MM-DDTHH:MM or YYYY-MM-DDTHH:MM:SS".to_string())?;
    if parsed < Local::now().naive_local() {
        return Err("runAt is in the past; choose Run now or a future time".into());
    }
    Ok(parsed)
}

/// iOS: press the composer at coordinates somebody measured once, checking each frame changed.
///
/// Unchanged behaviour, lifted out of `post_one_assignment` so the hierarchy route can sit
/// beside it rather than inside an `if` in the middle of a function that also opens leases and
/// cleans up media.
///
/// Failures before the write-ahead callback are retryable; failures after it are unknown. The
/// callback is the only trustworthy boundary on this coordinate-driven route because a failed
/// transport cannot otherwise prove whether the Post tap reached the phone.
pub(super) async fn post_through_the_pixel_grid(
    frames: &dyn FrameSource,
    session: &dyn riviu_core::driver::UiSession,
    campaign_id: &str,
    udid: &str,
    bundle: &riviu_core::PublishBundle,
    import: &str,
    before_post: &mut (dyn FnMut() -> anyhow::Result<()> + Send),
) -> PostOutcome {
    if bundle.images.len() > IOS_PIXEL_GRID_MAX_IMAGES {
        return PostOutcome::NothingPublished(format!(
            "bundle {} has {} images and this composer has {IOS_PIXEL_GRID_MAX_IMAGES} tap points",
            bundle.id,
            bundle.images.len()
        ));
    }
    let mut crossed_effect_boundary = false;
    let evidence = async {
        let before = wait_for_frame(frames, udid, Duration::from_secs(8)).await?;
        let before_sha = frame_sha256(&before);

        // **Each tap is proved against the frame *it* started from, not against the first
        // one.** All four compared with `before_sha`, so once the composer opened every later
        // wait was already satisfied — a dropped gallery tap "succeeded" because the screen
        // differed from the *feed*, and the fixed album and grid coordinates then landed on
        // the wrong screen. On this route that publishes unrelated photographs under the
        // requested caption.
        let mut previous = before_sha;
        for point in [PLUS_BUTTON, GALLERY_BUTTON, ALBUM_PICKER, ALBUM_ROW] {
            previous =
                frame_sha256(&tap_transition(frames, udid, session, point, &previous).await?);
        }

        let grid_x = [105.0, 230.0, 355.0];
        let grid_y = [131.0, 255.0, 380.0, 505.0];
        for index in 0..bundle.images.len() {
            let point = TapPoint {
                x: grid_x[index % 3],
                y: grid_y[index / 3],
            };
            session.tap(point).await?;
            tokio::time::sleep(Duration::from_millis(180)).await;
        }
        let selected = wait_for_frame(frames, udid, Duration::from_secs(5)).await?;
        let selected_sha = frame_sha256(&selected);
        session.tap(COMPOSER_NEXT).await?;
        // Same chaining as the entry taps: both of these compared with `selected_sha`, so the
        // second wait was already satisfied by the first screen change.
        let after_next = frame_sha256(
            &wait_for_changed_frame(frames, udid, &selected_sha, Duration::from_secs(8)).await?,
        );
        session.tap(EDIT_NEXT).await?;
        wait_for_changed_frame(frames, udid, &after_next, Duration::from_secs(8)).await?;

        if !session.supports_text_input() {
            anyhow::bail!("combined Agent text capability is not active for this session");
        }
        session.tap_native(CAPTION_FIELD).await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        session.type_text(&bundle.caption).await?;
        let typed = wait_for_frame(frames, udid, Duration::from_secs(5)).await?;
        let typed_sha = frame_sha256(&typed);
        let post_red_before = bottom_right_redness(&typed);
        before_post()?;
        crossed_effect_boundary = true;
        session.tap_native(POST_BUTTON).await?;
        let after_post_tap =
            wait_for_changed_frame(frames, udid, &typed_sha, Duration::from_secs(8)).await?;
        // An early bail only — this reading describes `after_post_tap`, not the frame the
        // verdict will hash, so it must not survive into the evidence.
        if frame_reports_account_lock(&after_post_tap)
            .await
            .is_locked()
        {
            anyhow::bail!("TikTok account status blocked the post: account_locked");
        }
        let confirmation_sha = if is_public_post_confirmation(&after_post_tap) {
            session.tap_native(PUBLIC_POST_CONFIRM).await?;
            frame_sha256(
                &wait_for_changed_frame(
                    frames,
                    udid,
                    &frame_sha256(&after_post_tap),
                    Duration::from_secs(8),
                )
                .await?,
            )
        } else {
            frame_sha256(&after_post_tap)
        };
        let (posted, screening) = wait_for_post_frame(
            frames,
            udid,
            &confirmation_sha,
            post_red_before,
            Duration::from_secs(15),
        )
        .await?;
        Ok::<serde_json::Value, anyhow::Error>(serde_json::json!({
            "state":"posted",
            "campaignId": campaign_id,
            "bundleId": bundle.id,
            "importId": import,
            "imageCount": bundle.images.len(),
            "captionSha256": bundle.caption_sha256,
            "frameSha256": frame_sha256(&posted),
            "postButtonRednessBefore": post_red_before,
            // **Say what this verdict is made of.** It is not a located control; it is a frame
            // that changed and lost its red corner. `accountLockScreened` reports what the
            // reader actually produced **on the frame `frameSha256` names** — the pair comes
            // back together from `wait_for_post_frame`, because recording an earlier frame's
            // reading here once let a run claim "not_locked" for a frame nobody had read.
            // `unavailable` covers both "no OCR on this host" and "OCR failed on this frame";
            // it used to be `cfg!(target_os = "macos")`, which answered a question about the
            // build rather than about this run.
            "postConfirmedBy": "frame_change_and_redness_drop",
            "accountLockScreened": screening.as_str(),
        }))
    }
    .await;
    // The write-ahead callback is the phase signal: before it, no Post tap was attempted; after
    // it, the transport/frame result cannot prove whether the tap landed.
    match evidence {
        Ok(evidence) => PostOutcome::Posted(evidence),
        Err(error) if crossed_effect_boundary => PostOutcome::Unknown(error.to_string()),
        Err(error) => PostOutcome::NothingPublished(error.to_string()),
    }
}

/// Android: drive the measured composer, and report what it proved.
///
/// # The verdict is carried through rather than flattened into an error
///
/// [`riviu_core::tiktok_composer::ComposerVerdict::may_retry`] is the one question that
/// matters after a failed post, and it is answerable here and nowhere else — the composer
/// knows whether it refused before opening anything or tapped Post and lost the answer. The
/// campaign loop used to turn every failure into `uncertain`, which is permanently
/// unclaimable; most of what it stranded had published nothing at all.
///
/// # Refusing on an unmeasured build, again, here
///
/// The campaign already refused these devices before transferring anything. This checks a
/// second time because the two checks answer at different moments and the phone can change in
/// between: an app that updated between transfer and post has a `versionName` this catalogue
/// may not know, and the resource ids it keys on are reassigned on every rebuild.
// The request fields remain explicit because the source-order regression tests verify that the
// immutable campaign/card identity and the one-shot callback reach the same measured composer.
#[allow(clippy::too_many_arguments)]
pub(super) async fn post_through_the_composer(
    control: &DeviceControlPlane,
    session: &dyn riviu_core::driver::UiSession,
    campaign_id: &str,
    udid: &str,
    bundle: &riviu_core::PublishBundle,
    import: &str,
    sound_policy: &riviu_core::PublishSoundPolicy,
    before_post: &mut (dyn FnMut(Option<&riviu_core::SoundSelectionEvidence>) -> anyhow::Result<()>
              + Send),
) -> PostOutcome {
    use riviu_core::tiktok_composer::{
        publish_carousel_with_sound_effect_intent, CarouselRequest, ComposerPlan, ComposerVerdict,
        Screen,
    };

    // **Every refusal in this block happens before the first tap**, so each is
    // `NothingPublished` rather than `Unknown`. They used to be `?`, which the caller turned
    // into `uncertain` — permanently unclaimable — for builds that had simply never been
    // measured, which today is all of them.
    let refuse = |reason: String| PostOutcome::NothingPublished(format!("{udid}: {reason}"));

    let package = match control.resolve_tiktok_package(udid).await {
        Ok(package) => package,
        Err(error) => return refuse(format!("không xác định được bản TikTok ({error})")),
    };
    let language = session.ui_language().await.unwrap_or_default();
    let version = session.app_version(&package).await.unwrap_or_default();
    let Some(labels) = riviu_core::tiktok_labels::controls_for(&package, &language, &version)
    else {
        return refuse(format!("chưa đo nhãn TikTok cho {package} / {language:?}"));
    };
    let plan = match ComposerPlan::resolve(&labels) {
        Ok(plan) => plan,
        Err(refusal) => return refuse(refusal.to_string()),
    };
    if !plan.can_publish() {
        return refuse(format!(
            "bản build này chưa đo {:?}",
            ComposerPlan::missing_to_publish(&labels)
        ));
    }
    let sound_plan = match sound_plan_for_build(&package, &language, &version) {
        Ok(plan) => plan,
        Err(error) => return refuse(error.to_string()),
    };
    let video_plan = if matches!(bundle.media_kind, riviu_core::PublishMediaKind::Video) {
        match video_plan_for_build(&package, &language, &version) {
            Ok(plan) => Some(plan),
            Err(error) => return refuse(error.to_string()),
        }
    } else {
        None
    };

    let (width, height) = match riviu_core::screen::measured_screen_size(session).await {
        Ok(size) => size,
        Err(error) => return refuse(format!("không đo được kích thước màn hình ({error})")),
    };
    let Some(screen) = Screen::new(width, height) else {
        return refuse(format!("máy báo màn hình {width}x{height}"));
    };

    // The same human-looking touch planner every other session uses, built by the crate that
    // owns the policy rather than assembled here.
    let plan_tap = riviu_core::tiktok_composer::human_taps(screen);

    let stop = std::sync::atomic::AtomicBool::new(false);
    // The callback divides a transport error into the provably pre-Post and may-be-live halves.
    let mut crossed_effect_boundary = false;
    let mut record_effect_intent = |selection: &riviu_core::SoundSelectionEvidence| {
        before_post(Some(selection))?;
        crossed_effect_boundary = true;
        Ok(())
    };
    let result = match bundle.media_kind {
        riviu_core::PublishMediaKind::Image => {
            let request = CarouselRequest {
                album: import,
                images: bundle.images.len(),
                caption: &bundle.caption,
                screen,
            };
            publish_carousel_with_sound_effect_intent(
                session,
                plan,
                sound_plan,
                sound_policy,
                plan_tap,
                &request,
                &stop,
                &mut record_effect_intent,
            )
            .await
        }
        riviu_core::PublishMediaKind::Video => {
            use riviu_core::tiktok_composer::{
                publish_video_with_sound_effect_intent, VideoRequest,
            };
            let request = VideoRequest {
                album: import,
                caption: &bundle.caption,
                screen,
            };
            publish_video_with_sound_effect_intent(
                session,
                plan,
                video_plan.expect("video branch resolves its tuple before the first tap"),
                sound_plan,
                sound_policy,
                plan_tap,
                &request,
                &stop,
                &mut record_effect_intent,
            )
            .await
        }
    };
    let (verdict, sound_selection) = match result {
        Ok(verdict) => verdict,
        Err(error) if crossed_effect_boundary => {
            return PostOutcome::Unknown(format!("{udid}: {error}"))
        }
        Err(error) => return PostOutcome::NothingPublished(format!("{udid}: {error}")),
    };
    let mut evidence = serde_json::json!({
        "state": if verdict.is_posted() { "posted" } else { "not_posted" },
        "route": "hierarchy",
        "verdict": format!("{verdict:?}"),
        "campaignId": campaign_id,
        "bundleId": bundle.id,
        "importId": import,
        "mediaKind": bundle.media_kind,
        "imageCount": bundle.images.len(),
        "captionSha256": bundle.caption_sha256,
        "labels": labels.provenance(),
        "soundPickerProvenance": sound_plan.provenance(),
        "soundSelection": sound_selection,
    });
    if let Some(video) = bundle.video.as_ref() {
        evidence["videoSha256"] = serde_json::Value::String(video.sha256.clone());
        evidence["videoFileName"] = serde_json::Value::String(video.file_name.clone());
        evidence["videoDurationMs"] = serde_json::Value::from(video.duration_ms);
        evidence["videoPickerProvenance"] = serde_json::Value::String(
            video_plan
                .expect("video evidence follows a resolved video tuple")
                .provenance()
                .to_string(),
        );
    }
    match verdict {
        ComposerVerdict::Posted => {
            // **Through the route, never off the feed.** The first wiring called
            // `capture_post_link` straight here, on the claim it would fail closed until M7.
            // It would not have: after Post the screen is the FEED, where Share belongs to
            // whatever video is playing — a control this build HAS measured, over a copy row
            // that matches the English needles — so it would have read back a stranger's
            // post link, which passes `looks_like_a_post_link` because it is one. A wrong
            // link is the single shape the outbox schema cannot tell from a right one.
            //
            // `capture_own_post_link` is the measured answer (§9.136): Profile tab, skip the
            // pinned tile, open tiles until one renders THIS run's caption, and only then
            // open the share sheet. The caption is the identity proof, because ownership is
            // not one — the pinned post is this account's too.
            //
            // Fail-closed all the way down, and it can never downgrade `Posted`: the
            // carousel is out before this line runs, so every refusal below is a statement
            // about the *link*. `postUrl` appears only for a link read off a page that
            // proved itself ours; `linkCaptureReason` always says what happened.
            let capture =
                riviu_core::tiktok_share::capture_own_post_link(session, &labels, &bundle.caption)
                    .await;
            if let Some(link) = capture.link() {
                evidence["postUrl"] = serde_json::Value::String(link.to_string());
            }
            evidence["linkCaptureReason"] = serde_json::Value::String(capture.reason());
            PostOutcome::Posted(evidence)
        }
        other if other.may_retry() => PostOutcome::NothingPublished(other.reason().to_string()),
        other => PostOutcome::Unknown(other.reason().to_string()),
    }
}
