//! Commands that drive a phone through the control plane: its apps, its media, its
//! input, and the lease around a manual control session.
//!
//! These are the ones that go through `state.control`, which is what separates them from
//! `android_ops` next door.

use super::*;

#[tauri::command]
pub async fn list_devices(state: State<'_, AppState>) -> Result<Vec<DeviceInfo>, CommandError> {
    Ok(state.registry.list())
}

#[tauri::command]
pub async fn refresh_devices(state: State<'_, AppState>) -> Result<Vec<DeviceInfo>, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let devices = state
        .control
        .list_devices()
        .await
        .map_err(CommandError::from)?;
    state.registry.upsert_many(devices.clone());
    Ok(devices)
}

#[tauri::command]
pub async fn prepare_device(
    state: State<'_, AppState>,
    udid: String,
) -> Result<DeviceInfo, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state
        .registry
        .set_status(&udid, riviu_core::DeviceStatus::Preparing, None);
    prepare_ui_with_control(&state.control, &udid).await?;
    // Two outcomes, and they used to be one. The confirming read is what turns "the
    // preparation ran" into "and here is the device it left behind"; when it failed, the
    // old code fell back to the *stale* registry record and then stamped `Ready` on it
    // anyway. A phone unplugged between the session closing and this read came back as a
    // Ready device with a healthy agent, described entirely from memory.
    //
    // What the preparation itself proves is kept, because it is proven:
    // `prepare_ui_with_control` installed the agent and opened and closed a UI session, so
    // `wda_ready` is earned on either path. What is not kept is a `Ready` status nobody
    // observed, and the silence about why.
    let refreshed = state.control.refresh_device(&udid).await;
    let observed = match &refreshed {
        Ok(device) => device.clone(),
        Err(_) => state
            .registry
            .get(&udid)
            .ok_or_else(|| CommandError::operation("device missing"))?,
    };
    let device = prepared_device(observed, refreshed.err().map(|error| error.to_string()));
    state.registry.upsert(device.clone());
    Ok(device)
}

#[tauri::command]
pub async fn install_ipa(
    state: State<'_, AppState>,
    udid: String,
    path: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let context = state
        .device_lease(&udid, DeviceWorkOwner::Repair, LeaseStream::Park)
        .await?;
    state
        .control
        .install_app(&context, &PathBuf::from(path))
        .await
        .map_err(CommandError::from)
}

/// Install one signed IPA onto every member of a device group.
///
/// Devices are processed one at a time: each takes its own exclusive Repair
/// lease, installs, then releases before the next. A single device's failure is
/// recorded and the batch continues, so one bad device never aborts the fleet.
#[tauri::command]
pub async fn install_ipa_to_group(
    state: State<'_, AppState>,
    group_id: String,
    path: String,
) -> Result<Vec<GroupInstallResult>, CommandError> {
    let _admission = state.ensure_accepting_work()?;

    let ipa = PathBuf::from(&path);
    if !ipa.is_file() {
        return Err(CommandError::invalid_argument(format!(
            "IPA not found at {path}"
        )));
    }

    let group = state
        .db
        .list_groups()
        .map_err(CommandError::operation)?
        .into_iter()
        .find(|group| group.id == group_id)
        .ok_or_else(|| {
            CommandError::code("GroupNotFound", format!("group {group_id} not found"))
        })?;

    if group.udids.is_empty() {
        return Err(CommandError::invalid_argument("group has no devices"));
    }

    let mut results = Vec::with_capacity(group.udids.len());
    for udid in group.udids {
        let outcome = async {
            let context = state
                .control
                .try_acquire_exclusive(&udid, DeviceWorkOwner::Repair)
                .await?;
            state.control.install_app(&context, &ipa).await
        }
        .await;
        results.push(match outcome {
            Ok(()) => GroupInstallResult {
                udid,
                ok: true,
                error: None,
            },
            Err(error) => GroupInstallResult {
                udid,
                ok: false,
                error: Some(error.to_string()),
            },
        });
    }

    Ok(results)
}

