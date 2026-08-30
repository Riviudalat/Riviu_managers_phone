use crate::command_error::CommandError;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{Local, NaiveDateTime};
use riviu_core::db::Database;
use riviu_core::DeviceControlPlane;
use riviu_core::{
    copy_bundle_to_managed, scan_publish_folder, DeviceWorkOwner, PublishCampaignDetail,
    PublishCampaignRecord, PublishCampaignRequest, PublishCleanupPolicy, PublishFolderManifest,
    PublishScanOptions, PublishVisibility,
};
use riviu_core::{FrameSource, InteractionSessionKind, TapPoint};
use tauri::State;
use uuid::Uuid;

use crate::state::AppState;

fn err(error: impl std::fmt::Display) -> CommandError {
    CommandError::operation(error)
}

#[tauri::command]
pub fn publish_scan_folder(
    state: State<'_, AppState>,
    source_root: String,
) -> Result<PublishFolderManifest, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    scan_publish_folder(PathBuf::from(source_root), PublishScanOptions::default()).map_err(err)
}

/// Deal `wanted` not-yet-published bundles onto the first `wanted` selected phones.
///
/// **The operator used to tick boxes.** With twenty-one folders and twenty phones that is a
/// pairing done by hand every run, and the pairing is positional all the way down — a mistake
/// there posts one account's photographs under another's caption, silently, with no delete.
///
/// The pool is what has **not** been dispatched, read from the assignment rows rather than
/// from a counter: see [`riviu_core::publish::auto_assign_bundles`] for the three ways a
/// counter got that wrong.
///
/// Returns the plan for the page to show. Nothing is created here — the operator still presses
/// the button that creates the campaign, with the pairing in front of them.
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
pub fn publish_create_campaign(
    state: State<'_, AppState>,
    source_root: String,
    bundle_ids: Vec<String>,
    udids: Vec<String>,
    run_at: Option<String>,
) -> Result<PublishCampaignRecord, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    if let Some(raw) = run_at.as_deref() {
        parse_run_at(raw)?;
    }
    for udid in &udids {
        if state.registry.get(udid).is_none() {
            return Err(err(format!("device is not connected: {udid}")));
        }
    }

    let manifest = scan_publish_folder(&source_root, PublishScanOptions::default()).map_err(err)?;
    let selected = bundle_ids
        .iter()
        .map(|id| {
            manifest
                .bundles
                .iter()
                .find(|bundle| bundle.id == *id)
                .cloned()
                .ok_or_else(|| format!("bundle not found in manifest: {id}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
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
    };
    match state.db.create_publish_campaign(&request, &managed) {
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
pub fn publish_cancel(state: State<'_, AppState>, campaign_id: String) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state.db.cancel_publish_campaign(&campaign_id).map_err(err)
}

/// Move a queued campaign into the explicit transfer state. The phone is not
/// touched here; transfer and post are separate effect boundaries.
#[tauri::command]
pub fn publish_prepare(
    state: State<'_, AppState>,
    campaign_id: String,
) -> Result<PublishCampaignDetail, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let detail = state
        .db
        .get_publish_campaign(&campaign_id)
        .map_err(err)?
        .ok_or_else(|| "publish campaign not found".to_string())?;
    // **`Posting` and `Verifying` are in this list now.** Prepare walks a campaign back to
    // `Ready`, which is a state Transfer accepts — so a campaign with phones currently inside
    // TikTok's composer could be pushed back to the start of the pipeline and re-transferred
    // underneath the run that was still going.
    if matches!(
        detail.campaign.state,
        riviu_core::PublishCampaignState::Cancelled
            | riviu_core::PublishCampaignState::Succeeded
            | riviu_core::PublishCampaignState::Uncertain
            | riviu_core::PublishCampaignState::Posting
            | riviu_core::PublishCampaignState::Verifying
    ) {
        return Err(err(format!(
            "campaign is already terminal or in flight: {:?}",
            detail.campaign.state
        )));
    }
    state
        .db
        .update_publish_campaign_state(&campaign_id, riviu_core::PublishCampaignState::Ready, None)
        .map_err(err)?;
    state
        .db
        .get_publish_campaign(&campaign_id)
        .map_err(err)?
        .ok_or_else(|| err("campaign disappeared after prepare"))
}

#[tauri::command]
pub async fn publish_transfer(
    state: State<'_, AppState>,
    campaign_id: String,
) -> Result<PublishCampaignDetail, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    transfer_publish_campaign_inner(
        state.control.clone(),
        state.db.clone(),
        state.events.clone(),
        state.active_agent_bundle_id.clone(),
        campaign_id,
    )
    .await
    .map_err(err)
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
    // Refused here too, not only before posting. Transferring first would push media onto a
    // phone that can never be posted from, and then leave it there.
    let mut reports = Vec::new();
    for assignment in &detail.assignments {
        reports.push((
            assignment.udid.as_str(),
            readiness_of(&control, &assignment.udid).await,
        ));
    }
    refuse_devices_whose_composer_is_not_measured(reports)?;
    // And the same argument for the bundle rather than the device: an image count this
    // composer's grid cannot reach fails at `post_one_assignment`, which is *after* the media
    // is on the phone and visible to TikTok.
    //
    // **Per device now, not one number for the campaign.** The two routes have different
    // ceilings — twelve tap points somebody wrote down, against however many grid cells fit on
    // the screen — and using the smaller for both refused Android bundles that post fine.
    refuse_assignments_whose_bundle_is_too_large(detail.assignments.iter().filter_map(
        |assignment| {
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
        },
    ))?;
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
        // above makes this unreachable today; it stays because the two guards answer different
        // questions, and a path that claimed the campaign some other way would still need it.
        if matches!(
            assignment.state,
            riviu_core::PublishCampaignState::Succeeded
                | riviu_core::PublishCampaignState::Posting
                | riviu_core::PublishCampaignState::Verifying
                | riviu_core::PublishCampaignState::Uncertain
        ) {
            continue;
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
        // A failure to write it is not a reason to abandon the transfer: the row is for
        // recovery, and losing recovery is better than losing the run.
        if let Err(error) = db.update_publish_assignment_state(
            &assignment.id,
            riviu_core::PublishCampaignState::Transferring,
            None,
            None,
        ) {
            log::warn!(
                "could not mark {} in flight before its transfer: {error}",
                assignment.udid
            );
        }
        let context = match control
            .acquire_exclusive(&assignment.udid, DeviceWorkOwner::Script)
            .await
        {
            Ok(context) => context,
            Err(error) => {
                let message = error.to_string();
                db.update_publish_assignment_state(
                    &assignment.id,
                    riviu_core::PublishCampaignState::Uncertain,
                    Some("media_transfer_context_failed"),
                    Some(&serde_json::json!({"message": message}).to_string()),
                )?;
                db.update_publish_campaign_state(
                    &campaign_id,
                    riviu_core::PublishCampaignState::Uncertain,
                    Some("media_transfer_context_failed"),
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
                    Err(error) => {
                        let message = error.to_string();
                        db.update_publish_assignment_state(
                            &assignment.id,
                            riviu_core::PublishCampaignState::Uncertain,
                            Some("media_transfer_native_failed"),
                            Some(&serde_json::json!({"message": message}).to_string()),
                        )?;
                        db.update_publish_campaign_state(
                            &campaign_id,
                            riviu_core::PublishCampaignState::Uncertain,
                            Some("media_transfer_native_failed"),
                        )?;
                        anyhow::bail!("native media import failed: {message}");
                    }
                }
            }
            Err(error) => {
                let message = error.to_string();
                db.update_publish_assignment_state(
                    &assignment.id,
                    riviu_core::PublishCampaignState::Uncertain,
                    Some("media_transfer_failed"),
                    Some(&serde_json::json!({"message": message}).to_string()),
                )?;
                db.update_publish_campaign_state(
                    &campaign_id,
                    riviu_core::PublishCampaignState::Uncertain,
                    Some("media_transfer_failed"),
                )?;
                anyhow::bail!("media transfer failed: {message}");
            }
        }
    }
    db.update_publish_campaign_state(
        &campaign_id,
        riviu_core::PublishCampaignState::Imported,
        None,
    )?;
    db.get_publish_campaign(&campaign_id)?
        .ok_or_else(|| anyhow::anyhow!("campaign disappeared after transfer"))
}

const PLUS_BUTTON: TapPoint = TapPoint { x: 187.0, y: 640.0 };
const GALLERY_BUTTON: TapPoint = TapPoint { x: 32.0, y: 632.0 };
const ALBUM_PICKER: TapPoint = TapPoint { x: 187.0, y: 47.0 };
const ALBUM_ROW: TapPoint = TapPoint { x: 187.0, y: 385.0 };
const COMPOSER_NEXT: TapPoint = TapPoint { x: 300.0, y: 618.0 };
const EDIT_NEXT: TapPoint = TapPoint { x: 280.0, y: 635.0 };
const CAPTION_FIELD: TapPoint = TapPoint { x: 180.0, y: 240.0 };
const POST_BUTTON: TapPoint = TapPoint { x: 330.0, y: 42.0 };
const PUBLIC_POST_CONFIRM: TapPoint = TapPoint { x: 275.0, y: 444.0 };

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
const PUBLISH_FAN_OUT_STAGGER: Duration = Duration::from_secs(2);

/// Why one phone did not post, and **whether anything may be live because of it**.
///
/// The empty-string sentinel this replaces was a real trap: the caller told a missing bundle
/// apart from every other failure by testing `reason.is_empty()`, so any failure whose message
/// happened to be empty became "bundle missing" — and, worse, the campaign could not tell a
/// phone that refused before opening anything from one that tapped Post and lost the answer.
enum PhoneFailure {
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
async fn post_one_phone(
    stagger: Duration,
    gate: Arc<tokio::sync::Semaphore>,
    control: Arc<DeviceControlPlane>,
    db: Arc<Database>,
    frames: Arc<dyn FrameSource>,
    events: riviu_core::events::EventBus,
    campaign_id: String,
    assignment: riviu_core::PublishAssignmentRecord,
    bundle: riviu_core::PublishBundle,
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

    // The same rule per assignment, and this is the one that stops the worst case: an
    // assignment already `Succeeded` is not walked back to `Posting` and posted again.
    match db.claim_publish_assignment_for_posting(
        &assignment.id,
        &serde_json::json!({"effectIntent":"post_carousel"}).to_string(),
    ) {
        Ok(true) => {}
        Ok(false) => {
            return Err(PhoneFailure::NothingPublished(format!(
                "{} đã đăng, đang được đăng, hoặc chiến dịch đã dừng — bỏ qua",
                assignment.udid
            )))
        }
        Err(error) => {
            return Err(PhoneFailure::NothingPublished(format!(
                "{}: {error}",
                assignment.udid
            )))
        }
    }
    announce(&events, &db, &campaign_id);

    let outcome = post_one_assignment(
        &control,
        &db,
        frames.as_ref(),
        &campaign_id,
        &assignment,
        &bundle,
    )
    .await;
    let (state, code) = state_for_outcome(&outcome);
    let (message, evidence) = match &outcome {
        PostOutcome::Posted(evidence) => (None, evidence.to_string()),
        PostOutcome::NothingPublished(reason) | PostOutcome::Unknown(reason) => (
            Some(reason.clone()),
            serde_json::json!({"message": reason, "effectIntent":"post_carousel"}).to_string(),
        ),
    };
    if let Err(error) =
        db.update_publish_assignment_state(&assignment.id, state, code, Some(&evidence))
    {
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

/// Tell the UI that a campaign moved, once per state write.
///
/// An id and a revision, never the row itself: the page re-reads, so a payload that was
/// already stale by the time it arrived cannot be rendered as current. Same shape as the
/// interaction path's event, and for the same reason.
///
/// **Best effort, and deliberately silent on failure.** Nothing about a post depends on the
/// screen having heard about it, and turning a broadcast error into a failed publish would be
/// the same mistake as letting a Sheets write fail one.
fn announce(events: &riviu_core::events::EventBus, db: &Database, campaign_id: &str) {
    let revision = db
        .publish_campaign_revision(campaign_id)
        .unwrap_or_default();
    events.emit(riviu_core::events::AppEvent::PublishUpdated {
        campaign_id: campaign_id.to_string(),
        revision,
    });
}

/// Which composer drives a given device.
///
/// The partition is `reports_element_bounds`, the same signal the interaction path uses: a
/// device that reports bounds is driven **by label**, and one that does not is driven by
/// pixel. They are not interchangeable, and running the wrong one presses arbitrary places in
/// a layout nobody measured — on a screen where the result cannot be taken down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublishRoute {
    /// iOS: fixed logical coordinates, verified frame by frame.
    PixelGrid,
    /// Android: `tiktok_composer`, every control located by a measured label.
    Hierarchy,
}

/// What a device can actually do, as far as publishing is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PublishReadiness {
    /// Driven by pixel, which this module has coordinates for.
    PixelGrid,
    /// Driven by label, and every label the publish path needs is measured on its build.
    HierarchyReady,
    /// Driven by label, and its build is missing these controls.
    HierarchyMissing(Vec<riviu_core::tiktok_labels::TikTokControl>),
    /// Driven by label, and its (package, language) pair has never been measured at all.
    HierarchyUnknownBuild(String),
}

/// Refuse a campaign holding a device whose composer is not measured.
///
/// **Android is no longer refused outright**, which is what this used to do. It is refused
/// *per build*, which is a different and much narrower statement: the label-driven composer
/// exists now, so the question is whether this phone's TikTok has had the controls read off
/// it — and a phone whose build is unmeasured must still be refused **before** its media is
/// transferred, because that is the last moment refusing is free.
///
/// Taking the readings rather than the control plane keeps it testable without a fleet.
fn refuse_devices_whose_composer_is_not_measured<'a>(
    reports: impl IntoIterator<Item = (&'a str, PublishReadiness)>,
) -> anyhow::Result<()> {
    let mut refusals = Vec::new();
    for (udid, readiness) in reports {
        match readiness {
            PublishReadiness::PixelGrid | PublishReadiness::HierarchyReady => {}
            PublishReadiness::HierarchyMissing(missing) => refusals.push(format!(
                "{udid}: bản TikTok trên máy này chưa đo {} nhãn cần cho việc đăng ({})",
                missing.len(),
                missing
                    .iter()
                    .map(|control| format!("{control:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            PublishReadiness::HierarchyUnknownBuild(detail) => {
                refusals.push(format!("{udid}: {detail}"))
            }
        }
    }
    anyhow::ensure!(
        refusals.is_empty(),
        "không đăng được trên {} máy, vì composer của bản build đó chưa đo:\n  {}\n\nĐo bằng \
         `cargo run -p riviu-android-driver --example composer_scout -- <serial> --album \"<album>\"`.",
        refusals.len(),
        refusals.join("\n  ")
    );
    Ok(())
}

/// What one device can do, read without holding a lease.
///
/// **Two gates, at two depths, and both refuse before anything irreversible.** This is the
/// shallow one: it answers "is there a composer for this device at all", which is a question
/// about the *package*, and `resolve_tiktok_package` already refuses an Android phone whose
/// TikTok is not one of the measured builds.
///
/// The deep one lives in [`post_through_the_composer`] and asks whether that build's *labels*
/// are measured — which needs a session, so it cannot run here, and runs before the first tap
/// instead.
async fn readiness_of(control: &DeviceControlPlane, udid: &str) -> PublishReadiness {
    if !control.reports_element_bounds(udid) {
        return PublishReadiness::PixelGrid;
    }
    let package = match control.resolve_tiktok_package(udid).await {
        Ok(package) => package,
        Err(error) => {
            return PublishReadiness::HierarchyUnknownBuild(format!(
                "không xác định được bản TikTok trên máy này: {error}"
            ))
        }
    };
    // **A necessary condition that can be checked without a lease**, and today it refuses the
    // whole fleet: if *no* catalogued language set for this package has the publishing tail
    // measured, then this phone cannot publish whichever language it is in — so its media must
    // not be transferred at all. When some set can, the phone's own language and app version
    // decide, and that check needs a session, so it runs in `post_through_the_composer` before
    // the first tap.
    let missing = shortest_publish_gap_for(&package);
    match missing {
        Some(missing) if !missing.is_empty() => PublishReadiness::HierarchyMissing(missing),
        Some(_) => PublishReadiness::HierarchyReady,
        None => PublishReadiness::HierarchyUnknownBuild(format!(
            "chưa đo bộ nhãn nào cho gói {package}"
        )),
    }
}

/// The smallest set of unmeasured publish controls across every language set for a package.
///
/// "Smallest" because the phone is in **one** language and this cannot tell which: reporting
/// the union would refuse a phone whose own set is complete. `None` when the package has no
/// catalogued set at all.
fn shortest_publish_gap_for(
    package: &str,
) -> Option<Vec<riviu_core::tiktok_labels::TikTokControl>> {
    riviu_core::tiktok_labels::TIKTOK_LABEL_SETS
        .iter()
        .filter(|set| set.package == package)
        .filter_map(|set| {
            let controls = riviu_core::tiktok_labels::controls_for(
                set.package,
                set.language,
                set.measured_app_version,
            )?;
            Some(riviu_core::tiktok_composer::ComposerPlan::missing_to_publish(&controls))
        })
        .min_by_key(Vec::len)
}

/// The route a device is driven by, from the same signal the gate uses.
fn route_of(control: &DeviceControlPlane, udid: &str) -> PublishRoute {
    if control.reports_element_bounds(udid) {
        PublishRoute::Hierarchy
    } else {
        PublishRoute::PixelGrid
    }
}

/// How many images each route's composer can select.
///
/// Two different facts, and neither is TikTok's own ceiling of 35. The pixel path is bound by
/// the twelve tap points somebody wrote down; the hierarchy path is bound by how many grid
/// cells fit on the screen, which it computes per device — this is only the ceiling used
/// **before transfer**, when no session exists to ask.
pub(crate) fn max_images_for(route: PublishRoute) -> usize {
    match route {
        PublishRoute::PixelGrid => IOS_PIXEL_GRID_MAX_IMAGES,
        PublishRoute::Hierarchy => {
            riviu_core::tiktok_composer::GRID_COLUMNS
                * riviu_core::tiktok_composer::GRID_MEASURED_ROWS
        }
    }
}

/// Refuse a bundle this composer has no tap point for, **before its media leaves the desktop**.
///
/// `post_one_assignment` already refuses one too large, but it refuses at the last possible
/// moment: by then `stage`/`prepare`/`import` have put tens of megabytes into a real phone's
/// gallery and made them visible to TikTok, and the failure leaves them there with no cleanup
/// owner. The scanner cannot make this check — the ceiling belongs to the composer's grid, and
/// a hierarchy-driven composer is not bound by it — so the campaign is the first place that
/// knows both the bundle and the path it is bound for.
///
/// Takes the count rather than reading a constant, for the same reason the sibling above takes
/// a predicate: it is testable without a fleet, and the Android path will pass its own number.
fn refuse_assignments_whose_bundle_is_too_large<'a>(
    rows: impl IntoIterator<Item = (&'a str, &'a riviu_core::PublishBundle, usize)>,
) -> anyhow::Result<()> {
    let oversized: Vec<String> = rows
        .into_iter()
        .filter(|(_, bundle, max_images)| bundle.images.len() > *max_images)
        .map(|(udid, bundle, max_images)| {
            format!(
                "{} ({} ảnh) trên {udid}, composer ở đó chọn được {max_images}",
                bundle.name,
                bundle.images.len()
            )
        })
        .collect();
    anyhow::ensure!(
        oversized.is_empty(),
        "những bài này nhiều ảnh hơn composer của chính máy đó chọn được: {}. Bỏ chúng ra \
         khỏi chiến dịch, hoặc gán vào máy điều khiển theo cây giao diện — composer đó định vị \
         từng ô nên lưới của nó rộng hơn.",
        oversized.join("; ")
    );
    Ok(())
}

#[tauri::command]
pub async fn publish_post(
    state: State<'_, AppState>,
    campaign_id: String,
) -> Result<PublishCampaignDetail, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    post_publish_campaign_inner(
        state.control.clone(),
        state.db.clone(),
        Arc::new(state.streams.clone()),
        state.events.clone(),
        campaign_id,
    )
    .await
    .map_err(err)
}

pub(crate) async fn post_publish_campaign_inner(
    control: Arc<DeviceControlPlane>,
    db: Arc<Database>,
    frames: Arc<dyn FrameSource>,
    events: riviu_core::events::EventBus,
    campaign_id: String,
) -> anyhow::Result<PublishCampaignDetail> {
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
    let mut reports = Vec::new();
    for assignment in &detail.assignments {
        reports.push((
            assignment.udid.as_str(),
            readiness_of(&control, &assignment.udid).await,
        ));
    }
    refuse_devices_whose_composer_is_not_measured(reports)?;
    // Asked per device, not once for the campaign. A campaign spans several
    // phones and a fleet can be mixed, so a single fleet-wide answer would
    // report one device's agent on behalf of the rest. Still fails fast, before
    // any state is mutated, and now names the devices that are short.
    let without_push_media: Vec<&str> = detail
        .assignments
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
    let gate = Arc::new(tokio::sync::Semaphore::new(
        control.stream_capacity().max(1),
    ));
    let mut running = Vec::with_capacity(detail.assignments.len());
    for (index, assignment) in detail.assignments.iter().enumerate() {
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
            assignment.clone(),
            bundle.clone(),
        )));
    }

    let mut failures = Vec::new();
    // **Whether anything may be live, tracked separately from whether anything failed.**
    // Collapsing the two made a run where every phone refused before opening anything end as
    // `uncertain` — permanently unclaimable — with children correctly marked retryable
    // underneath it.
    let mut may_be_live = false;
    for (assignment, task) in detail.assignments.iter().zip(running) {
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
    announce(&events, &db, &campaign_id);
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
enum PostOutcome {
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

async fn post_one_assignment(
    control: &DeviceControlPlane,
    _db: &Database,
    frames: &dyn FrameSource,
    campaign_id: &str,
    assignment: &riviu_core::PublishAssignmentRecord,
    bundle: &riviu_core::PublishBundle,
) -> PostOutcome {
    // **Everything before the phone opens is a refusal, not an unknown.** These used to be
    // `bail!`s that the caller turned into `uncertain` — permanently unclaimable — for a
    // caption nobody could have posted and a phone nobody had touched.
    if bundle.images.is_empty() {
        return PostOutcome::NothingPublished(format!("bundle {} has no images", bundle.id));
    }
    if bundle.caption.chars().count() > 2200 {
        return PostOutcome::NothingPublished(format!(
            "caption for {} exceeds TikTok's 2200 character limit",
            bundle.id
        ));
    }
    let Some(import) = assignment
        .evidence_json
        .as_deref()
        .and_then(import_id_from_evidence)
    else {
        return PostOutcome::NothingPublished(format!(
            "native import proof is missing for {}",
            assignment.udid
        ));
    };
    let context = match open_publish_context(control, &assignment.udid).await {
        Ok(context) => context,
        // The lease, the capacity reservation and the app launch all live in here. None of
        // them reaches TikTok's composer, so a failure means nothing was published — and the
        // media is still on the phone with no context to clean it from, which the operator
        // needs told rather than hidden inside `uncertain`.
        Err(error) => {
            return PostOutcome::NothingPublished(format!(
                "{}: không mở được phiên ({error}); ảnh vẫn còn trên máy",
                assignment.udid
            ))
        }
    };
    let session = match control.streaming_session(&context) {
        Ok(session) => session,
        Err(error) => {
            let cleanup =
                tidy_up_the_imported_media(control, context, &assignment.udid, &import).await;
            return fold_cleanup_into(
                PostOutcome::NothingPublished(format!("{}: {error}", assignment.udid)),
                cleanup,
            );
        }
    };
    if let Some(refusal) = refuse_when_the_route_authorities_disagree(
        &assignment.udid,
        control.reports_element_bounds(&assignment.udid),
        session.supports_element_bounds(),
    ) {
        let cleanup = tidy_up_the_imported_media(control, context, &assignment.udid, &import).await;
        return fold_cleanup_into(refusal, cleanup);
    }
    let action_result = if session.supports_element_bounds() {
        post_through_the_composer(
            control,
            session.as_ref(),
            campaign_id,
            &assignment.udid,
            bundle,
            &import,
        )
        .await
    } else {
        post_through_the_pixel_grid(
            frames,
            session.as_ref(),
            campaign_id,
            &assignment.udid,
            bundle,
            &import,
        )
        .await
    };
    // **Cleanup runs whatever the route said.** It used to sit behind `action_result?`, so
    // every error path left the campaign's images in a real phone's gallery with nothing
    // owning them — including the Android build gate, which refuses *before its first tap*.
    let cleanup = tidy_up_the_imported_media(control, context, &assignment.udid, &import).await;
    fold_cleanup_into(action_result, cleanup)
}

/// Attach the cleanup result to a posting outcome **without changing what the post did**.
///
/// A pure function because the rule it encodes was unreachable by any test while it lived
/// inside `post_one_assignment`, and a reversal proved it: making a cleanup failure downgrade
/// a published post to `Unknown` left the whole suite green. That downgrade is the exact bug
/// the rest of this path is built to prevent — `Unknown` is permanently unclaimable, so a
/// carousel that went out cleanly and left some files behind would need a person to look at a
/// phone where the only problem is disk space.
fn fold_cleanup_into(
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
fn refuse_when_the_route_authorities_disagree(
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

fn state_for_outcome(
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
async fn tidy_up_the_imported_media(
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

async fn open_publish_context(
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

async fn tap_transition(
    frames: &dyn FrameSource,
    udid: &str,
    session: &dyn riviu_core::UiSession,
    point: TapPoint,
    baseline_sha: &str,
) -> anyhow::Result<Arc<Vec<u8>>> {
    session.tap(point).await?;
    wait_for_changed_frame(frames, udid, baseline_sha, Duration::from_secs(8)).await
}

async fn wait_for_frame(
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

async fn wait_for_changed_frame(
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

async fn wait_for_post_frame(
    frames: &dyn FrameSource,
    udid: &str,
    baseline_sha: &str,
    before_redness: f64,
    timeout: Duration,
) -> anyhow::Result<Arc<Vec<u8>>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(frame) = frames.latest(udid) {
            if frame_sha256(&frame) != baseline_sha {
                if frame_reports_account_lock(&frame).await {
                    anyhow::bail!("TikTok account status blocked the post: account_locked");
                }
                let after_redness = bottom_right_redness(&frame);
                if before_redness < 0.01 || after_redness < before_redness * 0.65 {
                    return Ok(frame);
                }
            }
        }
        if Instant::now() >= deadline {
            anyhow::bail!("post frame did not leave the TikTok composer");
        }
        tokio::time::sleep(Duration::from_millis(160)).await;
    }
}

/// A post leaving the composer is only a transport transition. TikTok can
/// replace that transition with an account-status alert, which must remain an
/// uncertain effect rather than being reported as a successful post.
///
/// # This screen is only ever read on macOS, and the caller's success is a colour guess
///
/// **Off macOS this returns `false` for every frame, including a frame that is an
/// account-status alert.** The reader is Vision OCR, which exists on no other host, and there
/// is no iOS hardware on this fleet to measure a replacement against. So on the Windows machine
/// this project is developed and run on, the check is not a check.
///
/// That matters because of what the caller does with it. [`wait_for_post_frame`] decides a post
/// succeeded from two signals — the frame changed, and the red of the Post button left the
/// bottom-right corner — and an account-status alert satisfies both: it is a different screen,
/// and it has no red Post button on it. With OCR unavailable, a blocked post is recorded as
/// `Posted`, with a `frameSha256` of the alert.
///
/// Two things keep that from being silent rather than fixing it, and neither is a fix:
///
/// * The evidence written by the pixel route names how the verdict was reached
///   (`postConfirmedBy`) and whether the alert was screened for at all
///   (`accountLockScreened`), so a run on this host is distinguishable afterwards from a run on
///   a host that could read the screen.
/// * The route is unreachable for this fleet's twenty phones: they are Android, and
///   [`route_of`] sends them through the measured composer, which confirms a post by locating
///   controls rather than by counting red pixels.
///
/// Measuring a text-free signal for the alert — its layout, or a button label read through the
/// element tree rather than OCR — is the real fix, and it needs an iOS device to measure on.
async fn frame_reports_account_lock(frame: &[u8]) -> bool {
    #[cfg(target_os = "macos")]
    {
        let Ok(observations) = crate::interaction_ocr::recognize(frame).await else {
            return false;
        };
        let text = observations
            .iter()
            .map(|observation| observation.text.to_lowercase())
            .collect::<Vec<_>>()
            .join(" ");
        account_status_text_is_locked(&text)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = frame;
        false
    }
}

/// Read by the macOS OCR path; the tests exercise the matcher on every platform.
#[cfg(any(target_os = "macos", test))]
fn account_status_text_is_locked(text: &str) -> bool {
    let account_status = text.contains("trạng thái tài khoản")
        || text.contains("trang thai tai khoan")
        || text.contains("account status");
    let locked = text.contains("tài khoản của bạn đã bị khóa")
        || text.contains("tai khoan cua ban da bi khoa")
        || (text.contains("account") && text.contains("locked"));
    account_status && locked
}

fn bottom_right_redness(frame: &[u8]) -> f64 {
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

fn is_public_post_confirmation(frame: &[u8]) -> bool {
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

fn frame_sha256(frame: &[u8]) -> String {
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
fn bundle_for_assignment<'a>(
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
struct SingleBundleRoot(PathBuf);

impl SingleBundleRoot {
    fn path(&self) -> &Path {
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
fn stage_one_bundle(
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
fn device_campaign_id(campaign_id: &str, ordinal: u32) -> String {
    format!("{campaign_id}-{ordinal}")
}

fn import_id_from_evidence(raw: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let native = value.get("nativeImport")?;
    let value = native.get("value").unwrap_or(native);
    value
        .get("importId")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn parse_run_at(raw: &str) -> Result<NaiveDateTime, CommandError> {
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
/// **Every failure here is [`PostOutcome::Unknown`]**, and that is not pessimism: this path
/// has no signal that separates "the tap never went out" from "it went out and the frame did
/// not change in the way we expected". The hierarchy route does, which is why it can report
/// the difference and this one cannot.
async fn post_through_the_pixel_grid(
    frames: &dyn FrameSource,
    session: &dyn riviu_core::driver::UiSession,
    campaign_id: &str,
    udid: &str,
    bundle: &riviu_core::PublishBundle,
    import: &str,
) -> PostOutcome {
    if bundle.images.len() > IOS_PIXEL_GRID_MAX_IMAGES {
        return PostOutcome::NothingPublished(format!(
            "bundle {} has {} images and this composer has {IOS_PIXEL_GRID_MAX_IMAGES} tap points",
            bundle.id,
            bundle.images.len()
        ));
    }
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
        session.tap_native(POST_BUTTON).await?;
        let after_post_tap =
            wait_for_changed_frame(frames, udid, &typed_sha, Duration::from_secs(8)).await?;
        if frame_reports_account_lock(&after_post_tap).await {
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
        let posted = wait_for_post_frame(
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
            // that changed and lost its red corner, and on a host without OCR nothing screened
            // out an account-status alert. See [`frame_reports_account_lock`].
            "postConfirmedBy": "frame_change_and_redness_drop",
            "accountLockScreened": cfg!(target_os = "macos"),
        }))
    }
    .await;
    // Every failure on this route is unknown, and that is not pessimism: it has no signal that
    // separates "the tap never went out" from "it went out and the frame did not change the
    // way we expected".
    match evidence {
        Ok(evidence) => PostOutcome::Posted(evidence),
        Err(error) => PostOutcome::Unknown(error.to_string()),
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
async fn post_through_the_composer(
    control: &DeviceControlPlane,
    session: &dyn riviu_core::driver::UiSession,
    campaign_id: &str,
    udid: &str,
    bundle: &riviu_core::PublishBundle,
    import: &str,
) -> PostOutcome {
    use riviu_core::tiktok_composer::{
        publish_carousel, CarouselRequest, ComposerPlan, ComposerVerdict, Screen,
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

    let request = CarouselRequest {
        album: import,
        images: bundle.images.len(),
        caption: &bundle.caption,
        screen,
    };
    let stop = std::sync::atomic::AtomicBool::new(false);
    // A transport error from inside the walk is genuinely unknown: it can land either side of
    // the Post tap, and the composer cannot tell the caller which.
    let verdict = match publish_carousel(session, plan, plan_tap, &request, &stop).await {
        Ok(verdict) => verdict,
        Err(error) => return PostOutcome::Unknown(format!("{udid}: {error}")),
    };
    let evidence = serde_json::json!({
        "state": if verdict.is_posted() { "posted" } else { "not_posted" },
        "route": "hierarchy",
        "verdict": format!("{verdict:?}"),
        "campaignId": campaign_id,
        "bundleId": bundle.id,
        "importId": import,
        "imageCount": bundle.images.len(),
        "captionSha256": bundle.caption_sha256,
        "labels": labels.provenance(),
    });
    match verdict {
        ComposerVerdict::Posted => PostOutcome::Posted(evidence),
        other if other.may_retry() => PostOutcome::NothingPublished(other.reason().to_string()),
        other => PostOutcome::Unknown(other.reason().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::account_status_text_is_locked;
    use super::bundle_for_assignment;
    use super::fold_cleanup_into;
    use super::max_images_for;
    use super::refuse_assignments_whose_bundle_is_too_large;
    use super::refuse_devices_whose_composer_is_not_measured;
    use super::refuse_when_the_route_authorities_disagree;
    use super::state_for_outcome;
    use super::PostOutcome;
    use super::IOS_PIXEL_GRID_MAX_IMAGES;
    use super::PUBLISH_FAN_OUT_STAGGER;
    use super::{PublishReadiness, PublishRoute};
    use std::fs;
    use std::time::Duration;
    use uuid::Uuid;

    /// **Two readings of the same phone, and the post waits until they agree.**
    ///
    /// The campaign gate reads `reports_element_bounds` before any session exists; the
    /// dispatch reads `supports_element_bounds` off the live session. The driver contract
    /// permits those to differ, and nothing compared them: a `true` preflight with a `false`
    /// session cleared the measured-label gate and then pressed iOS pixel coordinates.
    #[test]
    fn a_phone_that_answers_the_route_question_twice_over_does_not_post() {
        for (preflight, session) in [(true, false), (false, true)] {
            let refusal = refuse_when_the_route_authorities_disagree("SN-1", preflight, session)
                .expect("a disagreement is a refusal");
            let PostOutcome::NothingPublished(reason) = &refusal else {
                panic!("a disagreement reached no composer, so it published nothing");
            };
            assert!(
                reason.contains("SN-1"),
                "the operator has to know which phone: {reason}"
            );
            // Both readings, in the message. "They disagreed" without the two values sends
            // whoever reads it back to the phone to take the measurement again.
            assert!(
                reason.contains(&format!("{preflight}")) && reason.contains(&format!("{session}")),
                "both readings belong in the message: {reason}"
            );
        }
    }

    /// And a disagreement leaves the campaign runnable again.
    ///
    /// `Unknown` is the permanently-unclaimable state, kept for a phone that may have posted.
    /// Nothing has reached the composer here, so spending it on a disagreement would strand a
    /// campaign that a second run might drive perfectly well.
    #[test]
    fn a_route_disagreement_is_retryable_not_uncertain() {
        let refusal =
            refuse_when_the_route_authorities_disagree("SN-1", true, false).expect("refusal");
        assert_eq!(
            state_for_outcome(&refusal).0,
            riviu_core::PublishCampaignState::FailedBeforeDispatch
        );
    }

    /// Agreement — either way — is not a refusal.
    #[test]
    fn two_authorities_that_agree_let_the_post_through() {
        for both in [true, false] {
            assert!(
                refuse_when_the_route_authorities_disagree("SN-1", both, both).is_none(),
                "agreeing on {both} is the ordinary case"
            );
        }
    }

    fn test_bundle(id: &str) -> riviu_core::PublishBundle {
        riviu_core::PublishBundle {
            id: id.into(),
            source_path: format!("/managed/req-7/{id}"),
            name: id.into(),
            media_kind: riviu_core::PublishMediaKind::Image,
            images: Vec::new(),
            caption_path: format!("/managed/req-7/{id}/caption.txt"),
            caption: String::new(),
            caption_sha256: String::new(),
            total_bytes: 0,
        }
    }

    fn test_assignment(
        id: &str,
        bundle_id: &str,
        udid: &str,
    ) -> riviu_core::PublishAssignmentRecord {
        riviu_core::PublishAssignmentRecord {
            id: id.into(),
            campaign_id: "campaign-1".into(),
            bundle_id: bundle_id.into(),
            ordinal: 0,
            udid: udid.into(),
            state: riviu_core::PublishCampaignState::Ready,
            effect_intent: None,
            evidence_json: None,
            error_code: None,
        }
    }

    #[test]
    fn every_phone_is_given_its_own_bundle_and_not_the_campaign_root() {
        // The defect this pins: the transfer took ONE source root for the whole campaign --
        // `bundles[0].source_path.parent()` -- and staged it to every phone, so the mapping
        // that pairs N folders with N phones decided nothing and phones published each
        // other's pictures under each other's captions, to live accounts.
        let bundles = vec![test_bundle("req-7:bundle-a"), test_bundle("req-7:bundle-b")];
        let first = test_assignment("assign-1", "req-7:bundle-a", "phone-1");
        let second = test_assignment("assign-2", "req-7:bundle-b", "phone-2");

        let for_first = bundle_for_assignment(&bundles, &first).expect("bundle a");
        let for_second = bundle_for_assignment(&bundles, &second).expect("bundle b");

        assert_eq!(for_first.id, "req-7:bundle-a");
        assert_eq!(for_second.id, "req-7:bundle-b");
        assert_ne!(for_first.source_path, for_second.source_path);
    }

    #[test]
    fn a_staged_root_holds_exactly_one_bundle_and_is_removed_afterwards() {
        // The shape is the whole point and it is easy to get wrong -- I got it wrong once
        // already. The iOS sidecar's manifest walker iterates the root's *subdirectories*
        // and only then reads files, so handing it the bundle directory itself yields an
        // empty manifest and stages nothing. The root must contain one bundle DIRECTORY.
        let temp = std::env::temp_dir().join(format!("riviu-stage-{}", Uuid::new_v4()));
        let bundle_dir = temp.join("bundle-a");
        fs::create_dir_all(&bundle_dir).expect("create the source bundle");
        fs::write(bundle_dir.join("01.png"), b"png").expect("write an image");

        let mut bundle = test_bundle("bundle-a");
        bundle.source_path = bundle_dir.display().to_string();
        bundle.caption = "xin chào".into();
        bundle.caption_sha256 = riviu_core::frame_sha256("xin chào".as_bytes());
        bundle.images = vec![riviu_core::PublishImage {
            path: bundle_dir.join("01.png").display().to_string(),
            file_name: "01.png".into(),
            order: 1,
            sha256: riviu_core::frame_sha256(b"png"),
            byte_len: 3,
            width: 1,
            height: 1,
        }];

        let root_path;
        {
            let staged = super::stage_one_bundle(&bundle, 0).expect("stage one bundle");
            root_path = staged.path().to_path_buf();
            let children: Vec<_> = fs::read_dir(staged.path())
                .expect("read the staged root")
                .map(|entry| entry.expect("entry"))
                .collect();
            assert_eq!(children.len(), 1, "exactly one bundle directory");
            assert!(children[0].file_type().expect("file type").is_dir());
            assert!(children[0].path().join("01.png").is_file());
            assert!(children[0].path().join("caption.txt").is_file());
        }
        // The guard drops with the scope, including on the error paths that bail out of the
        // transfer loop.
        assert!(!root_path.exists(), "the scratch root is removed");
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn a_device_scope_is_a_component_every_backend_accepts() {
        // Two phones, two scopes -- which is what gives them two staging directories, two
        // manifest hashes and two albums. And the string has to survive validators written
        // in three languages: `[A-Za-z0-9._-]`, 1..=128.
        let first = super::device_campaign_id("0f8f0e1e-1c4a-4b6f-9a2e-7c5d3b9a1f22", 0);
        let second = super::device_campaign_id("0f8f0e1e-1c4a-4b6f-9a2e-7c5d3b9a1f22", 1);
        assert_ne!(first, second);
        for scope in [&first, &second] {
            assert!(!scope.is_empty() && scope.len() <= 128, "{scope}");
            assert!(
                scope
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')),
                "{scope}"
            );
            assert!(scope != "." && scope != "..", "{scope}");
        }
    }

    #[test]
    fn an_assignment_naming_a_bundle_the_campaign_lost_is_refused_not_guessed() {
        // Falling back to "the first bundle" is exactly how this broke. A campaign whose
        // rows disagree must stop, not publish something plausible.
        let bundles = vec![test_bundle("req-7:bundle-a")];
        let orphan = test_assignment("assign-9", "req-7:bundle-missing", "phone-9");
        let error = bundle_for_assignment(&bundles, &orphan).expect_err("must refuse");
        assert!(
            error.to_string().contains("req-7:bundle-missing"),
            "{error}"
        );
    }

    /// **A phone whose build is unmeasured is refused; Android as such is not.**
    ///
    /// This gate used to refuse every device that reported element bounds, because there was
    /// no composer for them. There is one now, so the question narrowed: not "is this
    /// Android" but "has this phone's TikTok had the controls read off it". A mixed fleet
    /// must run the phones that are measured and refuse only the ones that are not.
    /// **A cleanup failure never turns a published post into a failed one.**
    ///
    /// `Unknown` is permanently unclaimable, so downgrading a good post to it means a person
    /// has to go and look at a phone whose only problem is some files left in a folder. The
    /// reversal that found this gap made exactly that change and every test stayed green.
    #[test]
    fn files_left_on_the_phone_do_not_unpublish_a_carousel() {
        let posted = PostOutcome::Posted(serde_json::json!({"state": "posted"}));
        let folded = fold_cleanup_into(posted, Err(anyhow::anyhow!("adb went away")));
        assert!(
            matches!(folded, PostOutcome::Posted(_)),
            "a cleanup failure downgraded a published post"
        );
        let (state, code) = state_for_outcome(&folded);
        assert_eq!(state, riviu_core::PublishCampaignState::Succeeded);
        assert_eq!(code, None);
        // And the problem is still recorded, rather than swallowed.
        if let PostOutcome::Posted(evidence) = folded {
            assert_eq!(evidence["cleanup"]["state"], "not_cleaned");
            assert!(evidence["cleanup"]["message"]
                .as_str()
                .is_some_and(|text| text.contains("adb went away")));
        }

        // **And the production path routes through it**, which testing the helper alone
        // cannot show: `let _ = cleanup; Ok(outcome)` at the call site left this green.
        let body = code_of("async fn post_one_assignment(");
        assert!(
            body.iter()
                .any(|line| line.contains("fold_cleanup_into(action_result, cleanup)")),
            "the posting path stopped folding the cleanup result into its outcome"
        );
        assert!(
            body.iter()
                .any(|line| line.contains("tidy_up_the_imported_media")),
            "the imported media is no longer cleaned up at all"
        );
    }

    /// **Three outcomes, three states — and the retryable one must not be stranded.**
    ///
    /// Every failure used to become `uncertain`, which the claim refuses forever. Most of what
    /// that stranded had refused before opening anything: an unmeasured build, a picker that
    /// would not arm, an album that was not there. Those need another run, not a person.
    #[test]
    fn only_an_outcome_that_may_have_published_is_made_unclaimable() {
        assert_eq!(
            state_for_outcome(&PostOutcome::NothingPublished("album not found".into())),
            (
                riviu_core::PublishCampaignState::FailedBeforeDispatch,
                Some("post_refused_before_dispatch")
            ),
            "a run that published nothing must stay claimable"
        );
        assert_eq!(
            state_for_outcome(&PostOutcome::Unknown("tapped Post, lost the answer".into())),
            (
                riviu_core::PublishCampaignState::Uncertain,
                Some("post_or_cleanup_failed")
            )
        );
        // The two must not be the same state, which is the whole content of this test.
        assert_ne!(
            state_for_outcome(&PostOutcome::NothingPublished(String::new())).0,
            state_for_outcome(&PostOutcome::Unknown(String::new())).0
        );
    }

    /// **A phone already inside the composer is never abandoned there.**
    ///
    /// The cancel is read once, *before the claim*, and nowhere else. Checking it later would
    /// stop a phone mid-post and leave the `uncertain` state that can never be retried;
    /// checking it not at all is what the button did before — it wrote `cancelled` to the
    /// database and every remaining phone posted anyway.
    ///
    /// A source gate because the ordering it pins lives in device code: the function acquires
    /// a stream permit, reads the cancel, claims the row, and only then touches a phone.
    #[test]
    fn a_cancel_is_read_before_the_claim_and_not_after_the_composer_opens() {
        // **Comments are stripped before anything is searched.** The first version counted
        // token occurrences over the raw lines, so writing
        // `// PublishCampaignState::Cancelled` above the claim satisfied it while the real
        // check was deleted — the gate measured the file's prose, not its behaviour.
        let body = code_of("async fn post_one_phone(");
        let at = |needle: &str| body.iter().position(|line| line.contains(needle));

        let cancel = at("PublishCampaignState::Cancelled")
            .expect("the cancel is no longer read; the button writes a flag nobody honours");
        let claim = at("claim_publish_assignment_for_posting")
            .expect("the claim is what stops a second run posting the same carousel");
        let post = at("post_one_assignment(").expect("this is what touches the phone");
        assert!(
            cancel < claim,
            "the cancel is read after the row is claimed, so a stopped run still claims phones"
        );
        assert!(
            claim < post,
            "a phone is driven before its row is claimed, which is how two runs post the same \
             carousel"
        );
        // And exactly once: a second check further down is the one that would abandon a phone
        // inside the composer, in the `uncertain` state that can never be retried.
        assert_eq!(
            body.iter()
                .filter(|line| line.contains("PublishCampaignState::Cancelled"))
                .count(),
            1,
            "the cancel is read more than once; the later read stops a phone mid-post"
        );

        // The two helpers this function must actually route through, rather than merely
        // mention. Hard-coding either one's answer inline left the pure tests green.
        assert!(
            at("state_for_outcome(&outcome)").is_some(),
            "the assignment state is decided somewhere other than `state_for_outcome`"
        );
        assert!(
            at("gate.acquire()").is_some(),
            "the fan-out permit is never acquired, so the semaphore bounds nothing"
        );
        assert!(
            at("tokio::time::sleep(stagger)").is_some(),
            "the stagger argument is not what delays the task"
        );
    }

    /// The lines of one top-level function, **with comments and blank lines removed**.
    ///
    /// Every source gate in this module goes through here. A gate that reads raw lines is
    /// satisfied by a comment saying the right words, which is the opposite of what it is for.
    fn code_of(signature: &str) -> Vec<&'static str> {
        let lines: Vec<&str> = include_str!("publish_commands.rs").lines().collect();
        let start = lines
            .iter()
            .position(|line| line.starts_with(signature))
            .unwrap_or_else(|| panic!("{signature} is no longer in this file"));
        let length = lines[start..]
            .iter()
            .position(|line| *line == "}")
            .expect("the function terminates at column zero");
        lines[start..start + length]
            .iter()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty() && !line.starts_with("//"))
            .collect()
    }

    /// **And `post_one_assignment` actually asks, before it picks a composer.**
    ///
    /// The three tests above prove what the refusal says; none of them proves it is ever
    /// reached. A pure decision nothing calls is the exact shape of the bug this replaced —
    /// two readings existed, and no line compared them. So this reads the function's own body:
    /// the check has to appear, and it has to appear **before** the branch on
    /// `supports_element_bounds`, because after that branch the route is already chosen.
    #[test]
    fn the_post_path_reconciles_the_two_route_authorities_before_it_branches() {
        let body = code_of("async fn post_one_assignment(");
        let asks = body
            .iter()
            .position(|line| line.contains("refuse_when_the_route_authorities_disagree"))
            .expect("post_one_assignment must reconcile the two readings");
        let branches = body
            .iter()
            .position(|line| line.contains("if session.supports_element_bounds()"))
            .expect("post_one_assignment must still branch on the session's answer");
        assert!(
            asks < branches,
            "the check is at line {asks} of the body and the branch at {branches}; \
             a check after the branch has nothing left to refuse"
        );
        // And the media is taken back off the phone on that path — it was imported before any
        // of this ran, and a refusal that leaves the campaign's images in the gallery is a
        // refusal the operator has to clean up by hand on twenty phones.
        assert!(
            body[asks..branches]
                .iter()
                .any(|line| line.contains("tidy_up_the_imported_media")),
            "a route refusal still has to clear the imported media"
        );
    }

    /// **The fan-out is bounded by the stream budget and staggered.**
    ///
    /// Both measured facts about this fleet rather than preferences. Each post holds a
    /// UI-with-stream context, and running past `stream_capacity` does not queue — it fails, on
    /// a phone whose gallery already holds the campaign's images. The stagger is the same two
    /// seconds the interaction path measured: twenty cold starts at once share one USB bus, and
    /// the tail runs past the 40-second foreground window.
    #[test]
    fn the_publish_fan_out_is_bounded_and_staggered() {
        // **Scoped to the module, not the file.** Two reversals proved why: this searched the
        // whole source, and the strings it looks for are written out again in its own
        // assertions — so removing them from the code left the test green on the strength of
        // its own text. The same shape once let `locate` stop reading an attribute.
        let source = include_str!("publish_commands.rs");
        let module = &source[..source
            .find("#[cfg(test)]")
            .expect("this file still has a test module")];
        assert!(
            // Two facts, matched separately, because `cargo fmt` decides where the line
            // breaks go and a gate that pins the whole expression breaks on reformatting
            // rather than on a real change. This one already did once.
            module.contains("Semaphore::new(") && module.contains("stream_capacity().max(1)"),
            "the fan-out no longer bounds itself by the stream budget"
        );
        assert!(
            module.contains("PUBLISH_FAN_OUT_STAGGER * index"),
            "the fan-out starts every phone at once again"
        );
        assert!(
            PUBLISH_FAN_OUT_STAGGER >= Duration::from_secs(1),
            "a stagger this short does not separate twenty cold starts"
        );
    }

    #[test]
    fn the_transfer_path_claims_the_campaign_instead_of_writing_it() {
        let body = code_of("pub(crate) async fn transfer_publish_campaign_inner(");
        assert!(
            body.iter()
                .any(|line| line.contains("claim_publish_campaign_for_transfer")),
            "transfer writes `Transferring` unconditionally again — which on a campaign that              already succeeded rebuilds exactly the state the posting claim accepts"
        );
        // And it must not go back to the unconditional write.
        assert!(
            !body
                .iter()
                .any(|line| line.contains("PublishCampaignState::Transferring")
                    && line.contains("update_publish_campaign_state")),
            "the unconditional write is back"
        );
        // A finished assignment is skipped, so the two guards do not depend on each other.
        assert!(
            body.iter()
                .any(|line| line.contains("PublishCampaignState::Succeeded")),
            "the loop no longer skips assignments that already reached a phone"
        );
    }

    #[test]
    fn the_publish_session_targets_the_device_own_tiktok_build() {
        let body = code_of("async fn open_publish_context(");
        assert!(
            body.iter()
                .any(|line| line.contains("resolve_tiktok_package")),
            "the publish context stopped asking the device which TikTok it runs"
        );
        assert!(
            !body.iter().any(|line| line.contains("IOS_TIKTOK_BUNDLE")),
            "the publish context is back to assuming the iOS bundle on every backend"
        );
        // **And the answer has to reach both calls.** Resolving the package and then passing a
        // literal to `terminate_app` satisfies the two checks above while doing exactly what
        // they exist to prevent.
        for call in [
            "terminate_app(&exclusive, &target_package)",
            "start_interaction_session(exclusive, &target_package",
        ] {
            assert!(
                body.iter().any(|line| line.contains(call)),
                "the resolved package does not reach `{call}`"
            );
        }
    }

    #[test]
    fn a_phone_whose_build_is_unmeasured_is_refused_and_its_neighbours_are_not() {
        let error = refuse_devices_whose_composer_is_not_measured([
            ("00008030-iphone", PublishReadiness::PixelGrid),
            ("ce0617164585646f0d7e", PublishReadiness::HierarchyReady),
            (
                "ce9917160000000000",
                PublishReadiness::HierarchyUnknownBuild("bản TikTok lạ".into()),
            ),
        ])
        .expect_err("the unmeasured phone must be refused");
        let message = format!("{error:#}");
        // Names the offending device: a fleet is mixed, and "some device" sends the
        // operator hunting through sixteen phones.
        assert!(message.contains("ce9917160000000000"), "{message}");
        assert!(!message.contains("00008030-iphone"), "{message}");
        assert!(!message.contains("ce0617164585646f0d7e"), "{message}");
        // And it says how to close the gap, because the reader is the person who would.
        assert!(message.contains("composer_scout"), "{message}");
    }

    /// A build missing labels is refused **by name**, so the measuring run knows what to get.
    #[test]
    fn a_build_missing_labels_is_refused_and_the_missing_ones_are_listed() {
        let error = refuse_devices_whose_composer_is_not_measured([(
            "ce0617164585646f0d7e",
            PublishReadiness::HierarchyMissing(vec![
                riviu_core::tiktok_labels::TikTokControl::PostButton,
            ]),
        )])
        .expect_err("a build without a Post button cannot publish");
        assert!(format!("{error:#}").contains("PostButton"), "{error:#}");
    }

    /// **Both routes pass when they are ready**, which is the case that must not regress.
    #[test]
    fn a_mixed_fleet_that_is_fully_measured_runs() {
        refuse_devices_whose_composer_is_not_measured([
            ("a-iphone", PublishReadiness::PixelGrid),
            ("an-android", PublishReadiness::HierarchyReady),
        ])
        .expect("both routes are measured");
    }

    /// **The composer's grid, refused before the media leaves the desktop.**
    ///
    /// `post_one_assignment` already refuses an over-sized bundle, but it refuses after
    /// `stage`/`prepare`/`import` have put the images into a real phone's gallery and made
    /// them visible to TikTok — where they stay, with no cleanup owner, because the campaign
    /// never reached a state that owns cleanup.
    #[test]
    fn a_bundle_too_wide_for_the_tap_grid_is_refused_before_transfer() {
        let fits = bundle_of("set1 13 spotlight", 11);
        let too_wide = bundle_of("set1 19 spotlightv3", 13);
        let error = refuse_assignments_whose_bundle_is_too_large([
            ("an-iphone", &fits, IOS_PIXEL_GRID_MAX_IMAGES),
            ("an-iphone-2", &too_wide, IOS_PIXEL_GRID_MAX_IMAGES),
        ])
        .expect_err("thirteen images cannot be reached by a twelve-cell grid");
        let message = format!("{error:#}");
        // Names the offending bundle, its count and the phone: an operator with twenty-one
        // folders and twenty phones needs all three.
        assert!(message.contains("set1 19 spotlightv3"), "{message}");
        assert!(message.contains("13"), "{message}");
        assert!(message.contains("an-iphone-2"), "{message}");
        // And does not accuse the one that fits.
        assert!(!message.contains("set1 13 spotlight"), "{message}");
    }

    /// **The ceiling is the device's, not the campaign's.**
    ///
    /// One number for the whole run refused Android bundles that its own composer selects
    /// fine — it locates each cell rather than tapping twelve coordinates somebody wrote
    /// down, so its grid is wider. The same twelve-image bundle passes on one route and
    /// refuses on the other, and that is the point.
    #[test]
    fn each_device_is_measured_against_its_own_composer() {
        let twelve = bundle_of("twelve", 12);
        assert!(refuse_assignments_whose_bundle_is_too_large([(
            "an-iphone",
            &twelve,
            max_images_for(PublishRoute::PixelGrid)
        )])
        .is_err());
        refuse_assignments_whose_bundle_is_too_large([(
            "an-android",
            &twelve,
            max_images_for(PublishRoute::Hierarchy),
        )])
        .expect("the hierarchy composer reaches twelve cells");
        assert!(
            max_images_for(PublishRoute::Hierarchy) > max_images_for(PublishRoute::PixelGrid),
            "if these are equal the split above proves nothing"
        );
    }

    #[test]
    fn a_bundle_that_fits_the_grid_passes() {
        // Exactly at the limit is inside it: eleven images is what the guard has always
        // allowed, and moving the constant must not move the boundary.
        let eleven = bundle_of("eleven", 11);
        refuse_assignments_whose_bundle_is_too_large([(
            "an-iphone",
            &eleven,
            IOS_PIXEL_GRID_MAX_IMAGES,
        )])
        .expect("eleven is the limit, not one past it");
    }

    /// A bundle with `count` images and nothing else that matters here.
    fn bundle_of(name: &str, count: usize) -> riviu_core::PublishBundle {
        riviu_core::PublishBundle {
            id: format!("{name}-id"),
            source_path: String::new(),
            name: name.to_string(),
            media_kind: riviu_core::PublishMediaKind::Image,
            images: (1..=count)
                .map(|order| riviu_core::PublishImage {
                    path: format!("{order:02}-slide.png"),
                    file_name: format!("{order:02}-slide.png"),
                    order: order as u32,
                    sha256: "11".repeat(32),
                    byte_len: 1,
                    width: 995,
                    height: 1405,
                })
                .collect(),
            caption_path: String::new(),
            caption: String::new(),
            caption_sha256: "00".repeat(32),
            total_bytes: count as u64,
        }
    }

    #[test]
    fn an_empty_assignment_list_is_not_the_refusal_this_gate_is_for() {
        // Emptiness is checked by its own error with its own message; this gate must not
        // steal that case and report a platform problem instead.
        refuse_devices_whose_composer_is_not_measured(std::iter::empty())
            .expect("nothing to refuse");
    }

    #[test]
    fn account_lock_alert_is_rejected_in_vietnamese() {
        assert!(account_status_text_is_locked(
            "trạng thái tài khoản tài khoản của bạn đã bị khóa"
        ));
    }

    #[test]
    fn account_lock_alert_is_rejected_in_english() {
        assert!(account_status_text_is_locked(
            "account status account locked"
        ));
    }

    #[test]
    fn ordinary_post_confirmation_is_not_account_lock() {
        assert!(!account_status_text_is_locked("đăng công khai xác nhận"));
    }
}
