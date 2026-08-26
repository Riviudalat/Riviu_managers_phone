//! Things about this installation rather than about a phone: the Apple ID, the WDA
//! signing identity, which driver mode came up, and the updater.

use super::*;

#[tauri::command]
pub fn get_apple_id(state: State<'_, AppState>) -> Result<riviu_core::AppleIdConfig, CommandError> {
    state.signing.apple_id_config().map_err(err)
}

#[tauri::command]
pub fn set_apple_id(
    state: State<'_, AppState>,
    email: String,
    password: String,
) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state.signing.save_apple_id(&email, &password).map_err(err)
}

#[tauri::command]
pub fn clear_apple_id(state: State<'_, AppState>) -> Result<(), CommandError> {
    let _admission = state.ensure_accepting_work()?;
    state.signing.clear_apple_id().map_err(err)
}

#[tauri::command]
pub async fn resign_wda(state: State<'_, AppState>, udid: String) -> Result<String, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    // Legacy stock tooling only; this agent does not provide trusted TikTok text input.
    state
        .registry
        .set_status(&udid, riviu_core::DeviceStatus::Preparing, None);
    let _context = state
        .control
        .try_acquire_exclusive(&udid, DeviceWorkOwner::Repair)
        .await
        .map_err(CommandError::from)?;
    match state
        .signing
        .sign_and_install_wda(&udid, &state.legacy_wda_bundle)
        .await
    {
        Ok(result) => {
            if let Some(mut device) = state.registry.get(&udid) {
                device.wda_ready = true;
                device.wda_expires_at = Some(result.expires_at);
                device.status = riviu_core::DeviceStatus::Ready;
                device.last_error = None;
                state.registry.upsert(device);
            }
            Ok(result.message)
        }
        Err(err) => {
            let msg = err.to_string();
            state
                .registry
                .set_status(&udid, riviu_core::DeviceStatus::Error, Some(msg.clone()));
            Err(CommandError::operation(msg))
        }
    }
}

#[tauri::command]
pub async fn bulk_resign_wda(
    state: State<'_, AppState>,
    udids: Vec<String>,
) -> Result<Vec<String>, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let mut contexts = Vec::with_capacity(udids.len());
    for udid in &udids {
        contexts.push(
            state
                .control
                .try_acquire_exclusive(udid, DeviceWorkOwner::Repair)
                .await
                .map_err(CommandError::from)?,
        );
    }
    let results = state
        .signing
        .bulk_resign(&udids, &state.legacy_wda_bundle)
        .await
        .map_err(CommandError::operation)?;
    let mut messages = Vec::new();
    for result in results {
        if let Some(mut device) = state.registry.get(&result.udid) {
            device.wda_ready = true;
            device.wda_expires_at = Some(result.expires_at);
            device.status = riviu_core::DeviceStatus::Ready;
            device.last_error = None;
            state.registry.upsert(device);
        }
        messages.push(result.message);
    }
    Ok(messages)
}

#[tauri::command]
pub fn driver_mode(state: State<'_, AppState>) -> String {
    match state.driver_mode {
        riviu_ios_driver::DriverMode::Mock => "mock".into(),
        riviu_ios_driver::DriverMode::Pymobiledevice3 => "pymobiledevice3".into(),
    }
}

/// Why real devices cannot be listed, or `None` when the sidecar is healthy.
/// Read-only, so it needs no admission gate.
///
/// Two sources, boot snapshot first. The snapshot is the more specific fact when it
/// exists — the sidecar never started, so there is nothing to list from. The live listing
/// error is what covers everything after boot: a sidecar that started fine and then had
/// `list` fail, or answered with `{"devices": [], "error": "pymobiledevice3 not installed"}`
/// at exit code 0. That second case used to be dropped entirely, so the fleet went empty
/// and the UI said nothing (AGENTS.md 9.29).
///
/// It is asked, not stored, so it clears itself: a listing that succeeds sets it back to
/// `None` and an operator who fixes the machine stops being told it is broken without
/// restarting the app.
#[tauri::command]
pub fn driver_degraded_reason(state: State<'_, AppState>) -> Option<String> {
    if let Some(reason) = state.driver_degraded_reason.clone() {
        return Some(reason);
    }
    state.driver_list_error.as_ref().and_then(|probe| probe())
}