#[tauri::command]
pub async fn uninstall_app(
    state: State<'_, AppState>,
    udid: String,
    bundle_id: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let context = state
        .device_lease(&udid, DeviceWorkOwner::Repair, LeaseStream::Park)
        .await?;
    state
        .control
        .uninstall_app(&context, &bundle_id)
        .await
        .map_err(CommandError::from)
}

/// Every app one phone reports as present.
///
/// One device at a time, deliberately not a `udids: Vec<String>` batch. The batch shape
/// in this app belongs to `agent_list_statuses`, which touches no phone at all — copying
/// it here would turn opening a panel into one blocking call that paints nothing until
/// the slowest phone in the fleet answers. The frontend fans out instead, so each row
/// appears when its own phone replies.
///
/// Takes admission but **no exclusive lease**: this reads and mutates nothing, and a
/// lease would let a panel refresh evict a running session. Admission is still right,
/// because it touches a device and shutdown must drain it before releasing the fleet.
///
/// A backend that cannot enumerate **refuses**, and the refusal names
/// `listInstalledApps`. It must never be flattened into an empty list — an iPhone that
/// answers `[]` reads as a phone with nothing installed, which is a lie the panel would
/// render as fact.
#[tauri::command]
pub async fn list_installed_apps(
    state: State<'_, AppState>,
    udid: String,
) -> Result<Vec<riviu_core::InstalledApp>, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state
        .control
        .list_installed_apps(&udid)
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn screenshot(state: State<'_, AppState>, udid: String) -> Result<String, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let dest = state.artifacts_dir.join("screenshots").join(format!(
        "{}-{}.jpg",
        safe_udid_stem(&udid),
        chrono::Utc::now().timestamp_millis()
    ));
    if let Some(bytes) = state.streams.latest(&udid) {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(CommandError::operation)?;
        }
        std::fs::write(&dest, bytes.as_slice()).map_err(CommandError::operation)?;
        return Ok(dest.display().to_string());
    }
    let context = state
        .device_lease(&udid, DeviceWorkOwner::ManualControl, LeaseStream::Keep)
        .await?;
    let path = state
        .control
        .screenshot(&context, &dest)
        .await
        .map_err(CommandError::from)?;
    Ok(path.display().to_string())
}

/// Run one operator-typed shell command on a device and return its output.
///
/// An escape hatch for a person, deliberately not a seam for code: nothing in this app
/// may call it to get work done, because a string typed by a human is the one input no
/// contract can describe. `adb shell <script>` only — never `adb <subcommand>`, which
/// would put `install`, `reboot`, `root` and above all `kill-server` one typo away.
/// AGENTS.md records that `adb kill-server` tears down every other tool's connection on
/// the machine, so it must not be reachable from a text box.
///
/// Takes an exclusive lease like `syslog`, and for a stronger reason: an arbitrary script
/// can reboot the phone or kill the app a running session is driving, so firing one at a
/// device somebody else holds has to be refused rather than raced.
#[tauri::command]
pub async fn device_shell(
    state: State<'_, AppState>,
    udid: String,
    script: String,
) -> Result<riviu_core::ShellOutcome, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    // `_keeping_stream`, following `screenshot` and deliberately not `syslog`: the plain
    // acquire parks the live preview, so running a command would black the tile the
    // operator is watching for its effect.
    let context = state
        .device_lease(&udid, DeviceWorkOwner::ManualControl, LeaseStream::Keep)
        .await?;
    state
        .control
        .device_shell(&context, &script)
        .await
        .map_err(CommandError::from)
}

