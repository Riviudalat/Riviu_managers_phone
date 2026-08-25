//! The live view: the settings behind it, the loopback endpoint the webview connects to,
//! and the per-tile controls.

use super::*;

#[tauri::command]
pub fn get_stream_settings(state: State<'_, AppState>) -> StreamSettings {
    state.stream_settings.read().clone()
}

/// Save the stream settings, and make the two that used to be ignored take effect.
///
/// `fps` was **overwritten** with the compiled-in constant here, so an operator could
/// move the frame-rate control and the value never left this function. `grid_quality`
/// had no reader anywhere in the tree. Both are now clamped rather than discarded and
/// pushed into the Android view path.
///
/// And the whole row now **survives a restart**. It did not: the value lived only in an
/// `Arc<RwLock<_>>` built from `Default` at bootstrap, so an operator's choice lasted until
/// they closed the app — a save that quietly forgets is not much better than the no-op this
/// already replaced once.
///
/// Running tiles are restarted so the change is visible: a settings row that only
/// applies to phones started later is the same silent no-op this replaces. The restart
/// is the path the watchdog already takes several times an hour, so it costs a second
/// of black tile and nothing else.
#[tauri::command]
pub async fn set_stream_settings(
    state: State<'_, AppState>,
    settings: StreamSettings,
) -> Result<StreamSettings, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let mut s = settings;
    s.fps = clamp_stream_fps(s.fps);
    // Persist before applying. A save that takes effect for this session and vanishes on
    // restart is worse than one that reports it could not be written.
    state
        .db
        .save_stream_settings(&s)
        .map_err(CommandError::operation)?;
    *state.stream_settings.write() = s.clone();

    if let Some(android) = &state.android {
        android.set_view_tuning(s.grid_quality.clone(), s.focus_quality.clone(), s.fps);
        for device in state.registry.list() {
            if device.platform != riviu_core::DevicePlatform::Android {
                continue;
            }
            if !android
                .view_is_running(&device.udid, riviu_android_driver::ViewPreset::Tile)
                .await
            {
                continue;
            }
            // Retune restarts the same producer rather than spawning a second one.
            // A failure here is logged and skipped: one phone that will not retune must
            // not stop the setting from reaching the rest of the fleet.
            //
            // Under the same ceiling as every other producer restart. The loop is already
            // sequential, so on an idle fleet it never waits; the permit is what stops it
            // stacking on top of recoveries the keeper started, which is how a settings
            // change used to become a twenty-first concurrent scrcpy spawn.
            let frames = state
                .view_paint
                .sample(&device.udid)
                .map(|report| report.frames)
                .unwrap_or(0);
            let permit = match state
                .view_recovery
                .admit_operator(&device.udid, frames)
                .await
            {
                Ok(permit) => permit,
                Err(error) => {
                    log::warn!(
                        "could not retune {} after a settings change: {}",
                        device.udid,
                        error.message
                    );
                    continue;
                }
            };
            if let Err(error) = android
                .set_view_preset(&device.udid, riviu_android_driver::ViewPreset::Tile)
                .await
            {
                log::warn!(
                    "could not retune {} after a settings change: {error:#}",
                    device.udid
                );
            }
            drop(permit);
            state.view_paint.clear(&device.udid);
        }
    }
    Ok(s)
}

#[tauri::command]
pub fn latest_frame(
    state: State<'_, AppState>,
    udid: String,
) -> Result<Option<String>, CommandError> {
    Ok(state.streams.latest(&udid).map(|bytes| {
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes.as_slice())
    }))
}

#[tauri::command]
pub fn view_endpoint(state: State<'_, AppState>) -> Result<Option<String>, CommandError> {
    Ok(state.view_hub.endpoint())
}