/// Why the Android half of the fleet is absent, or `None` when it joined.
///
/// Separate from [`driver_degraded_reason`] on purpose: "this machine has no
/// adb" and "the iOS sidecar failed" are different facts with different fixes,
/// and collapsing them into one string would send an operator looking in the
/// wrong place.
#[tauri::command]
pub fn android_unavailable_reason(state: State<'_, AppState>) -> Option<String> {
    state.android_unavailable_reason.clone()
}

/// What went wrong verifying the bundled Android tools at boot. Empty is the healthy answer.
///
/// Separate from [`android_unavailable_reason`] on purpose: that one means "no Android phone can
/// join the fleet", and this one means "they join and then cannot be driven". Reporting the
/// second as the first sends the operator to look at adb, which is the one file that *did* work.
#[tauri::command]
pub fn android_tool_problems(state: State<'_, AppState>) -> Vec<String> {
    state.android_tool_problems.clone()
}

/// How long one repeating frontend error stays quiet after it has reached the log once.
///
/// A throw inside a React render, or a rejected promise inside an effect, can repeat on every
/// frame. Unthrottled that is thousands of identical lines a minute, which is the log-noise
/// failure this project already measured once from the other direction: 83% of one release log
/// was a single sentence about a normal thing, and the real signal underneath was unreadable.
/// Ten seconds is short enough that a burst is still visibly a burst and long enough that a
/// render loop costs six lines a minute instead of sixty thousand.
const FRONTEND_ERROR_QUIET: std::time::Duration = std::time::Duration::from_secs(10);

/// The longest frontend message that gets written down.
///
/// A rejected `fetch` can carry a whole response body, and a React error can carry a component
/// stack hundreds of lines deep. The log is for recognising what broke, not for holding the
/// payload.
const FRONTEND_MESSAGE_MAX: usize = 2_000;

/// How many distinct frontend errors are tracked for throttling before the oldest is forgotten.
///
/// Bounded on purpose: a message containing a counter or a timestamp produces a new fingerprint
/// every time, and an unbounded map here would be a leak that only appears while the app is
/// already misbehaving -- the worst possible time to add one.
const FRONTEND_ERROR_TRACKED: usize = 64;

struct RepeatedFrontendError {
    last_logged: std::time::Instant,
    suppressed: u32,
}

/// Rate limiter for frontend error reports, keyed by what the error *is*.
///
/// Kept pure of Tauri and of `AppState` so a test can drive it with an explicit clock, and so
/// the command below works even when `AppState::bootstrap` is the thing that failed.
#[derive(Default)]
struct FrontendErrorRate {
    seen: std::collections::HashMap<String, RepeatedFrontendError>,
}

impl FrontendErrorRate {
    /// `Some(n)` means write this one down, and `n` identical ones were dropped since the last.
    /// `None` means stay quiet.
    fn admit(&mut self, fingerprint: &str, now: std::time::Instant) -> Option<u32> {
        if let Some(entry) = self.seen.get_mut(fingerprint) {
            if now.duration_since(entry.last_logged) < FRONTEND_ERROR_QUIET {
                entry.suppressed = entry.suppressed.saturating_add(1);
                return None;
            }
            let suppressed = entry.suppressed;
            entry.last_logged = now;
            entry.suppressed = 0;
            return Some(suppressed);
        }
        if self.seen.len() >= FRONTEND_ERROR_TRACKED {
            // Forget the least recently logged rather than refusing to track anything new: a
            // fresh error is the interesting one, and the evicted entry only loses its
            // throttling, not its right to be logged.
            if let Some(stalest) = self
                .seen
                .iter()
                .min_by_key(|(_, entry)| entry.last_logged)
                .map(|(key, _)| key.clone())
            {
                self.seen.remove(&stalest);
            }
        }
        self.seen.insert(
            fingerprint.to_string(),
            RepeatedFrontendError {
                last_logged: now,
                suppressed: 0,
            },
        );
        Some(0)
    }
}