/// Put a picture or a video into the phone's gallery, where the operator can see it.
///
/// Stage, prepare, then import — all three, which is the difference between this and
/// `push_material`. That one stops after staging, and staging lands the file in a hidden
/// dot-directory that MediaStore does not index: a row labelled "Import" that puts a file
/// somewhere the operator cannot find it would be the same lying button the Rotate row was
/// written to avoid. The import step is what moves it into a visible directory and tells
/// MediaStore about it.
///
/// One file per call, staged as a single-file campaign because that is the shape the
/// measured pipeline takes. `_keeping_stream`, because the operator is watching the tile to
/// see the picture appear.
#[tauri::command]
pub async fn import_media(
    state: State<'_, AppState>,
    udid: String,
    path: String,
) -> Result<String, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    import_one_media(&state, &udid, &path).await
}

/// Push a *different* file into each selected phone's gallery (xiaowei "File Distribution").
/// Same per-device batch shape as `group_input`/`distribute_text`: a phone that fails is
/// recorded and the run carries on, never aborting the batch.
#[tauri::command]
pub async fn distribute_files(
    state: State<'_, AppState>,
    assignments: Vec<DistributeFileItem>,
) -> Result<GroupInputReport, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let mut report = GroupInputReport {
        completed_udids: Vec::new(),
        skipped: Vec::new(),
    };
    for item in assignments {
        let DistributeFileItem { udid, path } = item;
        match import_one_media(&state, &udid, &path).await {
            Ok(_) => report.completed_udids.push(udid),
            Err(error) => report.skipped.push(open_failure_skip(udid, error)),
        }
    }
    Ok(report)
}

#[tauri::command]
pub async fn export_media(
    state: State<'_, AppState>,
    udid: String,
    dest_dir: String,
) -> Result<MediaExportReport, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let dest = PathBuf::from(&dest_dir);
    if !dest.is_dir() {
        return Err(CommandError::invalid_argument(format!(
            "không thấy thư mục {dest_dir}"
        )));
    }
    // Per device, so exporting two phones into one folder does not interleave their camera
    // rolls into an unsortable pile.
    let into = dest.join(format!("riviu-{}", safe_udid_stem(&udid)));
    let context = state
        .device_lease(&udid, DeviceWorkOwner::ManualControl, LeaseStream::Keep)
        .await?;
    let pulled = state
        .control
        .pull_media(&context, &into)
        .await
        .map_err(CommandError::from)?;
    Ok(MediaExportReport {
        fetched: pulled.fetched.len() as u32,
        found: pulled.found as u32,
        missed: pulled.missed() as u32,
    })
}