/// What the frontend painted, for every device it is tracking, as of its last tick.
///
/// The frontend is the only thing that can see whether a frame came out of the decoder, and
/// AGENTS.md 9.66 is why it cannot simply log it: vite forwards the page's console but not a
/// Web Worker's, so counters that stay inside the worker are invisible by construction. This
/// carries them to the one place that can act on them.
///
/// Deliberately **not** on the mutating-command inventory and deliberately without
/// `ensure_accepting_work`: it writes a ledger, touches no device, and must keep working
/// while the app is draining so the last reports before a quit are not lost.
///
/// One call per tick for the whole fleet rather than one per device per beat — at twenty
/// phones and a 1 s beat the per-device shape would be twenty IPC round trips a second to
/// say "still fine".
#[tauri::command]
pub fn view_report_paint(state: State<'_, AppState>, reports: Vec<PaintReport>) {
    let now = std::time::Instant::now();
    for report in &reports {
        // The hub's generation is the authority on which producer is current. A report from
        // before a restart is not evidence about the producer that replaced it.
        let current = state.view_hub.current_generation(&report.udid);
        state.view_paint.record(report, current, now);
        if report.generation == current {
            state
                .view_recovery
                .note_painted(&report.udid, report.frames);
        }
    }
}

#[tauri::command]
pub async fn view_ensure(state: State<'_, AppState>, udid: String) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let Some(android) = &state.android else {
        return Ok(());
    };
    let platform = state
        .registry
        .get(&udid)
        .map(|device| device.platform)
        .unwrap_or(riviu_core::DevicePlatform::Ios);
    if platform != riviu_core::DevicePlatform::Android {
        return Ok(());
    }
    // Do not tear down a producer somebody else is already replacing. This stops before it
    // claims, so losing that race used to leave the device with NO producer at all until the
    // keeper's next tick -- a stream the operator was watching went away because a recovery
    // for it started somewhere else. A start already in flight satisfies "ensure".
    if android.view_start_in_flight(&udid) {
        return Ok(());
    }
    // "Ensure" means two different operations depending on what is there, and only one of
    // them is rationed. With no producer running this is a first start: nothing is torn down,
    // so it takes no permit -- which is what keeps `startFleetPreview`'s twenty-way fan-out
    // (startPreview.ts) as fast as the bench says it can be. With one running, ensuring it
    // means replacing a picture that may be working, and that is precisely what the ceiling
    // is for.
    if !android.view_is_active(&udid).await {
        crate::view_watchdog::start_android_view(android, &state.registry, &udid)
            .await
            .map_err(CommandError::operation)?;
        return Ok(());
    }
    let frames = state
        .view_paint
        .sample(&udid)
        .map(|report| report.frames)
        .unwrap_or(0);
    let permit = state.view_recovery.admit_operator(&udid, frames).await?;
    // Stop-then-start, the preset the operator last asked for, and the ledger cleared -- all
    // of it in one place now, so the keeper and this command cannot drift apart.
    crate::view_watchdog::restart_android_view(
        android,
        &state.registry,
        &state.view_paint,
        &udid,
        permit,
    )
    .await
    .map_err(CommandError::operation)?;
    Ok(())
}

/// Ask the phone for a fresh keyframe. The cheap half of "the picture is stuck".
///
/// Takes **no recovery permit**, deliberately: it tears nothing down, so the ceiling that
/// bounds how much of the fleet can go dark at once has nothing to bound here. It is also
/// the operator-facing half of what the watchdog now tries first — one byte and a fresh IDR,
/// against ~11.5 s of black tile for a restart.
#[tauri::command]
pub async fn view_request_keyframe(
    state: State<'_, AppState>,
    udid: String,
) -> Result<bool, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let Some(android) = &state.android else {
        return Ok(false);
    };
    android
        .request_keyframe(&udid)
        .await
        .map_err(CommandError::operation)
}