fn frontend_error_rate() -> &'static parking_lot::Mutex<FrontendErrorRate> {
    static RATE: std::sync::OnceLock<parking_lot::Mutex<FrontendErrorRate>> =
        std::sync::OnceLock::new();
    RATE.get_or_init(|| parking_lot::Mutex::new(FrontendErrorRate::default()))
}

/// One frontend error report, as the line it becomes.
///
/// Pure so the shape can be pinned by a test: the kind, the message and the source all have to
/// survive, and a suppressed count has to say so rather than being folded into the text.
fn frontend_error_line(kind: &str, message: &str, source: Option<&str>, suppressed: u32) -> String {
    let mut line = format!("frontend {kind}: {message}");
    if let Some(source) = source {
        line.push_str(&format!(" (at {source})"));
    }
    if suppressed > 0 {
        line.push_str(&format!(" [+{suppressed} identical suppressed]"));
    }
    line
}

/// **Write a frontend failure into the app log, because nothing else did.**
///
/// Before this existed there was no path at all from a frontend error to any record: a grep of
/// `apps/desktop/src` found no `window.onerror`, no `unhandledrejection` listener and no error
/// boundary, and a grep of `src-tauri` found nothing that accepted one. So a throw during render
/// unmounted the React tree and left **a blank window and an empty log** -- the report that
/// started this work. This is the receiving end; `main.tsx` is the sending end.
///
/// Deliberately takes **no `State<AppState>`**. The moments worth recording include the ones
/// where `AppState::bootstrap` is what failed, and a command that needs the state would be
/// unavailable in exactly those. It is also exempt from admission for the same reason: refusing
/// it during shutdown drain would silence the errors most likely to explain the shutdown.
///
/// Returns nothing, and cannot fail. A reporting path that can itself fail invites a caller to
/// handle that failure, and on this side of the wire there is nowhere for that to go.
#[tauri::command]
pub fn log_frontend_error(kind: String, message: String, source: Option<String>) {
    let mut message = message;
    message.truncate(FRONTEND_MESSAGE_MAX);
    let source = source.map(|mut source| {
        source.truncate(FRONTEND_MESSAGE_MAX);
        source
    });
    let fingerprint = format!("{kind}|{message}|{}", source.as_deref().unwrap_or(""));
    let now = std::time::Instant::now();
    let Some(suppressed) = frontend_error_rate().lock().admit(&fingerprint, now) else {
        return;
    };
    log::error!(
        "{}",
        frontend_error_line(&kind, &message, source.as_deref(), suppressed)
    );
}