/// Ask a device to rotate, and report the rotation it actually settled at.
///
/// Returns the observed rotation, not `()`, because measured on both fleet phones the
/// request is frequently ignored: a portrait-locked foreground app wins over every
/// mechanism tried. The caller compares what it asked for with what came back and tells
/// the operator the truth.
#[tauri::command]
pub async fn set_screen_rotation(
    state: State<'_, AppState>,
    udid: String,
    rotation: u8,
) -> Result<u8, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    // Keeps the stream for the same reason: the whole point is to watch the tile turn.
    let context = state
        .device_lease(&udid, DeviceWorkOwner::ManualControl, LeaseStream::Keep)
        .await?;
    state
        .control
        .set_screen_rotation(&context, rotation)
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn syslog(
    state: State<'_, AppState>,
    udid: String,
    lines: Option<usize>,
) -> Result<String, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let context = state
        .device_lease(&udid, DeviceWorkOwner::ManualControl, LeaseStream::Park)
        .await?;
    state
        .control
        .syslog_tail(&context, lines.unwrap_or(100))
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn reboot_device(state: State<'_, AppState>, udid: String) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let context = state
        .device_lease(&udid, DeviceWorkOwner::Repair, LeaseStream::Park)
        .await?;
    state
        .control
        .reboot(&context)
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn backup_device(
    state: State<'_, AppState>,
    udid: String,
    dest: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let context = state
        .device_lease(&udid, DeviceWorkOwner::Repair, LeaseStream::Park)
        .await?;
    state
        .control
        .backup_device(&context, std::path::Path::new(&dest))
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn restore_device(
    state: State<'_, AppState>,
    udid: String,
    src: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let context = state
        .device_lease(&udid, DeviceWorkOwner::Repair, LeaseStream::Park)
        .await?;
    state
        .control
        .restore_device(&context, std::path::Path::new(&src))
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn device_tap(
    state: State<'_, AppState>,
    udid: String,
    x: f64,
    y: f64,
    image_w: Option<f64>,
    image_h: Option<f64>,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    with_manual_session(
        &state,
        &udid,
        DeviceWorkOwner::ManualControl,
        |session| async move {
            match (image_w, image_h) {
                (Some(w), Some(h)) if w > 0.0 && h > 0.0 => session.tap_image(x, y, w, h).await,
                _ => session.tap(TapPoint { x, y }).await,
            }
        },
    )
    .await
}

/// A drag as the path the finger actually took, not as its two endpoints.
///
/// `device_swipe` sends one `pointerMove`, which the framework receives as a perfectly
/// straight line at a perfectly constant velocity between the same two points every time.
/// The overlay was deciding the whole gesture at release from exactly two samples, so that
/// is all it could ever produce -- which is what "not sticking to the finger" was.
///
/// The agent's `/actions` takes an arbitrary number of moves with individual durations in
/// ONE round trip, so the curve costs no more than the straight line did.
#[tauri::command]
pub async fn device_swipe_path(
    state: State<'_, AppState>,
    udid: String,
    path: riviu_core::types::SwipePath,
    image_w: Option<f64>,
    image_h: Option<f64>,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    // A path with no steps is not a drag; refuse it here rather than letting it reach the
    // device as a touch that never moves and never lifts.
    if path.steps.is_empty() {
        return Err(CommandError::operation(anyhow::anyhow!(
            "a swipe path needs at least one step"
        )));
    }
    with_manual_session(
        &state,
        &udid,
        DeviceWorkOwner::ManualControl,
        |session| async move {
            match (image_w, image_h) {
                (Some(w), Some(h)) if w > 0.0 && h > 0.0 => {
                    session.swipe_path_image(path, w, h).await
                }
                _ => session.swipe_path(path).await,
            }
        },
    )
    .await
}

#[tauri::command]
pub async fn device_swipe(
    state: State<'_, AppState>,
    udid: String,
    gesture: SwipeGesture,
    image_w: Option<f64>,
    image_h: Option<f64>,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    with_manual_session(
        &state,
        &udid,
        DeviceWorkOwner::ManualControl,
        |session| async move {
            match (image_w, image_h) {
                (Some(w), Some(h)) if w > 0.0 && h > 0.0 => {
                    session
                        .swipe_image(gesture.from, gesture.to, w, h, gesture.duration_ms)
                        .await
                }
                _ => session.swipe(gesture).await,
            }
        },
    )
    .await
}

#[tauri::command]
pub async fn device_type_text(
    state: State<'_, AppState>,
    udid: String,
    text: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    with_manual_session(
        &state,
        &udid,
        DeviceWorkOwner::ManualControl,
        |session| async move { session.type_text(&text).await },
    )
    .await
}

#[tauri::command]
pub async fn device_home(state: State<'_, AppState>, udid: String) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    with_manual_session(
        &state,
        &udid,
        DeviceWorkOwner::ManualControl,
        |session| async move { session.home().await },
    )
    .await
}

#[tauri::command]
pub async fn device_key(
    state: State<'_, AppState>,
    udid: String,
    key: HardwareKey,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    with_manual_session(
        &state,
        &udid,
        DeviceWorkOwner::ManualControl,
        move |session| async move { session.press_hardware_key(key).await },
    )
    .await
}

/// Lock (screen off) or unlock a phone — xiaowei "锁屏/解锁", batched by the UI over a group
/// (D, iOS `useIphoneLockScreen`; cross-platform via `UiSession::set_locked`).
#[tauri::command]
pub async fn set_screen_locked(
    state: State<'_, AppState>,
    udid: String,
    locked: bool,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    with_manual_session(
        &state,
        &udid,
        DeviceWorkOwner::ManualControl,
        move |session| async move { session.set_locked(locked).await },
    )
    .await
}

#[tauri::command]
pub async fn device_control_begin(
    state: State<'_, AppState>,
    udid: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state.begin_overlay_session(&udid).await
}

#[tauri::command]
pub async fn device_control_end(
    state: State<'_, AppState>,
    udid: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state.end_overlay_session(&udid).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn group_input(
    state: State<'_, AppState>,
    udids: Vec<String>,
    kind: String,
    x: Option<f64>,
    y: Option<f64>,
    to_x: Option<f64>,
    to_y: Option<f64>,
    text: Option<String>,
    image_w: Option<f64>,
    image_h: Option<f64>,
    key: Option<HardwareKey>,
    sync: Option<GroupSyncPolicy>,
) -> Result<GroupInputReport, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    check_group_input(&kind, key.is_some())?;
    let scale = matches!((image_w, image_h), (Some(w), Some(h)) if w > 0.0 && h > 0.0);
    // Group-sync timing/offset (A1). Absent policy = the old lockstep behaviour, so callers
    // that never send `sync` are unchanged. One seed per operation keeps successive group
    // actions different while any single one stays reproducible (the policy is pure/tested).
    let sync = sync.unwrap_or_default();
    let group_count = udids.len();
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut report = GroupInputReport {
        completed_udids: Vec::new(),
        skipped: Vec::new(),
    };
    for (ordinal, udid) in udids.into_iter().enumerate() {
        // Compute this device's delay/offset before touching anything. Sleep *before*
        // opening the session so a staggered wait does not hold a GroupSync lease idle.
        let plan = sync.plan(ordinal, group_count, seed);
        if plan.delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(plan.delay_ms)).await;
        }
        // Bound for the whole iteration, not consumed here: dropping it early would let the
        // overlay close mid-gesture and hand this phone to another owner.
        let overlay_hold = state.overlay_ui_session(&udid).await;
        let owned = if overlay_hold.is_none() {
            match state
                .control
                .open_manual_session(&udid, DeviceWorkOwner::GroupSync)
                .await
            {
                Ok(context) => Some(context),
                Err(error) => {
                    report
                        .skipped
                        .push(open_failure_skip(udid, CommandError::from(error)));
                    continue;
                }
            }
        } else {
            None
        };
        // The same rule as the open above: a lookup that fails is this phone's problem,
        // not the batch's. `?` here threw away every phone already actioned.
        let session = match overlay_hold.as_ref() {
            Some(hold) => hold.session(),
            None => match state.control.session(
                owned
                    .as_ref()
                    .expect("group input opened a session when no overlay is held"),
            ) {
                Ok(session) => session,
                Err(error) => {
                    report
                        .skipped
                        .push(open_failure_skip(udid, CommandError::from(error)));
                    continue;
                }
            },
        };
        // In image mode the coordinates are pixels bounded by the frame, so jitter must be
        // clamped on-screen; in logical mode there is no upper bound (only floored at 0).
        let bound_w = scale.then(|| image_w.unwrap());
        let bound_h = scale.then(|| image_h.unwrap());
        let action = match kind.as_str() {
            "tap" => {
                let (x, y) = apply_offset(
                    x.unwrap_or(0.0),
                    y.unwrap_or(0.0),
                    plan.dx,
                    plan.dy,
                    bound_w,
                    bound_h,
                );
                if scale {
                    session
                        .tap_image(x, y, image_w.unwrap(), image_h.unwrap())
                        .await
                } else {
                    session.tap(TapPoint { x, y }).await
                }
            }
            "swipe" => {
                // Shift both endpoints by the same offset: the gesture's shape and length are
                // preserved, only its position on the screen jitters.
                let (fx, fy) = apply_offset(
                    x.unwrap_or(0.0),
                    y.unwrap_or(0.0),
                    plan.dx,
                    plan.dy,
                    bound_w,
                    bound_h,
                );
                let (tx, ty) = apply_offset(
                    to_x.unwrap_or(0.0),
                    to_y.unwrap_or(0.0),
                    plan.dx,
                    plan.dy,
                    bound_w,
                    bound_h,
                );
                let from = TapPoint { x: fx, y: fy };
                let to = TapPoint { x: tx, y: ty };
                if scale {
                    session
                        .swipe_image(from, to, image_w.unwrap(), image_h.unwrap(), 300)
                        .await
                } else {
                    session
                        .swipe(SwipeGesture {
                            from,
                            to,
                            duration_ms: 300,
                        })
                        .await
                }
            }
            "type" => session.type_text(text.as_deref().unwrap_or("")).await,
            "home" => session.home().await,
            // Validated before the loop, so this arm cannot be reached without a key.
            "key" => {
                session
                    .press_hardware_key(key.expect("key was validated"))
                    .await
            }
            _ => unreachable!("group input kind was validated"),
        };
        // The session is closed whatever the action did. Leaking a GroupSync lease because a
        // tap failed would take the phone out of the fleet until the app restarts.
        let cleanup = match owned {
            Some(context) => state.control.close_manual_session(context).err(),
            None => None,
        };
        // Both arms below consume `udid`, and the cleanup report after them needs it.
        let udid_for_cleanup = udid.clone();
        match action {
            Ok(()) => report.completed_udids.push(udid),
            // **Record and carry on, rather than abort.** This used to be `?`, so the first
            // phone that failed for any reason other than Busy threw away
            // `completed_udids` and told the operator the whole batch had failed — when in
            // a twenty-phone fleet nineteen of them may have worked. One fleet-batch shape
            // in this codebase, matching `install_ipa_to_group`.
            Err(error) => report.skipped.push(GroupInputSkip {
                udid,
                code: "ActionFailed".to_string(),
                current_owner: None,
                message: Some(error.to_string()),
            }),
        }
        // A cleanup that failed does not undo an input that landed, and it is not a
        // reason to abandon the phones after this one. The udid stays in
        // `completed_udids` because the tap really did happen; the failure is reported
        // beside it so the operator learns the session did not close cleanly. Appearing
        // in both lists is the accurate description of that, not a contradiction.
        if let Some(error) = cleanup {
            let error = CommandError::from(error);
            report.skipped.push(GroupInputSkip {
                udid: udid_for_cleanup,
                code: "CleanupFailed".to_string(),
                current_owner: None,
                message: Some(error.message.to_string()),
            });
        }
    }
    Ok(report)
}

