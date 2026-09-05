//! Manifest scanning, target preflight and measured device readiness.

use super::*;

// A scan reads and hashes every bundle. Limit simultaneous scans without occupying
// async runtime threads needed by device I/O and background workers.
static PUBLISH_SCAN_SLOTS: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(2)));

pub(super) async fn bounded_publish_scan<T, F>(
    slots: Arc<tokio::sync::Semaphore>,
    scan: F,
) -> anyhow::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    let permit = slots.acquire_owned().await?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        scan()
    })
    .await?
}

pub(super) fn err(error: impl std::fmt::Display) -> CommandError {
    CommandError::operation(error)
}

#[tauri::command]
pub async fn publish_scan_folder(
    state: State<'_, AppState>,
    source_root: String,
) -> Result<PublishFolderManifest, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    bounded_publish_scan(Arc::clone(&PUBLISH_SCAN_SLOTS), move || {
        scan_publish_folder(PathBuf::from(source_root), PublishScanOptions::default())
            .map_err(anyhow::Error::from)
    })
    .await
    .map_err(err)
}

pub(super) struct PreparedPublishPreflight {
    pub(super) report: riviu_core::PublishPreflightReport,
    pub(super) bundles: Vec<riviu_core::PublishBundle>,
}

pub(super) fn resolve_preflight_target(
    request: &riviu_core::PublishPreflightRequest,
    fleet_order: &[String],
    metas: &[riviu_core::DeviceMeta],
    groups: &[riviu_core::DeviceGroup],
) -> anyhow::Result<riviu_core::ResolvedTargetSnapshot> {
    let target_ref =
        request
            .target_ref
            .clone()
            .unwrap_or_else(|| riviu_core::TargetRef::Explicit {
                udids: request.udids.clone(),
            });
    let snapshot = riviu_core::resolve_target(&target_ref, fleet_order, metas, groups)?;
    let resolved_udids = snapshot
        .included
        .iter()
        .map(|device| device.udid.as_str())
        .collect::<Vec<_>>();
    let assignment_udids = request.udids.iter().map(String::as_str).collect::<Vec<_>>();
    anyhow::ensure!(
        resolved_udids == assignment_udids,
        "phạm vi semantic đã đổi so với danh sách ghép bài; resolve lại trước preflight"
    );
    Ok(snapshot)
}

#[tauri::command]
pub async fn publish_preflight(
    state: State<'_, AppState>,
    request: riviu_core::PublishPreflightRequest,
) -> Result<riviu_core::PublishPreflightReport, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    build_publish_preflight(&state.control, &state.registry, &state.db, request)
        .await
        .map(|prepared| prepared.report)
        .map_err(err)
}

