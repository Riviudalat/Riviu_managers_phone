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

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[tauri::command]
pub fn publish_scan_folder(
    state: State<'_, AppState>,
    source_root: String,
) -> Result<PublishFolderManifest, String> {
    let _admission = state.ensure_accepting_work().map_err(String::from)?;
    scan_publish_folder(PathBuf::from(source_root), PublishScanOptions::default()).map_err(err)
}

#[tauri::command]
pub fn publish_create_campaign(
    state: State<'_, AppState>,
    source_root: String,
    bundle_ids: Vec<String>,
    udids: Vec<String>,
    run_at: Option<String>,
) -> Result<PublishCampaignRecord, String> {
    let _admission = state.ensure_accepting_work().map_err(String::from)?;
    if let Some(raw) = run_at.as_deref() {
        parse_run_at(raw).map_err(err)?;
    }
    for udid in &udids {
        if state.registry.get(udid).is_none() {
            return Err(format!("device is not connected: {udid}"));
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
        run_at,
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
) -> Result<Vec<PublishCampaignRecord>, String> {
    state
        .db
        .list_publish_campaigns(limit.unwrap_or(50).clamp(1, 200))
        .map_err(err)
}

#[tauri::command]
pub fn publish_get(
    state: State<'_, AppState>,
    campaign_id: String,
) -> Result<Option<PublishCampaignDetail>, String> {
    state.db.get_publish_campaign(&campaign_id).map_err(err)
}

#[tauri::command]
pub fn publish_cancel(state: State<'_, AppState>, campaign_id: String) -> Result<(), String> {
    let _admission = state.ensure_accepting_work().map_err(String::from)?;
    state.db.cancel_publish_campaign(&campaign_id).map_err(err)
}

/// Move a queued campaign into the explicit transfer state. The phone is not
/// touched here; transfer and post are separate effect boundaries.
#[tauri::command]
pub fn publish_prepare(
    state: State<'_, AppState>,
    campaign_id: String,
) -> Result<PublishCampaignDetail, String> {
    let _admission = state.ensure_accepting_work().map_err(String::from)?;
    let detail = state
        .db
        .get_publish_campaign(&campaign_id)
        .map_err(err)?
        .ok_or_else(|| "publish campaign not found".to_string())?;
    if matches!(
        detail.campaign.state,
        riviu_core::PublishCampaignState::Cancelled
            | riviu_core::PublishCampaignState::Succeeded
            | riviu_core::PublishCampaignState::Uncertain
    ) {
        return Err(format!(
            "campaign is already terminal: {:?}",
            detail.campaign.state
        ));
    }
    state
        .db
        .update_publish_campaign_state(&campaign_id, riviu_core::PublishCampaignState::Ready, None)
        .map_err(err)?;
    state
        .db
        .get_publish_campaign(&campaign_id)
        .map_err(err)?
        .ok_or_else(|| "campaign disappeared after prepare".to_string())
}

#[tauri::command]
pub async fn publish_transfer(
    state: State<'_, AppState>,
    campaign_id: String,
) -> Result<PublishCampaignDetail, String> {
    let _admission = state.ensure_accepting_work().map_err(String::from)?;
    transfer_publish_campaign_inner(
        state.control.clone(),
        state.db.clone(),
        state.active_agent_bundle_id.clone(),
        campaign_id,
    )
    .await
    .map_err(err)
}

pub(crate) async fn transfer_publish_campaign_inner(
    control: Arc<DeviceControlPlane>,
    db: Arc<Database>,
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
    refuse_devices_this_path_cannot_drive(
        detail
            .assignments
            .iter()
            .map(|assignment| assignment.udid.as_str()),
        |udid| control.reports_element_bounds(udid),
    )?;
    let source_root = Path::new(&detail.bundles[0].source_path)
        .parent()
        .ok_or_else(|| anyhow::anyhow!("managed publish root is missing"))?
        .to_path_buf();
    db.update_publish_campaign_state(
        &campaign_id,
        riviu_core::PublishCampaignState::Transferring,
        None,
    )?;

    for assignment in &detail.assignments {
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
            .stage_publish_media(&context, &agent_bundle_id, &campaign_id, &source_root)
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
                            .prepare_publish_media(&context, &campaign_id, manifest_sha256)
                            .await
                            .map_err(anyhow::Error::new)?;
                        let native_import = control
                            .import_publish_media(&context, &campaign_id, manifest_sha256)
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

/// The publish path is iOS-only, and [`refuse_devices_this_path_cannot_drive`] enforces it.
///
/// Not a resolved value. The previous version of this comment claimed the Publish page
/// refuses an Android target before dispatch; it did not, and neither did anything else —
/// an Android phone could be mapped into a campaign and posted, which meant pressing iOS
/// logical coordinates against a different app's layout. Kept as the shared constant so
/// nobody mistakes it for a per-device answer.
const TIKTOK_BUNDLE_ID: &str = riviu_core::tiktok_target::IOS_TIKTOK_BUNDLE;
const PLUS_BUTTON: TapPoint = TapPoint { x: 187.0, y: 640.0 };
const GALLERY_BUTTON: TapPoint = TapPoint { x: 32.0, y: 632.0 };
const ALBUM_PICKER: TapPoint = TapPoint { x: 187.0, y: 47.0 };
const ALBUM_ROW: TapPoint = TapPoint { x: 187.0, y: 385.0 };
const COMPOSER_NEXT: TapPoint = TapPoint { x: 300.0, y: 618.0 };
const EDIT_NEXT: TapPoint = TapPoint { x: 280.0, y: 635.0 };
const CAPTION_FIELD: TapPoint = TapPoint { x: 180.0, y: 240.0 };
const POST_BUTTON: TapPoint = TapPoint { x: 330.0, y: 42.0 };
const PUBLIC_POST_CONFIRM: TapPoint = TapPoint { x: 275.0, y: 444.0 };

/// Refuse a campaign holding a device this module has no coordinates for.
///
/// Every tap constant above is an **iOS logical coordinate** and `TIKTOK_BUNDLE_ID` is the
/// iOS bundle, so an Android assignment would press arbitrary places in a layout nobody
/// measured — the exact thing the label-driven Android work exists to avoid, and worse here
/// because posting cannot be undone.
///
/// `supports_push_media` does not catch this. The Android driver answers `true` there,
/// correctly: pushing media into the gallery is the part it really does implement. What is
/// missing is the composer, and there is no capability that says so.
///
/// So the gate is `reports_element_bounds`, the same signal that partitions the interaction
/// path: a device that reports bounds is driven by label, and this module drives by pixel.
/// Taking a predicate rather than the control plane keeps it testable without a fleet.
fn refuse_devices_this_path_cannot_drive<'a>(
    udids: impl IntoIterator<Item = &'a str>,
    driven_by_label: impl Fn(&str) -> bool,
) -> anyhow::Result<()> {
    let by_label: Vec<&str> = udids
        .into_iter()
        .filter(|udid| driven_by_label(udid))
        .collect();
    anyhow::ensure!(
        by_label.is_empty(),
        "đường Đăng bài này chỉ chạy trên iPhone. {} điều khiển theo cây giao diện, còn mọi \
         toạ độ ở đây là toạ độ logic của iOS — chạy tiếp là bấm bừa lên một màn hình chưa ai \
         đo. Composer cho Android chưa được dựng.",
        by_label.join(", ")
    );
    Ok(())
}

#[tauri::command]
pub async fn publish_post(
    state: State<'_, AppState>,
    campaign_id: String,
) -> Result<PublishCampaignDetail, String> {
    let _admission = state.ensure_accepting_work().map_err(String::from)?;
    post_publish_campaign_inner(
        state.control.clone(),
        state.db.clone(),
        Arc::new(state.streams.clone()),
        campaign_id,
    )
    .await
    .map_err(err)
}

pub(crate) async fn post_publish_campaign_inner(
    control: Arc<DeviceControlPlane>,
    db: Arc<Database>,
    frames: Arc<dyn FrameSource>,
    campaign_id: String,
) -> anyhow::Result<PublishCampaignDetail> {
    let detail = db
        .get_publish_campaign(&campaign_id)?
        .ok_or_else(|| anyhow::anyhow!("publish campaign not found"))?;
    if detail.assignments.is_empty() || detail.bundles.is_empty() {
        anyhow::bail!("publish campaign has no imported assignment");
    }
    if !matches!(
        detail.campaign.state,
        riviu_core::PublishCampaignState::Imported
            | riviu_core::PublishCampaignState::Posting
            | riviu_core::PublishCampaignState::Verifying
    ) {
        anyhow::bail!(
            "campaign must be imported before post: {:?}",
            detail.campaign.state
        );
    }
    refuse_devices_this_path_cannot_drive(
        detail
            .assignments
            .iter()
            .map(|assignment| assignment.udid.as_str()),
        |udid| control.reports_element_bounds(udid),
    )?;
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
    db.update_publish_campaign_state(
        &campaign_id,
        riviu_core::PublishCampaignState::Posting,
        None,
    )?;

    let mut failures = Vec::new();
    for assignment in &detail.assignments {
        let Some(bundle) = detail
            .bundles
            .iter()
            .find(|bundle| bundle.id == assignment.bundle_id)
        else {
            failures.push(format!("bundle {} missing", assignment.bundle_id));
            continue;
        };
        db.update_publish_assignment_state(
            &assignment.id,
            riviu_core::PublishCampaignState::Posting,
            None,
            Some(&serde_json::json!({"effectIntent":"post_carousel"}).to_string()),
        )?;
        let result = post_one_assignment(
            &control,
            &db,
            frames.as_ref(),
            &campaign_id,
            assignment,
            bundle,
        )
        .await;
        match result {
            Ok(evidence) => {
                db.update_publish_assignment_state(
                    &assignment.id,
                    riviu_core::PublishCampaignState::Succeeded,
                    None,
                    Some(&evidence.to_string()),
                )?;
            }
            Err(error) => {
                let message = error.to_string();
                failures.push(format!("{}: {}", assignment.udid, message));
                db.update_publish_assignment_state(
                    &assignment.id,
                    riviu_core::PublishCampaignState::Uncertain,
                    Some("post_or_cleanup_failed"),
                    Some(
                        &serde_json::json!({"message": message, "effectIntent":"post_carousel"})
                            .to_string(),
                    ),
                )?;
            }
        }
    }

    if failures.is_empty() {
        db.update_publish_campaign_state(
            &campaign_id,
            riviu_core::PublishCampaignState::Succeeded,
            None,
        )?;
    } else {
        db.update_publish_campaign_state(
            &campaign_id,
            riviu_core::PublishCampaignState::Uncertain,
            Some("post_or_cleanup_failed"),
        )?;
    }
    let output = db
        .get_publish_campaign(&campaign_id)?
        .ok_or_else(|| anyhow::anyhow!("campaign disappeared after post"))?;
    if !failures.is_empty() {
        anyhow::bail!("{}", failures.join("; "));
    }
    Ok(output)
}

async fn post_one_assignment(
    control: &DeviceControlPlane,
    _db: &Database,
    frames: &dyn FrameSource,
    campaign_id: &str,
    assignment: &riviu_core::PublishAssignmentRecord,
    bundle: &riviu_core::PublishBundle,
) -> anyhow::Result<serde_json::Value> {
    if bundle.images.is_empty() || bundle.images.len() > 11 {
        anyhow::bail!("bundle {} has an invalid image count", bundle.id);
    }
    if bundle.caption.chars().count() > 2200 {
        anyhow::bail!(
            "caption for {} exceeds TikTok's 2200 character limit",
            bundle.id
        );
    }
    let import = assignment
        .evidence_json
        .as_deref()
        .and_then(import_id_from_evidence)
        .ok_or_else(|| anyhow::anyhow!("native import proof is missing for {}", assignment.udid))?;
    let context = open_publish_context(control, &assignment.udid).await?;
    let session = control.streaming_session(&context)?;
    let action_result = async {
        let before = wait_for_frame(frames, &assignment.udid, Duration::from_secs(8)).await?;
        let before_sha = frame_sha256(&before);

        tap_transition(
            frames,
            &assignment.udid,
            session.as_ref(),
            PLUS_BUTTON,
            &before_sha,
        )
        .await?;
        tap_transition(
            frames,
            &assignment.udid,
            session.as_ref(),
            GALLERY_BUTTON,
            &before_sha,
        )
        .await?;
        tap_transition(
            frames,
            &assignment.udid,
            session.as_ref(),
            ALBUM_PICKER,
            &before_sha,
        )
        .await?;
        tap_transition(
            frames,
            &assignment.udid,
            session.as_ref(),
            ALBUM_ROW,
            &before_sha,
        )
        .await?;

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
        let selected = wait_for_frame(frames, &assignment.udid, Duration::from_secs(5)).await?;
        let selected_sha = frame_sha256(&selected);
        session.tap(COMPOSER_NEXT).await?;
        wait_for_changed_frame(
            frames,
            &assignment.udid,
            &selected_sha,
            Duration::from_secs(8),
        )
        .await?;
        session.tap(EDIT_NEXT).await?;
        wait_for_changed_frame(
            frames,
            &assignment.udid,
            &selected_sha,
            Duration::from_secs(8),
        )
        .await?;

        if !session.supports_text_input() {
            anyhow::bail!("combined Agent text capability is not active for this session");
        }
        session.tap_native(CAPTION_FIELD).await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        session.type_text(&bundle.caption).await?;
        let typed = wait_for_frame(frames, &assignment.udid, Duration::from_secs(5)).await?;
        let typed_sha = frame_sha256(&typed);
        let post_red_before = bottom_right_redness(&typed);
        session.tap_native(POST_BUTTON).await?;
        let after_post_tap =
            wait_for_changed_frame(frames, &assignment.udid, &typed_sha, Duration::from_secs(8))
                .await?;
        if frame_reports_account_lock(&after_post_tap).await {
            anyhow::bail!("TikTok account status blocked the post: account_locked");
        }
        let confirmation_sha = if is_public_post_confirmation(&after_post_tap) {
            session.tap_native(PUBLIC_POST_CONFIRM).await?;
            frame_sha256(
                &wait_for_changed_frame(
                    frames,
                    &assignment.udid,
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
            &assignment.udid,
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
        }))
    }
    .await;
    let evidence = action_result?;
    let cleanup_while_live = control
        .cleanup_publish_media_with_ui(&context, &import)
        .await;
    let close_result = control.close_ui_context(context).await;
    close_result.map_err(anyhow::Error::new)?;
    let cleanup = match cleanup_while_live {
        Ok(cleanup) => cleanup,
        Err(first_error) => {
            let retry_context = control
                .acquire_exclusive(&assignment.udid, DeviceWorkOwner::Script)
                .await
                .map_err(anyhow::Error::new)?;
            let retry_result = control.cleanup_publish_media(&retry_context, &import).await;
            let close_retry = control.close_exclusive_context(retry_context);
            close_retry.map_err(anyhow::Error::new)?;
            retry_result.map_err(|retry_error| {
                anyhow::anyhow!(
                    "native cleanup failed while UI was live ({first_error}); retry failed: {retry_error}"
                )
            })?
        }
    };
    if cleanup
        .get("value")
        .and_then(|value| value.get("state"))
        .and_then(serde_json::Value::as_str)
        != Some("cleaned")
        && cleanup.get("state").and_then(serde_json::Value::as_str) != Some("cleaned")
    {
        anyhow::bail!("native media cleanup did not return cleaned");
    }
    Ok(serde_json::json!({"post": evidence, "cleanup": cleanup}))
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
    control
        .terminate_app(&exclusive, TIKTOK_BUNDLE_ID)
        .await
        .map_err(anyhow::Error::new)?;
    let kind = if control.requires_fresh_text_session(udid) {
        InteractionSessionKind::FreshText
    } else {
        InteractionSessionKind::Ordinary
    };
    let session = control
        .start_interaction_session(exclusive, TIKTOK_BUNDLE_ID, kind)
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

fn import_id_from_evidence(raw: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let native = value.get("nativeImport")?;
    let value = native.get("value").unwrap_or(native);
    value
        .get("importId")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn parse_run_at(raw: &str) -> Result<NaiveDateTime, String> {
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

#[cfg(test)]
mod tests {
    use super::account_status_text_is_locked;
    use super::refuse_devices_this_path_cannot_drive;

    #[test]
    fn a_campaign_holding_an_android_phone_is_refused_before_anything_is_touched() {
        // The hazard is not abstract: `supports_push_media` answers true for Android, so
        // the only pre-existing gate waved it through and the module would then press iOS
        // logical coordinates against TikTok's Android layout.
        let error = refuse_devices_this_path_cannot_drive(
            ["00008030-iphone", "ce0617164585646f0d7e"],
            |udid| udid == "ce0617164585646f0d7e",
        )
        .expect_err("an Android target must be refused");
        let message = format!("{error:#}");
        // Names the offending device: a fleet is mixed, and "some device" sends the
        // operator hunting through sixteen phones.
        assert!(message.contains("ce0617164585646f0d7e"), "{message}");
        assert!(!message.contains("00008030-iphone"), "{message}");
    }

    #[test]
    fn an_all_iphone_campaign_still_runs() {
        refuse_devices_this_path_cannot_drive(["a-iphone", "b-iphone"], |_| false)
            .expect("the pixel path is what this module is for");
    }

    #[test]
    fn an_empty_assignment_list_is_not_the_refusal_this_gate_is_for() {
        // Emptiness is checked by its own error with its own message; this gate must not
        // steal that case and report a platform problem instead.
        refuse_devices_this_path_cannot_drive(std::iter::empty(), |_| true)
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