/// Type a *different* string onto each selected phone (xiaowei "文字分发 / Text Distribution").
///
/// The frontend has already split the block and paired each piece to a phone in the operator's
/// chosen order, so here we only apply. Cross-platform: it goes through `UiSession::type_text`,
/// which on Android reaches `ACTION_SET_TEXT` (the one route that carries Vietnamese
/// diacritics) and on iOS reaches WDA — the same path `group_input`'s `type` uses. Same
/// per-device batch shape as `group_input`: a phone that fails is recorded and the run
/// carries on, never aborting the batch.
#[tauri::command]
pub async fn distribute_text(
    state: State<'_, AppState>,
    assignments: Vec<DistributeTextItem>,
) -> Result<GroupInputReport, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let mut report = GroupInputReport {
        completed_udids: Vec::new(),
        skipped: Vec::new(),
    };
    for item in assignments {
        let DistributeTextItem { udid, text } = item;
        // Bound for the whole iteration, not consumed here: dropping it early would let the
        // overlay close mid-gesture and hand this phone to another owner.
        let overlay_hold = state.overlay_ui_session(&udid).await;
        let owned = if overlay_hold.is_none() {
            match state
                .control
                .open_manual_session(&udid, DeviceWorkOwner::GroupSync)
                .await
            {
                Ok(context) => Some(context),
                Err(error) => {
                    report
                        .skipped
                        .push(open_failure_skip(udid, CommandError::from(error)));
                    continue;
                }
            }
        } else {
            None
        };
        let session = match overlay_hold.as_ref() {
            Some(hold) => hold.session(),
            None => match state.control.session(
                owned
                    .as_ref()
                    .expect("distribute_text opened a session when no overlay is held"),
            ) {
                Ok(session) => session,
                Err(error) => {
                    report
                        .skipped
                        .push(open_failure_skip(udid, CommandError::from(error)));
                    continue;
                }
            },
        };
        let action = session.type_text(&text).await;
        let cleanup = match owned {
            Some(context) => state.control.close_manual_session(context).err(),
            None => None,
        };
        let udid_for_cleanup = udid.clone();
        match action {
            Ok(()) => report.completed_udids.push(udid),
            Err(error) => report.skipped.push(GroupInputSkip {
                udid,
                code: "ActionFailed".to_string(),
                current_owner: None,
                message: Some(error.to_string()),
            }),
        }
        if let Some(error) = cleanup {
            let error = CommandError::from(error);
            report.skipped.push(GroupInputSkip {
                udid: udid_for_cleanup,
                code: "CleanupFailed".to_string(),
                current_owner: None,
                message: Some(error.message.to_string()),
            });
        }
    }
    Ok(report)
}