/// Whether a newer release is published, and whether now is a safe moment to take it.
///
/// Two answers in one call, and deliberately so. An updater that reports "an update is
/// available" without saying "you have sixteen phones mid-session" invites an operator to
/// take it at the worst possible moment: installing replaces the running binary, and this app
/// holds WDA relays, XCTest runners and device leases that only its own shutdown releases.
///
/// **Never installs.** It reports, and the operator decides. Nor does it run at startup — a
/// farm machine is frequently offline and nobody asked it to phone home, so the network call
/// happens when somebody asks for it and not before.
///
/// Read-only with respect to devices, so no admission gate — but it *reads* the admission
/// state, which is the whole point.
#[tauri::command]
pub async fn update_check(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<UpdateStatus, CommandError> {
    use tauri_plugin_updater::UpdaterExt;

    // Asked before the network call, so a busy fleet is reported even if GitHub is
    // unreachable: "do not update now" is the more urgent half of the answer.
    let busy = state.busy_reason();

    let update = app
        .updater()
        .map_err(|error| format!("không dựng được updater: {error}"))?
        .check()
        .await
        .map_err(|error| format!("không kiểm được bản mới: {error}"))?;

    Ok(match update {
        Some(update) => UpdateStatus {
            available: true,
            version: Some(update.version.clone()),
            current: update.current_version.clone(),
            busy_reason: busy,
        },
        None => UpdateStatus {
            available: false,
            version: None,
            current: app.package_info().version.to_string(),
            busy_reason: busy,
        },
    })
}

/// Download the published update and hand over to its installer.
///
/// Only ever reached by an explicit press, and it re-asks [`AppState::busy_reason`] rather
/// than trusting what the UI was showing: the check and the press are separated by however
/// long the operator took to read it, and a nurture session can start in between.
///
/// **The order is the whole point.** Download first, because a failed download must leave
/// the fleet exactly as it was. Only once the bytes are in hand does the app let go of the
/// phones, and only then does the installer run — on Windows `install` calls
/// `std::process::exit` itself, which skips `RunEvent::Exit` entirely. Waiting for the normal
/// exit path to release anything would leak a WDA relay, an XCTest runner and an adb
/// forward per device, and on a sixteen-phone farm that is sixteen of each.
///
/// **No admission gate, and not merely because it is read-only.** Holding a
/// [`CommandAdmission`](crate::state::AppState::ensure_accepting_work) would deadlock:
/// `graceful_shutdown` waits for in-flight mutating commands to drain, and this command
/// would be one of them, waiting for itself.
#[tauri::command]
pub async fn update_install(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    use tauri_plugin_updater::UpdaterExt;

    if let Some(reason) = state.busy_reason() {
        return Err(reason.into());
    }

    let update = app
        .updater()
        .map_err(|error| format!("không dựng được updater: {error}"))?
        .check()
        .await
        .map_err(|error| format!("không kiểm được bản mới: {error}"))?
        .ok_or_else(|| "không có bản mới để cài".to_string())?;

    let bytes = update
        .download(|_, _| {}, || {})
        .await
        .map_err(|error| format!("không tải được bản mới: {error}"))?;

    // A plain OS thread, not `spawn_blocking`: `graceful_shutdown` blocks on futures
    // through the global runtime, and doing that from inside a runtime-managed thread is
    // the one way to turn an orderly shutdown into a panic during shutdown.
    let handle = app.clone();
    let worker = std::thread::Builder::new()
        .name("riviu-updater-install".into())
        .spawn(move || {
            crate::graceful_shutdown(&handle);
            update.install(bytes)
        })
        .map_err(|error| format!("không tạo được luồng cài đặt: {error}"))?;

    // On Windows this join never returns — the installer exits the process from under it,
    // which is the success path. Elsewhere the archive is unpacked in place and the
    // operator reopens the app, which the fleet is already released for.
    match worker.join() {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(err(format!(
            "cài bản mới thất bại sau khi đã dừng phiên — mở lại app: {error}"
        ))),
        Err(_) => Err(err("luồng cài đặt dừng bất thường — mở lại app")),
    }
}

/// What [`update_check`] found, and whether acting on it is safe right now.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub available: bool,
    /// The published version, when one is newer than this build.
    pub version: Option<String>,
    pub current: String,
    /// Why installing now would interrupt work, or `None` when the fleet is idle.
    ///
    /// The frontend must refuse to offer the install while this is `Some`. Carried as a
    /// sentence rather than a bool so the operator is told *what* is running.
    pub busy_reason: Option<String>,
}

#[cfg(test)]
mod frontend_error_tests {
    use super::{
        frontend_error_line, FrontendErrorRate, FRONTEND_ERROR_QUIET, FRONTEND_ERROR_TRACKED,
    };
    use std::time::Instant;