/// One touch event, live, on the scrcpy control socket.
///
/// **Not a replacement for `device_tap`.** Taps, keys and text stay on uiautomator2, which
/// handles Vietnamese diacritics and does not care what size the video is. This exists for
/// the continuous middle of a drag, which until now reached the phone only after the operator
/// let go — `FocusStream` buffered the samples and posted a single swipe on release, so the
/// picture stood still under a moving finger. See AGENTS.md 9.77.
///
/// Deliberately outside `with_manual_session`. That helper claims device ownership and opens
/// a uiautomator2 session, neither of which this path needs — and a pointer at 60 Hz would be
/// claiming and releasing ownership sixty times a second. The control socket already belongs
/// to the producer that is drawing the picture being touched.
///
/// `Ok(false)` means the phone is not streaming, so the caller should fall back to the agent
/// rather than report a failure. A refusal to admit work still throws, because a drag during
/// shutdown should stop like everything else.
#[tauri::command]
pub async fn view_inject_touch(
    state: State<'_, AppState>,
    udid: String,
    action: String,
    x: f64,
    y: f64,
    image_w: f64,
    image_h: f64,
) -> Result<bool, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let Some(android) = &state.android else {
        return Ok(false);
    };
    let action =
        riviu_android_driver::TouchAction::parse(&action).map_err(CommandError::operation)?;
    android
        .inject_touch(&udid, action, x, y, image_w, image_h)
        .await
        .map_err(CommandError::operation)
}

#[tauri::command]
pub async fn view_set_preset(
    state: State<'_, AppState>,
    udid: String,
    preset: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let Some(android) = &state.android else {
        return Ok(());
    };
    let platform = state
        .registry
        .get(&udid)
        .map(|device| device.platform)
        .unwrap_or(riviu_core::DevicePlatform::Ios);
    if platform != riviu_core::DevicePlatform::Android {
        return Ok(());
    }
    let preset = match preset.as_str() {
        "overlay" => riviu_android_driver::ViewPreset::Overlay,
        _ => riviu_android_driver::ViewPreset::Tile,
    };
    // A retune restarts the same producer, so it costs the adb server exactly what a
    // recovery costs and it belongs under the same ceiling. It uses the operator lane
    // because it *is* an operator action — opening or closing an overlay — and that lane has
    // no per-device backoff: refusing a person's second click because their first was 40 s
    // ago would read as the app being broken.
    //
    // A refusal here is not the end of it. The keeper reconciles toward
    // `desired_view_preset` on its own tick, which is what makes this safe to refuse at all;
    // `set_view_preset` records the desire before it does any work.
    let frames = state
        .view_paint
        .sample(&udid)
        .map(|report| report.frames)
        .unwrap_or(0);
    let permit = state.view_recovery.admit_operator(&udid, frames).await?;
    let outcome = android
        .set_view_preset(&udid, preset)
        .await
        .map_err(CommandError::operation)
        .map(|_| ());
    // Held across the retune for the same reason the recovery path holds it: a producer
    // that has spawned and not yet published is still using the resource being rationed.
    drop(permit);
    // The producer was replaced, so evidence about the old one is not evidence about this
    // one -- the same rule `restart_android_view` applies.
    state.view_paint.clear(&udid);
    outcome
}

#[tauri::command]
pub fn save_view_snapshot(
    state: State<'_, AppState>,
    udid: String,
    jpeg: Vec<u8>,
) -> Result<String, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    if jpeg.len() < 3 || jpeg[0] != 0xff || jpeg[1] != 0xd8 {
        return Err(CommandError::operation("view snapshot is not a JPEG"));
    }
    let dest = state.artifacts_dir.join("screenshots").join(format!(
        "{}-{}.jpg",
        safe_udid_stem(&udid),
        chrono::Utc::now().timestamp_millis()
    ));
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(CommandError::operation)?;
    }
    std::fs::write(&dest, jpeg).map_err(CommandError::operation)?;
    Ok(dest.display().to_string())
}

/// The highest frame rate a settings row may ask for.
///
/// The scrcpy presets top out at 30 and the fleet measured 24–29 FPS, so a row offering
/// 60 would promise a rate no phone here delivers. Clamped rather than validated so a
/// stored value from an older build cannot refuse the whole save.
pub(crate) const MAX_SETTABLE_VIEW_FPS: u32 = 30;

/// The one place the settable frame rate is bounded.
///
/// Two callers now that the value is persisted — the save, and the load at startup — and
/// they must not drift: `get_stream_settings` reporting one number while the encoder runs
/// another is precisely the silent disagreement AGENTS.md 9.59 records as already fixed once.
pub(crate) fn clamp_stream_fps(fps: u32) -> u32 {
    fps.clamp(riviu_android_driver::MIN_VIEW_FPS, MAX_SETTABLE_VIEW_FPS)
}