/// Bring one app to the front of one phone (xiaowei's App List, where a click launches).
///
/// Goes through the control plane rather than the Android driver directly, because unlike
/// everything else in this block it is *not* Android-only: foregrounding an app is a thing
/// both platforms do, and the lease keeps the live tile up while it happens.
#[tauri::command]
pub async fn launch_device_app(
    state: State<'_, AppState>,
    udid: String,
    bundle_id: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let context = state
        .device_lease(&udid, DeviceWorkOwner::ManualControl, LeaseStream::Keep)
        .await?;
    state
        .control
        .launch_app(&context, &bundle_id)
        .await
        .map_err(CommandError::from)
}

/// Read the phone's clipboard onto this machine (xiaowei "Export Clipboard").
///
/// The one row of the reference product's menu that this app had a *session method* for and
/// no command over it, which is why it went missing for so long: `UiSession::get_clipboard`
/// has existed since the interaction work and nothing could call it.
///
/// The ceiling is [`MAX_INTERACTION_CLIPBOARD_BYTES`] and not a number chosen here. Measured
/// 21/08/2026 on 23021RAAEG: asking for 256 KiB — which looked like a generous, harmless
/// choice — is refused outright with `clipboard read limit exceeds 65536 bytes`, because the
/// capability contract pins the value on both platforms rather than treating it as a maximum.
/// So the constant is the contract's, and a clipboard bigger than that is the phone's answer
/// to report, not something to raise a limit for.
#[tauri::command]
pub async fn device_get_clipboard(
    state: State<'_, AppState>,
    udid: String,
) -> Result<ClipboardRead, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let read = std::sync::Arc::new(parking_lot::Mutex::new(None));
    let sink = read.clone();
    with_manual_session(
        &state,
        &udid,
        DeviceWorkOwner::ManualControl,
        move |session| async move {
            let (content_type, bytes) = session
                .get_clipboard(riviu_core::MAX_INTERACTION_CLIPBOARD_BYTES)
                .await?;
            *sink.lock() = Some((content_type, bytes));
            Ok(())
        },
    )
    .await?;
    let (content_type, bytes) = read
        .lock()
        .take()
        .ok_or_else(|| CommandError::operation("máy không trả về nội dung clipboard"))?;
    Ok(ClipboardRead {
        // Lossy on purpose: a clipboard holding half a UTF-8 sequence is still worth
        // showing, and refusing the whole read over one bad byte would lose the rest.
        text: String::from_utf8_lossy(&bytes).to_string(),
        bytes: bytes.len(),
        content_type,
    })
}