pub(super) async fn build_publish_preflight(
    control: &DeviceControlPlane,
    registry: &riviu_core::DeviceRegistry,
    db: &Database,
    mut request: riviu_core::PublishPreflightRequest,
) -> anyhow::Result<PreparedPublishPreflight> {
    request.source_root = request.source_root.trim().to_string();
    anyhow::ensure!(!request.source_root.is_empty(), "thư mục nguồn đang trống");
    if let Some(run_at) = request.run_at.as_deref() {
        parse_run_at(run_at).map_err(anyhow::Error::msg)?;
        request.run_at = Some(run_at.trim().to_string());
    }
    request.sound_policy.pool_size()?;
    riviu_core::publish::validate_publish_mapping(&request.bundle_ids, &request.udids)
        .map_err(anyhow::Error::new)?;

    let source_root = request.source_root.clone();
    let manifest = bounded_publish_scan(Arc::clone(&PUBLISH_SCAN_SLOTS), move || {
        scan_publish_folder(source_root, PublishScanOptions::default()).map_err(anyhow::Error::from)
    })
    .await?;
    let mut bundles = request
        .bundle_ids
        .iter()
        .map(|bundle_id| {
            manifest
                .bundles
                .iter()
                .find(|bundle| bundle.id == *bundle_id)
                .cloned()
                .with_context(|| format!("bundle không còn trong thư mục: {bundle_id}"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let overrides: HashMap<_, _> = request
        .caption_overrides
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    apply_caption_overrides(&mut bundles, Some(&overrides))?;

    let metas = db.list_device_metas()?;
    let groups = db.list_groups()?;
    let fleet_order = registry
        .list()
        .into_iter()
        .map(|device| device.udid)
        .collect::<Vec<_>>();
    let target_snapshot = resolve_preflight_target(&request, &fleet_order, &metas, &groups)?;
    let mut assignments = Vec::with_capacity(bundles.len());
    let mut observations = Vec::with_capacity(bundles.len());
    let mut issues = Vec::new();
    for (ordinal, (bundle, udid)) in bundles.iter().zip(&request.udids).enumerate() {
        let mut row_issues = Vec::new();
        // Android keeps the managed import and a MediaStore copy during composition. Reserve
        // both plus fixed working headroom, rather than discovering a full phone after transfer.
        let required_bytes = bundle
            .total_bytes
            .saturating_mul(2)
            .saturating_add(64 * 1024 * 1024);
        let route = route_of(control, udid);
        let media_ok =
            bundle_media_shape_is_ready(bundle, route) && !bundle.caption.trim().is_empty();
        if !media_ok {
            let message = if bundle.caption.trim().is_empty() {
                "caption rỗng nên không thể khóa đúng bài khi lấy link"
            } else if matches!(bundle.media_kind, riviu_core::PublishMediaKind::Video) {
                "bundle video phải có đúng một MP4 đã preflight và không được trộn ảnh"
            } else {
                "số ảnh không nằm trong giới hạn đã đo của composer trên máy"
            };
            row_issues.push(preflight_issue("media_unready", udid, &bundle.id, message));
        }

        let device = registry.get(udid);
        if device.is_none() {
            row_issues.push(preflight_issue(
                "device_missing",
                udid,
                &bundle.id,
                "máy không còn trong roster hiện tại",
            ));
        }
        let android = device
            .as_ref()
            .is_some_and(|device| matches!(device.platform, riviu_core::DevicePlatform::Android));
        if !android {
            row_issues.push(preflight_issue(
                "android_required",
                udid,
                &bundle.id,
                "đợt đăng có chọn nhạc này chỉ chứng nhận trên Android",
            ));
        }
        if !control.supports_push_media(udid) {
            row_issues.push(preflight_issue(
                "push_media_unavailable",
                udid,
                &bundle.id,
                "Riviu helper trên máy chưa quảng bá khả năng chuyển media",
            ));
        }

        let available_bytes = if android {
            match control.available_storage_bytes(udid).await {
                Ok(available) => {
                    if available < required_bytes {
                        row_issues.push(preflight_issue(
                            "storage_insufficient",
                            udid,
                            &bundle.id,
                            &format!(
                                "máy còn {available} byte nhưng lượt đăng cần tối thiểu {required_bytes} byte"
                            ),
                        ));
                    }
                    Some(available)
                }
                Err(error) => {
                    row_issues.push(preflight_issue(
                        "storage_unreadable",
                        udid,
                        &bundle.id,
                        &format!("không đọc được dung lượng trống của máy: {error}"),
                    ));
                    None
                }
            }
        } else {
            None
        };

        let (package_name, version, locale, composer_ok, sound_picker_ok) = if android {
            match control.tiktok_build(udid).await {
                Ok((package, version, locale)) => {
                    let base_composer_ok = matches!(
                        readiness_of_build(&package, &locale, &version),
                        PublishReadiness::HierarchyReady
                    );
                    let video_picker_ok =
                        !matches!(bundle.media_kind, riviu_core::PublishMediaKind::Video)
                            || (matches!(route, PublishRoute::Hierarchy)
                                && video_plan_for_build(&package, &locale, &version).is_ok());
                    let composer_ok = base_composer_ok && video_picker_ok;
                    let sound_picker_ok = sound_plan_for_build(&package, &locale, &version).is_ok();
                    if !composer_ok {
                        row_issues.push(preflight_issue(
                            if base_composer_ok {
                                "video_composer_unmeasured"
                            } else {
                                "composer_unmeasured"
                            },
                            udid,
                            &bundle.id,
                            if base_composer_ok {
                                "video picker chưa được đo tới editor cho đúng package/build/locale này"
                            } else {
                                "composer chưa đủ locator cho đúng package/build/locale này"
                            },
                        ));
                    }
                    if !sound_picker_ok {
                        row_issues.push(preflight_issue(
                            "sound_picker_unmeasured",
                            udid,
                            &bundle.id,
                            "sound picker chưa được đo cho đúng package/build/locale này",
                        ));
                    }
                    (
                        Some(package),
                        Some(version),
                        Some(locale),
                        composer_ok,
                        sound_picker_ok,
                    )
                }
                Err(error) => {
                    row_issues.push(preflight_issue(
                        "tiktok_build_unreadable",
                        udid,
                        &bundle.id,
                        &format!("không đọc được package/build/locale TikTok: {error}"),
                    ));
                    (None, None, None, false, false)
                }
            }
        } else {
            (None, None, None, false, false)
        };

        let meta = metas.iter().find(|meta| meta.udid == *udid);
        let storage_ok = available_bytes.is_some_and(|available| available >= required_bytes);
        observations.push(serde_json::json!({
            "ordinal": ordinal,
            "udid": udid,
            "number": meta.and_then(|meta| meta.number),
            "alias": meta.map(|meta| meta.alias.trim()).unwrap_or_default(),
            "packageName": package_name,
            "version": version,
            "locale": locale,
            "requiredBytes": required_bytes,
            "storage": if storage_ok { "pass" } else { "fail" },
            "availableBytes": available_bytes,
        }));
        issues.extend(row_issues.iter().cloned());
        assignments.push(riviu_core::PublishPreflightAssignmentReport {
            ordinal: u32::try_from(ordinal)?,
            bundle_id: bundle.id.clone(),
            udid: udid.clone(),
            package_name,
            version,
            locale,
            media: if media_ok {
                riviu_core::PublishPreflightCheck::Pass
            } else {
                riviu_core::PublishPreflightCheck::Fail
            },
            composer: if composer_ok {
                riviu_core::PublishPreflightCheck::Pass
            } else {
                riviu_core::PublishPreflightCheck::Fail
            },
            sound_picker: if sound_picker_ok {
                riviu_core::PublishPreflightCheck::Pass
            } else {
                riviu_core::PublishPreflightCheck::Fail
            },
            storage: if storage_ok {
                riviu_core::PublishPreflightCheck::Pass
            } else {
                riviu_core::PublishPreflightCheck::Fail
            },
            required_bytes,
            available_bytes,
            issues: row_issues,
        });
    }

    let input_digest =
        publish_preflight_digest(&request, &bundles, &target_snapshot, &observations)?;
    let webhook = db
        .get_setting(riviu_core::publish_sheet::WEBHOOK_URL_SETTING)?
        .unwrap_or_default();
    let token = db
        .get_setting(riviu_core::publish_sheet::WEBHOOK_TOKEN_SETTING)?
        .unwrap_or_default();
    let sheet_configured = riviu_core::publish_sheet::is_acceptable_webhook(webhook.trim())
        && !token.trim().is_empty();
    let report = riviu_core::PublishPreflightReport {
        input_digest,
        target_snapshot,
        can_execute: issues.is_empty(),
        assignments,
        issues,
        sheet_configured,
    };
    Ok(PreparedPublishPreflight { report, bundles })
}

pub(super) fn publish_preflight_digest(
    request: &riviu_core::PublishPreflightRequest,
    bundles: &[riviu_core::PublishBundle],
    target_snapshot: &riviu_core::ResolvedTargetSnapshot,
    observations: &[serde_json::Value],
) -> anyhow::Result<String> {
    let stable_observations = observations
        .iter()
        .cloned()
        .map(|mut observation| {
            if let serde_json::Value::Object(fields) = &mut observation {
                // Free space changes while the confirmation screen is open. The approval
                // binds the required threshold and its pass/fail verdict, while the exact
                // observed byte count remains available in the report for the operator.
                fields.remove("availableBytes");
            }
            observation
        })
        .collect::<Vec<_>>();
    let payload = serde_json::json!({
        "schemaVersion": 1,
        "request": request,
        "bundles": bundles,
        "targetSnapshot": target_snapshot,
        "targets": stable_observations,
    });
    Ok(frame_sha256(&serde_json::to_vec(&payload)?))
}

pub(super) fn require_current_preflight_digest(
    report: &riviu_core::PublishPreflightReport,
    approved_input_digest: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        report.input_digest == approved_input_digest.trim(),
        "preflight đã cũ: nguồn, caption, máy hoặc build TikTok đã đổi; kiểm tra lại trước khi tạo chiến dịch"
    );
    Ok(())
}

pub(super) fn preflight_issue(
    code: &str,
    udid: &str,
    bundle_id: &str,
    message: &str,
) -> riviu_core::PublishExecutionIssue {
    riviu_core::PublishExecutionIssue {
        code: code.to_string(),
        assignment_id: None,
        udid: Some(udid.to_string()),
        bundle_id: Some(bundle_id.to_string()),
        message: message.to_string(),
    }
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
pub(super) fn refuse_devices_whose_composer_is_not_measured<'a>(
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
pub(super) async fn readiness_of(control: &DeviceControlPlane, udid: &str) -> PublishReadiness {
    if !control.reports_element_bounds(udid) {
        return PublishReadiness::PixelGrid;
    }
    // **Ask the phone which build it is, and then ask the catalogue about THAT build.**
    //
    // This used to take the shortest gap across every catalogued (language, version) set for
    // the package — a question about the package, not about the phone. As a refusal that was
    // sound (if no set is complete, this phone cannot publish whichever one it is in), and
    // for one screenful of text it was fine. Turning the same answer into a positive claim
    // on the page is what made it wrong: a phone whose TikTok self-updates keeps a green
    // chip while `post_through_the_composer` refuses it on the exact pair, because
    // `composer_caption` is keyed to the version. So readiness now reads the pair and looks
    // it up, and a mismatch is `HierarchyUnknownBuild` — which is a state the strip could
    // not previously reach for a real build change.
    //
    // The reading is three adb round trips (package, dumpsys, getprop), which is why the
    // page asks per udid-set and offers a manual re-ask rather than polling.
    match control.tiktok_build(udid).await {
        Ok((package, version, locale)) => readiness_of_build(&package, &locale, &version),
        Err(error) => PublishReadiness::HierarchyUnknownBuild(format!(
            "không đọc được bản TikTok trên máy này: {error}"
        )),
    }
}

pub(super) fn sound_plan_for_build(
    package: &str,
    locale: &str,
    version: &str,
) -> anyhow::Result<riviu_core::tiktok_sound::SoundPickerPlan> {
    riviu_core::tiktok_sound::SoundPickerPlan::resolve(package, locale, version).ok_or_else(|| {
        anyhow::anyhow!("sound picker chưa được đo cho {package} / {locale} / {version}")
    })
}

pub(super) fn video_plan_for_build(
    package: &str,
    locale: &str,
    version: &str,
) -> anyhow::Result<riviu_core::tiktok_composer::VideoPickerPlan> {
    riviu_core::tiktok_composer::VideoPickerPlan::resolve(package, locale, version).ok_or_else(
        || {
            anyhow::anyhow!(
                "video picker chưa được đo tới editor cho {package} / {locale} / {version}"
            )
        },
    )
}

/// Refuse an unmeasured video tuple before any campaign media leaves the desktop.
pub(super) async fn refuse_unmeasured_video_assignments_before_transfer(
    control: &DeviceControlPlane,
    detail: &PublishCampaignDetail,
) -> anyhow::Result<()> {
    let mut refusals = Vec::new();
    for assignment in detail
        .assignments
        .iter()
        .filter(|assignment| !assignment_may_hold_the_post(&assignment.state))
    {
        let Some(bundle) = detail
            .bundles
            .iter()
            .find(|bundle| bundle.id == assignment.bundle_id)
        else {
            continue;
        };
        if !matches!(bundle.media_kind, riviu_core::PublishMediaKind::Video) {
            continue;
        }
        if !bundle_media_shape_is_ready(bundle, route_of(control, &assignment.udid)) {
            refusals.push(format!(
                "{}: bundle video không còn đúng snapshot",
                assignment.udid
            ));
            continue;
        }
        if !matches!(route_of(control, &assignment.udid), PublishRoute::Hierarchy) {
            refusals.push(format!(
                "{}: video picker chỉ được đo trên đường hierarchy Android",
                assignment.udid
            ));
            continue;
        }
        match control.tiktok_build(&assignment.udid).await {
            Ok((package, version, locale)) => {
                if let Err(error) = video_plan_for_build(&package, &locale, &version) {
                    refusals.push(format!("{}: {error}", assignment.udid));
                }
            }
            Err(error) => refusals.push(format!(
                "{}: không đọc được package/version/locale TikTok ({error})",
                assignment.udid
            )),
        }
    }
    anyhow::ensure!(
        refusals.is_empty(),
        "không thể chọn video trên {} máy trước khi chuyển media: {}",
        refusals.len(),
        refusals.join("; ")
    );
    Ok(())
}

/// Refuse an unknown picker before any campaign media leaves the desktop.
pub(super) async fn refuse_devices_whose_sound_picker_is_not_measured(
    control: &DeviceControlPlane,
    assignments: &[&riviu_core::PublishAssignmentRecord],
) -> anyhow::Result<()> {
    let mut refusals = Vec::new();
    for assignment in assignments {
        if !control.reports_element_bounds(&assignment.udid) {
            refusals.push(format!(
                "{}: sound picker cho đường pixel chưa được đo",
                assignment.udid
            ));
            continue;
        }
        match control.tiktok_build(&assignment.udid).await {
            Ok((package, version, locale)) => {
                if let Err(error) = sound_plan_for_build(&package, &locale, &version) {
                    refusals.push(format!("{}: {error}", assignment.udid));
                }
            }
            Err(error) => refusals.push(format!(
                "{}: không đọc được package/version/locale TikTok ({error})",
                assignment.udid
            )),
        }
    }
    anyhow::ensure!(
        refusals.is_empty(),
        "không thể chọn và xác nhận nhạc trên {} máy: {}",
        refusals.len(),
        refusals.join("; ")
    );
    Ok(())
}

/// The readiness verdict for one measured build triple.
///
/// Pure and named, because the decision was otherwise reachable only through three adb round
/// trips — and a decision buried in I/O is a decision no test can argue with, which is the
/// fourth time this file has had to learn that. It is also where the version-keying is
/// visible: the catalogue is asked about **this** `(package, locale, version)`, so a phone
/// whose TikTok updated lands on `HierarchyUnknownBuild` instead of keeping a green chip
/// from some other version's complete set.
pub(super) fn readiness_of_build(package: &str, locale: &str, version: &str) -> PublishReadiness {
    let Some(controls) = riviu_core::tiktok_labels::controls_for(package, locale, version) else {
        return PublishReadiness::HierarchyUnknownBuild(format!(
            "chưa đo bộ nhãn cho {package} / {locale} / {version}"
        ));
    };
    let missing = riviu_core::tiktok_composer::ComposerPlan::missing_to_publish(&controls);
    if missing.is_empty() {
        PublishReadiness::HierarchyReady
    } else {
        PublishReadiness::HierarchyMissing(missing)
    }
}

/// The route a device is driven by, from the same signal the gate uses.
pub(super) fn route_of(control: &DeviceControlPlane, udid: &str) -> PublishRoute {
    if control.reports_element_bounds(udid) {
        PublishRoute::Hierarchy
    } else {
        PublishRoute::PixelGrid
    }
}

pub(super) fn bundle_media_shape_is_ready(
    bundle: &riviu_core::PublishBundle,
    route: PublishRoute,
) -> bool {
    match bundle.media_kind {
        riviu_core::PublishMediaKind::Image => {
            bundle.video.is_none()
                && !bundle.images.is_empty()
                && bundle.images.len() <= max_images_for(route)
        }
        riviu_core::PublishMediaKind::Video => {
            bundle.images.is_empty()
                && bundle.video.as_ref().is_some_and(|video| {
                    video.byte_len > 0
                        && video.duration_ms > 0
                        && !video.path.trim().is_empty()
                        && !video.file_name.trim().is_empty()
                        && video.sha256.len() == 64
                        && video.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
        }
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
pub(super) fn refuse_assignments_whose_bundle_is_too_large<'a>(
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

/// One device's readiness, in wire shape, for the Publish page's per-device chips.
///
/// A serializable mirror of [`PublishReadiness`] rather than serde on the original: that
/// enum carries `TikTokControl`, which has no serde on purpose (the catalogue is not a wire
/// type), so the missing labels travel as their debug names — the same names
/// `composer_scout` prints and the refusal message already shows the operator.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(
    rename_all = "camelCase",
    tag = "kind",
    rename_all_fields = "camelCase"
)]
pub enum PublishReadinessWire {
    PixelGrid,
    HierarchyReady,
    HierarchyMissing { labels: Vec<String> },
    HierarchyUnknownBuild { version: String },
}

impl From<PublishReadiness> for PublishReadinessWire {
    fn from(readiness: PublishReadiness) -> Self {
        match readiness {
            PublishReadiness::PixelGrid => Self::PixelGrid,
            PublishReadiness::HierarchyReady => Self::HierarchyReady,
            PublishReadiness::HierarchyMissing(labels) => Self::HierarchyMissing {
                labels: labels
                    .into_iter()
                    .map(|label| format!("{label:?}"))
                    .collect(),
            },
            PublishReadiness::HierarchyUnknownBuild(version) => {
                Self::HierarchyUnknownBuild { version }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevicePublishReadiness {
    pub udid: String,
    pub readiness: PublishReadinessWire,
}

/// The same answer the preflight refusal is built from, offered per device **before** the
/// operator presses anything — so the page can say which phone would refuse and why,
/// instead of the whole campaign learning it as one thrown error string.
///
/// Read-only: no admission, no lease, no session — the same posture as `is_rooted`, and the
/// reason it belongs in lib.rs's `ADMISSION_EXEMPT` list (registration and exemption live in
/// lib.rs).
#[tauri::command]
pub async fn publish_readiness(
    state: State<'_, AppState>,
    udids: Vec<String>,
) -> Result<Vec<DevicePublishReadiness>, CommandError> {
    let mut out = Vec::with_capacity(udids.len());
    for udid in udids {
        let readiness = readiness_of(&state.control, &udid).await.into();
        out.push(DevicePublishReadiness { udid, readiness });
    }
    Ok(out)
}