    /// **The first one speaks, the repeats are counted, and the count is not lost.**
    ///
    /// This is the whole reason the throttle exists: a throw inside a render repeats every
    /// frame, and a log that prints all of them is a log nobody reads. But a log that *drops*
    /// them silently is worse than either -- it under-reports a storm as a hiccup. So the
    /// suppressed ones come back as a number on the next line that gets through.
    #[test]
    fn a_repeating_frontend_error_is_logged_once_then_counted() {
        let mut rate = FrontendErrorRate::default();
        let start = Instant::now();

        assert_eq!(rate.admit("render|boom|App.tsx", start), Some(0));
        assert_eq!(rate.admit("render|boom|App.tsx", start), None);
        assert_eq!(rate.admit("render|boom|App.tsx", start), None);

        let later = start + FRONTEND_ERROR_QUIET;
        assert_eq!(
            rate.admit("render|boom|App.tsx", later),
            Some(2),
            "the two dropped repeats have to be reported, not forgotten"
        );
        assert_eq!(
            rate.admit("render|boom|App.tsx", later),
            None,
            "and the window restarts from the line that just went out"
        );
    }

    /// One noisy error must not silence a different one.
    ///
    /// The throttle is keyed on what the error *is*, so a render loop screaming once a frame
    /// cannot hide the single unhandled rejection that actually explains the failure.
    #[test]
    fn two_different_errors_do_not_throttle_each_other() {
        let mut rate = FrontendErrorRate::default();
        let now = Instant::now();
        assert_eq!(rate.admit("render|boom|App.tsx", now), Some(0));
        assert_eq!(rate.admit("rejection|no such command|api.ts", now), Some(0));
    }

    /// **The tracking map is bounded, because this code runs while things are already wrong.**
    ///
    /// A message carrying a counter or a timestamp is a fresh fingerprint every time. Unbounded,
    /// that is a leak that only appears during an incident -- the worst moment to add one.
    #[test]
    fn the_tracked_set_cannot_grow_without_bound() {
        let mut rate = FrontendErrorRate::default();
        let now = Instant::now();
        for index in 0..(FRONTEND_ERROR_TRACKED * 4) {
            rate.admit(&format!("rejection|attempt {index} failed|api.ts"), now);
        }
        assert!(
            rate.seen.len() <= FRONTEND_ERROR_TRACKED,
            "tracked {} distinct errors, cap is {FRONTEND_ERROR_TRACKED}",
            rate.seen.len()
        );
    }

    /// A never-before-seen error is logged even once the map is full.
    ///
    /// Eviction has to give up *throttling*, not give up *reporting*. Returning `None` when the
    /// map is full would mean a fresh error goes unrecorded precisely because other errors were
    /// noisy first.
    #[test]
    fn a_new_error_is_still_reported_once_the_map_is_full() {
        let mut rate = FrontendErrorRate::default();
        let now = Instant::now();
        for index in 0..(FRONTEND_ERROR_TRACKED * 2) {
            rate.admit(&format!("rejection|filler {index}|api.ts"), now);
        }
        assert_eq!(
            rate.admit("render|the one that matters|Focus.tsx", now),
            Some(0)
        );
    }

    /// The line has to name the kind, the message and the place.
    #[test]
    fn a_frontend_line_carries_the_kind_the_message_and_the_place() {
        let line = frontend_error_line(
            "unhandledrejection",
            "deviceControlBegin is not a function",
            Some("App.tsx:412"),
            0,
        );
        assert!(line.contains("unhandledrejection"));
        assert!(line.contains("deviceControlBegin is not a function"));
        assert!(line.contains("App.tsx:412"));
        assert!(
            !line.contains("suppressed"),
            "a first occurrence must not claim anything was suppressed: {line}"
        );
    }

    /// A suppressed count is stated as a count, not folded into the prose.
    #[test]
    fn a_suppressed_count_is_stated_plainly() {
        let line = frontend_error_line("render", "boom", None, 431);
        assert!(line.contains("431"), "{line}");
        assert!(
            !line.contains("(at "),
            "no source means no empty location clause: {line}"
        );
    }
}