/// Write text onto the phone's clipboard, so the operator can paste it there by hand.
#[tauri::command]
pub async fn device_set_clipboard(
    state: State<'_, AppState>,
    udid: String,
    text: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    with_manual_session(
        &state,
        &udid,
        DeviceWorkOwner::ManualControl,
        move |session| async move { session.set_clipboard("text/plain", text.as_bytes()).await },
    )
    .await
}

/// One phone's health, in the terms every refusal in this app is written in.
///
/// **Read-only, no lease** — the model is `is_rooted`, not `prepare_device`: a diagnostics
/// screen must be able to describe a phone it cannot drive, and must not change the thing
/// it is describing. Every probe that fails becomes a note instead of an error, because
/// "this section could not be asked" is itself the diagnosis the operator came for.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DeviceHealthReport {
    pub udid: String,
    /// The roster's current word for the phone, or `None` when it is not listed at all.
    pub roster_status: Option<riviu_core::DeviceStatus>,
    /// The cached agent status every tile already shows — no device I/O.
    pub agent: riviu_core::AgentStatus,
    /// A live `/status` probe, `None` when the active backend is not Android.
    pub agent_ready_now: Option<bool>,
    /// Riviu helper reachable right now (cached client answered).
    pub helper_reachable: Option<bool>,
    /// Helper APK installed; `None` means the question itself failed — not "absent" (§9.97).
    pub helper_installed: Option<bool>,
    pub root: Option<riviu_core::DeviceRootStatus>,
    pub tiktok_package: Option<String>,
    pub tiktok_version: Option<String>,
    pub tiktok_locale: Option<String>,
    /// Every section that could not be asked, named in the operator's language.
    pub notes: Vec<String>,
}

#[tauri::command]
pub async fn device_health(
    state: State<'_, AppState>,
    udid: String,
) -> Result<DeviceHealthReport, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let roster_status = state
        .registry
        .list()
        .into_iter()
        .find(|device| device.udid == udid)
        .map(|device| device.status);
    let agent = state.control.cached_agent_status(&udid);
    let mut report = DeviceHealthReport {
        udid: udid.clone(),
        roster_status,
        agent,
        agent_ready_now: None,
        helper_reachable: None,
        helper_installed: None,
        root: None,
        tiktok_package: None,
        tiktok_version: None,
        tiktok_locale: None,
        notes: Vec::new(),
    };
    let Some(android) = &state.android else {
        report.notes.push(
            "Backend đang chạy không phải Android — chỉ đọc được roster và cache agent."
                .to_string(),
        );
        return Ok(report);
    };
    report.agent_ready_now = Some(android.agent_ready(&udid).await);
    let (helper_reachable, helper_installed) = android.helper_probe(&udid).await;
    report.helper_reachable = Some(helper_reachable);
    report.helper_installed = helper_installed;
    if helper_installed.is_none() {
        report.notes.push(
            "Không hỏi được máy về Riviu helper — chưa với tới được, không phải chưa cài."
                .to_string(),
        );
    }
    // The same two questions `is_rooted` answers, and the same subtraction: name the
    // route, not the union.
    let has_su = android.is_rooted(&udid).await;
    report.root = Some(riviu_core::DeviceRootStatus {
        has_su,
        shell_is_root: !has_su && android.can_run_privileged(&udid).await,
    });
    match android.tiktok_build(&udid).await {
        Ok((package, version, locale)) => {
            report.tiktok_package = Some(package);
            report.tiktok_version = (!version.is_empty()).then_some(version);
            report.tiktok_locale = (!locale.is_empty()).then_some(locale);
        }
        Err(error) => report
            .notes
            .push(format!("Không đọc được build TikTok: {error:#}")),
    }
    Ok(report)
}
